//! The deploy run a repository's declaration drives (docs/deploy.md, 「What the deploy run does, in order」):
//! the checks before anything changes, the gate, the backup, the release prepared and built beside the live
//! one, the steps before the flip, the record and the flip, the units relinked and restarted, the `bot`
//! service watched coming up and rolled back where it does not, the steps after the health, the prune and
//! the backup upload started, every fact a person should hear sent to the event hook.
//!
//! What the run reaches outside its own trees goes through two hands of its own: the gate, the suite's static
//! check and run, which the binary answers so this crate stands on no other crate of the testbed; and the
//! commands it runs, the service manager's among them, so a test drives a whole deployment with stand-ins.

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bmo_testbed_client::TestRun;
use bmo_testbed_core::cargo_lock::align_path_packages;
use bmo_testbed_core::declaration::{Declaration, DeployStep, Phase, Repository};
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::policy::declaration::DECLARATION;
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::deploy_steps::DeployStep as OwnStep;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::queue::HeldLock;
use bmo_testbed_core::release_target::mark_release_target;
use bmo_testbed_core::silence::{Output, Silence, Watch};
use bmo_testbed_core::turn::{
  Env, FRONT_VARIABLE, Lane, NOT_RUN_EXIT_CODE, TurnAsk, harness_pid, process_env, take_turn,
};
use bmo_testbed_core::verb::REFUSED_EXIT_CODE;
use bmo_testbed_core::version::{declared_version, deployed_version, platform_day};
use bmo_testbed_record::{GateGround, GatePass, Record, say_unwritten};
use serde_json::{Value, json};

use crate::adopt::{Adopting, adopt, prune_retired};
use crate::again::failed_gate;
use crate::backup::{BackupPlan, BackupTaken, take_backup};
use crate::boot::{
  HealthEnd, HealthWatch, StayUp, boot_report, boot_scene, stands_up, stay_report, unit_gave_up,
  unit_status, wait_for_health,
};
use crate::console::{ConsoleHands, MachineConsole, PinnedLayer, probe};
use crate::expansion::{
  BuildTree, TreeStore, bring_submodules, build_target, committed_trees, expansion_tree, lay_outside_trees,
  put_build_tree,
};
use crate::line::{DeployLine, DeployTicket, FirstMark, RootTurn, waiting_commits};
use crate::progress::{
  DeployRun, DeployStart, Outcome, Part, Stage, begin_deploy, deploy_progress_at, deploy_progress_path,
};
use crate::release::{
  LiveProcess, Preparing, differing_binaries, flip, link_of, live_processes, live_release, logged_before,
  pinned_detail, place_binaries, prepare, primary_checkout, prune, put_link, release_dir, release_holding,
};
use crate::remove::{git_tree_above, remove_tree};
use crate::rollback::{
  BootFailure, DeployTree, FailedDeploy, Rollback, ServiceHands, deploy_failure_notice, failure_line,
  journal_command, roll_back, rolled_back_detail,
};
use crate::run::{Ran, run_quiet, shell_words, words};
use crate::tree::{
  DEPLOY_LOG, DEPLOY_RECORD, DeployRecord, UnitMark, backup_name, backup_stamp, base_name, binary_restarts,
  bot_unit, link_units, mark_built_from, marked_units, outside_relinks, placed_units, read_deploy_record,
  relink_units, retired_units, set_aside_plain_units, short, timer_service, tree_fault, unit_link_plan,
  unit_links, units_to_link, user_unit_dir_from, write_deploy_record,
};
use crate::unit::{BACKUP_SLICE, first_chars, move_to_cgroup, own_cgroup, upload_unit_name};

/// The variable naming another backup directory over the declaration's.
pub const BACKUP_DIR_VARIABLE: &str = "BMO_BACKUP_DIR";
/// The variable naming another `bot` service, for a rehearsal.
pub const BOT_UNIT_VARIABLE: &str = "BMO_UNIT";
/// The backup directory under the deploy root where the declaration names none.
pub const BACKUP_DIR: &str = "backups";

/// Where a deployment lays its commit down, each place the declaration's unless the environment names
/// another (docs/boundary.md, 「The environment」); the deploy tree is the declaration's alone, since every
/// process a release starts carries `BMO_DEPLOY_DIR` naming its own release.
#[derive(Debug, Clone)]
pub struct DeployPlaces {
  /// The checkout the commit is taken from.
  pub source: PathBuf,
  /// The directory holding the deploy tree's link, its releases and its build tree.
  pub deploy_root: PathBuf,
  pub deploy_dir: PathBuf,
  pub data_root: PathBuf,
  pub backup_dir: Option<PathBuf>,
  pub unit_dir: PathBuf,
  /// The deployment record, under the deploy root.
  pub record: PathBuf,
  /// The deployment log, beside the record.
  pub log: PathBuf,
  pub progress: PathBuf,
  /// The `bot` service the environment names over the commit's mark.
  pub bot_unit: Option<String>,
}

impl DeployPlaces {
  /// The places of a repository's declaration, the environment's over them; why it deploys nothing where its
  /// declaration names no deploy root.
  pub fn of(repo: &Repository, env: &Env) -> Result<Self, String> {
    let said = &DEPLOY.run;
    let deploy = &repo.declaration.deploy;
    let named = |name: &str| env.get(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    let (Some(root), Some(data)) = (&deploy.root, &deploy.data_root) else {
      return Err(said.no_deploy.clone());
    };
    let deploy_root = repo.path(root, env);
    let deploy_dir = deploy_root.join(&deploy.link);
    let data_root = repo.path(data, env);
    let home = PathBuf::from(env.get("HOME").map_or("/", String::as_str));

    Ok(Self {
      source: repo.root.clone(),
      deploy_dir: link_of(&deploy_dir),
      backup_dir: named(BACKUP_DIR_VARIABLE).or_else(|| {
        deploy.backup.as_ref().map(|backup| match &backup.dir {
          Some(dir) => repo.path(dir, env),
          None => deploy_root.join(BACKUP_DIR),
        })
      }),
      unit_dir: user_unit_dir_from(env.get("XDG_CONFIG_HOME").map(String::as_str), &home),
      record: deploy_root.join(DEPLOY_RECORD),
      log: deploy_root.join(DEPLOY_LOG),
      progress: named(crate::progress::DEPLOY_PROGRESS_VARIABLE)
        .unwrap_or_else(|| deploy_progress_path(&deploy_root)),
      bot_unit: env.get(BOT_UNIT_VARIABLE).filter(|unit| !unit.is_empty()).cloned(),
      data_root,
      deploy_root,
    })
  }
}

/// What the caller asked of the run.
#[derive(Debug, Clone, Default)]
pub struct DeployAsk {
  /// The commit to put live; none is the checkout's `HEAD`.
  pub commit: Option<String>,
  /// Why the deploy is made, written into the record and the event; none takes the commit's subject.
  pub message: Option<String>,
  pub dry_run: bool,
  /// The gate left out, where the caller already ran it.
  pub no_check: bool,
  /// A command run between the `bot` service's stop and its start.
  pub before_start: Option<String>,
  /// Go on past the declared `quiet` command where work still stands under way once its wait is over.
  pub force_live: bool,
  /// The deployment's own unit, which its progress record names.
  pub unit: Option<String>,
  /// What the backup upload's unit is started with after `systemd-run`'s own words and the working directory
  /// the run names, the repository's primary checkout: its options, then the binary's `backups` verb.
  pub upload: Option<Vec<String>>,
  /// The testbed's data root whose record the deployment is written into; none writes into none.
  pub record: Option<PathBuf>,
  /// How the release of the console layer is read from the shared versions of the repository a declaration
  /// names, which the console probe reads a manifest by and holds its layer to (docs/deploy.md, 「The
  /// console probe」); none reads none, as though the declaration named no shared versions.
  pub layer_pin: Option<fn(&str, &Env) -> PinnedLayer>,
  /// The deployment line the run stands in from its gate to the end of its build (docs/deploy.md, 「The
  /// deployment line」); none stands in none.
  pub line: Option<DeployLine>,
  /// The mark putting the deployment first in the line, who made it and why (docs/deploy.md, 「The deployment
  /// line」); none stands it first come.
  pub first: Option<FirstMark>,
  /// How often a deployment waiting for its deploy root's turn looks (docs/deploy.md, 「The deployment
  /// line」); none is the lanes' pace. A test sets it short.
  pub root_poll: Option<Duration>,
  /// The pauses between the tries of the gate's work on another machine whose connection broke under it
  /// (docs/deploy.md, 「A connection that breaks under the placed machine」), one try more than pauses; none
  /// tries once.
  pub retry: Vec<Duration>,
}

/// What the gate is asked: the commit, in the checkout it is taken from, over the range from what was live,
/// whether its suite runs: not where a passed run of the whole suite on the commit stands already, and the
/// earlier pass on the footing it finds itself on, which leaves it out whole.
pub struct GateAsk<'a> {
  /// The checkout the commit is taken from, which a gate on another machine makes its bench from.
  pub root: &'a Path,
  /// The build tree at the commit, its outside trees laid at their committed heads, which a gate on this
  /// machine reads in place of the checkout's working tree (docs/deploy.md, 「The releases」).
  pub tree: &'a Path,
  pub commit: &'a str,
  pub since: Option<&'a str>,
  pub suite: bool,
  /// The head of every repository laid beside `tree`, which a gate on this machine judges with it.
  pub heads: &'a [String],
  /// The pass an earlier gate wrote on the footing given, asked once the gate knows its footing; none where
  /// none stands.
  pub earlier: &'a dyn Fn(&GateFooting) -> Option<GatePass>,
}

/// What a gate stands on beside the commit and its range: the head of every repository laid beside the tree it
/// judges, the toolchains its machine answered there, and that machine's label where it is not this one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GateFooting {
  pub heads: Vec<String>,
  pub toolchains: String,
  pub machine: Option<String>,
}

/// The gate's answer where an earlier pass on its footing leaves it out: passed, running nothing.
pub fn passed_before(footing: GateFooting, earlier: GatePass) -> GateOutcome {
  GateOutcome {
    passed: true,
    footing: Some(footing),
    earlier: Some(earlier),
    ..GateOutcome::default()
  }
}

/// A gate that asks nothing earlier: the footing it finds is never passed before.
pub fn none_earlier(_: &GateFooting) -> Option<GatePass> {
  None
}

/// A passed run of the whole suite on the commit, which the gate takes in place of its own suite: the test
/// run as the record holding it reads, and the machine whose record that is where it is not this one.
#[derive(Debug, Clone, PartialEq)]
pub struct SuitePassed {
  pub run: TestRun,
  pub machine: Option<String>,
}

/// One heavy step the gate ran, as the progress record writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct GateStep {
  pub name: String,
  pub code: i64,
  pub waited_seconds: f64,
  pub ran_seconds: f64,
}

/// What the gate came to: whether the commit passed, why not where it did not, whether that was the connection
/// to the gate's machine breaking under it, the tests that passed only when run once more, each with its unit,
/// the steps it ran, and the passed run it took in place of its suite where it found one.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GateOutcome {
  pub passed: bool,
  pub reason: String,
  /// The connection to the gate's machine broke under it, as `reason` says, so the run tries again.
  pub broken: bool,
  pub flaky: Vec<(String, String)>,
  pub steps: Vec<GateStep>,
  pub suite_passed: Option<SuitePassed>,
  /// What the gate stood on, where it could tell; a pass is written on it.
  pub footing: Option<GateFooting>,
  /// The earlier pass on the same footing that left the gate out whole.
  pub earlier: Option<GatePass>,
}

/// The ground a deployment's gate stands on: the repository, the commit, the range's start and the footing the
/// gate found, its heads in order so the same trees read alike however they were laid.
pub fn gate_ground(
  repository: &str,
  commit: &str,
  since: Option<&str>,
  footing: &GateFooting,
) -> GateGround {
  let mut heads = footing.heads.clone();

  heads.sort();
  heads.dedup();

  GateGround {
    repository: repository.to_owned(),
    commit: commit.to_owned(),
    since: since.map(str::to_owned),
    heads,
    toolchains: footing.toolchains.clone(),
  }
}

/// The newest pass the record under a data root holds on the ground, written within the policy's hours before
/// the moment given; none where it holds none or cannot be read.
pub fn gate_passed(data_root: &Path, ground: &GateGround, now_ms: i64) -> Option<GatePass> {
  let since = now_ms.saturating_sub(DEPLOY.numbers.passed_gate_hours.saturating_mul(60 * 60 * 1000));

  Record::open(data_root)
    .and_then(|record| record.gate_passed(ground, since))
    .ok()
    .flatten()
}

/// The line saying the gate is left out for an earlier pass on the same ground.
pub fn gate_left_out_before(earlier: &GatePass) -> String {
  let said = &DEPLOY.run;
  let on = earlier
    .machine
    .as_deref()
    .map(|machine| fill_marks(&said.on, &[("machine", machine)]))
    .unwrap_or_default();

  fill_marks(
    &said.gate_passed_before,
    &[("commit", &short(&earlier.ground.commit)), ("when", &iso(earlier.at)), ("on", &on)],
  )
}

/// The passed run of a repository's whole suite on a commit that the record under a data root holds, begun
/// within the policy's hours before the moment given (docs/deploy.md, 「What the deploy run does, in
/// order」); none where it holds none, or cannot be read.
pub fn passed_suite(data_root: &Path, repository: &str, commit: &str, now_ms: i64) -> Option<TestRun> {
  let since = now_ms.saturating_sub(DEPLOY.numbers.passed_suite_hours.saturating_mul(60 * 60 * 1000));

  Record::open(data_root)
    .and_then(|record| record.passed_suite(repository, commit, since))
    .ok()
    .flatten()
}

/// The line saying the gate's suite is left out for a passed run: the run, when it began, where and its log.
pub fn suite_left_out(passed: &SuitePassed) -> String {
  let said = &DEPLOY.run;
  let run = &passed.run;
  let on = passed
    .machine
    .as_deref()
    .map(|machine| fill_marks(&said.on, &[("machine", machine)]))
    .unwrap_or_default();

  fill_marks(
    &said.suite_left_out,
    &[
      ("id", &run.id.to_string()),
      ("commit", &short(run.commit.as_deref().unwrap_or_default())),
      ("when", &iso(run.started_at)),
      ("checkout", &run.checkout),
      ("on", &on),
      ("log", &run.log_path),
    ],
  )
}

/// The Cargo release build asked of the gate: the commit, the build tree it is read from, the profile, the
/// binaries wanted, and the directory they are to stand in once built.
pub struct BuildAsk<'a> {
  pub root: &'a Path,
  pub commit: &'a str,
  pub tree: &'a Path,
  pub profile: &'a str,
  pub binaries: &'a [String],
  pub built: &'a Path,
}

/// Why the Cargo release build the gate ran on its own machine did not end built: it failed, with the exit
/// code it ended on where one names it and what was said, or the connection to that machine broke under it,
/// as the line naming the machine says.
#[derive(Debug, Clone, PartialEq)]
pub enum GateFault {
  Failed { code: Option<i64>, said: String },
  Broken(String),
}

/// The units a live release marks for the placed machine, asked of the gate: each by its unit name with the
/// release's text of it, and the timers to start there (docs/deploy.md, 「The unit links」).
pub struct LaidUnits<'a> {
  pub units: &'a [(String, String)],
  pub timers: &'a [String],
}

/// The gate a deployment passes before anything changes: the declared static check and the suite; and the
/// Cargo release build and the units marked for it, where the gate's machine is another.
pub trait Gate {
  fn pass(&self, ask: &GateAsk<'_>) -> GateOutcome;

  /// The Cargo release build run elsewhere, its binaries brought into the directory asked, or none where
  /// the build runs here.
  fn build(&self, _: &BuildAsk<'_>) -> Option<Result<(), GateFault>> {
    None
  }

  /// The units laid on the gate's machine and its timers started there: that machine's label, with what
  /// it said where they were not; none where the gate's machine is this one.
  fn lay_units(&self, _: &LaidUnits<'_>) -> Option<(String, Result<(), String>)> {
    None
  }

  /// The testbed the gate's machine runs brought to a live release that carries one, which must then name
  /// `commit` as the release it was built from (docs/deploy.md, 「Where a deployment runs」): that machine's
  /// label, with the files written there, or what it answered where the testbed there was not brought to it;
  /// none where the gate's machine is this one, the caller keeps it as it stands, or the release carries no
  /// testbed.
  fn bring_testbed(&self, _release: &Path, _commit: &str) -> Option<(String, Result<Vec<String>, String>)> {
    None
  }

  /// The suites of the repositories that build on this one, run where the change is major. A gate that keeps
  /// none passes.
  fn dependents(&self, _: &GateAsk<'_>) -> GateOutcome {
    GateOutcome {
      passed: true,
      ..GateOutcome::default()
    }
  }
}

/// The gate a caller that already ran it hands in: passed.
pub struct Passed;

impl Gate for Passed {
  fn pass(&self, _: &GateAsk<'_>) -> GateOutcome {
    GateOutcome {
      passed: true,
      ..GateOutcome::default()
    }
  }
}

/// A step of the run's own or a declared one: its id, its words, where it runs, and the lane whose turn it
/// takes. It runs until it ends, or, holding a lane's turn, until it writes nothing for the silence limit.
#[derive(Debug, Clone)]
pub struct StepAsk {
  pub id: String,
  pub command: Vec<String>,
  pub cwd: PathBuf,
  pub lane: Option<Lane>,
}

/// What a step came to: its code, 75 where its turn did not come; how long it waited and ran; and the start of
/// what it said.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRan {
  pub code: i64,
  pub waited_seconds: f64,
  pub ran_seconds: f64,
  pub said: String,
}

/// What one ask of a declared `quiet` command answered, by its exit code (docs/deploy.md, 「What the deploy run
/// does, in order」): 0 is no work under way, 1 is work under way, and anything else is a quiet that could not
/// tell, carrying how it ended.
enum QuietAnswer {
  Quiet,
  Busy,
  Unanswered(String),
}

impl QuietAnswer {
  fn of(asked: &Ran) -> Self {
    let said = &DEPLOY.run;

    match asked.code {
      Some(0) => Self::Quiet,
      Some(1) => Self::Busy,
      Some(code) => Self::Unanswered(fill_marks(&said.quiet_exit, &[("code", &code.to_string())])),
      None => Self::Unanswered(said.quiet_no_exit.clone()),
    }
  }
}

/// What the run does outside its own trees: commands run to their end, the service manager's among them;
/// heavy steps; its lines; the moment; and the live processes a prune spares.
pub trait DeployHands {
  /// A command run to its end in a directory, its output gathered.
  fn run(&self, line: &[String], cwd: &Path) -> Ran;
  /// A step run to its end, in its lane's turn where it names one, ended as hung there once it falls silent.
  fn step(&self, ask: &StepAsk) -> StepRan;
  fn say(&self, line: &str);
  fn now_ms(&self) -> i64;
  /// The live processes of this account; none where they could not be read.
  fn live_processes(&self) -> Option<Vec<LiveProcess>>;
  /// What the console probe asks through: the machine's own unless a test stands one in.
  fn console(&self) -> &dyn ConsoleHands {
    &MachineConsole
  }
}

/// The hands of the machine the run stands on.
pub struct Machine;

// A child's output read to its end on a thread of its own, written on as it comes and counted into `heard`.
fn gathered(
  mut from: impl std::io::Read + Send + 'static,
  heard: Arc<AtomicU64>,
) -> std::thread::JoinHandle<Vec<u8>> {
  std::thread::spawn(move || {
    let mut all = Vec::new();
    let mut chunk = [0u8; 16 * 1024];

    while let Ok(read) = from.read(&mut chunk) {
      if read == 0 {
        break;
      }

      heard.fetch_add(read as u64, Ordering::Relaxed);
      let _ = std::io::Write::write_all(&mut std::io::stderr(), &chunk[..read]);
      all.extend_from_slice(&chunk[..read]);
    }

    all
  })
}

impl DeployHands for Machine {
  fn run(&self, line: &[String], cwd: &Path) -> Ran {
    run_quiet(line, Some(cwd))
  }

  fn step(&self, ask: &StepAsk) -> StepRan {
    let mut env = process_env();

    // A deployment stands at the front of every lane's line (docs/testbed.md, 「The front of the line」).
    env.insert(FRONT_VARIABLE.to_owned(), "1".to_owned());

    let say = |line: &str| eprintln!("{line}");
    let turn = ask.lane.map(|lane| {
      take_turn(&TurnAsk {
        command: ask.id.clone(),
        lane,
        say: &say,
        path: None,
        env: env.clone(),
        checkout: ask.cwd.to_string_lossy().into_owned(),
        poll: None,
        notice_seconds: None,
      })
    });
    let waited_seconds = turn.as_ref().map_or(0.0, |turn| turn.waited_seconds());

    if let Some(turn) = turn.as_ref().filter(|turn| !turn.had()) {
      return StepRan {
        code: i64::from(NOT_RUN_EXIT_CODE),
        waited_seconds,
        ran_seconds: 0.0,
        said: turn.words(),
      };
    }

    let started = Instant::now();
    let heard = Arc::new(AtomicU64::new(0));
    // A step holding a lane's turn is watched for silence; one that holds none runs until it ends.
    let output = if turn.is_some() {
      Output::Read(Arc::clone(&heard))
    } else {
      Output::Files(Vec::new())
    };
    let watch = Watch::new(output, Silence::at(&ask.cwd));
    let mut command = crate::run::command(&ask.command, Some(&ask.cwd));

    command
      .envs(turn.as_ref().and_then(|turn| turn.held_variable()))
      .envs(watch.variable())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .process_group(0);

    let mut child = match command.spawn() {
      Ok(child) => child,
      Err(why) => {
        return StepRan {
          code: 127,
          waited_seconds,
          ran_seconds: 0.0,
          said: why.to_string(),
        };
      }
    };
    let out = gathered(child.stdout.take().expect("the output is piped"), Arc::clone(&heard));
    let err = gathered(child.stderr.take().expect("the errors are piped"), heard);
    // The step is waited for until it ends, however long that takes while it writes.
    let waited = watch.wait(&mut child, &ask.command.join(" "));
    let mut said = format!(
      "{}{}",
      String::from_utf8_lossy(&out.join().unwrap_or_default()),
      String::from_utf8_lossy(&err.join().unwrap_or_default())
    );

    // A step ended as hung says so first, where the start of what it said is kept.
    if let Some(hung) = &waited.hung {
      eprintln!("{hung}");
      said = format!("{hung}\n{said}");
    }

    StepRan {
      code: waited.code().map_or(1, i64::from),
      waited_seconds,
      ran_seconds: started.elapsed().as_secs_f64(),
      said: first_chars(said.trim(), DEPLOY.numbers.quoted_chars),
    }
  }

  fn say(&self, line: &str) {
    println!("{line}");
  }

  fn now_ms(&self) -> i64 {
    bmo_testbed_core::queue::now_ms()
  }

  fn live_processes(&self) -> Option<Vec<LiveProcess>> {
    live_processes(Path::new("/proc"))
  }
}

fn now_ms() -> i64 {
  bmo_testbed_core::queue::now_ms()
}

// Git's NUL-separated listing as paths; none where git refused.
fn listed(ran: Ran) -> Vec<String> {
  if !ran.ok {
    return Vec::new();
  }

  ran.out.split('\0').filter(|path| !path.is_empty()).map(str::to_owned).collect()
}

// A path the declaration names under the data root.
fn under_data(data_root: &Path, path: &Path) -> PathBuf {
  if path.is_absolute() { path.to_path_buf() } else { data_root.join(path) }
}

// The units of a commit's tracked files the declaration's globs reach, every one where it names none: its
// services, timers and slices.
fn unit_files(tracked: &[String], globs: Option<&[String]>) -> Vec<String> {
  let patterns: Vec<glob::Pattern> = globs
    .unwrap_or_default()
    .iter()
    .filter_map(|one| glob::Pattern::new(one).ok())
    .collect();

  tracked
    .iter()
    .filter(|path| [".service", ".timer", ".slice"].iter().any(|suffix| path.ends_with(suffix)))
    .filter(|path| globs.is_none() || patterns.iter().any(|pattern| pattern.matches(path)))
    .cloned()
    .collect()
}

/// The binary targets of a Cargo workspace, read off what `cargo metadata` prints, each once in name order.
pub fn workspace_binaries(metadata: &str) -> Result<Vec<String>, String> {
  let read: Value = serde_json::from_str(metadata).map_err(|why| why.to_string())?;
  let mut found: Vec<String> = read["packages"]
    .as_array()
    .into_iter()
    .flatten()
    .flat_map(|package| package["targets"].as_array().into_iter().flatten())
    .filter(|target| target["kind"].as_array().is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin")))
    .filter_map(|target| target["name"].as_str().map(str::to_owned))
    .collect();

  found.sort();
  found.dedup();
  Ok(found)
}

/// The Cargo release build of every package in the workspace, at the profile and target directory the
/// declaration names.
pub fn release_build(profile: &str, target: &Path) -> Vec<String> {
  let mut command =
    words(["cargo", "build", "--workspace", "--locked", "--profile", profile, "--target-dir"]);

  command.push(target.to_string_lossy().into_owned());
  command
}

// The services, through the run's hands.
struct Services<'a> {
  run: &'a Laying<'a>,
  bot: String,
  /// The binaries whose bytes differ between the failed release and the one rolled back to.
  differing: Vec<String>,
  bound: Vec<(String, String)>,
}

impl ServiceHands for Services<'_> {
  fn stop(&self) -> bool {
    self.run.systemctl(&["stop", &self.bot]).ok
  }

  fn restart(&self) -> bool {
    self.run.systemctl(&["daemon-reload"]);

    for unit in binary_restarts(&self.bound, &self.differing, &[]) {
      self.run.systemctl(&["restart", &unit]);
    }

    self.run.systemctl(&["reset-failed", &self.bot]);
    self.run.systemctl(&["restart", &self.bot]).ok
  }

  async fn answers(&self) -> bool {
    self.run.came_up(&self.bot).await.is_none()
  }

  fn say(&self, line: &str) {
    self.run.hands.say(line);
  }

  fn now_iso(&self) -> String {
    iso(self.run.hands.now_ms())
  }
}

/// A moment, epoch milliseconds, as an ISO 8601 time in UTC.
pub fn iso(at_ms: i64) -> String {
  chrono::DateTime::from_timestamp_millis(at_ms)
    .unwrap_or_default()
    .format("%Y-%m-%dT%H:%M:%S%.3fZ")
    .to_string()
}

// One run under way: what it was handed, and what it has learned.
struct Laying<'a> {
  /// The checkout's root with the declaration the run goes by: the checkout's until the commit is picked,
  /// the commit's own from then on.
  repo: Repository,
  /// The places the run lays its commit down in, the checkout's declaration's until the commit is picked and
  /// the commit's own from then on, which stand where the checkout's do.
  places: DeployPlaces,
  ask: &'a DeployAsk,
  hands: &'a dyn DeployHands,
  gate: &'a dyn Gate,
  hook: &'a Hook,
  progress: Option<DeployRun>,
  commit: String,
  subject: String,
  /// The deployed version: the release the commit's tag names, else the day's nightly; empty where the
  /// commit declares none.
  version: String,
  previous: Option<DeployRecord>,
  restarted_at: std::cell::Cell<i64>,
  /// Whether the run is rolling back, whose wait for the service is no stage of its own.
  rolling_back: std::cell::Cell<bool>,
  // The step after the health that failed the deployment, where one did.
  step_failed: std::cell::RefCell<Option<String>>,
  warnings: std::cell::RefCell<Vec<String>>,
}

impl<'a> Laying<'a> {
  fn say(&self, line: &str) {
    self.hands.say(line);

    if let Some(progress) = &self.progress {
      progress.said(line);
    }
  }

  fn warn(&self, line: String) {
    self.say(&fill_marks(&DEPLOY.run.warning, &[("warning", &line)]));
    self.warnings.borrow_mut().push(line);
  }

  fn git(&self, args: &[&str], cwd: &Path) -> Ran {
    let line: Vec<String> = std::iter::once("git").chain(args.iter().copied()).map(str::to_owned).collect();

    self.hands.run(&line, cwd)
  }

  fn systemctl(&self, args: &[&str]) -> Ran {
    if let (Some(&verb @ ("stop" | "restart")), Some(progress)) = (args.first(), &self.progress) {
      let said = &DEPLOY.restarts;
      let deployment = progress.deployment_id().map(|id| id.to_string()).unwrap_or_default();
      let reason = fill_marks(
        if self.rolling_back.get() { &said.rolling_back } else { &said.putting },
        &[
          ("deployment", &deployment),
          ("verb", verb),
          ("commit", &short(&self.commit)),
          ("repository", &self.repo.name()),
        ],
      );

      for unit in args[1..].iter().filter(|word| !word.starts_with('-')) {
        progress.restarting(unit, &reason);
      }
    }

    let line: Vec<String> = ["systemctl", "--user"]
      .into_iter()
      .chain(args.iter().copied())
      .map(str::to_owned)
      .collect();

    self.hands.run(&line, &self.places.source)
  }

  fn stage(&self, stage: Stage) {
    if let Some(progress) = &self.progress {
      progress.stage(stage);
    }
  }

  fn step_done(&self, name: &str, part: Option<Part>, code: i64, waited: f64, ran: f64) {
    if let Some(progress) = &self.progress {
      progress.step(name, part, code, waited, ran);
    }
  }

  // Work on the gate's machine asked again while the connection to that machine breaks under it, the ask's
  // pauses between the tries (docs/deploy.md, 「A connection that breaks under the placed machine」): its last
  // answer, with how many tries it took.
  fn tried<T>(
    &self,
    work: &str,
    mut ask: impl FnMut() -> T,
    broke: impl Fn(&T) -> Option<String>,
  ) -> (T, usize) {
    let pauses = &self.ask.retry;
    let tries = (pauses.len() + 1).to_string();
    let mut taken = 1;

    loop {
      let answer = ask();
      let (Some(said), Some(pause)) = (broke(&answer), pauses.get(taken - 1)) else {
        return (answer, taken);
      };

      self.say(&fill_marks(
        &DEPLOY.run.retrying,
        &[
          ("said", &said),
          ("work", work),
          ("seconds", &pause.as_secs().to_string()),
          ("next", &(taken + 1).to_string()),
          ("tries", &tries),
        ],
      ));
      std::thread::sleep(*pause);
      taken += 1;
    }
  }

  // A step run through the hands and written into the progress record as it begins and ends.
  fn step(&self, ask: &StepAsk) -> StepRan {
    if let Some(progress) = &self.progress {
      progress.began(&ask.id, None);
    }

    let ran = self.hands.step(ask);

    self.step_done(&ask.id, None, ran.code, ran.waited_seconds, ran.ran_seconds);
    ran
  }

  // A refusal: nothing was changed, and the caller hears why with the code of a refusal.
  fn refused(&self, line: String) -> u8 {
    self.say(&fill_marks(&DEPLOY.run.refused, &[("why", &line)]));

    if let Some(progress) = &self.progress {
      progress.refuse(&line);
    }

    REFUSED_EXIT_CODE
  }

  // The deployment ended before anything went live: said, written as its ending, then sent as
  // `deploy-failed`, so no hook stands between the deployment and its end.
  fn failed(&self, stage: Stage, line: String, outcome: Outcome) -> u8 {
    self.say(&line);

    if let Some(progress) = &self.progress {
      progress.end(outcome, Some(&line));
    }

    self.hook.send(&Event {
      name: "deploy-failed",
      commit: Some(self.commit.clone()),
      summary: line.clone(),
      detail: json!({
        "commit": self.commit,
        "stage": serde_json::to_value(stage).unwrap_or(Value::Null),
        "reason": line,
        "rolled_back": false,
        "journal": "",
      }),
    });

    1
  }

  // The release of every deployment of the deploy root still under way (docs/deploy.md, 「The releases」): the
  // progress record's run and the testbed's record's deployments of the repository, each unended with its
  // process alive; why not where the testbed's record cannot be read.
  fn releases_under_way(&self) -> Result<Vec<PathBuf>, String> {
    let recorded = match self.ask.record.as_deref() {
      Some(root) => Record::open(root)?.deployments_under_way(&self.repo.name())?,
      None => Vec::new(),
    };
    let progress = deploy_progress_at(&self.places.progress);
    let commits = progress
      .as_ref()
      .and_then(|standing| standing.under_way())
      .into_iter()
      .chain(recorded.iter().filter_map(|deployment| deployment.commit.as_deref()));

    Ok(commits.map(|commit| release_dir(&self.places.deploy_dir, commit)).collect())
  }

  // Every release but the ones kept, the ones of the deployments still under way and the ones a live process
  // stands in taken away, none where the deployments under way cannot be read; then the retired trees nothing
  // stands in.
  fn prune_releases(&self, mut keep: Vec<PathBuf>) {
    let processes = self.hands.live_processes();

    match self.releases_under_way() {
      Ok(under_way) => {
        keep.extend(under_way);
        self.prune_release_trees(keep, processes.as_deref());
      }
      Err(why) => self.warn(fill_marks(&DEPLOY.release.unread_under_way, &[("why", &why)])),
    }

    self.prune_retired_trees(processes.as_deref());
  }

  // Every release but the ones kept and the ones a live process stands in taken away; more releases left
  // standing than the policy allows is sent as `releases-pinned`.
  fn prune_release_trees(&self, mut keep: Vec<PathBuf>, processes: Option<&[LiveProcess]>) {
    let said = &DEPLOY.run;
    let places = &self.places;

    // The release the run itself runs from is never taken away under it.
    keep.extend(std::env::current_exe().ok().and_then(|exe| release_holding(&places.deploy_dir, &exe)));

    let git = |args: &[&str], cwd: &Path| self.git(args, cwd);
    let pruned = prune(&places.deploy_dir, &keep, processes, &places.source, &places.data_root, &git);

    if !pruned.removed.is_empty() {
      self.say(&fill_marks(&said.pruned, &[("names", &pruned.removed.join(", "))]));
    }

    for (release, holders) in &pruned.pinned {
      let pids: Vec<String> = holders.iter().map(|process| process.pid.to_string()).collect();

      self.say(&fill_marks(&said.kept_pinned, &[("release", release), ("pids", &pids.join(", "))]));
    }

    for warning in &pruned.warnings {
      self.warn(warning.clone());
    }

    if let Some(detail) = pinned_detail(&pruned) {
      self.hook.send(&Event {
        name: "releases-pinned",
        commit: None,
        summary: fill_marks(&said.pinned_summary, &[("count", &pruned.standing.to_string())]),
        detail,
      });
    }
  }

  // The retired trees nothing stands in taken away, none where the live processes cannot be read.
  fn prune_retired_trees(&self, processes: Option<&[LiveProcess]>) {
    let said = &DEPLOY.run;
    let places = &self.places;
    let git = |args: &[&str], cwd: &Path| self.git(args, cwd);
    let in_use = |tree: &Path| {
      processes.is_none_or(|processes| processes.iter().any(|process| process.stands_in(tree)))
    };
    let retired = prune_retired(&places.deploy_dir, &places.source, &places.data_root, &git, &in_use);

    if !retired.removed.is_empty() {
      self.say(&fill_marks(&said.retired_trees, &[("names", &retired.removed.join(", "))]));
    }

    for warning in &retired.warnings {
      self.warn(warning.clone());
    }
  }

  // The setup steps given run in order in the tree given, so a fresh tree holds what its check and its
  // release read.
  fn set_up_test_tree(&self, steps: &[String], tree: &Path) -> Result<(), String> {
    for setup in steps {
      let ran = self.hands.run(&shell_words(setup), tree);

      if !ran.ok {
        return Err(fill_marks(
          &DEPLOY.run.setup_failed,
          &[
            ("command", setup),
            ("release", &tree.to_string_lossy()),
            ("said", &first_chars(&ran.out, DEPLOY.numbers.quoted_chars)),
          ],
        ));
      }
    }

    Ok(())
  }

  // The checkout's root with the declaration the picked commit carries, read from the commit among its tracked
  // files: every default where it carries none, the refusal where it does not read.
  fn declared_at(&self, tracked: &[String]) -> Result<Repository, String> {
    let file = &DECLARATION.file;
    let declaration = if tracked.iter().any(|path| path == file) {
      let named = format!("{}:{file}", self.commit);
      let shown = self.git(&["show", &named], &self.places.source);

      if !shown.ok {
        return Err(fill_marks(
          &DECLARATION.refusal.unreadable,
          &[("file", &named), ("table", &DECLARATION.refusal.top_level), ("error", &shown.out)],
        ));
      }

      Declaration::parse(&named, &shown.out).map_err(|refusal| refusal.0)?
    } else {
      Declaration::default()
    };

    Ok(Repository {
      root: self.repo.root.clone(),
      declaration,
    })
  }

  // A release set up the way any checkout of the repository is: its submodules brought to their pins, the
  // declared setup steps, then the declared env links.
  fn set_up(&self, release: &Path) -> Result<(), String> {
    let run = |line: &[String], cwd: &Path| self.hands.run(line, cwd);

    bring_submodules(release, &self.places.data_root, &run)?;
    self.lay_committed_trees(release)?;
    self.set_up_test_tree(&self.repo.declaration.test.setup, release)?;

    for link in &self.repo.declaration.deploy.env_links {
      let at = release.join(&link.link);

      if let Some(parent) = at.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
      }

      put_link(&at, &under_data(&self.places.data_root, &link.target)).map_err(|why| why.to_string())?;
    }

    Ok(())
  }

  // The trees the workspace names outside the repository laid beside `tree` as links into their committed
  // trees, each repository put at its main head in the committed trees' folder (docs/deploy.md, 「The
  // releases」).
  fn lay_committed_trees(&self, tree: &Path) -> Result<Vec<String>, String> {
    let run = |line: &[String], cwd: &Path| self.hands.run(line, cwd);
    let store = committed_trees(&self.places.deploy_dir);

    lay_outside_trees(
      tree,
      &self.places.source,
      &TreeStore {
        store: &store,
        data_root: &self.places.data_root,
      },
      &run,
    )
  }

  // The build tree put at the commit with its outside trees laid at their committed heads, which the gate on
  // this machine and the release build read: its path, and the heads laid beside it.
  fn put_commit_tree(&self) -> Result<(PathBuf, Vec<String>), String> {
    let places = &self.places;
    let tree = expansion_tree(&places.deploy_dir);
    let run = |line: &[String], cwd: &Path| self.hands.run(line, cwd);

    put_build_tree(
      &BuildTree {
        source: &places.source,
        tree: tree.clone(),
        head: &self.commit,
        data_root: &places.data_root,
      },
      &run,
    )?;
    let heads = self.lay_committed_trees(&tree)?;

    Ok((tree, heads))
  }

  // The release built in the build tree at the commit: the Cargo release build where the commit has a Cargo
  // workspace, the declared build steps, the binaries placed, and `bin/built-from` written last. The
  // binaries whose bytes differ from the live release's; the line saying what failed where a step did.
  fn build(&self, release: &Path, live: Option<&Path>) -> Result<Vec<String>, String> {
    let said = &DEPLOY.run;
    let places = &self.places;
    let build = &self.repo.declaration.build;
    let (tree, _) = self.put_commit_tree()?;

    let profile = &build.cargo_profile;
    let target = build_target(&places.deploy_dir);
    let cargo = tree.join("Cargo.toml").exists();

    // Marked before anything is built into it, so the prune never reads it as a stray `target/` from here until
    // the binaries are placed, while no Cargo process works in the tree (docs/deploy.md, 「The releases」).
    mark_release_target(&target).map_err(|why| {
      fill_marks(
        &said.target_unmarked,
        &[("target", &target.to_string_lossy()), ("why", &why.to_string())],
      )
    })?;
    let failed_at = |id: &str, ran: &StepRan| {
      fill_marks(
        &said.build_failed,
        &[("step", id), ("code", &ran.code.to_string()), ("said", &ran.said)],
      )
    };

    let binaries = match (&build.binaries, cargo) {
      (Some(declared), _) => declared.clone(),
      (None, true) => {
        // What Cargo says of the workspace is its standard output alone, its warnings standing apart.
        let read = crate::run::command(
          &words(["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"]),
          Some(&tree),
        )
        .output()
        .map_err(|why| fill_marks(&said.metadata_failed, &[("why", &why.to_string())]))?;

        if !read.status.success() {
          let why = first_chars(String::from_utf8_lossy(&read.stderr).trim(), DEPLOY.numbers.quoted_chars);

          return Err(fill_marks(&said.metadata_failed, &[("why", &why)]));
        }

        workspace_binaries(&String::from_utf8_lossy(&read.stdout))
          .map_err(|why| fill_marks(&said.metadata_failed, &[("why", &why)]))?
      }
      (None, false) => Vec::new(),
    };
    // Cargo writes the `dev` profile under `debug`, every other under its own name.
    let built = target.join(if profile == "dev" { "debug" } else { profile.as_str() });

    if cargo {
      let step = OwnStep::CargoRelease.name();
      let ask = BuildAsk {
        root: &self.repo.root,
        commit: &self.commit,
        tree: &tree,
        profile,
        binaries: &binaries,
        built: &built,
      };
      let (elsewhere, tries) = self.tried(
        step,
        || self.gate.build(&ask),
        |built| match built {
          Some(Err(GateFault::Broken(said))) => Some(said.clone()),
          _ => None,
        },
      );

      match elsewhere {
        Some(Ok(())) => {}
        Some(Err(GateFault::Failed {
          code: Some(code),
          said: why,
        })) => {
          return Err(fill_marks(
            &said.build_failed,
            &[("step", step), ("code", &code.to_string()), ("said", &why)],
          ));
        }
        Some(Err(GateFault::Failed {
          code: None,
          said: why,
        })) => {
          return Err(fill_marks(&said.build_unfinished, &[("step", step), ("said", &why)]));
        }
        Some(Err(GateFault::Broken(broke))) => {
          let lost = fill_marks(&said.connection_lost, &[("said", &broke), ("tries", &tries.to_string())]);

          return Err(fill_marks(&said.build_unfinished, &[("step", step), ("said", &lost)]));
        }
        None => {
          let ask = StepAsk {
            id: OwnStep::CargoRelease.name().to_owned(),
            command: release_build(profile, &target),
            cwd: tree.clone(),
            lane: Some(Lane::Wide),
          };
          let cargo = || crate::run::command(&ask.command[..1], None);
          let alignment = align_path_packages(&tree, &ask.command[1..], &cargo);

          if let Some(said) = alignment.said() {
            self.say(&said);
          }

          let ran = self.step(&ask);

          alignment.put_back();

          if ran.code != 0 {
            return Err(failed_at(&ask.id, &ran));
          }
        }
      }
    }

    for one in &build.steps {
      let ask = StepAsk {
        id: one.id.clone(),
        command: shell_words(&one.command),
        cwd: one.dir.as_ref().map_or_else(|| tree.clone(), |dir| tree.join(dir)),
        lane: one.lane,
      };
      let ran = self.step(&ask);

      if ran.code != 0 {
        return Err(failed_at(&ask.id, &ran));
      }
    }

    let bin = release.join("bin");
    let changed = place_binaries(&binaries, &built, live.map(|live| live.join("bin")).as_deref(), &bin)
      .map_err(|why| fill_marks(&said.place_failed, &[("why", &why.to_string())]))?;

    mark_built_from(&bin, Some(&self.commit))
      .map_err(|why| fill_marks(&said.place_failed, &[("why", &why.to_string())]))?;
    self.say(&fill_marks(
      &said.placed,
      &[("count", &binaries.len().to_string()), ("changed", &changed.join(", "))],
    ));

    Ok(changed)
  }

  // A declared step of a phase run from the release, with its time; what it came to.
  fn declared_step(&self, step: &DeployStep, release: &Path) -> StepRan {
    self.step(&StepAsk {
      id: step.id.clone(),
      command: shell_words(&step.command),
      cwd: release.to_path_buf(),
      lane: None,
    })
  }

  // Whether the `bot` service came up: the declared health answered before the unit gave up starting and
  // before the declared bound passed, where one is declared, and the unit is read running; why not where it
  // did not, said with what was read of it.
  async fn came_up(&self, bot: &str) -> Option<BootFailure> {
    let said = &DEPLOY.run;
    let deploy = &self.repo.declaration.deploy;
    let run = |line: &[String]| self.hands.run(line, &self.places.source);
    let status = || unit_status(&run, bot);

    if let Some(health) = &deploy.health {
      let mut watch = HealthWatch::new(health, &self.places.deploy_dir, deploy.health_wait_seconds);
      let first = status();

      self.say(&fill_marks(&said.waiting, &[("health", health)]));
      watch.gave_up = Some(Box::new(move || unit_gave_up(first.as_ref(), status().as_ref())));

      let wait = wait_for_health(&watch).await;
      let failure = match wait.end {
        HealthEnd::Answered => None,
        HealthEnd::GaveUp => Some(BootFailure::NotAnswering),
        HealthEnd::RanOut => Some(BootFailure::RanOut),
      };

      if let Some(failure) = failure {
        for line in boot_report(&boot_scene(&run, bot, self.restarted_at.get(), wait)) {
          self.say(&line);
        }

        return Some(failure);
      }
    }

    self.say(&fill_marks(&said.watching, &[("unit", bot)]));

    if !self.rolling_back.get() {
      self.stage(Stage::StayUp);
    }

    if let StayUp::Fell { why, status } = stands_up(&status) {
      for line in stay_report(&run, bot, self.restarted_at.get(), &why, status) {
        self.say(&line);
      }

      return Some(BootFailure::DiedAfterStart);
    }

    None
  }

  // The service did not come up: the deploy tree rolled back onto the release that was live before where
  // that can be done, the facts sent to the hook, and the deployment failed whatever the rollback came to.
  async fn boot_failed(
    &self,
    failure: BootFailure,
    bot: &str,
    written: &DeployRecord,
    release: &Path,
    bound: &[(String, String)],
  ) -> u8 {
    let places = &self.places;
    let failed = FailedDeploy {
      commit: self.commit.clone(),
      failure,
      previous: self.previous.clone(),
      written: written.clone(),
      before_start: self.ask.before_start.is_some(),
      unit: bot.to_owned(),
      step: self.step_failed.borrow().clone(),
    };
    let said = |line: &str| self.say(line);
    let git = |args: &[&str], cwd: &Path| self.git(args, cwd);
    let mut tree = DeployTree {
      deploy_dir: places.deploy_dir.clone(),
      record_path: places.record.clone(),
      log_path: places.log.clone(),
      unit_dir: places.unit_dir.clone(),
      git: &git,
      say: &said,
    };
    let back = self.previous.as_ref().map(|previous| release_dir(&places.deploy_dir, &previous.commit));
    let services = Services {
      run: self,
      bot: bot.to_owned(),
      differing: back.map(|back| differing_binaries(release, &back)).unwrap_or_default(),
      bound: bound.to_vec(),
    };

    self.stage(Stage::Rollback);
    self.rolling_back.set(true);

    let outcome = roll_back(&failed, &mut tree, &services).await;
    let notice = deploy_failure_notice(&failed, &outcome);

    self.say(&notice);

    let rolled_back = matches!(outcome, Rollback::RolledBack { .. });
    let rolled_back_event = rolled_back_detail(&failed, &outcome);

    if let Some(progress) = &self.progress {
      if rolled_back_event.is_some() {
        progress.rolled_back();
      }

      progress.end(
        if rolled_back { Outcome::RolledBack } else { Outcome::Failed },
        Some(&failure_line(&failed)),
      );
    }

    // The facts go to the hook once the deployment's ending is written, so no hook stands between the
    // deployment and its end.
    if let Some(detail) = rolled_back_event {
      self.hook.send(&Event {
        name: "deploy-rolled-back",
        commit: Some(self.commit.clone()),
        summary: detail["reason"].as_str().unwrap_or_default().to_owned(),
        detail,
      });
    }

    let stage = match failure {
      BootFailure::DiedAfterStart => Stage::StayUp,
      BootFailure::StepFailed | BootFailure::ConsoleRefused => Stage::After,
      _ => Stage::Restart,
    };

    self.hook.send(&Event {
      name: "deploy-failed",
      commit: Some(self.commit.clone()),
      summary: failure_line(&failed),
      detail: json!({
        "commit": self.commit,
        "stage": serde_json::to_value(stage).unwrap_or(Value::Null),
        "reason": notice,
        "rolled_back": rolled_back,
        "journal": journal_command(&failed),
      }),
    });

    1
  }
}

// The release of the console layer the shared versions a declaration names pin, read as the caller reads them,
// which the console probe reads a manifest by and holds its layer to (docs/deploy.md, 「The console probe」).
fn pinned_layer(repo: &Repository, ask: &DeployAsk, env: &Env) -> PinnedLayer {
  match (&repo.declaration.check.shared_versions, ask.layer_pin) {
    (Some(shared), Some(read)) => read(&shared.repository, env),
    _ => PinnedLayer::Unnamed,
  }
}

// The checks before anything changes, each a refusal where it fails: no harness's shell, and the data root and
// the backup directory where they may stand.
fn place_refusal(places: &DeployPlaces, env: &Env) -> Option<String> {
  let said = &DEPLOY.run;
  let data = &places.data_root;
  let shown = |path: &Path| path.to_string_lossy().into_owned();

  if harness_pid(env).is_some() {
    return Some(said.harness.clone());
  }

  if !data.is_absolute() {
    return Some(fill_marks(&said.data_relative, &[("data", &shown(data))]));
  }

  if let Some(tree) = git_tree_above(data) {
    return Some(fill_marks(&said.data_in_tree, &[("data", &shown(data)), ("tree", &shown(&tree))]));
  }

  if !data.exists() {
    return Some(fill_marks(&said.data_missing, &[("data", &shown(data))]));
  }

  if let Some(backups) = &places.backup_dir {
    if !backups.is_absolute() || backups.starts_with(data) {
      return Some(fill_marks(
        &said.backup_in_data,
        &[("backup", &shown(backups)), ("data", &shown(data))],
      ));
    }

    if let Some(tree) = git_tree_above(backups) {
      return Some(fill_marks(
        &said.backup_in_tree,
        &[("backup", &shown(backups)), ("tree", &shown(&tree))],
      ));
    }
  }

  None
}

// The places the declaration a commit carries gives its deployment, where the deployment can run by it in the
// places the checkout's declaration gave: a refusal where it names the repository otherwise, stands the
// deployment elsewhere, deploys nothing, puts its backup directory where it may not stand, or links an env
// file that is missing.
fn declared_places(
  checkout: &Repository,
  declared: &Repository,
  places: &DeployPlaces,
  env: &Env,
  commit: &str,
) -> Result<DeployPlaces, String> {
  let said = &DEPLOY.run;
  let shown = |path: &Path| path.to_string_lossy().into_owned();
  let at_commit = DeployPlaces::of(declared, env)
    .map_err(|_| fill_marks(&said.commit_no_deploy, &[("commit", &short(commit))]))?;
  let elsewhere = declared.name() != checkout.name()
    || at_commit.deploy_root != places.deploy_root
    || at_commit.deploy_dir != places.deploy_dir
    || at_commit.data_root != places.data_root;

  if elsewhere {
    return Err(fill_marks(
      &said.commit_elsewhere,
      &[("commit", &short(commit)), ("source", &shown(&places.source))],
    ));
  }

  if let Some(refusal) = place_refusal(&at_commit, env) {
    return Err(refusal);
  }

  let missing = declared.declaration.deploy.env_links.iter().find_map(|link| {
    let target = under_data(&at_commit.data_root, &link.target);

    (!target.exists()).then(|| {
      fill_marks(
        &said.env_missing,
        &[("target", &shown(&target)), ("link", &link.link.to_string_lossy())],
      )
    })
  });

  missing.map_or(Ok(at_commit), Err)
}

/// The major of a version: its leading whole number, or none where it does not begin with one.
pub fn version_major(version: &str) -> Option<u64> {
  let version = version.trim().strip_prefix('v').unwrap_or(version.trim());
  let major = version.split('.').next()?;

  (!major.is_empty() && major.bytes().all(|byte| byte.is_ascii_digit()))
    .then(|| major.parse().ok())
    .flatten()
}

/// Whether a version is a major change from the one before it: both name a major and the majors differ. A
/// version absent on either side, as a first deployment has none before it, is not known to be major.
pub fn major_change(version: &str, before: &str) -> bool {
  matches!(
    (version_major(version), version_major(before)),
    (Some(now), Some(before)) if now != before
  )
}

/// The deploy run over a repository's declaration, in the places handed it; the code it ends with: 0 where
/// the commit went live, or would on a dry run, 1 where it failed or was rolled back, 2 where it was refused
/// with nothing changed.
pub async fn deploy(
  repo: &Repository,
  places: &DeployPlaces,
  ask: &DeployAsk,
  env: &Env,
  gate: &dyn Gate,
  hands: &dyn DeployHands,
  hook: &Hook,
) -> u8 {
  laid(repo, places, ask, env, gate, hands, hook, None).await
}

// The commit a deployment waiting for the deploy root's turn asks for that carries the one in hand, the furthest
// on where several do (docs/deploy.md, 「The deployment line」). The remote is read again first, since a commit
// asked for after this deployment began may not be in the checkout yet.
fn later_asked(run: &Laying<'_>, lock: &Path, commit: &str) -> Option<String> {
  let source = &run.places.source;
  let asked = waiting_commits(lock, &run.repo.name());

  if asked.is_empty() {
    return None;
  }

  let _ = run.git(&["fetch", "--quiet", "origin"], source);
  let carries =
    |later: &str, earlier: &str| run.git(&["merge-base", "--is-ancestor", earlier, later], source).ok;
  // A commit whose newest ended deployment failed in its gate would fail there again.
  let failed = |later: &str| {
    run
      .ask
      .record
      .as_deref()
      .is_some_and(|data_root| failed_gate(data_root, &run.repo.name(), later, 0).is_some())
  };

  asked
    .iter()
    .filter_map(|named| {
      let resolved = run.git(&["rev-parse", "--verify", "--quiet", &format!("{named}^{{commit}}")], source);

      resolved.ok.then(|| resolved.out.trim().to_owned())
    })
    .filter(|later| later != commit && carries(later, commit) && !failed(later))
    .fold(None, |furthest: Option<String>, later| match furthest {
      Some(furthest) if !carries(&later, &furthest) => Some(furthest),
      _ => Some(later),
    })
}

// The deploy run itself, under its deploy root's turn where one is handed in: a run that took the commit of a
// deployment waiting behind it starts over on that commit, holding the turn it already has.
#[allow(clippy::too_many_arguments)]
async fn laid(
  repo: &Repository,
  places: &DeployPlaces,
  ask: &DeployAsk,
  env: &Env,
  gate: &dyn Gate,
  hands: &dyn DeployHands,
  hook: &Hook,
  turn: Option<HeldLock>,
) -> u8 {
  let said = &DEPLOY.run;
  let original = places.clone();
  let mut run = Laying {
    repo: repo.clone(),
    places: places.clone(),
    ask,
    hands,
    gate,
    hook,
    progress: None,
    commit: String::new(),
    subject: String::new(),
    version: String::new(),
    previous: None,
    restarted_at: std::cell::Cell::new(0),
    rolling_back: std::cell::Cell::new(false),
    step_failed: std::cell::RefCell::new(None),
    warnings: std::cell::RefCell::new(Vec::new()),
  };

  // 1. Where it stands.
  if let Some(refusal) = place_refusal(places, env) {
    return run.refused(refusal);
  }

  // 2. The commit, on the remote.
  let source = &places.source;
  let fetched = run.git(&["fetch", "--quiet", "origin"], source);

  if !fetched.ok {
    return run.refused(fill_marks(
      &said.fetch_failed,
      &[("said", &first_chars(&fetched.out, DEPLOY.numbers.quoted_chars))],
    ));
  }

  let named = ask.commit.as_deref().unwrap_or("HEAD");
  let resolved = run.git(&["rev-parse", "--verify", "--quiet", &format!("{named}^{{commit}}")], source);

  if !resolved.ok {
    return run.refused(fill_marks(&said.no_commit, &[("commit", named)]));
  }

  run.commit = resolved.out.trim().to_owned();

  let dirty: Vec<String> = if ask.commit.is_none() {
    run
      .git(&["status", "--porcelain", "--untracked-files=no"], source)
      .out
      .lines()
      .filter_map(|line| line.get(3..))
      .map(str::to_owned)
      .collect()
  } else {
    Vec::new()
  };
  let on_remote = !run
    .git(&["branch", "--remotes", "--contains", &run.commit], source)
    .out
    .trim()
    .is_empty();

  if let Some(fault) = tree_fault(&dirty, &run.commit, on_remote, &said.remote) {
    return run.refused(fault);
  }

  run.subject = run.git(&["log", "-1", "--format=%s", &run.commit], source).out.trim().to_owned();
  // The deployed version (docs/deploy.md, 「The deployed version」): the release the commit's tag names, else the
  // day's nightly after the version the commit declares; a commit declaring none deploys all the same.
  let declared = declared_version(&|file: &str| {
    let shown = run.git(&["show", &format!("{}:{file}", run.commit)], source);

    shown.ok.then_some(shown.out)
  });
  let tags: Vec<String> = run
    .git(&["tag", "--points-at", &run.commit], source)
    .out
    .lines()
    .map(str::to_owned)
    .collect();

  run.version = deployed_version(&declared, &tags, platform_day(hands.now_ms()));
  run.previous = read_deploy_record(&places.record);

  let commit = run.commit.clone();
  let shown = short(&commit);

  run.say(&fill_marks(&said.deploying, &[("commit", &shown), ("subject", &run.subject)]));

  if run.previous.as_ref().is_some_and(|previous| previous.commit == commit) {
    run.say(&fill_marks(&said.again, &[("commit", &shown)]));
  }

  // The unit links, judged against the commit while a refusal still changes nothing.
  let release = release_dir(&places.deploy_dir, &commit);
  let tracked = listed(run.git(&["ls-tree", "-r", "-z", "--name-only", &commit], source));

  // The commit's own declaration, which everything the run does with the commit goes by from here on; the
  // checkout's still answers the gate, which judges the checkout.
  let declared = match run.declared_at(&tracked) {
    Ok(declared) => declared,
    Err(why) => return run.refused(why),
  };

  match declared_places(repo, &declared, places, env, &commit) {
    Ok(at_commit) => run.places = at_commit,
    Err(why) => return run.refused(why),
  }

  run.repo = declared;

  // The checkout the deployment was asked from may go while it waits or runs, as a job's does once its work is
  // merged: from here on the repository is read through its primary checkout (docs/deploy.md, 「The releases」).
  let primary = primary_checkout(&run.places.source, &|args, cwd| run.git(args, cwd));

  if primary != run.places.source {
    run.say(&fill_marks(&said.primary, &[("checkout", &primary.to_string_lossy())]));
  }

  run.places.source = primary;

  let checkout = repo;
  let repo = &run.repo;
  let places = &run.places;
  let source = &places.source;
  let links = unit_links(&places.unit_dir);
  let (relinks, faults) = unit_link_plan(&links, &places.deploy_dir, &tracked, &|path| {
    !tracked.iter().any(|one| one == path) && release.join(path).exists()
  });

  if !faults.is_empty() {
    return run.refused(fill_marks(
      &said.unit_faults,
      &[("faults", &faults.join("; ")), ("dir", &places.unit_dir.to_string_lossy())],
    ));
  }

  if ask.dry_run {
    let live = run
      .previous
      .as_ref()
      .map_or_else(|| said.nothing_live.clone(), |previous| short(&previous.commit));

    run.say(&fill_marks(&said.dry, &[("commit", &shown), ("live", &live)]));
    return 0;
  }

  // The deploy root's turn and then the deployment line, once nothing is left to refuse, both taken before the
  // progress record is begun so two deployments of one repository never write one record at once; then what
  // the deployment before it put live, read again (docs/deploy.md, 「The deployment line」). A run carrying on
  // the one whose unit it stands in takes no turn of its own: that run holds it.
  let ticket = DeployTicket {
    first: ask.first.clone(),
    before_start: ask.before_start.is_some(),
    root: Some(places.deploy_root.to_string_lossy().into_owned()),
    ..DeployTicket::asked(&repo.name(), &commit, ask.unit.as_deref())
  };
  let carried =
    deploy_progress_at(&places.progress).is_some_and(|standing| standing.carried_in(ask.unit.as_deref()));
  let root = RootTurn {
    poll: ask.root_poll,
    ..RootTurn::under(&places.deploy_root)
  };
  let taken_now = turn.is_none() && !carried;

  // The deploy root's turn enrolled beside the line, so the line's service order reads who waits for it.
  if taken_now && let Some(line) = &ask.line {
    line.enrol(&root.lock);
  }

  let root_turn = match turn {
    Some(held) => Some(held),
    None => (!carried).then(|| root.take(&ticket, &|line| run.say(line))).flatten(),
  };

  // A deployment waiting behind this one asks for a commit that carries it: that commit is put live in its
  // place, read afresh under the turn already held. A gate left out stands on a pass of this commit alone.
  if taken_now
    && !ask.no_check
    && let Some(later) = later_asked(&run, &root.lock, &commit)
  {
    run.say(&fill_marks(&said.joined, &[("commit", &shown), ("later", &short(&later))]));

    let joined = DeployAsk {
      commit: Some(later),
      ..ask.clone()
    };

    return Box::pin(laid(checkout, &original, &joined, env, gate, hands, hook, root_turn)).await;
  }

  let _root_turn = root_turn;
  let line = ask.line.as_ref().and_then(|line| line.take(&ticket, &|line| run.say(line)));

  // A gate of this very commit that failed while this deployment waited would fail again: it ends here, before
  // its gate, with nothing written, since that failure was told already.
  if !ask.no_check
    && let Some(failed) = ask.record.as_deref().and_then(|data_root| {
      failed_gate(data_root, &repo.name(), &commit, ticket.asked_at.unwrap_or_default())
    })
  {
    drop(line);

    return run.refused(fill_marks(
      &said.failed_waiting,
      &[
        ("commit", &shown),
        ("unit", failed.unit_name.as_deref().unwrap_or_default()),
        ("reason", failed.reason.as_deref().unwrap_or_default()),
      ],
    ));
  }

  run.previous = read_deploy_record(&places.record);

  // A commit already live when the turn comes is what this deployment would lay down: it ends there, live.
  let already_live = run.previous.as_ref().is_some_and(|previous| previous.commit == commit);

  // The deploy root holds the run's own record, log and progress file; the first deployment makes it
  // before the run begins writing there.
  let _ = std::fs::create_dir_all(&places.deploy_root);

  run.progress = Some(begin_deploy(
    &places.progress,
    &DeployStart {
      unit: ask.unit.clone(),
      pid: i64::from(std::process::id()),
      commit: Some(commit.clone()),
      plan: Stage::PLANNED.to_vec(),
      now: hands.now_ms(),
      repository: Some(repo.name()),
      record: ask.record.clone(),
    },
    now_ms,
  ));

  if already_live {
    run.say(&fill_marks(&said.already_live, &[("commit", &shown)]));

    if let Some(progress) = &run.progress {
      progress.end(Outcome::Live, None);
    }

    drop(line);
    return 0;
  }

  // The deploy tree still a directory, taken into the release layout first.
  let git = |args: &[&str], cwd: &Path| run.git(args, cwd);
  let stamp = crate::run::stamp_for_name(hands.now_ms());
  let preparing = |at: &str| {
    prepare(
      &Preparing {
        source,
        deploy_dir: &places.deploy_dir,
        commit: at,
        data_root: &places.data_root,
      },
      &git,
      &|release| run.set_up(release),
      &|line| run.say(line),
    )
    .map(|prepared| prepared.release)
  };
  let adopted = adopt(
    &Adopting {
      source,
      deploy_dir: &places.deploy_dir,
      data_root: &places.data_root,
      stamp: &stamp,
    },
    &git,
    &preparing,
    &|line| run.say(line),
  );

  match adopted {
    Ok(Some(adopted)) => run.say(&fill_marks(
      &said.adopted,
      &[
        ("dir", &places.deploy_dir.to_string_lossy()),
        ("release", &adopted.release.to_string_lossy()),
        ("retired", &adopted.retired.to_string_lossy()),
      ],
    )),
    Ok(None) => {}
    Err(why) => {
      return run.failed(Stage::Check, fill_marks(&said.adopt_failed, &[("why", &why)]), Outcome::Failed);
    }
  }

  let live = live_release(&places.deploy_dir);

  // 3. The gate.
  run.stage(Stage::Check);

  // A gate's suite runs in this process, and its fence moves the process into a scope of its own; where it
  // stood before the gate is captured, so it is put back there once the gate answers and everything it says
  // afterwards, a failed stage's line with it, is read from the deployment unit's journal.
  let unit_cgroup = ask.unit.is_some().then(own_cgroup).flatten();
  let mut flaky = Vec::new();

  if ask.no_check || !checkout.declaration.deploy.gate {
    run.say(&said.gate_left_out);
  } else {
    // The gate on this machine reads the commit's committed trees, never the checkout's working tree: the
    // build tree put at the commit with its outside trees laid at their committed heads, set up by the
    // declared steps so a fresh tree holds the packages its check needs (docs/deploy.md, 「What the deploy run
    // does, in order」).
    let tree = run.put_commit_tree().and_then(|(tree, heads)| {
      run.set_up_test_tree(&checkout.declaration.test.setup, &tree).map(|()| (tree, heads))
    });
    let (tree, heads) = match tree {
      Ok(tree) => tree,
      Err(why) => {
        return run.failed(
          Stage::Check,
          fill_marks(&said.gate_failed, &[("commit", &shown), ("reason", &why)]),
          Outcome::Stopped,
        );
      }
    };

    let since = run.previous.as_ref().map(|previous| previous.commit.clone());
    // A passed run of the whole suite on the commit, in this machine's own record, leaves the gate's suite
    // out; the gate asks its own machine's record where it stands elsewhere.
    let passed_here = ask
      .record
      .as_deref()
      .and_then(|data_root| passed_suite(data_root, &repo.name(), &commit, hands.now_ms()));

    if let Some(found) = &passed_here {
      run.say(&suite_left_out(&SuitePassed {
        run: found.clone(),
        machine: None,
      }));
    }

    // An earlier pass on the footing the gate finds leaves it out whole; this machine's record holds it, wherever
    // the gate ran.
    let ground_of = |footing: &GateFooting| gate_ground(&repo.name(), &commit, since.as_deref(), footing);
    let earlier = |footing: &GateFooting| {
      ask
        .record
        .as_deref()
        .and_then(|data_root| gate_passed(data_root, &ground_of(footing), hands.now_ms()))
    };

    // The gate is asked again while the connection to its machine breaks under it, every try's steps written.
    let (passed, tries) = run.tried(
      &said.gate_work,
      || {
        let passed = gate.pass(&GateAsk {
          root: source,
          tree: &tree,
          commit: &commit,
          since: since.as_deref(),
          suite: passed_here.is_none(),
          heads: &heads,
          earlier: &earlier,
        });

        if let Some(cgroup) = &unit_cgroup {
          let _ = move_to_cgroup(cgroup);
        }

        for step in &passed.steps {
          run.step_done(&step.name, None, step.code, step.waited_seconds, step.ran_seconds);
        }

        passed
      },
      |passed| passed.broken.then(|| passed.reason.clone()),
    );

    if let Some(found) = &passed.suite_passed {
      run.say(&suite_left_out(found));
    }

    if let Some(earlier) = &passed.earlier {
      run.say(&gate_left_out_before(earlier));
    } else if let (true, Some(footing), Some(data_root)) = (passed.passed, &passed.footing, &ask.record) {
      let pass = GatePass {
        ground: ground_of(footing),
        machine: footing.machine.clone(),
        at: hands.now_ms(),
      };

      if let Err(why) = Record::open(data_root).and_then(|mut record| record.write_gate_pass(&pass)) {
        say_unwritten(&why);
      }
    }

    if !passed.passed {
      let reason = if passed.broken {
        fill_marks(&said.connection_lost, &[("said", &passed.reason), ("tries", &tries.to_string())])
      } else {
        passed.reason
      };

      return run.failed(
        Stage::Check,
        fill_marks(&said.gate_failed, &[("commit", &shown), ("reason", &reason)]),
        Outcome::Stopped,
      );
    }

    for (test, unit) in &passed.flaky {
      run.say(&fill_marks(&said.flaky, &[("test", test), ("unit", unit)]));
    }

    flaky = passed.flaky;
  }

  // A major change runs the suites of the repositories that build on this one, before anything changes.
  let previous_version = run.previous.as_ref().map(|previous| previous.version.clone()).unwrap_or_default();

  if major_change(&run.version, &previous_version) {
    run.say(&fill_marks(&said.major, &[("version", &run.version), ("previous", &previous_version)]));

    let passed = gate.dependents(&GateAsk {
      root: source,
      tree: &expansion_tree(&places.deploy_dir),
      commit: &commit,
      since: run.previous.as_ref().map(|previous| previous.commit.as_str()),
      suite: true,
      heads: &[],
      earlier: &none_earlier,
    });

    if let Some(cgroup) = &unit_cgroup {
      let _ = move_to_cgroup(cgroup);
    }

    for step in &passed.steps {
      run.step_done(&step.name, None, step.code, step.waited_seconds, step.ran_seconds);
    }

    if !passed.passed {
      return run.failed(
        Stage::Check,
        fill_marks(&said.gate_failed, &[("commit", &shown), ("reason", &passed.reason)]),
        Outcome::Stopped,
      );
    }
  }

  // 4. The backup, before anything changes.
  run.stage(Stage::Backup);

  let backup = match (&places.backup_dir, &repo.declaration.deploy.backup) {
    (Some(backups), Some(declared)) => {
      let taken: Vec<String> = std::fs::read_dir(backups)
        .map(|entries| {
          entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
        })
        .unwrap_or_default();
      let name = declared.name.clone().unwrap_or_else(|| repo.name());
      let Some(file) = backup_name(&name, &backup_stamp(chrono::Local::now()), &taken) else {
        return run.failed(Stage::Backup, said.backup_unnamed.clone(), Outcome::Failed);
      };
      let plan = BackupPlan {
        data_root: &places.data_root,
        backup_dir: backups,
        name: &file,
        snapshot: declared.snapshot.as_deref(),
        verify: declared.verify.as_deref(),
        exclude: &declared.exclude,
        cwd: source,
      };
      let remove = |path: &Path| {
        if let Err(why) = remove_tree(path, &places.data_root) {
          run.warn(why);
        }
      };
      let timed = |part: Part, seconds: f64, done: bool| {
        let step = match part {
          Part::Snapshot => OwnStep::BackupSnapshot,
          Part::Pack => OwnStep::BackupPack,
          Part::Check => OwnStep::BackupCheck,
        };

        run.step_done(step.name(), Some(part), i64::from(!done), 0.0, seconds);
      };

      match take_backup(&plan, &remove, &timed) {
        BackupTaken::Taken {
          path,
          bytes,
          warnings,
          ..
        } => {
          if !warnings.is_empty() {
            run.say(&warnings);
          }

          run.say(&fill_marks(
            &said.backup_taken,
            &[("path", &path.to_string_lossy()), ("bytes", &bytes.to_string())],
          ));
          file
        }
        BackupTaken::Failed { failure, left, .. } => {
          return run.failed(
            Stage::Backup,
            fill_marks(&said.backup_failed, &[("failure", &failure), ("left", left.said())]),
            Outcome::Failed,
          );
        }
      }
    }
    _ => {
      run.say(&said.backup_none);
      String::new()
    }
  };

  // 5. The release, the releases pruned first.
  run.stage(Stage::Tree);

  let record_path = places.record.clone();
  let log_path = places.log.clone();
  let before = run.previous.as_ref().and_then(|previous| {
    std::fs::read_to_string(&log_path)
      .ok()
      .and_then(|log| logged_before(&log, &previous.commit))
  });

  run.prune_releases(
    [
      Some(release.clone()),
      live.clone(),
      before.map(|commit| release_dir(&places.deploy_dir, &commit)),
    ]
    .into_iter()
    .flatten()
    .collect(),
  );

  let prepared = match prepare(
    &Preparing {
      source,
      deploy_dir: &places.deploy_dir,
      commit: &commit,
      data_root: &places.data_root,
    },
    &git,
    &|release| run.set_up(release),
    &|line| run.say(line),
  ) {
    Ok(prepared) => prepared,
    Err(why) => {
      return run.failed(
        Stage::Tree,
        fill_marks(&said.prepare_failed, &[("commit", &shown), ("why", &why)]),
        Outcome::Failed,
      );
    }
  };

  run.say(&fill_marks(
    &said.prepared,
    &[
      ("release", &release.to_string_lossy()),
      ("how", if prepared.made { &said.made } else { &said.taken }),
    ],
  ));

  // 6. The build, for a release made afresh; a taken one compared with the live one.
  run.stage(Stage::Build);

  let changed = if prepared.made {
    match run.build(&release, live.as_deref()) {
      Ok(changed) => changed,
      Err(why) => return run.failed(Stage::Build, why, Outcome::Failed),
    }
  } else {
    live.as_deref().map(|live| differing_binaries(live, &release)).unwrap_or_default()
  };

  // The release is built, so the next deployment's gate may begin while this one restarts.
  drop(line);

  // 7. The steps before the flip.
  run.stage(Stage::Before);

  for step in repo.declaration.deploy.steps.iter().filter(|step| step.phase == Phase::BeforeFlip) {
    let ran = run.declared_step(step, &release);

    if ran.code != 0 {
      return run.failed(
        Stage::Before,
        fill_marks(
          &said.before_failed,
          &[("step", &step.id), ("code", &ran.code.to_string()), ("said", &ran.said)],
        ),
        Outcome::Failed,
      );
    }
  }

  // 8. The wait for quiet, the last step before the record, so its answer stands seconds before the stop. It
  // waits until no work stands under way, however long that takes, and fails once the quiet could not tell for
  // the policy's asks running; `--force-live` goes on at once.
  if let Some(quiet) = &repo.declaration.deploy.quiet {
    let line = shell_words(quiet);
    let asks = DEPLOY.numbers.quiet_unanswered_asks;
    let mut unanswered = 0;

    run.say(&fill_marks(&said.quiet_asking, &[("quiet", quiet)]));

    loop {
      let asked = hands.run(&line, &release);
      let named = first_chars(asked.out.trim(), DEPLOY.numbers.quoted_chars);

      match QuietAnswer::of(&asked) {
        QuietAnswer::Quiet => break,
        QuietAnswer::Busy if run.ask.force_live => {
          run.warn(fill_marks(&said.quiet_forced, &[("said", &named)]));
          break;
        }
        QuietAnswer::Busy => {
          unanswered = 0;
          run.say(&fill_marks(&said.quiet_live, &[("said", &named)]));
        }
        QuietAnswer::Unanswered(ended) if run.ask.force_live => {
          run.warn(fill_marks(&said.quiet_forced_unanswered, &[("ended", &ended), ("said", &named)]));
          break;
        }
        QuietAnswer::Unanswered(ended) => {
          unanswered += 1;

          if unanswered >= asks {
            return run.failed(
              Stage::Before,
              fill_marks(
                &said.quiet_failed,
                &[("asks", &asks.to_string()), ("ended", &ended), ("said", &named)],
              ),
              Outcome::Failed,
            );
          }

          run.say(&fill_marks(
            &said.quiet_unanswered,
            &[
              ("ended", &ended),
              ("ask", &unanswered.to_string()),
              ("asks", &asks.to_string()),
              ("said", &named),
            ],
          ));
        }
      }

      tokio::time::sleep(Duration::from_millis(DEPLOY.numbers.quiet_poll_ms)).await;
    }
  }

  // 9. The record, written just before the flip.
  let message = run.ask.message.clone().unwrap_or_else(|| run.subject.clone());
  let record = DeployRecord {
    commit: commit.clone(),
    subject: run.subject.clone(),
    message,
    version: run.version.clone(),
    at: iso(hands.now_ms()),
    backup: backup.clone(),
  };
  let put_back = || match &run.previous {
    Some(previous) => {
      let _ = write_deploy_record(&record_path, previous);
    }
    None => {
      let _ = std::fs::remove_file(&record_path);
    }
  };

  if let Err(why) = write_deploy_record(&record_path, &record) {
    return run.failed(
      Stage::Restart,
      fill_marks(&said.unrecorded, &[("why", &why.to_string())]),
      Outcome::Failed,
    );
  }

  let line = format!("{} {} {} {}\n", record.at, record.commit, record.backup, record.subject);
  let _ = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(&log_path)
    .and_then(|mut log| std::io::Write::write_all(&mut log, line.as_bytes()));

  run.say(&fill_marks(&said.recorded, &[("path", &record_path.to_string_lossy())]));

  // 10. The flip, every link into the deploy tree first confirmed to resolve in the release.
  let relinked: Vec<&str> = relinks.iter().map(|one| one.name.as_str()).collect();
  let unresolved: Vec<String> = links
    .iter()
    .filter(|link| !relinked.contains(&link.name.as_str()))
    .filter_map(|link| {
      let path = link.target.strip_prefix(&places.deploy_dir).ok()?;

      (!path.as_os_str().is_empty() && !release.join(path).exists()).then(|| link.name.clone())
    })
    .collect();

  if !unresolved.is_empty() {
    put_back();
    return run.failed(
      Stage::Restart,
      fill_marks(&said.unresolved, &[("links", &unresolved.join(", "))]),
      Outcome::Failed,
    );
  }

  if let Err(why) = flip(&places.deploy_dir, &release) {
    put_back();
    return run.failed(
      Stage::Restart,
      fill_marks(&said.flip_failed, &[("release", &release.to_string_lossy()), ("why", &why.to_string())]),
      Outcome::Failed,
    );
  }

  run.say(&fill_marks(&said.flipped, &[("release", &release.to_string_lossy())]));

  let dangling = relink_units(&places.unit_dir, &places.deploy_dir, &relinks).unwrap_or_default();

  for one in &relinks {
    run.say(&fill_marks(&said.relinked, &[("name", &one.name), ("to", &one.to.to_string_lossy())]));
  }

  if !dangling.is_empty() {
    return run.failed(
      Stage::Restart,
      fill_marks(&said.dangling, &[("names", &dangling.join(", "))]),
      Outcome::Failed,
    );
  }

  // 11. The units linked, and the services restarted.
  run.stage(Stage::Restart);

  let deploy_dir = &places.deploy_dir;
  let files = unit_files(
    &listed(run.git(&["ls-files", "-z"], deploy_dir)),
    repo.declaration.deploy.units.as_deref(),
  );
  let read = |path: &str| std::fs::read_to_string(deploy_dir.join(path)).unwrap_or_default();
  let marked = |suffix: &str, mark: UnitMark| marked_units(&files, &read, suffix, mark);
  let bots = marked(".service", UnitMark::Bot);
  let restarted = marked(".service", UnitMark::Restart);
  let bound = marked(".service", UnitMark::Binary);
  let timers = marked(".timer", UnitMark::Timer);
  // A slice is linked alone too: the units placed in it start it.
  let linked_only: Vec<String> = [".service", ".slice"]
    .into_iter()
    .flat_map(|suffix| marked(suffix, UnitMark::Link))
    .collect();
  let declared: Vec<String> = restarted
    .iter()
    .chain(&bound)
    .chain(&timers)
    .cloned()
    .chain(timers.iter().map(|timer| timer_service(timer)))
    .chain(linked_only.iter().cloned())
    .chain(bots.iter().cloned())
    .collect();

  for name in set_aside_plain_units(&places.unit_dir, &declared) {
    run.say(&fill_marks(&said.set_aside, &[("name", &name)]));
  }

  let installed: Vec<String> = std::fs::read_dir(&places.unit_dir)
    .map(|entries| {
      entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
    })
    .unwrap_or_default();
  let retired = retired_units(&files, &read, &installed);
  let installs = units_to_link(&declared, deploy_dir, &installed);

  if let Err(why) = link_units(&places.unit_dir, &installs) {
    return run.failed(
      Stage::Restart,
      fill_marks(&said.link_failed, &[("why", &why.to_string())]),
      Outcome::Failed,
    );
  }

  for one in &installs {
    run.say(&fill_marks(&said.linked, &[("name", &one.name), ("to", &one.to.to_string_lossy())]));
  }

  let repointed = outside_relinks(&unit_links(&places.unit_dir), &declared, deploy_dir);
  let _ = relink_units(&places.unit_dir, deploy_dir, &repointed);

  for one in &repointed {
    run.say(&fill_marks(&said.relinked, &[("name", &one.name), ("to", &one.to.to_string_lossy())]));
  }

  let linked_now: Vec<String> = installs
    .iter()
    .map(|one| one.name.clone())
    .chain(repointed.iter().map(|one| one.name.clone()))
    .collect();

  run.systemctl(&["daemon-reload"]);

  for timer in timers.iter().map(|path| base_name(path)) {
    if !run.systemctl(&["enable", "--now", &timer]).ok {
      run.warn(fill_marks(&said.timer_failed, &[("timer", &timer)]));
    }
  }

  for unit in restarted.iter().map(|path| base_name(path)) {
    run.systemctl(&["enable", &unit]);

    if run.systemctl(&["restart", &unit]).ok {
      run.say(&fill_marks(&said.restarted, &[("unit", &unit)]));
    } else {
      run.warn(fill_marks(&said.restart_failed, &[("unit", &unit)]));
    }
  }

  let bound_units: Vec<(String, String)> = bound.iter().map(|path| (base_name(path), read(path))).collect();
  let due = binary_restarts(&bound_units, &changed, &linked_now);

  for (unit, _) in &bound_units {
    run.systemctl(&["enable", unit]);

    if !due.contains(unit) {
      run.say(&fill_marks(&said.left_running, &[("unit", unit)]));
    } else if run.systemctl(&["restart", unit]).ok {
      run.say(&fill_marks(&said.restarted, &[("unit", unit)]));
    } else {
      run.warn(fill_marks(&said.restart_failed, &[("unit", unit)]));
    }
  }

  let bot = places.bot_unit.clone().or_else(|| bot_unit(&files, &read));

  match &bot {
    None => {
      run.say(&said.no_bot);

      if let Some(progress) = &run.progress {
        progress.passes_over(&[Stage::StayUp]);
      }
    }
    Some(bot) => {
      if let Some(command) = &ask.before_start {
        if !run.systemctl(&["stop", bot]).ok {
          return run.failed(
            Stage::Restart,
            fill_marks(&said.stop_failed, &[("unit", bot)]),
            Outcome::Failed,
          );
        }

        run.say(&fill_marks(&said.before_start, &[("command", command)]));

        let ran = run.step(&StepAsk {
          id: said.before_start_step.clone(),
          command: shell_words(command),
          cwd: deploy_dir.clone(),
          lane: None,
        });

        if ran.code != 0 {
          return run.failed(
            Stage::Restart,
            fill_marks(&said.before_start_failed, &[("said", &ran.said), ("unit", bot)]),
            Outcome::Failed,
          );
        }
      }

      run.systemctl(&["enable", bot]);
      run.systemctl(&["reset-failed", bot]);
      run.restarted_at.set(hands.now_ms());

      let verb = if ask.before_start.is_some() { "start" } else { "restart" };

      if !run.systemctl(&[verb, bot]).ok {
        return run.boot_failed(BootFailure::RestartRefused, bot, &record, &release, &bound_units).await;
      }

      if let Some(failure) = run.came_up(bot).await {
        return run.boot_failed(failure, bot, &record, &release, &bound_units).await;
      }

      run.say(&fill_marks(&said.stayed, &[("unit", bot)]));
    }
  }

  // The units the commit marks as going, once the service has stayed up.
  for unit in &retired {
    run.systemctl(&["stop", unit]);
    run.systemctl(&["disable", unit]);

    let at = places.unit_dir.join(unit);
    let is_link = std::fs::symlink_metadata(&at).is_ok_and(|meta| meta.file_type().is_symlink());

    if !is_link {
      run.say(&fill_marks(&said.retired_plain, &[("unit", unit)]));
    } else if let Err(why) = std::fs::remove_file(&at) {
      run.warn(fill_marks(&said.unretired, &[("unit", unit), ("why", &why.to_string())]));
    } else {
      run.say(&fill_marks(&said.retired, &[("unit", unit)]));
    }
  }

  if !retired.is_empty() {
    run.systemctl(&["daemon-reload"]);
  }

  // 11. The console face probed, then the steps after the health: either failing fails the deployment, which
  // rolls back where a service stands to restart (docs/deploy.md, 「The console probe」, 「Rollback」).
  run.stage(Stage::After);

  if let Some(console) = &repo.declaration.deploy.console {
    let say = |line: &str| run.say(line);

    let pinned = pinned_layer(repo, ask, env);

    if let Err(refusal) = probe(&console.reach, &release, &pinned, hands.console(), &say).await {
      let line = refusal.said();

      return match &bot {
        Some(bot) => {
          run.say(&line);
          run.step_failed.replace(Some(refusal.point.clone()));
          run.boot_failed(BootFailure::ConsoleRefused, bot, &record, &release, &bound_units).await
        }
        None => run.failed(Stage::After, line, Outcome::Failed),
      };
    }
  }

  for step in repo.declaration.deploy.steps.iter().filter(|step| step.phase == Phase::AfterHealth) {
    let ran = run.declared_step(step, &release);

    if ran.code != 0 {
      let line = fill_marks(
        &said.after_failed,
        &[("step", &step.id), ("code", &ran.code.to_string()), ("said", &ran.said)],
      );

      return match &bot {
        Some(bot) => {
          run.say(&line);
          run.step_failed.replace(Some(step.id.clone()));
          run.boot_failed(BootFailure::StepFailed, bot, &record, &release, &bound_units).await
        }
        None => run.failed(Stage::After, line, Outcome::Failed),
      };
    }
  }

  // 12. The releases pruned again, the one rolled back to kept.
  run.prune_releases(
    [
      Some(release.clone()),
      live.clone(),
      run.previous.as_ref().map(|previous| release_dir(deploy_dir, &previous.commit)),
    ]
    .into_iter()
    .flatten()
    .collect(),
  );

  // The units the release marks for the placed machine, laid there; what fails there is a warning.
  let placed = placed_units(&files, &read);

  if !placed.laid.is_empty() {
    let units: Vec<(String, String)> =
      placed.laid.iter().map(|path| (base_name(path), read(path))).collect();
    let names = units.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ");

    match gate.lay_units(&LaidUnits {
      units: &units,
      timers: &placed.timers,
    }) {
      None => {}
      Some((machine, Ok(()))) => {
        run.say(&fill_marks(&said.placed_laid, &[("names", &names), ("machine", &machine)]));
      }
      Some((machine, Err(why))) => run.warn(fill_marks(
        &said.placed_unlaid,
        &[("names", &names), ("machine", &machine), ("said", &why)],
      )),
    }
  }

  // The testbed the placed machine runs brought to this release, where it carries one; what fails there is a
  // warning, since the release here stays live whatever that machine answers.
  match gate.bring_testbed(&release, &commit) {
    None => {}
    Some((machine, Ok(files))) => run.say(&fill_marks(
      &said.testbed_brought,
      &[("machine", &machine), ("files", &files.join(", ")), ("commit", &shown)],
    )),
    Some((machine, Err(why))) => run.warn(fill_marks(
      &said.testbed_unbrought,
      &[("machine", &machine), ("commit", &shown), ("said", &why)],
    )),
  }

  // 13. The backup upload, in a unit of its own, not waited on.
  let remote = repo.declaration.deploy.backup.as_ref().and_then(|backup| backup.remote.as_ref());

  if let (Some(_), Some(upload)) = (remote, &ask.upload) {
    let unit = upload_unit_name(hands.now_ms());
    let line: Vec<String> = ["systemd-run", "--user", "--quiet", "--collect"]
      .into_iter()
      .map(str::to_owned)
      .chain([
        format!("--unit={unit}"),
        format!("--slice={BACKUP_SLICE}"),
        format!("--working-directory={}", source.display()),
      ])
      .chain(upload.iter().cloned())
      .collect();
    let started = hands.run(&line, source);

    if started.ok {
      run.say(&fill_marks(&said.upload_started, &[("unit", &unit)]));
    } else {
      run.warn(fill_marks(
        &said.upload_failed,
        &[("said", &first_chars(&started.out, DEPLOY.numbers.quoted_chars))],
      ));
    }
  }

  let warnings = run.warnings.borrow().clone();

  run.say(&if backup.is_empty() {
    fill_marks(&said.live_alone, &[("commit", &shown)])
  } else {
    fill_marks(&said.live, &[("commit", &shown), ("backup", &backup)])
  });
  // The deployment's ending is written before the hook hears of it, so no hook stands between the
  // deployment and its end.
  if let Some(progress) = &run.progress {
    progress.end(Outcome::Live, None);
  }

  hook.send(&Event {
    name: "deployed",
    commit: Some(commit.clone()),
    summary: fill_marks(&said.deployed_summary, &[("commit", &shown), ("repository", &repo.name())]),
    detail: json!({
      "commit": commit,
      "message": run.ask.message.clone().unwrap_or_else(|| run.subject.clone()),
      "previous": run.previous.as_ref().map(|previous| previous.commit.clone()),
      "flaky": flaky.iter().map(|(test, unit)| json!({ "test": test, "unit": unit })).collect::<Vec<_>>(),
      "warnings": warnings,
    }),
  });

  0
}
