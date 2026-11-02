//! The placement endpoint asked over HTTP (docs/boundary.md, 「Placement」), a member of the placement type:
//! before its turn a run posts what it is, and runs here unless the answer sends it to another machine.
//! There it runs in a bench copy of the checkout the caller makes ready over `ssh`, its commit pushed into
//! the bench and its open files copied over, and its output and result come back on the run's event stream.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::placement::{PlaceAsk, Placement};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::turn::{Env, Lane, NOT_RUN_EXIT_CODE, stands_at_front, stands_next};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use sse_stream::SseStream;

/// The variable naming the placement endpoint over the declaration's.
pub const PLACEMENT_VARIABLE: &str = "BMO_TESTBED_PLACEMENT";
/// The branch a bench is handed the caller's commit on.
pub const BENCH_BRANCH: &str = "bmo-bench";

// The events a placed run's stream carries.
const OUTPUT_EVENT: &str = "output";
const RESULT_EVENT: &str = "result";

/// The placement endpoint asked over HTTP.
pub struct HttpPlacement {
  pub endpoint: String,
}

impl HttpPlacement {
  /// The endpoint the environment names, or the declaration of the repository a directory stands in; none
  /// where neither names one, or where the declaration does not read.
  pub fn named(env: &Env, dir: &Path) -> Option<Self> {
    let endpoint = env
      .get(PLACEMENT_VARIABLE)
      .filter(|named| !named.is_empty())
      .cloned()
      .or_else(|| Repository::read(dir).ok()?.declaration.runner.placement)?;

    Some(Self { endpoint })
  }
}

// What a run posts to the endpoint.
#[derive(Serialize)]
struct Asked<'a> {
  checkout: String,
  remote: String,
  commit: String,
  command: &'a [String],
  lane: Lane,
  front: bool,
  next: bool,
}

/// Where the caller's copy of its checkout stands on the placed machine.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Bench {
  pub destination: String,
  pub path: String,
  #[serde(default = "Bench::ssh")]
  pub ssh: Vec<String>,
}

impl Bench {
  fn ssh() -> Vec<String> {
    vec!["ssh".to_owned()]
  }
}

// What the endpoint answers.
#[derive(Deserialize)]
#[serde(tag = "place", rename_all = "lowercase")]
enum Answer {
  Local,
  Remote {
    machine: String,
    bench: Bench,
    run: String,
  },
}

// The result a placed run's stream ends on.
#[derive(Deserialize)]
struct Ended {
  line: String,
  code: i64,
}

fn git(dir: &Path, args: &[&str]) -> String {
  Command::new("git")
    .args(args)
    .current_dir(dir)
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output()
    .ok()
    .filter(|done| done.status.success())
    .map(|done| String::from_utf8_lossy(&done.stdout).trim().to_owned())
    .unwrap_or_default()
}

fn quoted(word: &str) -> String {
  shlex::try_quote(word).map_or_else(|_| word.to_owned(), |word| word.into_owned())
}

fn said(output: &std::io::Result<Output>) -> String {
  match output {
    Ok(done) => String::from_utf8_lossy(&done.stderr).trim().to_owned(),
    Err(why) => why.to_string(),
  }
}

fn succeeded(output: &std::io::Result<Output>) -> bool {
  output.as_ref().is_ok_and(|done| done.status.success())
}

/// The bench made ready from a checkout: made where it holds no repository, from the checkout's remote or
/// empty, handed the checkout's commit and checked out at it, and the checkout's open files copied over it.
/// The line saying what failed where a step did.
pub fn ready_bench(bench: &Bench, checkout: &Path, remote: &str) -> Result<(), String> {
  let said_of = &LANES.placed;
  let failed = |line: &str, output: &std::io::Result<Output>| fill_marks(line, &[("said", &said(output))]);
  let Some((program, options)) = bench.ssh.split_first() else {
    return Err(fill_marks(&said_of.failed, &[("said", "")]));
  };
  let ssh_command = bench.ssh.iter().map(|word| quoted(word)).collect::<Vec<_>>().join(" ");
  let ssh = |line: String| {
    Command::new(program)
      .args(options)
      .arg(&bench.destination)
      .arg(line)
      .stdin(Stdio::null())
      .output()
  };
  let path = quoted(&bench.path);
  let make = if remote.is_empty() {
    format!("git init -q {path}")
  } else {
    format!("git clone -q {} {path}", quoted(remote))
  };
  let made = ssh(format!("test -d {} || {make}", quoted(&format!("{}/.git", bench.path))));

  if !succeeded(&made) {
    return Err(failed(&said_of.failed, &made));
  }

  let pushed = Command::new("git")
    .args(["push", "-q", "--force"])
    .arg(format!("{}:{}", bench.destination, bench.path))
    .arg(format!("HEAD:refs/heads/{BENCH_BRANCH}"))
    .current_dir(checkout)
    .env("GIT_SSH_COMMAND", &ssh_command)
    .stdin(Stdio::null())
    .output();

  if !succeeded(&pushed) {
    return Err(failed(&said_of.unpushed, &pushed));
  }

  let checked_out = ssh(format!("cd {path} && git checkout -q --force --detach {BENCH_BRANCH}"));

  if !succeeded(&checked_out) {
    return Err(failed(&said_of.failed, &checked_out));
  }

  let copied = Command::new("rsync")
    .args(["-a", "--delete", "--filter=:- .gitignore", "--exclude", ".git", "-e", &ssh_command])
    .arg(format!("{}/", checkout.display()))
    .arg(format!("{}:{}/", bench.destination, bench.path))
    .stdin(Stdio::null())
    .output();

  if !succeeded(&copied) {
    return Err(failed(&said_of.uncopied, &copied));
  }

  Ok(())
}

// A run the placed machine dropped: the line saying so, and the code of a run not run.
fn dropped(machine: &str, said: &str) -> u8 {
  println!("{}", fill_marks(&LANES.run.dropped, &[("machine", machine), ("said", said)]));
  NOT_RUN_EXIT_CODE
}

impl HttpPlacement {
  // The endpoint's answer to the run, or why there is none that reads.
  async fn asked(&self, client: &reqwest::Client, asked: &Asked<'_>) -> Result<Answer, String> {
    let answered = client
      .post(&self.endpoint)
      .json(asked)
      .send()
      .await
      .and_then(reqwest::Response::error_for_status)
      .map_err(|why| why.to_string())?;

    answered.json::<Answer>().await.map_err(|why| why.to_string())
  }

  // The run read off its stream: its output to standard error, its result line to standard output.
  async fn streamed(client: &reqwest::Client, machine: &str, run: &str) -> u8 {
    let said = &LANES.placed;
    let opened = client.get(run).send().await.and_then(reqwest::Response::error_for_status);
    let response = match opened {
      Ok(response) => response,
      Err(why) => return dropped(machine, &fill_marks(&said.unread, &[("why", &why.to_string())])),
    };
    let mut events = std::pin::pin!(SseStream::from_bytes_stream(response.bytes_stream()));

    while let Some(event) = events.next().await {
      let Ok(event) = event else { break };
      let data = event.data.unwrap_or_default();

      match event.event.as_deref() {
        Some(OUTPUT_EVENT) => eprintln!("{data}"),
        Some(RESULT_EVENT) => match serde_json::from_str::<Ended>(&data) {
          Ok(ended) => {
            println!("{}", ended.line);
            return u8::try_from(ended.code).unwrap_or(1);
          }
          Err(why) => return dropped(machine, &fill_marks(&said.unread, &[("why", &why.to_string())])),
        },
        _ => {}
      }
    }

    dropped(machine, &said.no_result)
  }

  async fn placed(&self, ask: &PlaceAsk<'_>) -> Option<u8> {
    let found = git(&ask.cwd, &["rev-parse", "--show-toplevel"]);
    let checkout = if found.is_empty() { ask.cwd.clone() } else { PathBuf::from(found) };
    let remote = git(&checkout, &["remote", "get-url", "origin"]);
    let asked = Asked {
      checkout: checkout.to_string_lossy().into_owned(),
      remote: remote.clone(),
      commit: git(&checkout, &["rev-parse", "HEAD"]),
      command: ask.command,
      lane: ask.lane,
      front: stands_at_front(ask.env),
      next: stands_next(ask.env),
    };
    let client = reqwest::Client::new();
    let local = |why: &str| {
      eprintln!("{}", fill_marks(&LANES.placed.local, &[("endpoint", &self.endpoint), ("why", why)]));
      None
    };

    match self.asked(&client, &asked).await {
      Ok(Answer::Local) => None,
      Ok(Answer::Remote {
        machine,
        bench,
        run,
      }) => {
        if let Err(said) = ready_bench(&bench, &checkout, &remote) {
          return Some(dropped(&machine, &said));
        }

        eprintln!(
          "{}",
          fill_marks(&LANES.placed.running, &[("machine", &machine), ("bench", &bench.path)])
        );

        Some(Self::streamed(&client, &machine, &run).await)
      }
      Err(why) => local(&why),
    }
  }
}

impl Placement for HttpPlacement {
  fn place(&self, ask: &PlaceAsk<'_>) -> Option<u8> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;

    runtime.block_on(self.placed(ask))
  }
}
