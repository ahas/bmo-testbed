//! The two placements that send a run elsewhere (docs/boundary.md, 「Placement」), members of the placement
//! type: the placed machine the settings name, reached over `ssh`, where the run is the testbed's `placed`
//! verb in a bench copy of the checkout; and the placement endpoint asked over HTTP, whose answer names the
//! bench and the run's event stream. Either way the caller makes the bench ready over `ssh`: the commit
//! pushed into it, the open files and the outside trees copied over, and the output and the result come back.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::placement::{PlaceAsk, Placement};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use bmo_testbed_core::turn::{
  FRONT_VARIABLE, Lane, NEXT_VARIABLE, NOT_RUN_EXIT_CODE, process_env, stands_at_front, stands_next,
};
use bmo_testbed_deploy::release::built_from;
use bmo_testbed_deploy::tree::{BUILT_FROM, short};
use bmo_testbed_suite::crates::checkout_target_name;
use bmo_testbed_suite::quick::shared_versions::{main_shared_text, shared_checkout};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use sse_stream::SseStream;

use crate::words::BINARY;

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

/// The remote a bench on another machine clones: the checkout's `origin`, and where that names a repository on
/// this machine, which reads as nothing there, the `origin` that repository names in turn, followed until one
/// names a repository elsewhere. Empty where the chain ends on this machine or comes back on itself.
pub fn reachable_remote(checkout: &Path) -> String {
  let mut seen: Vec<PathBuf> = std::fs::canonicalize(checkout).into_iter().collect();
  let mut here = checkout.to_path_buf();

  loop {
    let remote = git(&here, &["remote", "get-url", "origin"]);
    let Some(next) = local_repository(&here, &remote) else {
      return remote;
    };

    if seen.contains(&next) {
      return String::new();
    }

    seen.push(next.clone());
    here = next;
  }
}

// The directory on this machine a remote names: a `file` URL's path, or the remote read as a path from the
// repository naming it, as git reads it; none where the remote is empty or names no directory here.
fn local_repository(from: &Path, remote: &str) -> Option<PathBuf> {
  if remote.is_empty() {
    return None;
  }

  let path = url::Url::parse(remote)
    .ok()
    .filter(|url| url.scheme() == "file")
    .and_then(|url| url.to_file_path().ok())
    .unwrap_or_else(|| from.join(remote));

  std::fs::canonicalize(path).ok().filter(|path| path.is_dir())
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

/// A checkout the bench is made from: its root, its reachable remote, the commit the bench is put at, whether
/// its open files go over, the outside trees copied to their own places under the bench root, and the files
/// laid over those copies.
pub struct BenchSource<'a> {
  pub checkout: &'a Path,
  pub remote: &'a str,
  pub commit: &'a str,
  pub open_files: bool,
  /// Each outside tree, and where it stands under the bench root.
  pub trees: Vec<(PathBuf, String)>,
  /// Each file written over a copied tree once every tree is copied: its place there, and its text.
  pub laid: Vec<(String, String)>,
}

// A path on the placed machine as its shell reads it: `~` opening it is that machine's home, left for the
// shell; the rest is quoted.
fn far_path(path: &str) -> String {
  match path.strip_prefix("~/") {
    Some(rest) => format!("~/{}", quoted(rest)),
    None => quoted(path),
  }
}

// The text written to a file on the far machine through one `ssh` command reading it from its input: into a
// file beside it named by the far shell's process, then renamed over it, so a run there reading the file reads
// the whole of one text or the other.
fn laid_over(
  program: &str,
  options: &[String],
  destination: &str,
  file: &str,
  text: &str,
) -> std::io::Result<Output> {
  let mut child = Command::new(program)
    .args(options)
    .arg(destination)
    .arg(format!("cat > {far}.$$ && mv -f {far}.$$ {far}", far = far_path(file)))
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;

  // The input is closed before the wait, which ends `cat`, and the command is waited on whether or not the
  // writing went through.
  let written = child.stdin.take().map_or(Ok(()), |mut input| input.write_all(text.as_bytes()));
  let done = child.wait_with_output();

  written.and(done)
}

// A tree copied over with `rsync`, the files git ignores, the `.git` directory and each path `kept` names left
// out, so the copy neither writes nor deletes them.
fn rsync_over(ssh_command: &str, from: &Path, to: &str, kept: &[String]) -> std::io::Result<Output> {
  let mut command = Command::new("rsync");

  command.args(["-a", "--delete", "--filter=:- .gitignore", "--exclude", ".git", "-e", ssh_command]);

  for path in kept {
    command.arg("--exclude").arg(path);
  }

  command
    .arg(format!("{}/", from.display()))
    .arg(format!("{to}/"))
    .stdin(Stdio::null())
    .output()
}

/// The bench made ready from a checkout: made where it holds no repository, from the checkout's remote, or
/// empty where it has none or the clone fails there, handed the commit and checked out at it, the checkout's
/// open files copied over it where asked, and each outside tree copied to its place. The line saying what
/// failed where a step did.
pub fn ready_bench(bench: &Bench, source: &BenchSource<'_>) -> Result<(), String> {
  let checkout = source.checkout;
  let remote = source.remote;
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
  let path = far_path(&bench.path);
  let empty = format!("git init -q {path}");
  // A remote the machine there cannot read, or asks it for credentials it has none for, leaves the bench
  // empty, and the push carries the history it lacks.
  let make = if remote.is_empty() {
    empty
  } else {
    format!("GIT_TERMINAL_PROMPT=0 git clone -q {} {path} || {empty}", quoted(remote))
  };
  // A folder an earlier run copied there as another checkout's outside tree is made a repository in place; the
  // push then carries what it lacks.
  let made = ssh(format!("test -d {path}/.git || {{ test -d {path} && git init -q {path}; }} || {make}"));

  if !succeeded(&made) {
    return Err(failed(&said_of.failed, &made));
  }

  let pushed = Command::new("git")
    .args(["push", "-q", "--force"])
    .arg(format!("{}:{}", bench.destination, bench.path))
    .arg(format!("{}:refs/heads/{BENCH_BRANCH}", source.commit))
    .current_dir(checkout)
    .env("GIT_SSH_COMMAND", &ssh_command)
    .stdin(Stdio::null())
    .output();

  if !succeeded(&pushed) {
    return Err(failed(&said_of.unpushed, &pushed));
  }

  // A bench for a deployment is the commit alone: whatever an earlier run's open files left standing goes.
  let clean = if source.open_files { "" } else { " && git clean -qfd" };
  let checked_out = ssh(format!("cd {path} && git checkout -q --force --detach {BENCH_BRANCH}{clean}"));

  if !succeeded(&checked_out) {
    return Err(failed(&said_of.failed, &checked_out));
  }

  if source.open_files {
    let copied = rsync_over(&ssh_command, checkout, &format!("{}:{}", bench.destination, bench.path), &[]);

    if !succeeded(&copied) {
      return Err(failed(&said_of.uncopied, &copied));
    }
  }

  for (tree, place) in &source.trees {
    let made = ssh(format!("mkdir -p {}", far_path(place)));

    if !succeeded(&made) {
      return Err(failed(&said_of.failed, &made));
    }

    // A file laid over this tree is left out of its copy, with the one its laying writes beside it, so neither
    // this copy nor another run's copying the same tree there puts the tree's own file back over it.
    let kept: Vec<String> = source
      .laid
      .iter()
      .filter_map(|(file, _)| file.strip_prefix(&format!("{place}/")))
      .map(|relative| format!("/{relative}*"))
      .collect();
    let copied = rsync_over(&ssh_command, tree, &format!("{}:{}", bench.destination, place), &kept);

    if !succeeded(&copied) {
      return Err(fill_marks(
        &said_of.tree_uncopied,
        &[("tree", &tree.to_string_lossy()), ("said", &said(&copied))],
      ));
    }
  }

  for (file, text) in &source.laid {
    let laid = laid_over(program, options, &bench.destination, file, text);

    if !succeeded(&laid) {
      return Err(fill_marks(&said_of.file_unlaid, &[("file", file), ("said", &said(&laid))]));
    }
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
    let remote = reachable_remote(&checkout);
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
        let source = BenchSource {
          checkout: &checkout,
          remote: &remote,
          commit: &asked.commit,
          open_files: true,
          trees: Vec::new(),
          laid: Vec::new(),
        };

        if let Err(said) = ready_bench(&bench, &source) {
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

/// The placed machine the settings name (docs/boundary.md, 「The placed machine」), read from
/// `<data root>/env/placement.toml`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MachineSettings {
  pub destination: String,
  #[serde(default)]
  pub machine: Option<String>,
  #[serde(default = "MachineSettings::bench")]
  pub bench: String,
  #[serde(default = "Bench::ssh")]
  pub ssh: Vec<String>,
  #[serde(default = "MachineSettings::testbed")]
  pub testbed: String,
  #[serde(default = "MachineSettings::cargo_target")]
  pub cargo_target: String,
}

/// The settings file's name under the data root's `env/`.
pub const PLACEMENT_SETTINGS: &str = "placement.toml";

impl MachineSettings {
  fn bench() -> String {
    "~/.local/share/bmo/bench".to_owned()
  }

  fn testbed() -> String {
    "~/.local/share/bmo-testbed/deploy/bin/bmo-testbed".to_owned()
  }

  fn cargo_target() -> String {
    "~/.local/share/bmo/cargo-target".to_owned()
  }

  /// The settings under a data root: none where no file stands there, the reason where one does not read.
  pub fn read(data_root: &Path) -> Option<Result<Self, String>> {
    let file = data_root.join("env").join(PLACEMENT_SETTINGS);
    let text = std::fs::read_to_string(&file).ok()?;

    Some(toml::from_str::<Self>(&text).map_err(|why| {
      fill_marks(&LANES.placed.unsettled, &[("file", &file.to_string_lossy()), ("why", &why.to_string())])
    }))
  }

  /// The label the lines name the machine by.
  pub fn label(&self) -> &str {
    self.machine.as_deref().unwrap_or(&self.destination)
  }

  /// Where a tree of this machine stands under the bench root: at its path under this machine's home, or at
  /// its whole path where it stands outside it.
  pub fn place_of(&self, tree: &Path, home: &Path) -> String {
    let relative = tree.strip_prefix(home).unwrap_or(tree);

    format!(
      "{}/{}",
      self.bench.trim_end_matches('/'),
      relative.to_string_lossy().trim_start_matches('/')
    )
  }

  fn as_bench(&self, path: String) -> Bench {
    Bench {
      destination: self.destination.clone(),
      path,
      ssh: self.ssh.clone(),
    }
  }
}

/// A bench made ready: its path on the placed machine, and the checkouts a run there knows, the bench and
/// every tree copied beside it.
pub struct ReadyBench {
  pub path: String,
  pub checkouts: Vec<String>,
}

/// The placed machine, a member of the placement type: every run goes there over `ssh`, as the testbed's
/// `placed` verb in a bench copy of the checkout.
pub struct MachinePlacement {
  pub settings: MachineSettings,
  pub home: PathBuf,
  /// The commit the running testbed's release was built from, which the testbed there must be built from
  /// too; none where this one runs from no release, such as a checkout's own build, and then nothing is
  /// compared.
  pub release: Option<String>,
}

/// The commit the running binary's release was built from: the `built-from` beside it in the release's
/// `bin/`. None where the binary stands in no release.
pub fn own_release() -> Option<String> {
  let binary = std::env::current_exe().ok()?;

  built_from(binary.parent()?.parent()?)
}

/// What a run on the placed machine came to: it ran there, with its code; or it did not go, said on a line,
/// and the run is the caller's: the machine unreached or its testbed another release than this one's, or, for
/// a run that goes only to a free seat, every seat there held or waited for.
pub enum Placed {
  Ran(u8),
  Here,
  Full,
}

impl MachinePlacement {
  /// The placement under a data root for an account's home: none where the settings name no machine or do
  /// not read, the reason said on a line.
  pub fn named(data_root: &Path, home: &Path) -> Option<Self> {
    match MachineSettings::read(data_root)? {
      Ok(settings) => Some(Self {
        settings,
        home: home.to_path_buf(),
        release: own_release(),
      }),
      Err(why) => {
        eprintln!("{why}");
        None
      }
    }
  }

  fn ssh(&self) -> Command {
    let (program, options) = self.settings.ssh.split_first().map_or(("ssh", &[][..]), |(p, o)| (p, o));
    let mut command = Command::new(program);

    command.args(options).arg(&self.settings.destination).stdin(Stdio::null());
    command
  }

  /// Whether a seat of the wide lane there is free now with nobody waiting for one, as that machine's
  /// `status --free` answers; the `full` line where it is not, or where the answer does not come.
  pub fn seat_free(&self) -> bool {
    let asked = self.ssh().arg(format!("{} status --free", self.settings.testbed)).output();

    if succeeded(&asked) {
      return true;
    }

    eprintln!("{}", fill_marks(&LANES.placed.full, &[("machine", self.settings.label())]));

    false
  }

  /// Whether the machine answers; what `ssh` said where it does not, on the `unreached` line.
  pub fn reached(&self) -> bool {
    let probed = self.ssh().arg("true").output();

    if succeeded(&probed) {
      return true;
    }

    eprintln!(
      "{}",
      fill_marks(&LANES.placed.unreached, &[("machine", self.settings.label()), ("said", &said(&probed))])
    );

    false
  }

  /// Whether the testbed there is this one's release (docs/boundary.md, 「The placed machine」): the commit
  /// the `built-from` beside its binary names, compared with this one's own; the line naming both where they
  /// differ or none reads there. Nothing is compared where this one runs from no release.
  pub fn same_release(&self) -> Result<(), String> {
    let Some(own) = &self.release else { return Ok(()) };
    let words = &LANES.placed;
    let machine = self.settings.label();
    let file = Path::new(&self.settings.testbed).with_file_name(BUILT_FROM);
    let file = file.to_string_lossy();
    let read = self.ssh().arg(format!("cat {}", far_path(&file))).output();
    let far = read
      .as_ref()
      .ok()
      .filter(|done| done.status.success())
      .map(|done| String::from_utf8_lossy(&done.stdout).trim().to_owned())
      .filter(|commit| !commit.is_empty());

    match far {
      Some(far) if far == *own => Ok(()),
      Some(far) => Err(fill_marks(
        &words.stale,
        &[("machine", machine), ("far", &short(&far)), ("own", &short(own))],
      )),
      None => Err(fill_marks(
        &words.unreleased,
        &[("machine", machine), ("file", &file), ("said", &said(&read)), ("own", &short(own))],
      )),
    }
  }

  /// The bench of a checkout made ready at a commit, its outside trees with it, and the checkout holding the
  /// shared versions the declaration names, found through this machine's registry; the bench's path there,
  /// and the far places of every tree copied, which are that run's checkouts there. Nothing is made ready
  /// where the testbed there is not this one's release.
  pub fn ready(&self, checkout: &Path, commit: &str, open_files: bool) -> Result<ReadyBench, String> {
    self.same_release()?;
    self.made_ready(checkout, commit, open_files)
  }

  // The bench made ready, the release there taken as already compared.
  fn made_ready(&self, checkout: &Path, commit: &str, open_files: bool) -> Result<ReadyBench, String> {
    let remote = reachable_remote(checkout);
    let mut trees = bmo_testbed_deploy::expansion::outside_roots(checkout, checkout)?;
    // The shared versions' file as the shared checkout's main holds it, laid over that checkout's copy, since
    // the copy is its working tree and the run there reads the copy as it stands.
    let mut laid = Vec::new();

    if let Some(shared) =
      Repository::read(checkout).ok().and_then(|repo| repo.declaration.check.shared_versions)
    {
      let there = shared_checkout(&shared.repository, &process_env())?;
      let file = &QUICK_CHECKS.shared_versions.file;

      laid.push((
        format!("{}/{file}", self.settings.place_of(&there.root, &self.home)),
        main_shared_text(&there)?,
      ));
      trees.push(there.root);
    }

    trees.sort();
    trees.dedup();

    let trees: Vec<(PathBuf, String)> = trees
      .into_iter()
      .map(|tree| (self.settings.place_of(&tree, &self.home).clone(), tree))
      .map(|(place, tree)| (tree, place))
      .collect();
    let path = self.settings.place_of(checkout, &self.home);
    let checkouts = std::iter::once(path.clone())
      .chain(trees.iter().map(|(_, place)| place.clone()))
      .collect();
    let source = BenchSource {
      checkout,
      remote: &remote,
      commit,
      open_files,
      trees,
      laid,
    };

    ready_bench(&self.settings.as_bench(path.clone()), &source)?;
    Ok(ReadyBench { path, checkouts })
  }

  // The one `ssh` command running the words in the bench through the `placed` verb.
  fn placed_line(&self, bench: &ReadyBench, words: &[String], setup: bool) -> String {
    let env = process_env();
    // The caller's place in the line goes with it, so a deployment stands at the front there as it does here.
    let place = [(FRONT_VARIABLE, stands_at_front(&env)), (NEXT_VARIABLE, stands_next(&env))]
      .into_iter()
      .filter(|(_, stands)| *stands)
      .map(|(name, _)| format!("{name}=1 "))
      .collect::<String>();
    let mut line =
      format!("cd {} && {place}{} placed", far_path(&bench.path), far_path(&self.settings.testbed));

    if setup {
      line.push_str(" --setup");
    }

    for checkout in &bench.checkouts {
      line.push_str(" --checkout ");
      line.push_str(&far_path(checkout));
    }

    line.push_str(" --");

    // The testbed binary there is named from that machine's home, which its shell resolves.
    for word in words {
      line.push(' ');
      line.push_str(&if word == &self.settings.testbed { far_path(word) } else { quoted(word) });
    }

    line
  }

  /// The words run in the bench through the `placed` verb, the output and the result this process's; the
  /// code it ended with, 75 where the connection broke.
  pub fn run(&self, bench: &ReadyBench, words: &[String], setup: bool) -> u8 {
    let said = &LANES.placed;
    let line = self.placed_line(bench, words, setup);

    eprintln!(
      "{}",
      fill_marks(&said.running, &[("machine", self.settings.label()), ("bench", &bench.path)])
    );

    let status = self.ssh().arg(line).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status();

    match status.ok().and_then(|status| status.code()) {
      // 255 is `ssh`'s own: the connection broke or was never made.
      Some(255) | None => dropped(self.settings.label(), &said.broken),
      Some(code) => u8::try_from(code).unwrap_or(1),
    }
  }

  /// The words run in the bench through the `placed` verb with no setup, what they say on standard error this
  /// process's; what they printed on standard output where they ended 0, none otherwise.
  pub fn read(&self, bench: &ReadyBench, words: &[String]) -> Option<String> {
    let done = self
      .ssh()
      .arg(self.placed_line(bench, words, false))
      .stdout(Stdio::piped())
      .stderr(Stdio::inherit())
      .output()
      .ok()?;

    done.status.success().then(|| String::from_utf8_lossy(&done.stdout).into_owned())
  }

  /// The binaries read back into a directory here from the bench's own directory under the machine's shared
  /// Cargo build directory, the one that machine's Cargo verb builds the bench in.
  pub fn fetch_binaries(
    &self,
    bench: &ReadyBench,
    profile: &str,
    binaries: &[String],
    into: &Path,
  ) -> Result<(), String> {
    let words = &LANES.placed;
    let ssh_command = self.settings.ssh.iter().map(|word| quoted(word)).collect::<Vec<_>>().join(" ");
    let unfetched =
      |what: &str| fill_marks(&words.unfetched, &[("machine", self.settings.label()), ("said", what)]);
    // The bench's whole path there, which that machine's testbed names its build directory by.
    let there = self.ssh().arg(format!("cd {} && pwd", far_path(&bench.path))).output();

    if !succeeded(&there) {
      return Err(unfetched(&said(&there)));
    }

    let whole = there
      .map(|done| String::from_utf8_lossy(&done.stdout).trim().to_owned())
      .unwrap_or_default();
    let built = format!(
      "{}/checkouts/{}/{profile}",
      self.settings.cargo_target.trim_end_matches('/'),
      checkout_target_name(Path::new(&whole))
    );

    std::fs::create_dir_all(into).map_err(|why| unfetched(&why.to_string()))?;

    for binary in binaries {
      let fetched = Command::new("rsync")
        .args(["-a", "-e", &ssh_command])
        .arg(format!("{}:{built}/{binary}", self.settings.destination))
        .arg(format!("{}/", into.display()))
        .stdin(Stdio::null())
        .output();

      if !succeeded(&fetched) {
        return Err(unfetched(&said(&fetched)));
      }
    }

    Ok(())
  }

  /// A run placed on the machine: the bench made ready at the checkout's `HEAD` with its open files, and
  /// the words run there as the testbed's own `run`.
  fn placed(&self, ask: &PlaceAsk<'_>) -> Placed {
    if !self.reached() {
      return Placed::Here;
    }

    // A testbed there of another release would meet the same command the same way until it is brought to this
    // release, so the run is this machine's.
    if let Err(said) = self.same_release() {
      let machine = self.settings.label();

      eprintln!("{}", fill_marks(&LANES.placed.other_release, &[("machine", machine), ("said", &said)]));

      return Placed::Here;
    }

    if ask.to_free_seat && !self.seat_free() {
      return Placed::Full;
    }

    let found = git(&ask.cwd, &["rev-parse", "--show-toplevel"]);
    let checkout = if found.is_empty() { ask.cwd.clone() } else { PathBuf::from(found) };
    let commit = git(&checkout, &["rev-parse", "HEAD"]);
    let bench = match self.made_ready(&checkout, &commit, true) {
      Ok(bench) => bench,
      Err(said) => return Placed::Ran(dropped(self.settings.label(), &said)),
    };
    let mut words = vec![self.settings.testbed.clone()];

    // A verb of this binary, the suite's, runs as the same verb of the testbed there; anything else as a
    // run of its own there.
    if ask.command.first().is_some_and(|first| first == BINARY) {
      words.extend(ask.command.iter().skip(1).cloned());
    } else {
      words.push("run".to_owned());

      if ask.lane == Lane::Narrow {
        words.push("--narrow".to_owned());
      }

      if let Some(adapter) = ask.adapter {
        words.extend(["--adapter".to_owned(), adapter.to_owned()]);
      }

      words.push("--".to_owned());
      words.extend(ask.command.iter().cloned());
    }

    Placed::Ran(self.run(&bench, &words, true))
  }
}

impl Placement for MachinePlacement {
  fn place(&self, ask: &PlaceAsk<'_>) -> Option<u8> {
    match self.placed(ask) {
      Placed::Ran(code) => Some(code),
      Placed::Here | Placed::Full => None,
    }
  }
}
