//! The two placements that send a run elsewhere (docs/boundary.md, 「Placement」), members of the placement
//! type: the placed machine the settings name, reached over `ssh`, where the run is the testbed's `placed`
//! verb in a bench copy of the checkout; and the placement endpoint asked over HTTP, whose answer names the
//! bench and the run's event stream. Either way the caller makes the bench ready over `ssh`: the commit
//! pushed into it, the open files and the outside trees copied over, and the output and the result come back.
//! While the run lasts it stands here as a placed run of its lane, running once the far side says its turn
//! there came (docs/boundary.md, 「A run placed elsewhere stands here too」).

use std::io::{BufRead as _, BufReader, Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::Hook;
use bmo_testbed_core::placement::{PlaceAsk, PlacedRun, Placement, SetAsk, lane_full};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use bmo_testbed_core::silence::{Heard, Silence, pass_on, said_while_quiet};
use bmo_testbed_core::turn::{
  FRONT_VARIABLE, Lane, NEXT_VARIABLE, NOT_RUN_EXIT_CODE, process_env, stands_at_front, stands_next,
};
use bmo_testbed_deploy::expansion::{committed_tree, outside_roots, outside_roots_at};
use bmo_testbed_suite::crates::checkout_target_name;
use bmo_testbed_suite::quick::shared_versions::{main_shared_text, shared_checkout};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use sse_stream::SseStream;

use crate::words::BINARY;

mod far_release;
pub mod git_ignored;
mod machine_hook;
mod outage;

pub use far_release::{FarWait, LiveRelease, MachineState, PLACED_TESTBED_LOCK, running_deploy_tree};
pub use machine_hook::{MACHINE_ENV_FILE, machine_hook, notice_hook};
pub use outage::{OUTAGE_DIR, Outage, RELEASE_FAILED_DIR, ReleaseFailure};

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
  directory: String,
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

/// Where a directory stands in its checkout: its path from the checkout's top with `/` between its parts, empty
/// at the top or outside any checkout. A run started there runs in the same folder of a bench copy.
fn place_in_checkout(dir: &Path) -> String {
  git(dir, &["rev-parse", "--show-prefix"]).trim_end_matches('/').to_owned()
}

/// Whether a run stays here for the directory it was started in: one the bench copy leaves out, a folder git
/// ignores in its checkout or one inside it, which the bench would not hold; said on a line naming it. A
/// directory in no checkout is not kept for this: its copy says what fails there.
fn kept_outside_copy(ask: &PlaceAsk<'_>) -> bool {
  let top = git(&ask.cwd, &["rev-parse", "--show-toplevel"]);
  let ignored = !top.is_empty()
    && git_ignored::ignored_in(Path::new(&top))
      .is_ok_and(|ignored| git_ignored::lies_in_ignored(&place_in_checkout(&ask.cwd), &ignored));

  if ignored {
    eprintln!("{}", fill_marks(&LANES.placed.ignored_dir, &[("dir", &ask.cwd.to_string_lossy())]));
  }

  ignored
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
/// its open files go over, the outside trees laid at their own places under the bench root, and the files
/// laid over those trees.
pub struct BenchSource<'a> {
  pub checkout: &'a Path,
  pub remote: &'a str,
  pub commit: &'a str,
  pub open_files: bool,
  /// Each outside tree, as it is laid under the bench root.
  pub trees: Vec<BenchTree>,
  /// Each file written over a laid tree once every tree is laid: its place there, and its text.
  pub laid: Vec<(String, String)>,
}

/// A tree laid beside the bench at its place under the bench root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BenchTree {
  /// A tree copied as it stands in its checkout, open files and all: what a run reads.
  AsItStands { tree: PathBuf, place: String },
  /// A repository put at a commit the way the bench is, its untracked files taken away: what a deployment
  /// reads, since it builds committed trees only (docs/deploy.md, 「The releases」).
  Committed {
    repository: PathBuf,
    remote: String,
    commit: String,
    place: String,
  },
}

impl BenchTree {
  /// Where the tree stands under the bench root.
  pub fn place(&self) -> &str {
    match self {
      Self::AsItStands { place, .. } | Self::Committed { place, .. } => place,
    }
  }
}

// A path on the placed machine as its shell reads it: `~` opening it is that machine's home, left for the
// shell; the rest is quoted.
fn far_path(path: &str) -> String {
  match path.strip_prefix("~/") {
    Some(rest) => format!("~/{}", quoted(rest)),
    None => quoted(path),
  }
}

// The bytes written to a file on the far machine through one `ssh` command reading them from its input: into a
// file beside it named by the far shell's process, given the octal `mode` where one is named, then renamed over
// it, so a run there reading the file reads the whole of one content or the other, and a process running the
// file there goes on with the one it started.
fn laid_over(
  program: &str,
  options: &[String],
  destination: &str,
  file: &str,
  bytes: &[u8],
  mode: Option<&str>,
) -> std::io::Result<Output> {
  let far = far_path(file);
  let moded = mode.map_or_else(String::new, |mode| format!("chmod {mode} {far}.$$ && "));
  let mut child = Command::new(program)
    .args(options)
    .arg(destination)
    .arg(format!("cat > {far}.$$ && {moded}mv -f {far}.$$ {far}"))
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;

  // The input is closed before the wait, which ends `cat`, and the command is waited on whether or not the
  // writing went through.
  let written = child.stdin.take().map_or(Ok(()), |mut input| input.write_all(bytes));
  let done = child.wait_with_output();

  written.and(done)
}

// A tree copied over with `rsync`, the `.git` directory, each path git ignores in it here and each pattern
// `kept` names left out, so the copy neither writes nor deletes them, and each path of `protected`, those the
// far side's own git ignores there, left standing there; what git said where it cannot name what it ignores
// here, or `rsync` where it failed. The rules go to `rsync` on its input as one merged rule file, read before
// it starts the far side.
fn rsync_over(
  ssh_command: &str,
  from: &Path,
  to: &str,
  kept: &[String],
  protected: &[Vec<u8>],
) -> Result<(), String> {
  let left_out = git_ignored::ignored_in(from)?;
  let rule = |kind: &[u8], pattern: &[u8]| [kind, pattern, b"\0"].concat();
  let rules: Vec<u8> = left_out
    .iter()
    .map(|path| rule(b"- ", &git_ignored::rsync_pattern(path)))
    .chain(kept.iter().map(|pattern| rule(b"- ", pattern.as_bytes())))
    .chain(protected.iter().map(|path| rule(b"P ", &git_ignored::rsync_pattern(path))))
    .collect::<Vec<Vec<u8>>>()
    .concat();

  let mut child = Command::new("rsync")
    .args(["-a", "--delete", "--exclude", ".git", "--from0", "--filter=. -", "-e", ssh_command])
    .arg(format!("{}/", from.display()))
    .arg(format!("{to}/"))
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|why| why.to_string())?;
  // The input is closed before the wait, which ends `rsync`'s reading of the rules, and the command is waited
  // on whether or not the writing went through.
  let written = child.stdin.take().map_or(Ok(()), |mut input| input.write_all(&rules));
  let done = written.and(child.wait_with_output());

  if succeeded(&done) { Ok(()) } else { Err(said(&done)) }
}

/// A repository put at a commit there: where its place holds none, cloned from its remote, or made empty where
/// it has none or the clone fails there, handed the commit and checked out at it detached, its untracked
/// files taken away where asked.
struct AtCommit<'a> {
  repository: &'a Path,
  remote: &'a str,
  commit: &'a str,
  place: &'a str,
  clean: bool,
}

// The repository put at its commit at its place there, through `ssh`; the line saying what failed where a
// step did.
fn put_at_commit(
  bench: &Bench,
  ssh: &dyn Fn(String) -> std::io::Result<Output>,
  ssh_command: &str,
  at: &AtCommit<'_>,
) -> Result<(), String> {
  let said_of = &LANES.placed;
  let failed = |line: &str, output: &std::io::Result<Output>| fill_marks(line, &[("said", &said(output))]);
  let path = far_path(at.place);
  let empty = format!("git init -q {path}");
  // A remote the machine there cannot read, or asks it for credentials it has none for, leaves the bench
  // empty, and the push carries the history it lacks.
  let make = if at.remote.is_empty() {
    empty
  } else {
    format!("GIT_TERMINAL_PROMPT=0 git clone -q {} {path} || {empty}", quoted(at.remote))
  };
  // A folder an earlier run copied there as another checkout's outside tree is made a repository in place; the
  // push then carries what it lacks.
  let made = ssh(format!("test -d {path}/.git || {{ test -d {path} && git init -q {path}; }} || {make}"));

  if !succeeded(&made) {
    return Err(failed(&said_of.failed, &made));
  }

  let pushed = Command::new("git")
    .args(["push", "-q", "--force"])
    .arg(format!("{}:{}", bench.destination, at.place))
    .arg(format!("{}:refs/heads/{BENCH_BRANCH}", at.commit))
    .current_dir(at.repository)
    .env("GIT_SSH_COMMAND", ssh_command)
    .stdin(Stdio::null())
    .output();

  if !succeeded(&pushed) {
    return Err(failed(&said_of.unpushed, &pushed));
  }

  let clean = if at.clean { " && git clean -qfd" } else { "" };
  let checked_out = ssh(format!("cd {path} && git checkout -q --force --detach {BENCH_BRANCH}{clean}"));

  if !succeeded(&checked_out) {
    return Err(failed(&said_of.failed, &checked_out));
  }

  Ok(())
}

/// The bench made ready from a checkout: put at the commit the way any repository there is, the checkout's
/// open files copied over it where asked, and each outside tree laid at its place. The line saying what
/// failed where a step did.
pub fn ready_bench(bench: &Bench, source: &BenchSource<'_>) -> Result<(), String> {
  let checkout = source.checkout;
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

  // A bench for a deployment is the commit alone: whatever an earlier run's open files left standing goes.
  put_at_commit(
    bench,
    &ssh,
    &ssh_command,
    &AtCommit {
      repository: checkout,
      remote: source.remote,
      commit: source.commit,
      place: &bench.path,
      clean: !source.open_files,
    },
  )?;

  if source.open_files {
    // What the bench's own git ignores there stays, an installed package or a build's output among them,
    // though this checkout holds none of it.
    let listing = git_ignored::LISTING.map(quoted).join(" ");
    let listed = ssh(format!("cd {} && {listing}", far_path(&bench.path)));

    if !succeeded(&listed) {
      return Err(failed(&said_of.uncopied, &listed));
    }

    let protected = listed.map(|done| git_ignored::listed(&done.stdout)).unwrap_or_default();

    rsync_over(
      &ssh_command,
      checkout,
      &format!("{}:{}", bench.destination, bench.path),
      &[],
      &protected,
    )
    .map_err(|said| fill_marks(&said_of.uncopied, &[("said", &said)]))?;
  }

  for laid_tree in &source.trees {
    let (tree, place) = match laid_tree {
      BenchTree::AsItStands { tree, place } => (tree, place),
      BenchTree::Committed {
        repository,
        remote,
        commit,
        place,
      } => {
        put_at_commit(
          bench,
          &ssh,
          &ssh_command,
          &AtCommit {
            repository,
            remote,
            commit,
            place,
            clean: true,
          },
        )?;
        continue;
      }
    };
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
    // The copy of a tree there holds no repository, so what it keeps is what git ignores in the tree here.
    rsync_over(&ssh_command, tree, &format!("{}:{}", bench.destination, place), &kept, &[]).map_err(
      |said| fill_marks(&said_of.tree_uncopied, &[("tree", &tree.to_string_lossy()), ("said", &said)]),
    )?;
  }

  for (file, text) in &source.laid {
    let laid = laid_over(program, options, &bench.destination, file, text.as_bytes(), None);

    if !succeeded(&laid) {
      return Err(fill_marks(&said_of.file_unlaid, &[("file", file), ("said", &said(&laid))]));
    }
  }

  Ok(())
}

// Whether a line of the far run's output is the one saying its turn there came.
fn says_turn_came(line: &[u8]) -> bool {
  let line = line.strip_suffix(b"\n").unwrap_or(line);

  line.strip_suffix(b"\r").unwrap_or(line) == LANES.placed.turn_came.as_bytes()
}

// The far run's standard error passed on to this process's as it comes, line by line, but for the line saying
// its turn there came, which marks the placed run running here, where one stands, and goes no further.
fn passed_on(from: impl Read, placed: Option<&PlacedRun>) {
  let mut from = BufReader::new(from);
  let mut line = Vec::new();
  let mut to = std::io::stderr();

  while from.read_until(b'\n', &mut line).is_ok_and(|read| read > 0) {
    if says_turn_came(&line) {
      if let Some(placed) = placed {
        placed.turn_came();
      }
    } else {
      let _ = to.write_all(&line).and_then(|()| to.flush());
    }

    line.clear();
  }
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
  async fn streamed(
    client: &reqwest::Client,
    machine: &str,
    run: &str,
    placed: &PlacedRun,
    heard: &AtomicU64,
  ) -> u8 {
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

      heard.fetch_add(1, Ordering::Relaxed);

      match event.event.as_deref() {
        Some(OUTPUT_EVENT) if says_turn_came(data.as_bytes()) => placed.turn_came(),
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

  async fn placed(&self, ask: &PlaceAsk<'_>, heard: &AtomicU64) -> Option<u8> {
    let found = git(&ask.cwd, &["rev-parse", "--show-toplevel"]);
    let checkout = if found.is_empty() { ask.cwd.clone() } else { PathBuf::from(found) };
    let remote = reachable_remote(&checkout);
    let asked = Asked {
      checkout: checkout.to_string_lossy().into_owned(),
      remote: remote.clone(),
      commit: git(&checkout, &["rev-parse", "HEAD"]),
      command: ask.command,
      directory: place_in_checkout(&ask.cwd),
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
        let placed = PlacedRun::stand(ask, &checkout, &machine);
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

        Some(Self::streamed(&client, &machine, &run, &placed, heard).await)
      }
      Err(why) => local(&why),
    }
  }
}

impl Placement for HttpPlacement {
  fn place(&self, ask: &PlaceAsk<'_>) -> Option<u8> {
    if kept_outside_copy(ask) {
      return None;
    }

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    let heard = AtomicU64::new(0);

    // The endpoint names the machine only once it answers, so the line names the endpoint.
    standing(&self.endpoint, &heard, || runtime.block_on(self.placed(ask, &heard)))
  }
}

/// The placed machine the settings name (docs/boundary.md, 「The placed machine」), read from
/// `<data root>/env/placement.toml`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MachineSettings {
  pub destination: String,
  #[serde(default)]
  pub machine: Option<String>,
  #[serde(default = "MachineSettings::bench")]
  pub bench: String,
  /// The deployments' bench root there, apart from the runs' (docs/boundary.md, 「A deployment's bench stands
  /// apart」).
  #[serde(default = "MachineSettings::deploy_bench")]
  pub deploy_bench: String,
  #[serde(default = "Bench::ssh")]
  pub ssh: Vec<String>,
  #[serde(default = "MachineSettings::testbed")]
  pub testbed: String,
  #[serde(default = "MachineSettings::cargo_target")]
  pub cargo_target: String,
  /// The free space in gigabytes below which a disk there sends `placed-disk-low`; the prune policy's where
  /// none is named.
  #[serde(default)]
  pub disk_free_gigabytes: Option<f64>,
  /// The paths there whose disks' free space is read beside the build directory's.
  #[serde(default)]
  pub disk_paths: Vec<String>,
  /// The machines a run that can be placed goes to, first choice first, by name; the placed machine, then this
  /// one, where none is named (docs/boundary.md, 「Placement」).
  #[serde(default)]
  pub order: Option<Vec<String>>,
  /// The machines a change check over several checkouts goes to, first choice first, by name; the placed
  /// machine alone where none is named (docs/boundary.md, 「Placement」).
  #[serde(default)]
  pub combined_order: Option<Vec<String>>,
}

/// A machine of the placement order: this one, or the placed machine the settings name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ordered {
  Here,
  Placed,
}

/// The filesystem holding a disk path of the placed machine: its source, which for a WSL drive mount is the
/// drive itself, and its free space in bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FarDisk {
  pub source: String,
  pub free_bytes: u64,
}

// `findmnt`'s JSON answer for one target: the filesystems holding it, the deepest mount first.
#[derive(Deserialize)]
struct Mounts {
  filesystems: Vec<Mount>,
}

#[derive(Deserialize)]
struct Mount {
  source: String,
  avail: MountBytes,
}

// A size in `findmnt`'s JSON: a number, or a string of digits where its version writes every column as text.
#[derive(Deserialize)]
#[serde(untagged)]
enum MountBytes {
  Number(u64),
  Text(String),
}

impl Mount {
  fn disk(self) -> Option<FarDisk> {
    let free_bytes = match self.avail {
      MountBytes::Number(bytes) => bytes,
      MountBytes::Text(text) => text.parse().ok()?,
    };

    Some(FarDisk {
      source: self.source,
      free_bytes,
    })
  }
}

/// Where the placed machine keeps the user units laid on it, under its home.
pub const FAR_UNIT_DIR: &str = "~/.config/systemd/user";

/// The settings file's name under the data root's `env/`.
pub const PLACEMENT_SETTINGS: &str = "placement.toml";

impl MachineSettings {
  fn bench() -> String {
    "~/.local/share/bmo/bench".to_owned()
  }

  fn deploy_bench() -> String {
    "~/.local/share/bmo/deploy-bench".to_owned()
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

    let unsettled =
      |why: &str| fill_marks(&LANES.placed.unsettled, &[("file", &file.to_string_lossy()), ("why", why)]);

    Some(
      toml::from_str::<Self>(&text)
        .map_err(|why| unsettled(&why.to_string()))
        .and_then(|settings| match settings.disk_free_gigabytes {
          Some(floor) if !floor.is_finite() || floor < 0.0 => Err(unsettled(&LANES.placed.negative_floor)),
          _ => Ok(settings),
        })
        .and_then(|settings| match settings.order().and(settings.combined_order()) {
          Ok(_) => Ok(settings),
          Err(why) => Err(unsettled(&why)),
        }),
    )
  }

  /// The placement order, first choice first: the machines the settings' `order` names, each by the placed
  /// machine's label or this machine's name, or the placed machine then this one where it names none. Why it
  /// does not read where it is empty, names a machine twice or names one that is neither.
  pub fn order(&self) -> Result<Vec<Ordered>, String> {
    match &self.order {
      Some(names) => self.ordered(names),
      None => Ok(vec![Ordered::Placed, Ordered::Here]),
    }
  }

  /// The order a change check over several checkouts walks, first choice first: the machines the settings'
  /// `combined_order` names, read as `order` is, or the placed machine alone where it names none, so the set
  /// never falls to this machine.
  pub fn combined_order(&self) -> Result<Vec<Ordered>, String> {
    match &self.combined_order {
      Some(names) => self.ordered(names),
      None => Ok(vec![Ordered::Placed]),
    }
  }

  // The machines named, each by the placed machine's label or this machine's name; why the names do not read
  // where they are none, name a machine twice or name one that is neither.
  fn ordered(&self, names: &[String]) -> Result<Vec<Ordered>, String> {
    let words = &LANES.placed;

    if names.is_empty() {
      return Err(words.order_empty.clone());
    }

    let mut order = Vec::with_capacity(names.len());

    for name in names {
      let ordered = if *name == words.here {
        Ordered::Here
      } else if name == self.label() {
        Ordered::Placed
      } else {
        return Err(fill_marks(
          &words.order_unknown,
          &[("name", name), ("machine", self.label()), ("here", &words.here)],
        ));
      };

      if order.contains(&ordered) {
        return Err(fill_marks(&words.order_twice, &[("name", name)]));
      }

      order.push(ordered);
    }

    Ok(order)
  }

  /// The free space below which a disk there sends `placed-disk-low`, in bytes.
  pub fn disk_floor_bytes(&self) -> u64 {
    let gigabytes = self.disk_free_gigabytes.unwrap_or(PRUNE.placed_disk_free_gigabytes);

    (gigabytes * 1e9) as u64
  }

  /// The paths there whose disks' free space is read: the build directory's, then each the settings name,
  /// once each.
  pub fn disk_paths(&self) -> Vec<String> {
    let mut paths = vec![self.cargo_target.clone()];

    for path in &self.disk_paths {
      if !paths.contains(path) {
        paths.push(path.clone());
      }
    }

    paths
  }

  /// The label the lines name the machine by.
  pub fn label(&self) -> &str {
    self.machine.as_deref().unwrap_or(&self.destination)
  }

  /// Where a tree of this machine stands under the root of a bench made ready for `purpose`: at its path
  /// under this machine's home, or at its whole path where it stands outside it.
  pub fn place_of(&self, purpose: BenchFor<'_>, tree: &Path, home: &Path) -> String {
    let relative = tree.strip_prefix(home).unwrap_or(tree);

    format!("{}/{}", purpose.root(self), relative.to_string_lossy().trim_start_matches('/'))
  }

  fn as_bench(&self, path: String) -> Bench {
    Bench {
      destination: self.destination.clone(),
      path,
      ssh: self.ssh.clone(),
    }
  }
}

/// What a bench on the placed machine is made ready for, which says where its trees stand there and whether
/// the checkout's open files go over (docs/boundary.md, 「The placed machine」).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchFor<'a> {
  /// A run: the commit with the open files over it, its trees under the runs' bench root, where a run of
  /// another checkout copies the same trees as they stand.
  Run,
  /// A deployment of the named repository: the commit alone, its trees under that repository's folder of the
  /// deployments' bench root, which no run and no other repository's deployment copies into.
  Deployment(&'a str),
}

impl BenchFor<'_> {
  /// The root the bench and its trees stand under there.
  pub fn root(self, settings: &MachineSettings) -> String {
    match self {
      Self::Run => settings.bench.trim_end_matches('/').to_owned(),
      Self::Deployment(repository) => {
        format!("{}/{repository}", settings.deploy_bench.trim_end_matches('/'))
      }
    }
  }

  /// Whether the checkout's open files go over the commit.
  pub fn open_files(self) -> bool {
    matches!(self, Self::Run)
  }

  /// Where the bench of a checkout stands there: a run's at the checkout's own place; a deployment's at the
  /// place its repository's main checkout would have beside the checkout, so every checkout of the repository
  /// deploys from one bench (docs/boundary.md, 「A deployment's bench stands apart」).
  pub fn bench_place(self, settings: &MachineSettings, checkout: &Path, home: &Path) -> String {
    match self {
      Self::Run => settings.place_of(self, checkout, home),
      Self::Deployment(_) => settings.place_of(self, &main_checkout_place(checkout), home),
    }
  }

  /// How a tree the checkout's workspace names outside it is laid beside the bench: for a run, copied as it
  /// stands; for a deployment, the repository holding it at the head of its main branch, since a deployment
  /// builds committed trees only (docs/deploy.md, 「The releases」). Why a tree has no committed tree to lay.
  pub fn tree(
    self,
    tree: &Path,
    checkout: &Path,
    settings: &MachineSettings,
    home: &Path,
  ) -> Result<BenchTree, String> {
    match self {
      Self::Run => Ok(BenchTree::AsItStands {
        tree: tree.to_path_buf(),
        place: settings.place_of(self, tree, home),
      }),
      Self::Deployment(_) => {
        let committed = committed_tree(tree, checkout)?;

        Ok(BenchTree::Committed {
          remote: reachable_remote(&committed.repository),
          place: settings.place_of(self, &committed.repository, home),
          repository: committed.repository,
          commit: committed.head,
        })
      }
    }
  }
}

/// Where the repository's main checkout would stand beside a checkout: in the folder holding the checkout, under
/// the folder name of the work tree the checkout's common git directory belongs to. A checkout whose common git
/// directory is no work tree's `.git`, or cannot be read, keeps its own place.
pub fn main_checkout_place(checkout: &Path) -> PathBuf {
  let common = Command::new("git")
    .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
    .current_dir(checkout)
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output()
    .ok()
    .filter(|done| done.status.success())
    .map(|done| PathBuf::from(String::from_utf8_lossy(&done.stdout).trim()));
  let main_name = common
    .filter(|dir| dir.file_name().is_some_and(|name| name == ".git"))
    .and_then(|dir| dir.parent().and_then(Path::file_name).map(ToOwned::to_owned));

  match (main_name, checkout.parent()) {
    (Some(name), Some(folder)) => folder.join(name),
    _ => checkout.to_path_buf(),
  }
}

/// A bench made ready: its path on the placed machine, the checkouts a run there knows, the bench and every tree
/// copied beside it, the heads of the committed trees laid beside it, none for a run's, and the folder of the
/// bench the words run in, from its top, empty for the top itself.
pub struct ReadyBench {
  pub path: String,
  pub checkouts: Vec<String>,
  pub heads: Vec<String>,
  pub within: String,
}

/// The placed machine, a member of the placement type: every run goes there over `ssh`, as the testbed's
/// `placed` verb in a bench copy of the checkout.
pub struct MachinePlacement {
  pub settings: MachineSettings,
  pub home: PathBuf,
  /// The testbed deploy tree whose live release the testbed there is kept to (docs/boundary.md, 「The placed
  /// machine」); none where this one runs from no release, such as a checkout's own build, and then nothing is
  /// compared.
  pub deploy_tree: Option<PathBuf>,
  /// The data root, whose `locks/` holds the lock every bringing of the testbed there to a release holds.
  pub data_root: PathBuf,
  /// How a run waits for the machine where it does not answer.
  pub unreached: FarWait,
  /// Where the machine's own events go: its outage, its return and a testbed there that could not be brought to the
  /// live release (docs/boundary.md, 「Events」).
  pub hook: Hook,
}

/// What words run on the placed machine came to: the code they ended with there, or the connection broken
/// under them, `ssh`'s own 255 or no code at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FarEnd {
  Ended(u8),
  Broken,
}

impl MachinePlacement {
  /// The placement under a data root for an account's home, its events going to the machine's own hook: none where
  /// the settings name no machine or do not read, the reason said on a line.
  pub fn named(data_root: &Path, home: &Path) -> Option<Self> {
    match MachineSettings::read(data_root)? {
      Ok(settings) => Some(Self {
        settings,
        home: home.to_path_buf(),
        deploy_tree: running_deploy_tree(),
        data_root: data_root.to_path_buf(),
        unreached: FarWait::policy(),
        hook: machine_hook(data_root, &process_env()),
      }),
      Err(why) => {
        eprintln!("{why}");
        None
      }
    }
  }

  // The `ssh` program the settings name, and its options.
  fn ssh_program(&self) -> (&str, &[String]) {
    self
      .settings
      .ssh
      .split_first()
      .map_or(("ssh", &[][..]), |(program, options)| (program, options))
  }

  fn ssh(&self) -> Command {
    let (program, options) = self.ssh_program();
    let mut command = Command::new(program);

    command.args(options).arg(&self.settings.destination).stdin(Stdio::null());
    command
  }

  /// The units a release marks for this machine laid there (docs/deploy.md, 「The unit links」): each written
  /// whole under that machine's user units and renamed over whatever stood at its name, then its service
  /// manager reloaded and each timer enabled and started; what that machine or `ssh` said where a step failed.
  pub fn lay_units(&self, units: &[(String, String)], timers: &[String]) -> Result<(), String> {
    let (program, options) = self.ssh_program();
    let made = self.ssh().arg(format!("mkdir -p {}", far_path(FAR_UNIT_DIR))).output();

    if !succeeded(&made) {
      return Err(said(&made));
    }

    for (name, text) in units {
      let file = format!("{FAR_UNIT_DIR}/{name}");
      let laid = laid_over(program, options, &self.settings.destination, &file, text.as_bytes(), None);

      if !succeeded(&laid) {
        return Err(said(&laid));
      }
    }

    let started = std::iter::once("systemctl --user daemon-reload".to_owned())
      .chain(timers.iter().map(|timer| format!("systemctl --user enable --now {}", quoted(timer))))
      .collect::<Vec<_>>()
      .join(" && ");
    let done = self.ssh().arg(started).output();

    if succeeded(&done) { Ok(()) } else { Err(said(&done)) }
  }

  /// The filesystem holding each disk path there (docs/deploy.md, 「The Cargo build directory」), in the
  /// settings' order, none for a path whose filesystem does not read; what `ssh` said where the machine was not
  /// reached.
  pub fn disk_free(&self) -> Result<Vec<(String, Option<FarDisk>)>, String> {
    let paths = self.settings.disk_paths();
    // One JSON value a path, in order: `findmnt`'s own answer for the filesystem holding it, or `null` where
    // `findmnt` cannot read the path.
    let line = paths
      .iter()
      .map(|path| {
        format!(
          "{{ findmnt --json --bytes --output SOURCE,AVAIL --target {} 2>/dev/null || echo null; }}",
          far_path(path)
        )
      })
      .collect::<Vec<_>>()
      .join("; ");
    let read = self.ssh().arg(line).output();

    if !succeeded(&read) {
      return Err(said(&read));
    }

    let out = read.map(|done| String::from_utf8_lossy(&done.stdout).into_owned()).unwrap_or_default();
    let mut answers = serde_json::Deserializer::from_str(&out).into_iter::<Option<Mounts>>();

    Ok(
      paths
        .into_iter()
        .map(|path| {
          let disk = answers
            .next()
            .and_then(Result::ok)
            .flatten()
            .and_then(|mounts| mounts.filesystems.into_iter().next())
            .and_then(Mount::disk);

          (path, disk)
        })
        .collect(),
    )
  }

  /// Whether a seat of the lane there is free now with nobody waiting for one, as that machine's `status
  /// --free` answers; the `full` line where it is not, or where the answer does not come.
  pub fn seat_free(&self, lane: Lane) -> bool {
    let narrow = if lane == Lane::Narrow { " --narrow" } else { "" };
    let asked = self.ssh().arg(format!("{} status --free{narrow}", self.settings.testbed)).output();

    if succeeded(&asked) {
      return true;
    }

    eprintln!("{}", fill_marks(&LANES.placed.full, &[("machine", self.settings.label())]));

    false
  }

  /// Whether the machine answers; what `ssh` said where it does not, on the `unreached` line.
  pub fn reached(&self) -> bool {
    match self.answers() {
      Ok(()) => true,
      Err(said) => {
        eprintln!(
          "{}",
          fill_marks(&LANES.placed.unreached, &[("machine", self.settings.label()), ("said", &said)])
        );
        false
      }
    }
  }

  /// The bench of a checkout made ready at a commit for `purpose`, its outside trees with it, and the
  /// checkout holding the shared versions the declaration names, found through this machine's registry; the
  /// bench's path there, and the far places of every tree copied, which are that run's checkouts there.
  /// Nothing is made ready where the testbed there is not the live testbed release.
  pub fn ready(&self, checkout: &Path, commit: &str, purpose: BenchFor<'_>) -> Result<ReadyBench, String> {
    self.live_there()?;
    self.made_ready(checkout, commit, purpose, &[])
  }

  // The bench made ready, the release there taken as already compared, no tree laid at or under a place
  // `benches` names: those are benches of their own, made ready beside it.
  fn made_ready(
    &self,
    checkout: &Path,
    commit: &str,
    purpose: BenchFor<'_>,
    benches: &[String],
  ) -> Result<ReadyBench, String> {
    let remote = reachable_remote(checkout);
    // A deployment's bench lays the trees its commit's workspace names, never the checkout's working tree's
    // (docs/deploy.md, 「The releases」); a run's lays what the checkout names as it stands.
    let mut trees = match purpose {
      BenchFor::Deployment(_) => {
        let git = |line: &[String], cwd: &Path| bmo_testbed_deploy::run::run_quiet(line, Some(cwd));

        outside_roots_at(checkout, checkout, commit, &self.data_root, &git)?
      }
      BenchFor::Run => outside_roots(checkout, checkout)?,
    };
    // The shared versions' file as the shared checkout's main holds it, laid over that checkout's copy, since
    // the copy is its working tree and the run there reads the copy as it stands.
    let mut laid = Vec::new();
    // A checkout of the repository holding the shared versions is that repository, known once by the copy of
    // the checkout the registry finds: the bench is listed only where it is that copy, so a worker's copy is
    // never a second checkout giving the name (docs/boundary.md, 「The placed machine」).
    let mut known_by_its_shared_copy = false;
    let repo = Repository::read(checkout).ok();

    // The trees its tests read by a path no manifest names, laid as a tree a manifest names is.
    for sibling in repo.iter().flat_map(|repo| &repo.declaration.test.sibling_trees) {
      let tree = std::fs::canonicalize(checkout.join(sibling)).map_err(|why| {
        fill_marks(
          &LANES.placed.sibling_absent,
          &[("tree", &sibling.to_string_lossy()), ("why", &why.to_string())],
        )
      })?;

      trees.push(tree);
    }

    if let Some(repo) = &repo
      && let Some(shared) = &repo.declaration.check.shared_versions
    {
      let there = shared_checkout(&shared.repository, &process_env())?;
      known_by_its_shared_copy = repo.name() == shared.repository;
      let file = &QUICK_CHECKS.shared_versions.file;

      laid.push((
        format!("{}/{file}", self.settings.place_of(purpose, &there.root, &self.home)),
        main_shared_text(&there)?,
      ));
      trees.push(there.root);
    }

    trees.sort();
    trees.dedup();

    let mut trees = trees
      .iter()
      .map(|tree| purpose.tree(tree, checkout, &self.settings, &self.home))
      .collect::<Result<Vec<BenchTree>, String>>()?;

    // Two trees of one repository are that repository laid once.
    let mut seen = std::collections::HashSet::new();

    trees.retain(|tree| seen.insert(tree.place().to_owned()));
    let path = purpose.bench_place(&self.settings, checkout, &self.home);
    let (beside, trees): (Vec<BenchTree>, Vec<BenchTree>) = trees.into_iter().partition(|tree| {
      benches.iter().any(|bench| {
        *bench != path && (tree.place() == bench || tree.place().starts_with(&format!("{bench}/")))
      })
    });
    let places = trees.iter().map(|tree| tree.place().to_owned()).collect::<Vec<String>>();
    let checkouts = (!known_by_its_shared_copy)
      .then(|| path.clone())
      .into_iter()
      .chain(trees.iter().chain(&beside).map(|tree| tree.place().to_owned()))
      .collect();
    let heads = trees
      .iter()
      .filter_map(|tree| match tree {
        BenchTree::Committed { commit, .. } => Some(commit.clone()),
        BenchTree::AsItStands { .. } => None,
      })
      .collect();
    let source = BenchSource {
      checkout,
      remote: &remote,
      commit,
      open_files: purpose.open_files(),
      trees,
      laid,
    };

    ready_bench(&self.settings.as_bench(path.clone()), &source)?;

    if !purpose.open_files() {
      self.remove_stale_benches(checkout, commit, &path, &places);
      self.trees_stand(&places)?;
    }

    Ok(ReadyBench {
      path,
      checkouts,
      heads,
      within: String::new(),
    })
  }

  // The benches other checkouts of the repository left beside its one deployment bench there taken away: each
  // repository in the bench's folder whose root commits are the commit's own, but for the bench itself and for
  // every folder `laid` names or holds one of; said on a line, and failing nothing (docs/boundary.md,
  // 「A deployment's bench stands apart」).
  fn remove_stale_benches(&self, checkout: &Path, commit: &str, bench: &str, laid: &[String]) {
    let words = &LANES.placed;
    let roots = Command::new("git")
      .args(["rev-list", "--max-parents=0", commit])
      .current_dir(checkout)
      .stdin(Stdio::null())
      .stderr(Stdio::null())
      .output()
      .ok()
      .filter(|done| done.status.success())
      .map(|done| {
        let mut roots: Vec<String> =
          String::from_utf8_lossy(&done.stdout).split_whitespace().map(str::to_owned).collect();

        roots.sort();
        roots.iter().map(|root| format!("{root} ")).collect::<String>()
      })
      .filter(|roots| !roots.is_empty());
    let (Some(roots), Some((folder, name))) = (roots, bench.rsplit_once('/')) else {
      return;
    };
    // A tree laid for this deployment is kept, and so is the folder of the bench's folder holding one deeper
    // down: a sibling repository whose history was split from the deployed repository's shares its root
    // commits, so the roots alone would take that dependency for another checkout's leftover.
    let mut kept = vec![name.to_owned()];
    let below = format!("{folder}/");

    for place in laid {
      if let Some(rest) = place.strip_prefix(&below) {
        let first = rest.split('/').next().unwrap_or_default();

        if !first.is_empty() && !kept.iter().any(|kept| kept == first) {
          kept.push(first.to_owned());
        }
      }
    }

    let skips = kept
      .iter()
      .map(|kept| format!("[ \"$d\" = {} ] && continue;", quoted(kept)))
      .collect::<Vec<String>>()
      .join(" ");
    // Each folder's root commits are read as git lists them, sorted and each followed by a space, as `roots` is.
    let line = format!(
      "cd {} 2>/dev/null || exit 0; for d in */; do d=${{d%/}}; {skips} \
       [ -e \"$d/.git\" ] || continue; \
       r=$(git -C \"$d\" rev-list --max-parents=0 HEAD 2>/dev/null | sort | tr '\\n' ' '); \
       [ \"$r\" = {} ] || continue; rm -rf -- \"$d\" && echo \"$d\"; done; exit 0",
      far_path(folder),
      quoted(&roots),
    );
    let done = self.ssh().arg(line).output();
    let machine = self.settings.label();

    match done {
      Ok(done) if done.status.success() => {
        let names: Vec<String> = String::from_utf8_lossy(&done.stdout).lines().map(str::to_owned).collect();

        if !names.is_empty() {
          eprintln!(
            "{}",
            fill_marks(&words.stale_removed, &[("machine", machine), ("names", &names.join(", "))])
          );
        }
      }
      done => eprintln!(
        "{}",
        fill_marks(&words.stale_unremoved, &[("machine", machine), ("said", &said(&done))])
      ),
    }
  }

  // Every tree a deployment's manifests name by a path standing on the bench before any step reads it: where
  // one does not, the bench is at fault and not the commit, the making of the bench failing with the missing
  // trees named (docs/boundary.md, 「A deployment's bench stands apart」).
  fn trees_stand(&self, laid: &[String]) -> Result<(), String> {
    if laid.is_empty() {
      return Ok(());
    }

    let words = &LANES.placed;
    let listed = laid.iter().map(|place| far_path(place)).collect::<Vec<String>>().join(" ");
    let line = format!(
      "for p in {listed}; do git -C \"$p\" rev-parse --verify -q HEAD >/dev/null 2>&1 || echo \"$p\"; \
       done; exit 0"
    );

    match self.ssh().arg(line).output() {
      Ok(done) if done.status.success() => {
        let missing: Vec<String> =
          String::from_utf8_lossy(&done.stdout).lines().map(str::to_owned).collect();

        if missing.is_empty() {
          return Ok(());
        }

        Err(fill_marks(
          &words.trees_gone,
          &[("machine", self.settings.label()), ("names", &missing.join(", "))],
        ))
      }
      done => Err(fill_marks(
        &words.trees_unread,
        &[("machine", self.settings.label()), ("said", &said(&done))],
      )),
    }
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

    if !bench.within.is_empty() {
      line.push_str(" --dir ");
      line.push_str(&quoted(&bench.within));
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

  /// The words run in the bench through the `placed` verb, as `run_there` runs them; the code they ended with,
  /// 75 where the connection broke, said on the `dropped` line.
  pub fn run(
    &self,
    bench: &ReadyBench,
    words: &[String],
    setup: bool,
    placed: Option<&PlacedRun>,
    heard: &Arc<AtomicU64>,
  ) -> u8 {
    match self.run_there(bench, words, setup, placed, heard) {
      FarEnd::Ended(code) => code,
      FarEnd::Broken => dropped(self.settings.label(), &LANES.placed.broken),
    }
  }

  /// The words run in the bench through the `placed` verb, the output and the result this process's, the
  /// placed run standing here, where one does, marked running once its turn there came, and every byte that
  /// comes back counted into `heard`: the code they ended with there, or the connection broken under them.
  pub fn run_there(
    &self,
    bench: &ReadyBench,
    words: &[String],
    setup: bool,
    placed: Option<&PlacedRun>,
    heard: &Arc<AtomicU64>,
  ) -> FarEnd {
    let said = &LANES.placed;
    let line = self.placed_line(bench, words, setup);

    eprintln!(
      "{}",
      fill_marks(&said.running, &[("machine", self.settings.label()), ("bench", &bench.path)])
    );

    // The far side's `placed` verb ends a command there that falls silent; this side passes the output on, and
    // tells its own holder while it comes.
    let look = Silence::default().look;
    let started = self.ssh().arg(line).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let status = started.and_then(|mut child| {
      let far_output = child.stdout.take().map(|out| Heard::new(out, Arc::clone(heard), look));
      let far_errors = child.stderr.take().map(|errors| Heard::new(errors, Arc::clone(heard), look));

      std::thread::scope(|scope| {
        if let Some(far_output) = far_output {
          scope.spawn(|| pass_on(far_output, std::io::stdout()));
        }

        if let Some(far_errors) = far_errors {
          scope.spawn(|| passed_on(far_errors, placed));
        }

        child.wait()
      })
    });

    match status.ok().and_then(|status| status.code()) {
      // 255 is `ssh`'s own: the connection broke or was never made.
      Some(255) | None => FarEnd::Broken,
      Some(code) => FarEnd::Ended(u8::try_from(code).unwrap_or(1)),
    }
  }

  /// The line saying the connection to the machine broke, as a deployment's work there names it.
  pub fn broke(&self) -> String {
    fill_marks(&LANES.placed.broke, &[("machine", self.settings.label())])
  }

  /// The words run in the bench through the `placed` verb with no setup, what they say on standard error this
  /// process's; what they printed on standard output where they ended 0, none otherwise.
  pub fn read(&self, bench: &ReadyBench, words: &[String]) -> Option<String> {
    let mut child = self
      .ssh()
      .arg(self.placed_line(bench, words, false))
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn()
      .ok()?;
    let far_errors = child.stderr.take();
    let done = std::thread::scope(|scope| {
      if let Some(far_errors) = far_errors {
        scope.spawn(|| passed_on(far_errors, None));
      }

      child.wait_with_output()
    })
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

  /// A run placed on the machine: the machine waited for and its testbed brought to the live release, the bench
  /// made ready at the checkout's `HEAD` with its open files, and the words run there as the testbed's own `run`;
  /// the code it ended with there, or the code of a run not run where the machine did not answer within the wait
  /// or its testbed could not be brought to the live release. None where it did not go: for a run that goes there
  /// only to a seat free now since another machine follows it in the order, every seat of its lane there held or
  /// waited for, said on a line. From the moment it goes there it says it still runs every minute none of its
  /// output came back.
  fn placed(&self, ask: &PlaceAsk<'_>, to_free_seat: bool) -> Option<u8> {
    let heard = Arc::new(AtomicU64::new(0));

    standing(self.settings.label(), &heard, || self.placed_heard(ask, to_free_seat, &heard))
  }

  // The run placed as `placed` places it, every byte of its output that comes back counted into `heard`.
  fn placed_heard(&self, ask: &PlaceAsk<'_>, to_free_seat: bool, heard: &Arc<AtomicU64>) -> Option<u8> {
    if let Err(code) = self.waited_ready(ask) {
      return Some(code);
    }

    if to_free_seat && !self.seat_free(ask.lane) {
      return None;
    }

    let found = git(&ask.cwd, &["rev-parse", "--show-toplevel"]);
    let checkout = if found.is_empty() { ask.cwd.clone() } else { PathBuf::from(found) };
    let placed = PlacedRun::stand(ask, &checkout, self.settings.label());
    let commit = git(&checkout, &["rev-parse", "HEAD"]);
    let bench = match self.made_ready(&checkout, &commit, BenchFor::Run, &[]) {
      Ok(bench) => ReadyBench {
        within: place_in_checkout(&ask.cwd),
        ..bench
      },
      Err(said) => return Some(dropped(self.settings.label(), &said)),
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

    Some(self.run(&bench, &words, true, Some(&placed), heard))
  }

  /// A change check over several checkouts placed on the machine as one run (docs/boundary.md, 「Placement」):
  /// the machine waited for and its testbed brought to the live release once, every checkout made ready at its
  /// own place under the bench root at its `HEAD` with its open files, the trees they read laid beside them once,
  /// each checkout's declared setup run in its own bench, and the set run in the first checkout's bench as the
  /// testbed's change check there. None where it did not go, as for one run. It says it still runs as one run
  /// does.
  fn placed_set(&self, ask: &SetAsk<'_>, to_free_seat: bool) -> Option<u8> {
    let heard = Arc::new(AtomicU64::new(0));

    standing(self.settings.label(), &heard, || self.placed_set_heard(ask, to_free_seat, &heard))
  }

  // The set placed as `placed_set` places it, every byte of its output that comes back counted into `heard`.
  fn placed_set_heard(&self, ask: &SetAsk<'_>, to_free_seat: bool, heard: &Arc<AtomicU64>) -> Option<u8> {
    if let Err(code) = self.waited_ready(&ask.run) {
      return Some(code);
    }

    if to_free_seat && !self.seat_free(ask.run.lane) {
      return None;
    }

    let first = ask.checkouts.first()?;
    let machine = self.settings.label();
    let placed = PlacedRun::stand(&ask.run, first, machine);
    let places: Vec<String> = ask
      .checkouts
      .iter()
      .map(|checkout| self.settings.place_of(BenchFor::Run, checkout, &self.home))
      .collect();
    let mut benches = Vec::with_capacity(ask.checkouts.len());

    for checkout in ask.checkouts {
      let commit = git(checkout, &["rev-parse", "HEAD"]);

      match self.made_ready(checkout, &commit, BenchFor::Run, &places) {
        Ok(bench) => benches.push(bench),
        Err(said) => return Some(dropped(machine, &said)),
      }
    }

    // The run there knows every bench and every tree laid beside them.
    let mut known: Vec<String> = Vec::new();

    for one in benches.iter().flat_map(|bench| &bench.checkouts) {
      if !known.contains(one) {
        known.push(one.clone());
      }
    }

    let knowing = |bench: &ReadyBench| ReadyBench {
      path: bench.path.clone(),
      checkouts: known.clone(),
      heads: vec![],
      within: String::new(),
    };

    // The first checkout's setup runs with the set; every other one's in its own bench before it.
    for (checkout, bench) in ask.checkouts.iter().zip(&benches).skip(1) {
      let declares_setup =
        Repository::read(checkout).is_ok_and(|repo| !repo.declaration.test.setup.is_empty());

      if declares_setup {
        let code = self.run(&knowing(bench), &[SETUP_ALONE.to_owned()], true, None, heard);

        if code != 0 {
          return Some(code);
        }
      }
    }

    let paths: Vec<String> = places.iter().map(|place| path_from(&places[0], place)).collect();
    let words: Vec<String> =
      std::iter::once(self.settings.testbed.clone()).chain((ask.words)(&paths)).collect();

    Some(self.run(&knowing(&benches[0]), &words, true, Some(&placed), heard))
  }
}

/// The variable naming how often a placed run whose output has not come back says it still runs, in
/// milliseconds, over the lanes policy's; a test sets it short.
pub const STANDING_VARIABLE: &str = "BMO_TESTBED_PLACED_STANDING_MS";

// `work`, a run placed on `machine`, done while a line says it still runs there at the end of every
// `placed_standing_seconds` in which none of its output came back into `heard` (docs/testbed.md, 「A run on
// another machine」).
fn standing<T>(machine: &str, heard: &AtomicU64, work: impl FnOnce() -> T) -> T {
  let every = std::env::var(STANDING_VARIABLE)
    .ok()
    .and_then(|given| given.parse::<u64>().ok())
    .filter(|ms| *ms > 0)
    .map_or_else(|| Duration::from_secs(LANES.numbers.placed_standing_seconds), Duration::from_millis);
  let say = |seconds: u64| {
    eprintln!(
      "{}",
      fill_marks(&LANES.placed.still_running, &[("machine", machine), ("seconds", &seconds.to_string())])
    );
  };

  said_while_quiet(every, heard, &say, work)
}

// The command run after a bench's setup where the setup alone is asked for: one that does nothing.
const SETUP_ALONE: &str = "true";

/// The path of `to` as seen from `from`, both places under one bench root written with `/`: up from `from` to
/// where they part, then down to `to`; `.` where they are one place.
pub fn path_from(from: &str, to: &str) -> String {
  let from: Vec<&str> = from.split('/').filter(|part| !part.is_empty()).collect();
  let to: Vec<&str> = to.split('/').filter(|part| !part.is_empty()).collect();
  let shared = from.iter().zip(&to).take_while(|(one, other)| one == other).count();
  let parts: Vec<&str> = std::iter::repeat_n("..", from.len() - shared)
    .chain(to[shared..].iter().copied())
    .collect();

  if parts.is_empty() { ".".to_owned() } else { parts.join("/") }
}

impl Placement for MachinePlacement {
  /// The run walks the order: it goes to the first machine with a seat of its lane free now, and the last
  /// takes it whatever it holds. A placed machine that does not take it passes it on to the next, and a run
  /// no machine took stays here.
  fn place(&self, ask: &PlaceAsk<'_>) -> Option<u8> {
    if kept_outside_copy(ask) {
      return None;
    }

    let order = self.settings.order().unwrap_or_default();
    let last = order.len().saturating_sub(1);

    for (at, ordered) in order.into_iter().enumerate() {
      let to_free_seat = at < last;

      match ordered {
        Ordered::Here => {
          if !to_free_seat || !lane_full(ask.env, ask.lane) {
            return None;
          }
        }
        Ordered::Placed => {
          if let Some(code) = self.placed(ask, to_free_seat) {
            return Some(code);
          }
        }
      }
    }

    None
  }

  /// The set walks `combined_order` as a run walks `order`: by default the placed machine alone, which takes it
  /// whatever it holds, so the set waits in that machine's line and never falls to this one.
  fn place_set(&self, ask: &SetAsk<'_>) -> Option<u8> {
    let order = self.settings.combined_order().unwrap_or_else(|_| vec![Ordered::Placed]);
    let last = order.len().saturating_sub(1);

    for (at, ordered) in order.into_iter().enumerate() {
      let to_free_seat = at < last;

      match ordered {
        Ordered::Here => {
          if !to_free_seat || !lane_full(ask.run.env, ask.run.lane) {
            return None;
          }
        }
        Ordered::Placed => {
          if let Some(code) = self.placed_set(ask, to_free_seat) {
            return Some(code);
          }
        }
      }
    }

    None
  }
}
