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
use std::time::{Duration, Instant};

use bmo_testbed_core::declaration::{DeployStep, Phase, Repository};
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::deploy_steps::DeployStep as OwnStep;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::turn::{
  Env, FRONT_VARIABLE, Lane, NOT_RUN_EXIT_CODE, TurnAsk, job_pid, process_env, stop_tree, take_turn,
};
use serde_json::{Value, json};

use crate::adopt::{Adopting, adopt, prune_retired};
use crate::backup::{BackupPlan, BackupTaken, take_backup};
use crate::boot::{
  HealthWatch, StayUp, boot_report, boot_scene, stay_report, stays_up, unit_gave_up, unit_status,
  wait_for_health,
};
use crate::expansion::{BuildTree, bring_submodules, expansion_tree, put_build_tree};
use crate::progress::{DeployRun, DeployStart, Outcome, Part, Stage, begin_deploy, deploy_progress_path};
use crate::release::{
  LiveProcess, Preparing, differing_binaries, flip, link_of, live_processes, live_release, logged_before,
  pinned_detail, place_binaries, prepare, prune, put_link, release_dir, release_holding,
};
use crate::remove::{git_tree_above, remove_tree};
use crate::rollback::{
  BootFailure, DeployTree, FailedDeploy, Rollback, ServiceHands, deploy_failure_notice, failure_line,
  journal_command, roll_back, rolled_back_detail,
};
use crate::run::{Ran, run_quiet, shell_words, words};
use crate::tree::{
  DEPLOY_LOG, DEPLOY_RECORD, DeployRecord, UnitMark, backup_name, backup_stamp, base_name, binary_restarts,
  bot_unit, link_units, mark_built_from, marked_units, outside_relinks, read_deploy_record, relink_units,
  retired_units, set_aside_plain_units, short, timer_service, tree_fault, unit_link_plan, unit_links,
  units_to_link, user_unit_dir_from, write_deploy_record,
};
use crate::unit::{first_chars, upload_unit_name};

/// The variable naming another backup directory over the declaration's.
pub const BACKUP_DIR_VARIABLE: &str = "BMO_BACKUP_DIR";
/// The variable naming another `bot` service, for a rehearsal.
pub const BOT_UNIT_VARIABLE: &str = "BMO_UNIT";

/// Where a deployment lays its commit down, each place the declaration's unless the environment names
/// another (docs/boundary.md, 「The environment」); the deploy tree is the declaration's alone, since every
/// process a release starts carries `BMO_DEPLOY_DIR` naming its own release.
#[derive(Debug, Clone)]
pub struct DeployPlaces {
  /// The checkout the commit is taken from.
  pub source: PathBuf,
  pub deploy_dir: PathBuf,
  pub data_root: PathBuf,
  pub backup_dir: Option<PathBuf>,
  pub unit_dir: PathBuf,
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
    let deploy_dir = repo.path(root, env).join(&deploy.link);
    let data_root = repo.path(data, env);
    let home = PathBuf::from(env.get("HOME").map_or("/", String::as_str));

    Ok(Self {
      source: repo.root.clone(),
      deploy_dir: link_of(&deploy_dir),
      backup_dir: named(BACKUP_DIR_VARIABLE)
        .or_else(|| deploy.backup.as_ref().map(|backup| repo.path(&backup.dir, env))),
      unit_dir: user_unit_dir_from(env.get("XDG_CONFIG_HOME").map(String::as_str), &home),
      progress: named(crate::progress::DEPLOY_PROGRESS_VARIABLE)
        .unwrap_or_else(|| deploy_progress_path(&data_root)),
      bot_unit: env.get(BOT_UNIT_VARIABLE).filter(|unit| !unit.is_empty()).cloned(),
      data_root,
    })
  }
}

/// What the caller asked of the run.
#[derive(Debug, Clone, Default)]
pub struct DeployAsk {
  /// The commit to put live; none is the checkout's `HEAD`.
  pub commit: Option<String>,
  pub dry_run: bool,
  /// The gate left out, where the caller already ran it.
  pub no_check: bool,
  /// A command run between the `bot` service's stop and its start.
  pub before_start: Option<String>,
  /// The deployment's own unit, which its progress record names.
  pub unit: Option<String>,
  /// What the backup upload's unit is started with after `systemd-run`'s own words: its options, then the
  /// binary's `backups` verb.
  pub upload: Option<Vec<String>>,
}

/// What the gate is asked: the commit, in the checkout it is taken from, over the range from what was live.
pub struct GateAsk<'a> {
  pub root: &'a Path,
  pub commit: &'a str,
  pub since: Option<&'a str>,
}

/// One heavy step the gate ran, as the progress record writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct GateStep {
  pub name: String,
  pub code: i64,
  pub waited_seconds: f64,
  pub ran_seconds: f64,
}

/// What the gate came to: whether the commit passed, why not where it did not, the tests that passed only
/// when run once more, each with its unit, and the steps it ran.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GateOutcome {
  pub passed: bool,
  pub reason: String,
  pub flaky: Vec<(String, String)>,
  pub steps: Vec<GateStep>,
}

/// The gate a deployment passes before anything changes: the declared static check and the suite.
pub trait Gate {
  fn pass(&self, ask: &GateAsk<'_>) -> GateOutcome;
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

/// A step of the run's own or a declared one: its id, its words, where it runs, the lane whose turn it takes
/// and the time it is given.
#[derive(Debug, Clone)]
pub struct StepAsk {
  pub id: String,
  pub command: Vec<String>,
  pub cwd: PathBuf,
  pub lane: Option<Lane>,
  pub timeout: Option<Duration>,
}

/// What a step came to: its code, 75 where its turn did not come; how long it waited and ran; the start of
/// what it said; and whether it was stopped past its time.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRan {
  pub code: i64,
  pub waited_seconds: f64,
  pub ran_seconds: f64,
  pub said: String,
  pub overran: bool,
}

/// What the run does outside its own trees: commands run to their end, the service manager's among them;
/// heavy steps; its lines; the moment; and the live processes a prune spares.
pub trait DeployHands {
  /// A command run to its end in a directory, its output gathered.
  fn run(&self, line: &[String], cwd: &Path) -> Ran;
  /// A step run to its end, in its lane's turn where it names one, stopped past its time.
  fn step(&self, ask: &StepAsk) -> StepRan;
  fn say(&self, line: &str);
  fn now_ms(&self) -> i64;
  /// The live processes of this account; none where they could not be read.
  fn live_processes(&self) -> Option<Vec<LiveProcess>>;
}

/// The hands of the machine the run stands on.
pub struct Machine;

// A child's output read to its end on a thread of its own, written on as it comes.
fn gathered(mut from: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
  std::thread::spawn(move || {
    let mut all = Vec::new();
    let mut chunk = [0u8; 16 * 1024];

    while let Ok(read) = from.read(&mut chunk) {
      if read == 0 {
        break;
      }

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
        wait_seconds: None,
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
        overran: false,
      };
    }

    let started = Instant::now();
    let mut command = crate::run::command(&ask.command, Some(&ask.cwd));

    command
      .envs(turn.as_ref().and_then(|turn| turn.held_variable()))
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
          overran: false,
        };
      }
    };
    let out = gathered(child.stdout.take().expect("the output is piped"));
    let err = gathered(child.stderr.take().expect("the errors are piped"));
    let mut overran = false;
    let status = loop {
      match child.try_wait() {
        Ok(Some(status)) => break Some(status),
        Ok(None) if ask.timeout.is_some_and(|limit| started.elapsed() >= limit) => {
          overran = true;
          stop_tree(i64::from(child.id()), Duration::from_secs(1));
          break child.wait().ok();
        }
        Ok(None) => std::thread::sleep(Duration::from_millis(DEPLOY.numbers.step_poll_ms)),
        Err(_) => break None,
      }
    };
    let said = format!(
      "{}{}",
      String::from_utf8_lossy(&out.join().unwrap_or_default()),
      String::from_utf8_lossy(&err.join().unwrap_or_default())
    );

    StepRan {
      code: status.and_then(|status| status.code()).map_or(1, i64::from),
      waited_seconds,
      ran_seconds: started.elapsed().as_secs_f64(),
      said: first_chars(said.trim(), DEPLOY.numbers.quoted_chars),
      overran,
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

// The units of a commit's tracked files the declaration's globs reach, every one where it names none.
fn unit_files(tracked: &[String], globs: Option<&[String]>) -> Vec<String> {
  let patterns: Vec<glob::Pattern> = globs
    .unwrap_or_default()
    .iter()
    .filter_map(|one| glob::Pattern::new(one).ok())
    .collect();

  tracked
    .iter()
    .filter(|path| path.ends_with(".service") || path.ends_with(".timer"))
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

fn iso(at_ms: i64) -> String {
  chrono::DateTime::from_timestamp_millis(at_ms)
    .unwrap_or_default()
    .format("%Y-%m-%dT%H:%M:%S%.3fZ")
    .to_string()
}

// One run under way: what it was handed, and what it has learned.
struct Laying<'a> {
  repo: &'a Repository,
  places: &'a DeployPlaces,
  ask: &'a DeployAsk,
  hands: &'a dyn DeployHands,
  hook: &'a Hook,
  progress: Option<DeployRun>,
  commit: String,
  subject: String,
  previous: Option<DeployRecord>,
  restarted_at: std::cell::Cell<i64>,
  /// Whether the run is rolling back, whose wait for the service is no stage of its own.
  rolling_back: std::cell::Cell<bool>,
  warnings: std::cell::RefCell<Vec<String>>,
}

impl<'a> Laying<'a> {
  fn say(&self, line: &str) {
    self.hands.say(line);
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
      progress.end(Outcome::Stopped, Some(&line));
    }

    2
  }

  // The deployment ended before anything went live: said, sent as `deploy-failed`, and written as its ending.
  fn failed(&self, stage: Stage, line: String, outcome: Outcome) -> u8 {
    self.say(&line);
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

    if let Some(progress) = &self.progress {
      progress.end(outcome, Some(&line));
    }

    1
  }

  // Every release but the ones kept and the ones a live process stands in taken away, and the retired trees
  // nothing stands in; more releases left standing than the policy allows is sent as `releases-pinned`.
  fn prune_releases(&self, mut keep: Vec<PathBuf>) {
    let said = &DEPLOY.run;
    let processes = self.hands.live_processes();
    let places = self.places;

    // The release the run itself runs from is never taken away under it.
    keep.extend(std::env::current_exe().ok().and_then(|exe| release_holding(&places.deploy_dir, &exe)));

    let git = |args: &[&str], cwd: &Path| self.git(args, cwd);
    let pruned =
      prune(&places.deploy_dir, &keep, processes.as_deref(), &places.source, &places.data_root, &git);

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

    let in_use = |tree: &Path| {
      processes
        .as_deref()
        .is_none_or(|processes| processes.iter().any(|process| process.stands_in(tree)))
    };
    let retired = prune_retired(&places.deploy_dir, &places.source, &places.data_root, &git, &in_use);

    if !retired.removed.is_empty() {
      self.say(&fill_marks(&said.retired_trees, &[("names", &retired.removed.join(", "))]));
    }

    for warning in &retired.warnings {
      self.warn(warning.clone());
    }
  }

  // A release set up the way any checkout of the repository is: its submodules brought to their pins, the
  // declared setup steps, then the declared env links.
  fn set_up(&self, release: &Path) -> Result<(), String> {
    let run = |line: &[String], cwd: &Path| self.hands.run(line, cwd);

    bring_submodules(release, &self.places.data_root, &run)?;

    for setup in &self.repo.declaration.test.setup {
      let ran = self.hands.run(&shell_words(setup), release);

      if !ran.ok {
        return Err(fill_marks(
          &DEPLOY.run.setup_failed,
          &[
            ("command", setup),
            ("release", &release.to_string_lossy()),
            ("said", &first_chars(&ran.out, DEPLOY.numbers.quoted_chars)),
          ],
        ));
      }
    }

    for link in &self.repo.declaration.deploy.env_links {
      let at = release.join(&link.link);

      if let Some(parent) = at.parent() {
        std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
      }

      put_link(&at, &under_data(&self.places.data_root, &link.target)).map_err(|why| why.to_string())?;
    }

    Ok(())
  }

  // The release built in the build tree at the commit: the Cargo release build where the commit has a Cargo
  // workspace, the declared build steps, the binaries placed, and `bin/built-from` written last. The
  // binaries whose bytes differ from the live release's; the line saying what failed where a step did.
  fn build(&self, release: &Path, live: Option<&Path>) -> Result<Vec<String>, String> {
    let said = &DEPLOY.run;
    let places = self.places;
    let build = &self.repo.declaration.build;
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

    let profile = &build.cargo_profile;
    let target = tree.join("target");
    let cargo = tree.join("Cargo.toml").exists();
    let failed_at = |id: &str, ran: &StepRan| {
      fill_marks(
        &said.build_failed,
        &[("step", id), ("code", &ran.code.to_string()), ("said", &ran.said)],
      )
    };

    if cargo {
      let mut command = words(["cargo", "build", "--locked", "--profile", profile, "--target-dir"]);

      command.push(target.to_string_lossy().into_owned());

      let ask = StepAsk {
        id: OwnStep::CargoRelease.name().to_owned(),
        command,
        cwd: tree.clone(),
        lane: Some(Lane::Wide),
        timeout: None,
      };
      let ran = self.step(&ask);

      if ran.code != 0 {
        return Err(failed_at(&ask.id, &ran));
      }
    }

    for one in &build.steps {
      let ask = StepAsk {
        id: one.id.clone(),
        command: shell_words(&one.command),
        cwd: one.dir.as_ref().map_or_else(|| tree.clone(), |dir| tree.join(dir)),
        lane: one.lane,
        timeout: None,
      };
      let ran = self.step(&ask);

      if ran.code != 0 {
        return Err(failed_at(&ask.id, &ran));
      }
    }

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
      timeout: Some(Duration::from_secs(step.timeout_seconds)),
    })
  }

  // Whether the `bot` service came up: the declared health answered within its wait, where one is declared,
  // and the service stayed up; why not where it did not, said with what was read of it.
  async fn came_up(&self, bot: &str) -> Option<BootFailure> {
    let said = &DEPLOY.run;
    let deploy = &self.repo.declaration.deploy;
    let run = |line: &[String]| self.hands.run(line, &self.places.source);
    let status = || unit_status(&run, bot);

    if let Some(health) = &deploy.health {
      let seconds = deploy.health_wait_seconds;
      let mut watch = HealthWatch::new(health, &self.places.deploy_dir, seconds * 1000);

      self.say(&fill_marks(&said.waiting, &[("seconds", &seconds.to_string()), ("health", health)]));
      watch.gave_up = Some(Box::new(|| unit_gave_up(status().as_ref())));

      let wait = wait_for_health(&watch).await;

      if !wait.answered {
        for line in boot_report(&boot_scene(&run, bot, self.restarted_at.get(), wait)) {
          self.say(&line);
        }

        return Some(BootFailure::NotAnswering);
      }
    }

    let hold = deploy.stay_up_seconds * 1000;

    self.say(&fill_marks(
      &said.watching,
      &[("unit", bot), ("seconds", &deploy.stay_up_seconds.to_string())],
    ));

    if !self.rolling_back.get() {
      self.stage(Stage::StayUp);
    }

    if let StayUp::Fell { why, status } = stays_up(hold, DEPLOY.numbers.stay_poll_ms, &status).await {
      for line in stay_report(&run, bot, self.restarted_at.get(), hold, &why, status) {
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
    let places = self.places;
    let deploy = &self.repo.declaration.deploy;
    let failed = FailedDeploy {
      commit: self.commit.clone(),
      failure,
      previous: self.previous.clone(),
      written: written.clone(),
      before_start: self.ask.before_start.is_some(),
      unit: bot.to_owned(),
      health_wait_seconds: deploy.health_wait_seconds,
      stay_up_seconds: deploy.stay_up_seconds,
    };
    let said = |line: &str| self.say(line);
    let git = |args: &[&str], cwd: &Path| self.git(args, cwd);
    let mut tree = DeployTree {
      deploy_dir: places.deploy_dir.clone(),
      record_path: places.data_root.join(DEPLOY_RECORD),
      log_path: places.data_root.join(DEPLOY_LOG),
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

    if let Some(detail) = rolled_back_detail(&failed, &outcome) {
      if let Some(progress) = &self.progress {
        progress.rolled_back();
      }

      self.hook.send(&Event {
        name: "deploy-rolled-back",
        commit: Some(self.commit.clone()),
        summary: detail["reason"].as_str().unwrap_or_default().to_owned(),
        detail,
      });
    }

    let stage = if failure == BootFailure::DiedAfterStart { Stage::StayUp } else { Stage::Restart };

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

    if let Some(progress) = &self.progress {
      progress.end(
        if rolled_back { Outcome::RolledBack } else { Outcome::Failed },
        Some(&failure_line(&failed)),
      );
    }

    1
  }
}

// The checks before anything changes, each a refusal where it fails: no job's shell, the data root, the
// backup directory and the env files where they may stand.
fn place_refusal(repo: &Repository, places: &DeployPlaces, env: &Env) -> Option<String> {
  let said = &DEPLOY.run;
  let data = &places.data_root;
  let shown = |path: &Path| path.to_string_lossy().into_owned();

  if job_pid(env).is_some() {
    return Some(said.job.clone());
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

  repo.declaration.deploy.env_links.iter().find_map(|link| {
    let target = under_data(data, &link.target);

    (!target.exists()).then(|| {
      fill_marks(
        &said.env_missing,
        &[("target", &shown(&target)), ("link", &link.link.to_string_lossy())],
      )
    })
  })
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
  let said = &DEPLOY.run;
  let mut run = Laying {
    repo,
    places,
    ask,
    hands,
    hook,
    progress: None,
    commit: String::new(),
    subject: String::new(),
    previous: None,
    restarted_at: std::cell::Cell::new(0),
    rolling_back: std::cell::Cell::new(false),
    warnings: std::cell::RefCell::new(Vec::new()),
  };

  // 1. Where it stands.
  if let Some(refusal) = place_refusal(repo, places, env) {
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
  run.previous = read_deploy_record(&places.data_root);

  let commit = run.commit.clone();
  let shown = short(&commit);

  run.say(&fill_marks(&said.deploying, &[("commit", &shown), ("subject", &run.subject)]));

  if run.previous.as_ref().is_some_and(|previous| previous.commit == commit) {
    run.say(&fill_marks(&said.again, &[("commit", &shown)]));
  }

  // The unit links, judged against the commit while a refusal still changes nothing.
  let release = release_dir(&places.deploy_dir, &commit);
  let tracked = listed(run.git(&["ls-tree", "-r", "-z", "--name-only", &commit], source));
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

  run.progress = Some(begin_deploy(
    &places.progress,
    &DeployStart {
      unit: ask.unit.clone(),
      pid: i64::from(std::process::id()),
      commit: Some(commit.clone()),
      plan: Stage::PLANNED.to_vec(),
      now: hands.now_ms(),
    },
    now_ms,
  ));

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

  let mut flaky = Vec::new();

  if ask.no_check || !repo.declaration.deploy.gate {
    run.say(&said.gate_left_out);
  } else {
    let since = run.previous.as_ref().map(|previous| previous.commit.clone());
    let passed = gate.pass(&GateAsk {
      root: source,
      commit: &commit,
      since: since.as_deref(),
    });

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

    for (test, unit) in &passed.flaky {
      run.say(&fill_marks(&said.flaky, &[("test", test), ("unit", unit)]));
    }

    flaky = passed.flaky;
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

  let record_path = places.data_root.join(DEPLOY_RECORD);
  let log_path = places.data_root.join(DEPLOY_LOG);
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

  // 8. The record, written just before the flip.
  let record = DeployRecord {
    commit: commit.clone(),
    subject: run.subject.clone(),
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

  // 9. The flip, every link into the deploy tree first confirmed to resolve in the release.
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

  // 10. The units linked, and the services restarted.
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
  let linked_only = marked(".service", UnitMark::Link);
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
          timeout: None,
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

  // 11. The steps after the health, each a warning where it fails.
  run.stage(Stage::After);

  for step in repo.declaration.deploy.steps.iter().filter(|step| step.phase == Phase::AfterHealth) {
    let ran = run.declared_step(step, &release);

    if ran.overran {
      run.warn(fill_marks(
        &said.after_overran,
        &[("step", &step.id), ("seconds", &step.timeout_seconds.to_string())],
      ));
    } else if ran.code != 0 {
      run.warn(fill_marks(
        &said.after_failed,
        &[("step", &step.id), ("code", &ran.code.to_string()), ("said", &ran.said)],
      ));
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

  // 13. The backup upload, in a unit of its own, not waited on.
  let remote = repo.declaration.deploy.backup.as_ref().and_then(|backup| backup.remote.as_ref());

  if let (Some(_), Some(upload)) = (remote, &ask.upload) {
    let unit = upload_unit_name(hands.now_ms());
    let line: Vec<String> = ["systemd-run", "--user", "--quiet", "--collect"]
      .into_iter()
      .map(str::to_owned)
      .chain([format!("--unit={unit}")])
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
  hook.send(&Event {
    name: "deployed",
    commit: Some(commit.clone()),
    summary: fill_marks(&said.deployed_summary, &[("commit", &shown), ("repository", &repo.name())]),
    detail: json!({
      "commit": commit,
      "previous": run.previous.as_ref().map(|previous| previous.commit.clone()),
      "flaky": flaky.iter().map(|(test, unit)| json!({ "test": test, "unit": unit })).collect::<Vec<_>>(),
      "warnings": warnings,
    }),
  });

  if let Some(progress) = &run.progress {
    progress.end(Outcome::Live, None);
  }

  0
}
