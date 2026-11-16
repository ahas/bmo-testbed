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
use std::sync::atomic::AtomicU64;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::placement::{PlaceAsk, PlacedRun, Placement};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use bmo_testbed_core::silence::{Heard, Silence, pass_on};
use bmo_testbed_core::turn::{
  FRONT_VARIABLE, Lane, NEXT_VARIABLE, NOT_RUN_EXIT_CODE, process_env, stands_at_front, stands_next,
};
use bmo_testbed_deploy::expansion::committed_tree;
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
    let copied = rsync_over(&ssh_command, checkout, &format!("{}:{}", bench.destination, bench.path), &[]);

    if !succeeded(&copied) {
      return Err(failed(&said_of.uncopied, &copied));
    }
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
    let copied = rsync_over(&ssh_command, tree, &format!("{}:{}", bench.destination, place), &kept);

    if !succeeded(&copied) {
      return Err(fill_marks(
        &said_of.tree_uncopied,
        &[("tree", &tree.to_string_lossy()), ("said", &said(&copied))],
      ));
    }
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
  async fn streamed(client: &reqwest::Client, machine: &str, run: &str, placed: &PlacedRun) -> u8 {
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

        Some(Self::streamed(&client, &machine, &run, &placed).await)
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
        }),
    )
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
/// copied beside it, and the heads of the committed trees laid beside it, none for a run's.
pub struct ReadyBench {
  pub path: String,
  pub checkouts: Vec<String>,
  pub heads: Vec<String>,
}

/// The placed machine, a member of the placement type: every run goes there over `ssh`, as the testbed's
/// `placed` verb in a bench copy of the checkout.
pub struct MachinePlacement {
  pub settings: MachineSettings,
  pub home: PathBuf,
  /// The running testbed's release, which the testbed there must be built from too; none where this one runs
  /// from no release, such as a checkout's own build, and then nothing is compared.
  pub release: Option<OwnRelease>,
}

/// The release the running testbed stands in: its directory, and the commit it was built from.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnRelease {
  pub dir: PathBuf,
  pub commit: String,
}

/// The running binary's release: the directory above its `bin/`, and the commit the `built-from` beside it
/// names. None where the binary stands in no release.
pub fn own_release() -> Option<OwnRelease> {
  let binary = std::env::current_exe().ok()?;
  let dir = binary.parent()?.parent()?.to_path_buf();

  Some(OwnRelease {
    commit: built_from(&dir)?,
    dir,
  })
}

/// What a run on the placed machine came to: it ran there, with its code; or it did not go, said on a line,
/// and the run is the caller's: the machine unreached or its testbed another release than this one's, or, for
/// a run that goes only to a free seat, every seat there held or waited for.
pub enum Placed {
  Ran(u8),
  Here,
  Full,
}

/// What words run on the placed machine came to: the code they ended with there, or the connection broken
/// under them, `ssh`'s own 255 or no code at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FarEnd {
  Ended(u8),
  Broken,
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
    let Some(OwnRelease { commit: own, .. }) = &self.release else { return Ok(()) };
    let words = &LANES.placed;
    let machine = self.settings.label();

    match self.far_built_from() {
      Ok(far) if far == *own => Ok(()),
      Ok(far) => Err(fill_marks(
        &words.stale,
        &[("machine", machine), ("far", &short(&far)), ("own", &short(own))],
      )),
      Err(said) => Err(fill_marks(
        &words.unreleased,
        &[
          ("machine", machine),
          ("file", &self.far_built_from_file()),
          ("said", &said),
          ("own", &short(own)),
        ],
      )),
    }
  }

  // The `built-from` beside the testbed binary there.
  fn far_built_from_file(&self) -> String {
    Path::new(&self.settings.testbed)
      .with_file_name(BUILT_FROM)
      .to_string_lossy()
      .into_owned()
  }

  // The commit the `built-from` there names; what reading it said where it names none.
  fn far_built_from(&self) -> Result<String, String> {
    let read = self.ssh().arg(format!("cat {}", far_path(&self.far_built_from_file()))).output();

    read
      .as_ref()
      .ok()
      .filter(|done| done.status.success())
      .map(|done| String::from_utf8_lossy(&done.stdout).trim().to_owned())
      .filter(|commit| !commit.is_empty())
      .ok_or_else(|| said(&read))
  }

  /// The binary of a release the testbed there would run: the file in the release's `bin/` of the name the
  /// settings' `testbed` ends in; none where the release holds no such file, so it carries no testbed.
  pub fn testbed_in(&self, release: &Path) -> Option<PathBuf> {
    Some(release.join(self.testbed_name())).filter(|binary| binary.is_file())
  }

  // The name the settings' `testbed` ends in, under `bin/`.
  fn testbed_name(&self) -> String {
    format!(
      "bin/{}",
      Path::new(&self.settings.testbed).file_name().unwrap_or_default().to_string_lossy()
    )
  }

  /// The testbed there brought to a release's (docs/boundary.md, 「The placed machine」): the release's
  /// `binary` written onto the settings' `testbed`, the `built-from` beside it onto the one beside that, last,
  /// and that `built-from` read back, which must name `commit`. The files written there; the reason where the
  /// machine is not reached, a file is not written, or what is read back names another commit or none.
  pub fn bring_testbed(&self, binary: &Path, commit: &str) -> Result<Vec<String>, String> {
    let words = &LANES.placed;
    let probed = self.ssh().arg("true").output();

    if !succeeded(&probed) {
      return Err(fill_marks(&words.testbed_unreached, &[("said", &said(&probed))]));
    }

    let unwritten =
      |file: &str, why: &str| fill_marks(&words.testbed_unwritten, &[("file", file), ("said", why)]);
    let far_testbed = self.settings.testbed.clone();
    let far_built_from = self.far_built_from_file();
    let far_bin = Path::new(&far_testbed)
      .parent()
      .map(|bin| bin.to_string_lossy().into_owned())
      .unwrap_or_default();

    if !far_bin.is_empty() {
      let made = self.ssh().arg(format!("mkdir -p {}", far_path(&far_bin))).output();

      if !succeeded(&made) {
        return Err(unwritten(&far_bin, &said(&made)));
      }
    }

    let (program, options) = self.ssh_program();
    let written = [
      (binary.to_path_buf(), &far_testbed, Some("755")),
      (binary.with_file_name(BUILT_FROM), &far_built_from, None),
    ];

    for (from, to, mode) in written {
      let bytes = std::fs::read(&from).map_err(|why| unwritten(to, &why.to_string()))?;
      let laid = laid_over(program, options, &self.settings.destination, to, &bytes, mode);

      if !succeeded(&laid) {
        return Err(unwritten(to, &said(&laid)));
      }
    }

    match self.far_built_from() {
      Ok(far) if far == commit => Ok(vec![far_testbed, far_built_from]),
      Ok(far) => Err(fill_marks(&words.testbed_other, &[("far", &short(&far)), ("own", &short(commit))])),
      Err(why) => Err(fill_marks(&words.testbed_unread, &[("file", &far_built_from), ("said", &why)])),
    }
  }

  /// The testbed there brought to this one's own release before a deployment's bench (docs/deploy.md, 「Where a
  /// deployment runs」), where it is not that release already, said on a line; the reason naming both commits
  /// and what the machine answered where it could not be.
  pub fn brought_to_own(&self) -> Result<(), String> {
    let (Err(stale), Some(own)) = (self.same_release(), &self.release) else {
      return Ok(());
    };
    let words = &LANES.placed;
    let binary = self.testbed_in(&own.dir).ok_or_else(|| {
      fill_marks(
        &words.own_uncarried,
        &[("release", &own.dir.to_string_lossy()), ("binary", &self.testbed_name())],
      )
    });

    match binary.and_then(|binary| self.bring_testbed(&binary, &own.commit)) {
      Ok(_) => {
        eprintln!(
          "{}",
          fill_marks(
            &words.brought_own,
            &[("machine", self.settings.label()), ("own", &short(&own.commit))]
          )
        );
        Ok(())
      }
      Err(why) => Err(fill_marks(&words.unbrought_own, &[("stale", &stale), ("said", &why)])),
    }
  }

  /// The bench of a checkout made ready at a commit for `purpose`, its outside trees with it, and the
  /// checkout holding the shared versions the declaration names, found through this machine's registry; the
  /// bench's path there, and the far places of every tree copied, which are that run's checkouts there.
  /// Nothing is made ready where the testbed there is not this one's release.
  pub fn ready(&self, checkout: &Path, commit: &str, purpose: BenchFor<'_>) -> Result<ReadyBench, String> {
    self.same_release()?;
    self.made_ready(checkout, commit, purpose)
  }

  // The bench made ready, the release there taken as already compared.
  fn made_ready(&self, checkout: &Path, commit: &str, purpose: BenchFor<'_>) -> Result<ReadyBench, String> {
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
    let checkouts = std::iter::once(path.clone())
      .chain(trees.iter().map(|tree| tree.place().to_owned()))
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
      self.remove_stale_benches(checkout, commit, &path);
    }

    Ok(ReadyBench {
      path,
      checkouts,
      heads,
    })
  }

  // The benches other checkouts of the repository left beside its one deployment bench there taken away: each
  // repository in the bench's folder whose root commits are the commit's own, but for the bench itself; said on
  // a line, and failing nothing (docs/boundary.md, 「A deployment's bench stands apart」).
  fn remove_stale_benches(&self, checkout: &Path, commit: &str, bench: &str) {
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
    // Each folder's root commits are read as git lists them, sorted and each followed by a space, as `roots` is.
    let line = format!(
      "cd {} 2>/dev/null || exit 0; for d in */; do d=${{d%/}}; [ \"$d\" = {} ] && continue; \
       [ -e \"$d/.git\" ] || continue; \
       r=$(git -C \"$d\" rev-list --max-parents=0 HEAD 2>/dev/null | sort | tr '\\n' ' '); \
       [ \"$r\" = {} ] || continue; rm -rf -- \"$d\" && echo \"$d\"; done; exit 0",
      far_path(folder),
      quoted(name),
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

  /// The words run in the bench through the `placed` verb, as `run_there` runs them; the code they ended with,
  /// 75 where the connection broke, said on the `dropped` line.
  pub fn run(&self, bench: &ReadyBench, words: &[String], setup: bool, placed: Option<&PlacedRun>) -> u8 {
    match self.run_there(bench, words, setup, placed) {
      FarEnd::Ended(code) => code,
      FarEnd::Broken => dropped(self.settings.label(), &LANES.placed.broken),
    }
  }

  /// The words run in the bench through the `placed` verb, the output and the result this process's, the
  /// placed run standing here, where one does, marked running once its turn there came: the code they ended
  /// with there, or the connection broken under them.
  pub fn run_there(
    &self,
    bench: &ReadyBench,
    words: &[String],
    setup: bool,
    placed: Option<&PlacedRun>,
  ) -> FarEnd {
    let said = &LANES.placed;
    let line = self.placed_line(bench, words, setup);

    eprintln!(
      "{}",
      fill_marks(&said.running, &[("machine", self.settings.label()), ("bench", &bench.path)])
    );

    // The far side's `placed` verb ends a command there that falls silent; this side passes the output on, and
    // tells its own holder while it comes.
    let count = Arc::new(AtomicU64::new(0));
    let look = Silence::default().look;
    let started = self.ssh().arg(line).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let status = started.and_then(|mut child| {
      let far_output = child.stdout.take().map(|out| Heard::new(out, Arc::clone(&count), look));
      let far_errors = child.stderr.take().map(|errors| Heard::new(errors, Arc::clone(&count), look));

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
    let placed = PlacedRun::stand(ask, &checkout, self.settings.label());
    let commit = git(&checkout, &["rev-parse", "HEAD"]);
    let bench = match self.made_ready(&checkout, &commit, BenchFor::Run) {
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

    Placed::Ran(self.run(&bench, &words, true, Some(&placed)))
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
