//! `deploy` and `backups` (docs/boundary.md, 「The verbs」; docs/deploy.md): the deploy run started in a
//! transient unit of its own and run there, its gate answered by the suite crate; and the backups sent away
//! and rotated, one run at a time. The runs themselves are the deploy crate's; this is the verbs' side.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bmo_testbed_client::{CommandRun, PageQuery, TestRun, TestRunState};
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::main_branch::MainBranch;
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::deploy_steps::DeployStep as OwnStep;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::policy::versions::VERSIONS;
use bmo_testbed_core::queue::{LockWait, now_ms, take_lock};
use bmo_testbed_core::turn::{Env, FRONT_VARIABLE, NOT_RUN_EXIT_CODE, harness_pid, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_deploy::again::{AskAgain, Asked, judged};
use bmo_testbed_deploy::console::{LayerPin, LayerRelease, PinnedLayer};
use bmo_testbed_deploy::deployment::{
  BuildAsk, DeployAsk, DeployPlaces, Gate, GateAsk, GateFault, GateFooting, GateOutcome, GateStep,
  LaidUnits, Machine, Passed, SuitePassed, deploy as deploy_run, iso, passed_before,
};
use bmo_testbed_deploy::line::{
  DeployLine, DeployTicket, FirstMark, RaiseAsk, RootTurn, raise_first as raise_first_in_line,
};
use bmo_testbed_deploy::progress::DEPLOY_PROGRESS_VARIABLE;
use bmo_testbed_deploy::release::primary_checkout;
use bmo_testbed_deploy::run::run_quiet;
use bmo_testbed_deploy::tree::short;
use bmo_testbed_deploy::unit::{
  DEPLOY_SLICE, DEPLOY_UNIT_PREFIX, DeployUnitStart, deploy_unit_name, deploy_unit_of_process, hand_off,
  this_again,
};
use bmo_testbed_deploy::upload::{
  AwsHands, BackupNames, Remote, send_backups, upload_failed_detail, upload_failed_summary,
};
use bmo_testbed_record::Record;
use bmo_testbed_suite::log::{FailedTest, failed_tests_of, failures_of};
use bmo_testbed_suite::once_more::run_judged;
use bmo_testbed_suite::quick::shared_versions::find_shared;
use bmo_testbed_suite::run::{SuiteAsk, SuiteWords, ask_of};
use bmo_testbed_suite::static_check::{check_steps, run_static_check, spawn_step};
use bmo_testbed_suite::toolchain::{toolchain_text, toolchains};

use crate::home_of;
use crate::placement::{BenchFor, FarEnd, MachinePlacement, Ordered, PLACEMENT_VARIABLE, ReadyBench};
use crate::recorded::suite_record;
use crate::suite::repository;
use crate::words::{BINARY, DeployFlags, FirstArgs};

/// The gate a deployment passes: the declared static check over the range from what was live, then the
/// suite over the same range, what failed run once more, unless the deployment left the suite out; nothing
/// where an earlier pass stands on the same footing.
pub struct SuiteGate<'a> {
  pub repo: &'a Repository,
  pub hook: &'a Hook,
}

impl Gate for SuiteGate<'_> {
  fn pass(&self, ask: &GateAsk<'_>) -> GateOutcome {
    // The gate judges the commit's committed trees, by the commit's own declaration (docs/deploy.md, 「What the
    // deploy run does, in order」).
    let repo = Repository {
      root: ask.tree.to_path_buf(),
      declaration: ask.declaration.clone(),
    };
    let footing = GateFooting {
      heads: ask.heads.to_vec(),
      toolchains: toolchain_text(&toolchains(&repo.root)),
      machine: None,
    };

    if let Some(earlier) = (ask.earlier)(&footing) {
      return passed_before(footing, earlier);
    }

    GateOutcome {
      footing: Some(footing),
      ..self.judged(&repo, ask)
    }
  }

  fn dependents(&self, _: &GateAsk<'_>) -> GateOutcome {
    let started = Instant::now();
    let ended = crate::dependents::ended(self.repo, true);
    let step = GateStep {
      name: "dependents".to_owned(),
      code: i64::from(ended.code),
      waited_seconds: 0.0,
      ran_seconds: started.elapsed().as_secs_f64(),
    };

    GateOutcome {
      passed: ended.code == 0,
      reason: ended.line,
      steps: vec![step],
      ..GateOutcome::default()
    }
  }
}

impl SuiteGate<'_> {
  // The static check, then the suite where it is asked, in the tree the gate judges.
  fn judged(&self, repo: &Repository, ask: &GateAsk<'_>) -> GateOutcome {
    let mut steps = Vec::new();
    let failed = run_static_check(
      &repo.root,
      &check_steps(repo, ask.since),
      &mut |text| eprint!("{text}"),
      &spawn_step,
      &mut |done| {
        steps.push(GateStep {
          name: done.id.clone(),
          code: i64::from(done.code),
          waited_seconds: done.waited_seconds,
          ran_seconds: done.ran_seconds,
        })
      },
    );

    if let Some(line) = failed {
      return GateOutcome {
        passed: false,
        reason: line,
        steps,
        ..GateOutcome::default()
      };
    }

    if !ask.suite {
      return GateOutcome {
        passed: true,
        steps,
        ..GateOutcome::default()
      };
    }

    let words = SuiteWords {
      since: ask.since.map(str::to_owned),
      ..SuiteWords::default()
    };
    let started = Instant::now();
    let command = [BINARY, "suite", "--again"]
      .into_iter()
      .map(str::to_owned)
      .chain(ask.since.iter().flat_map(|since| ["--since".to_owned(), (*since).to_owned()]))
      .collect::<Vec<_>>()
      .join(" ");
    let judged = SuiteAsk {
      record: Some(suite_record(&data_root(), repo, command)),
      ..ask_of(&words, &repo.declaration.test)
    };
    let (ended, flaky) = run_judged(repo, &judged, &data_root(), self.hook);
    let code = ended.code;

    steps.push(GateStep {
      name: OwnStep::Tests.name().to_owned(),
      code: i64::from(code),
      waited_seconds: 0.0,
      ran_seconds: started.elapsed().as_secs_f64(),
    });

    GateOutcome {
      passed: code == 0,
      reason: suite_reason(i64::from(code), ended.log.as_deref()),
      flaky: flaky.tests.iter().map(|one| (one.test.clone(), one.file.clone())).collect(),
      steps,
      ..GateOutcome::default()
    }
  }
}

/// The gate passed on the placed machine the settings name (docs/deploy.md, 「Where a deployment runs」): the
/// commit benched there with the checkout's outside trees, in the repository's own deployment bench, the
/// static check and the suite run there as that machine's own verbs, the suite left out where that machine's
/// record holds a passed run of the whole suite on the commit, and the Cargo release build run there and
/// read back. Nothing of it runs here: work the machine does not answer for, or the connection breaks under,
/// answers broken, which the run tries again (docs/deploy.md, 「A connection that breaks under the placed
/// machine」).
pub struct PlacedGate<'a> {
  pub machine: MachinePlacement,
  /// The deployed repository's name, which its bench there stands under.
  pub repository: String,
  pub here: &'a dyn Gate,
  /// Whether the gate is passed at all; the build goes to the machine either way.
  pub checked: bool,
  /// Whether the testbed there is brought to the live testbed release before the bench and to a live release
  /// that carries one; not where the caller set the placement empty (docs/boundary.md, 「The environment」).
  pub brings_testbed: bool,
}

// What one step the gate ran there came to: its step as the progress record writes it, whether the connection
// broke under it, the last line its standard output gave, and the failing logs it kept there, brought here.
struct FarStep {
  step: GateStep,
  broken: bool,
  last_line: Option<String>,
  brought: Vec<PathBuf>,
}

// The reason a gate's suite ended as it did: its exit, and where it failed the tests its log names as failed
// and the log itself, where one stands to be read (docs/deploy.md, 「What the deploy run does, in order」).
fn suite_reason(code: i64, log: Option<&Path>) -> String {
  let said = &DEPLOY.run;
  let reason = fill_marks(&said.gate_suite, &[("code", &code.to_string())]);
  let Some(log) = log.filter(|_| code != 0) else {
    return reason;
  };
  // Each failing test by its name and its unit, as Cargo's and bun's lines give them; where none stands under a
  // unit, what the sweep names as failed.
  let failed = std::fs::read_to_string(log)
    .map(|text| {
      let placed = failed_tests_of(&text).iter().map(FailedTest::named).collect::<Vec<_>>();

      if placed.is_empty() { failures_of(&text) } else { placed }
    })
    .unwrap_or_default();
  let shown = DEPLOY.numbers.failed_tests_shown;
  let reason = if failed.is_empty() {
    reason
  } else {
    let named = failed.iter().take(shown).cloned().collect::<Vec<_>>().join(&said.gate_tests_joiner);
    let tests = if failed.len() > shown {
      fill_marks(
        &said.gate_more_tests,
        &[("tests", &named), ("count", &(failed.len() - shown).to_string())],
      )
    } else {
      named
    };

    fill_marks(&said.gate_failed_tests, &[("reason", &reason), ("tests", &tests)])
  };

  logged_reason(reason, Some(log))
}

// A gate's reason with the log it names, where there is one.
fn logged_reason(reason: String, log: Option<&Path>) -> String {
  match log {
    Some(log) => fill_marks(&DEPLOY.run.gate_log, &[("reason", &reason), ("log", &log.to_string_lossy())]),
    None => reason,
  }
}

impl PlacedGate<'_> {
  fn testbed(&self) -> String {
    self.machine.settings.testbed.clone()
  }

  // Whether the machine answers for the deployment's work; where it does not, the work is broken: it comes here
  // where the order names this machine, and is tried again there otherwise.
  fn reached(&self) -> bool {
    self.machine.reached()
  }

  // Whether the deployment's work comes here where the placed machine breaks under it: where the placement order
  // names this machine (docs/deploy.md, 「A connection that breaks under the placed machine」). Said on its line
  // where it does.
  fn comes_here(&self, work: &str, said: &str) -> bool {
    let here = self.machine.settings.order().is_ok_and(|order| order.contains(&Ordered::Here));

    if here {
      eprintln!(
        "{}",
        fill_marks(
          &LANES.placed.here_instead,
          &[("machine", self.machine.settings.label()), ("work", work), ("said", said)],
        )
      );
    }

    here
  }

  // Work there that failed, as the gate's machine left it: broken where that machine no longer answers, the
  // failure's own otherwise.
  fn failed(&self, why: String) -> GateFault {
    if self.machine.reached() {
      GateFault::Failed {
        code: None,
        said: why,
      }
    } else {
      GateFault::Broken(self.machine.broke())
    }
  }

  // The repository's deployment bench there, made ready at the commit once the testbed there is brought to
  // the live testbed release, unless the caller keeps it as it stands.
  fn bench(&self, root: &Path, commit: &str) -> Result<ReadyBench, GateFault> {
    let made = if self.brings_testbed { self.machine.brought_live() } else { Ok(()) };

    made
      .and_then(|()| self.machine.ready(root, commit, BenchFor::Deployment(&self.repository)))
      .map_err(|why| self.failed(why))
  }

  // One verb of the machine's testbed run in the bench, timed: its step, which a broken connection ends as not
  // run.
  fn step(&self, bench: &ReadyBench, name: &str, words: &[String], setup: bool) -> FarStep {
    let started = Instant::now();
    let ran = self.machine.run_read(bench, words, setup, None, &Arc::default());
    let (code, broken) = match ran.end {
      FarEnd::Ended(code) => (i64::from(code), false),
      FarEnd::Broken => (i64::from(NOT_RUN_EXIT_CODE), true),
    };

    FarStep {
      step: GateStep {
        name: name.to_owned(),
        code,
        waited_seconds: 0.0,
        ran_seconds: started.elapsed().as_secs_f64(),
      },
      broken,
      last_line: ran.last_line,
      brought: ran.brought,
    }
  }

  // The suite's log a failed run there named on its `--json` line, brought here; none where the line names none
  // or the log could not be brought, which is said.
  fn suite_log(&self, ran: &FarStep) -> Option<PathBuf> {
    let named: CommandRun = serde_json::from_str(ran.last_line.as_deref()?).ok()?;

    (ran.step.code != 0 && !named.log_path.is_empty())
      .then(|| self.machine.bring(&named.log_path))
      .flatten()
  }

  // The gate where the connection to the machine broke under it: judged here where the order names this machine,
  // and otherwise broken, with the steps it ran there, so the deployment tries it again.
  fn broken_gate(&self, ask: &GateAsk<'_>, steps: Vec<GateStep>) -> GateOutcome {
    if self.comes_here("gate", &self.machine.broke()) {
      return self.here.pass(ask);
    }

    self.broken(steps)
  }

  // The build where the connection to the machine broke under it: none, so it runs here, where the order names this
  // machine, and otherwise broken, so the deployment tries it again.
  fn broken_build(&self) -> Option<Result<(), GateFault>> {
    let broke = self.machine.broke();

    if self.comes_here("build", &broke) {
      return None;
    }

    Some(Err(GateFault::Broken(broke)))
  }

  // The gate's answer where the connection to the machine broke under it, with the steps it ran.
  fn broken(&self, steps: Vec<GateStep>) -> GateOutcome {
    GateOutcome {
      passed: false,
      reason: self.machine.broke(),
      broken: true,
      steps,
      ..GateOutcome::default()
    }
  }

  // What the gate stands on there: the heads laid beside the bench and the toolchains that machine's testbed
  // answers in it, with the machine's label; none where the toolchains could not be read.
  fn footing(&self, bench: &ReadyBench) -> Option<GateFooting> {
    let toolchains = self.machine.read(bench, &[self.testbed(), "toolchain".to_owned()])?;

    Some(GateFooting {
      heads: bench.heads.clone(),
      toolchains,
      machine: Some(self.machine.settings.label().to_owned()),
    })
  }

  // The static check there, then the suite where it is asked and the machine's own record holds no passed run
  // of the whole suite on the commit, each as that machine's own verb in the bench.
  fn judged_there(&self, bench: &ReadyBench, ask: &GateAsk<'_>) -> GateOutcome {
    let since = ask.since.iter().flat_map(|since| ["--since".to_owned(), (*since).to_owned()]);
    let check = [self.testbed(), "check".to_owned(), "static".to_owned()]
      .into_iter()
      .chain(since.clone())
      .collect::<Vec<_>>();
    let checked = self.step(bench, "check", &check, true);
    let mut steps = vec![checked.step];

    if checked.broken {
      return self.broken(steps);
    }

    if steps[0].code != 0 {
      return GateOutcome {
        passed: false,
        reason: logged_reason(
          fill_marks(&DEPLOY.run.gate_check, &[("code", &steps[0].code.to_string())]),
          checked.brought.first().map(PathBuf::as_path),
        ),
        steps,
        ..GateOutcome::default()
      };
    }

    // Where the deploying machine's record held no passed run of the whole suite on the commit, the placed
    // machine's own record is asked, since a suite placed there is written there alone.
    let suite_passed = if ask.suite { self.passed_there(bench, ask.commit) } else { None };

    if !ask.suite || suite_passed.is_some() {
      return GateOutcome {
        passed: true,
        steps,
        suite_passed,
        ..GateOutcome::default()
      };
    }

    // Asked `--json`, the suite there ends on the line naming its log, which a failure brings here.
    let suite = [self.testbed(), "suite".to_owned(), "--again".to_owned(), "--json".to_owned()]
      .into_iter()
      .chain(since)
      .collect::<Vec<_>>();
    let ran = self.step(bench, OwnStep::Tests.name(), &suite, false);

    if ran.broken {
      steps.push(ran.step);
      return self.broken(steps);
    }

    let code = ran.step.code;
    let log = self.suite_log(&ran);

    steps.push(ran.step);

    GateOutcome {
      passed: code == 0,
      reason: suite_reason(code, log.as_deref()),
      steps,
      ..GateOutcome::default()
    }
  }

  // The passed run of the whole suite on the commit that the machine's own record holds, asked of its
  // `passed` verb in the bench; none where it answers none, answers a run that is not one, or does not answer
  // at all.
  fn passed_there(&self, bench: &ReadyBench, commit: &str) -> Option<SuitePassed> {
    let words = [
      self.testbed(),
      "passed".to_owned(),
      "--commit".to_owned(),
      commit.to_owned(),
      "--json".to_owned(),
    ];
    let run: TestRun = serde_json::from_str(self.machine.read(bench, &words)?.trim()).ok()?;
    let taken = run.commit.as_deref() == Some(commit)
      && run.whole == Some(true)
      && run.clean == Some(true)
      && run.state == TestRunState::Passed;

    taken.then(|| SuitePassed {
      run,
      machine: Some(self.machine.settings.label().to_owned()),
    })
  }
}

impl Gate for PlacedGate<'_> {
  fn pass(&self, ask: &GateAsk<'_>) -> GateOutcome {
    if !self.checked {
      return self.here.pass(ask);
    }

    if !self.reached() {
      return self.broken_gate(ask, Vec::new());
    }

    let bench = match self.bench(ask.root, ask.commit) {
      Ok(bench) => bench,
      Err(GateFault::Broken(_)) => return self.broken_gate(ask, Vec::new()),
      Err(GateFault::Failed { said, .. }) => {
        return GateOutcome {
          passed: false,
          reason: said,
          ..GateOutcome::default()
        };
      }
    };
    let footing = self.footing(&bench);

    if let Some(footing) = &footing
      && let Some(earlier) = (ask.earlier)(footing)
    {
      return passed_before(footing.clone(), earlier);
    }

    let judged = self.judged_there(&bench, ask);

    if judged.broken {
      return self.broken_gate(ask, judged.steps);
    }

    GateOutcome { footing, ..judged }
  }

  fn dependents(&self, ask: &GateAsk<'_>) -> GateOutcome {
    self.here.dependents(ask)
  }

  fn build(&self, ask: &BuildAsk<'_>) -> Option<Result<(), GateFault>> {
    if !self.reached() {
      return self.broken_build();
    }

    let bench = match self.bench(ask.root, ask.commit) {
      Ok(bench) => bench,
      Err(GateFault::Broken(_)) => return self.broken_build(),
      Err(fault) => return Some(Err(fault)),
    };
    // The deployed version goes with the build to the machine it runs on (docs/deploy.md, 「The deployed version」).
    let version = format!("{}={}", VERSIONS.variable, ask.version);
    let build = ["env".to_owned(), version, self.testbed(), "cargo".to_owned()]
      .into_iter()
      .chain(["build", "--workspace", "--locked", "--profile", ask.profile].map(str::to_owned))
      .collect::<Vec<_>>();
    let built = self.step(&bench, OwnStep::CargoRelease.name(), &build, true);

    if built.broken {
      return self.broken_build();
    }

    if built.step.code != 0 {
      let there = fill_marks(
        &LANES.placed.ran_there,
        &[("machine", self.machine.settings.label()), ("bench", &bench.path)],
      );

      return Some(Err(GateFault::Failed {
        code: Some(built.step.code),
        said: there,
      }));
    }

    match self
      .machine
      .fetch_binaries(&bench, ask.profile, ask.binaries, ask.built)
      .map_err(|why| self.failed(why))
    {
      Err(GateFault::Broken(_)) => self.broken_build(),
      fetched => Some(fetched),
    }
  }

  fn lay_units(&self, ask: &LaidUnits<'_>) -> Option<(String, Result<(), String>)> {
    Some((self.machine.settings.label().to_owned(), self.machine.lay_units(ask.units, ask.timers)))
  }

  fn bring_testbed(&self, release: &Path, commit: &str) -> Option<(String, Result<Vec<String>, String>)> {
    let binary = self.machine.testbed_in(release).filter(|_| self.brings_testbed)?;

    Some((self.machine.settings.label().to_owned(), self.machine.bring_testbed(&binary, commit)))
  }
}

/// Whether a deployment brings the placed machine's testbed to its release: not where the caller set the
/// placement empty, which keeps that testbed as it stands.
pub fn brings_testbed(env: &Env) -> bool {
  env.get(PLACEMENT_VARIABLE).is_none_or(|placement| !placement.is_empty())
}

// The variables a unit of the run is handed so it aims at what this caller aimed at: the places resolved,
// and whatever of the testbed's own the caller named.
fn handed(places: &DeployPlaces, env: &Env) -> Vec<(String, String)> {
  let shown = |path: &Path| path.to_string_lossy().into_owned();
  let mut handed = vec![
    (DEPLOY_PROGRESS_VARIABLE.to_owned(), shown(&places.progress)),
    ("BMO_DATA_DIR".to_owned(), shown(&data_root())),
    (FRONT_VARIABLE.to_owned(), "1".to_owned()),
  ];

  if let Some(config) = places.unit_dir.parent().and_then(Path::parent) {
    handed.push(("XDG_CONFIG_HOME".to_owned(), shown(config)));
  }

  if let Some(backups) = &places.backup_dir {
    handed.push(("BMO_BACKUP_DIR".to_owned(), shown(backups)));
  }

  // A placement set empty is handed on as it is, since it keeps the placed machine's testbed as it stands.
  if !brings_testbed(env) {
    handed.push((PLACEMENT_VARIABLE.to_owned(), String::new()));
  }

  for name in [
    "BMO_UNIT",
    "BMO_TESTBED_EVENT_HOOK",
    "BMO_CARGO_TARGET_DIR",
    "BMO_TESTBED_LOCK",
    "BMO_TESTBED_CHECKOUTS",
  ] {
    if let Some(value) = env.get(name).filter(|value| !value.is_empty()) {
      handed.push((name.to_owned(), value.clone()));
    }
  }

  handed
}

/// The pauses between the tries of a deployment's work on the placed machine (docs/deploy.md, 「A connection that
/// breaks under the placed machine」): the policy's pause, as many times as the policy's minutes hold it.
pub fn retry_pauses() -> Vec<Duration> {
  let numbers = &DEPLOY.numbers;
  let pause = numbers.retry_pause_seconds.max(1);
  let pauses = usize::try_from(numbers.retry_minutes * 60 / pause).unwrap_or(usize::MAX);

  vec![Duration::from_secs(pause); pauses]
}

fn refused(line: &str) -> u8 {
  eprintln!("{}", fill_marks(&DEPLOY.run.refused, &[("why", line)]));
  REFUSED_EXIT_CODE
}

// The console layer the shared versions of a repository pin, its pinned release and the compatible ones, whose
// vendored release the console probe reads a manifest by and holds its layer to (docs/deploy.md,
// 「The console probe」); the deploy run asks it of the repository the deployed commit's declaration names,
// where that commit brings a console screen.
fn pinned_layer(repository: &str, env: &Env) -> PinnedLayer {
  let layer_release = |version: &str, file: &str, sha256: &str| LayerRelease {
    version: version.to_owned(),
    file: file.to_owned(),
    sha256: sha256.to_owned(),
  };

  match find_shared(repository, env) {
    Ok(found) => match found.packages.get(&DEPLOY.console.layer_package) {
      Some(pin) => PinnedLayer::Pinned(LayerPin {
        pinned: layer_release(&pin.version, &pin.file, &pin.sha256),
        compatible: pin
          .compatible
          .iter()
          .map(|release| layer_release(&release.version, &release.file, &release.sha256))
          .collect(),
      }),
      None => PinnedLayer::Unpinned,
    },
    Err(why) => PinnedLayer::Unread(why),
  }
}

/// `deploy`: outside a deployment's unit it starts one and ends; inside it, the run. A refusal and a dry
/// run stay in the caller's process.
pub fn deploy(args: &DeployFlags) -> u8 {
  let env = process_env();

  if let Some(archive) = &args.artifact {
    return from_artifact(archive, args, &env);
  }

  let repo = match repository() {
    Ok(repo) => repo,
    Err(code) => return code,
  };
  let places = match DeployPlaces::of(&repo, &env) {
    Ok(places) => places,
    Err(why) => return refused(&why),
  };

  if harness_pid(&env).is_some() {
    return refused(&DEPLOY.run.harness);
  }

  let unit = deploy_unit_of_process();

  if unit.is_none() && !args.dry_run {
    if let Some(code) = asked_again(args, &repo, &places, &env) {
      return code;
    }

    let start = DeployUnitStart {
      unit: deploy_unit_name(bmo_testbed_core::queue::now_ms()),
      slice: DEPLOY_SLICE.to_owned(),
      cwd: repo.root.clone(),
      command: this_again(),
      env: handed(&places, &env),
    };

    return hand_off(&start, env.get("PATH").map_or("", String::as_str), &|line| println!("{line}"));
  }

  // Inside its unit the run's own process stands in the repository's primary checkout, so nothing it starts
  // without a directory of its own inherits a checkout that may go while it runs (docs/deploy.md, 「The
  // releases」); every path the run names is absolute.
  if unit.is_some() {
    let git = |args: &[&str], cwd: &Path| {
      let line: Vec<String> =
        std::iter::once("git").chain(args.iter().copied()).map(str::to_owned).collect();

      run_quiet(&line, Some(cwd))
    };
    let _ = std::env::set_current_dir(primary_checkout(&repo.root, &git));
  }

  let hook = Hook::of(Some(&repo), &env);
  // The upload runs in a unit of its own, in the repository's primary checkout the run names, aiming at the
  // backups this run took.
  let upload: Vec<String> = handed(&places, &env)
    .into_iter()
    .map(|(name, value)| format!("--setenv={name}={value}"))
    .chain([this_again()[0].clone(), "backups".to_owned()])
    .collect();
  let ask = DeployAsk {
    commit: args.commit.clone(),
    message: args.message.clone(),
    dry_run: args.dry_run,
    no_check: args.no_check,
    before_start: args.before_start.clone(),
    force_live: args.force_live,
    unit,
    upload: Some(upload),
    record: Some(data_root()),
    layer_pin: Some(pinned_layer),
    line: Some(DeployLine::under(&data_root())),
    first: args.first.as_deref().map(|why| FirstMark::now(&who(args.by.as_deref(), &env), why)),
    root_poll: None,
    retry: retry_pauses(),
  };
  let suite = SuiteGate {
    repo: &repo,
    hook: &hook,
  };
  let here: &dyn Gate = if args.no_check { &Passed } else { &suite };
  let placed = MachinePlacement::named(&data_root(), &home_of(&env)).map(|machine| PlacedGate {
    machine,
    repository: repo.name(),
    here,
    checked: !args.no_check,
    brings_testbed: brings_testbed(&env),
  });
  let gate: &dyn Gate = placed.as_ref().map_or(here, |placed| placed);
  let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(runtime) => runtime,
    Err(why) => return refused(&why.to_string()),
  };

  runtime.block_on(deploy_run(&repo, &places, &ask, &env, gate, &Machine, &hook))
}

// What a call that would start a deployment finds already standing for its commit (docs/deploy.md, 「The
// deployment line」), judged in the caller's own process before any unit starts: a deployment of the same
// commit, read as a commit, standing for the deploy root, which the call attaches to, raising it first where
// asked; or a commit whose newest ended deployment failed in its gate while main still stands on it, refused.
// None where the call starts its own, among them every call whose commit or remote cannot be read here.
fn asked_again(args: &DeployFlags, repo: &Repository, places: &DeployPlaces, env: &Env) -> Option<u8> {
  let said = &DEPLOY.run;
  let git = |words: &[&str]| {
    let line: Vec<String> =
      std::iter::once("git").chain(words.iter().copied()).map(str::to_owned).collect();

    run_quiet(&line, Some(&repo.root))
  };
  let resolve = |named: &str| {
    let resolved = git(&["rev-parse", "--verify", "--quiet", &format!("{named}^{{commit}}")]);

    resolved.ok.then(|| resolved.out.trim().to_owned())
  };

  if !git(&["fetch", "--quiet", "origin"]).ok {
    return None;
  }

  let commit = resolve(args.commit.as_deref().unwrap_or("HEAD"))?;
  let main_head = || MainBranch::new(&repo.root, &repo.declaration.check.main).head().ok();
  let judged = judged(&AskAgain {
    repository: &repo.name(),
    commit: &commit,
    turn: &RootTurn::under(&places.deploy_root).lock,
    data_root: &data_root(),
    own_run: args.before_start.is_some() || args.no_check || args.force_live,
    no_check: args.no_check,
    first: args.first.is_some(),
    resolve: &resolve,
    main_head: &main_head,
  });

  // A deployment of the repository still waiting takes this later commit in place of its own, so the line keeps one
  // deployment of the repository rather than one more (docs/deploy.md, 「The deployment line」).
  if matches!(judged, Asked::Start)
    && args.before_start.is_none()
    && !args.no_check
    && !args.force_live
    && args.first.is_none()
  {
    let turn = RootTurn::under(&places.deploy_root).lock;
    let line = bmo_testbed_deploy::line::DeployLine::under(&data_root()).lock;
    let carries = |later: &str, earlier: &str| git(&["merge-base", "--is-ancestor", earlier, later]).ok;

    if let Some(waiting) =
      bmo_testbed_deploy::again::waiting_behind(&turn, &line, &repo.name(), &commit, &carries)
      && let Some(unit) = waiting.unit.as_deref()
      && bmo_testbed_deploy::line::leave_later(&turn, unit, &commit)
    {
      println!(
        "{}",
        fill_marks(
          &said.joined_waiting,
          &[("unit", unit), ("words", &waiting.words), ("commit", &short(&commit))]
        )
      );
      return Some(0);
    }
  }

  match judged {
    Asked::Standing(standing) => Some(attach(args, &standing, env)),
    Asked::FailedUnmoved(failed) => Some(refused(&fill_marks(
      &said.failed_unmoved,
      &[
        ("commit", &short(&commit)),
        ("unit", failed.unit_name.as_deref().unwrap_or_default()),
        ("when", &iso(failed.ended_at.unwrap_or(failed.started_at))),
        ("reason", failed.reason.as_deref().unwrap_or_default()),
      ],
    ))),
    Asked::Start => None,
  }
}

// The call attached to the deployment standing for its commit: raised to first where it asks so, the raise's
// own line said whether it took or not, since one holding the line or marked first already is followed as it
// stands; then the line naming the unit it follows. Exits 0.
fn attach(args: &DeployFlags, standing: &DeployTicket, env: &Env) -> u8 {
  let unit = standing.unit.as_deref().unwrap_or_default();

  if let Some(why) = &args.first {
    let mark = FirstMark::now(&who(args.by.as_deref(), env), why);

    match raise_unit(unit, mark) {
      Ok(line) | Err(line) => println!("{line}"),
    }
  }

  let seconds = standing.asked_at.map_or(0, |at| (now_ms() - at).max(0) / 1000);

  println!(
    "{}",
    fill_marks(
      &DEPLOY.run.attached,
      &[("unit", unit), ("words", &standing.words), ("seconds", &seconds.to_string())]
    )
  );
  0
}

// Who marks a deployment first: the one named, else the account the verb runs as.
fn who(named: Option<&str>, env: &Env) -> String {
  named
    .map(str::to_owned)
    .or_else(|| env.get("USER").filter(|user| !user.is_empty()).cloned())
    .unwrap_or_else(|| DEPLOY.run.raise_who.clone())
}

// Whether the testbed's record holds a deployment run in the unit, read newest first and no further back than
// the moment the unit's name carries, since its deployment begins its record after it.
fn recorded_in(record: &Record, unit: &str) -> Result<bool, String> {
  let since = unit.strip_prefix(DEPLOY_UNIT_PREFIX).and_then(|at| at.parse::<i64>().ok()).unwrap_or(0);
  let mut query = PageQuery::default();

  loop {
    let page = record.deployments(&query)?;

    if page.items.iter().any(|deployment| deployment.unit_name.as_deref() == Some(unit)) {
      return Ok(true);
    }

    match page.items.last() {
      Some(last) if page.more && last.started_at >= since => query.after = Some(last.id),
      _ => return Ok(false),
    }
  }
}

// A waiting deployment raised to first in the machine's line by its unit: the line saying so, or the refusal.
fn raise_unit(unit: &str, mark: FirstMark) -> Result<String, String> {
  let line = DeployLine::under(&data_root());
  let running = |unit: &str| {
    unit.starts_with(DEPLOY_UNIT_PREFIX)
      && run_quiet(&["systemctl", "--user", "is-active", "--quiet", unit].map(str::to_owned), None).ok
  };
  let recorded = |unit: &str| Record::open(&data_root()).and_then(|record| recorded_in(&record, unit));

  raise_first_in_line(&RaiseAsk {
    line: &line.lock,
    unit,
    mark,
    running: &running,
    recorded: &recorded,
  })
}

/// `deploy first`: a waiting deployment raised to first in the deployment line by its unit (docs/deploy.md,
/// 「The deployment line」); a refusal exits 2.
pub fn raise_first(args: &FirstArgs) -> u8 {
  let env = process_env();

  match raise_unit(&args.unit, FirstMark::now(&who(args.by.as_deref(), &env), &args.why)) {
    Ok(line) => {
      println!("{line}");
      0
    }
    Err(why) => refused(&why),
  }
}

/// `backups`: every backup not yet confirmed sent and read back, the rotation after it, one run at a time;
/// 1 where any failed to send, the rest sent all the same.
pub fn backups() -> u8 {
  let said = &VERBS.backups;
  let env = process_env();
  let repo = match repository() {
    Ok(repo) => repo,
    Err(code) => return code,
  };
  let places = match DeployPlaces::of(&repo, &env) {
    Ok(places) => places,
    Err(why) => return refused(&why),
  };
  let (Some(declared), Some(dir)) = (&repo.declaration.deploy.backup, &places.backup_dir) else {
    return refused(&said.none_declared);
  };
  let Some(address) = &declared.remote else {
    println!("{}", fill_marks(&said.local_only, &[("dir", &dir.to_string_lossy())]));
    return 0;
  };
  let remote = match Remote::parse(address) {
    Ok(remote) => remote,
    Err(why) => return refused(&why),
  };
  let lock = data_root().join("locks").join(format!("backups-{}.lock", repo.name()));
  // Another run's upload is waited out, however long it takes.
  let notice = LANES.numbers.notice_seconds;
  let mut wait_on = |_: Option<&str>| -> Option<i64> {
    eprintln!("{}", said.locked);

    Some(now_ms() + i64::try_from(notice * 1000).unwrap_or(i64::MAX))
  };
  let wait = LockWait {
    holder: said.holder.clone(),
    wait_on: Some(&mut wait_on),
    ..LockWait::default()
  };
  let Some(_held) = take_lock(&lock, notice as f64, wait) else {
    return NOT_RUN_EXIT_CODE;
  };
  let mut hands = AwsHands {
    dir: dir.clone(),
    remote,
    profile: declared.profile.clone(),
    say: Box::new(|line| println!("{line}")),
  };
  let names = BackupNames::new(declared.name.as_deref().unwrap_or(&repo.name()));
  let outcome = send_backups(&mut hands, &names, usize::try_from(declared.keep).unwrap_or(usize::MAX));

  if outcome.failed.is_empty() {
    return 0;
  }

  Hook::of(Some(&repo), &env).send(&Event {
    name: "backup-upload-failed",
    commit: None,
    summary: upload_failed_summary(&outcome),
    detail: upload_failed_detail(&outcome),
  });

  1
}

// A deployment from an artifact (docs/artifacts.md, 「A deployment from an artifact」): no checkout is read, and the
// run says each line as it comes and the last one on its own line.
fn from_artifact(archive: &std::path::Path, args: &DeployFlags, env: &bmo_testbed_core::turn::Env) -> u8 {
  let ask = bmo_testbed_deploy::artifact_deploy::ArtifactAsk {
    archive: archive.to_path_buf(),
    message: args.message.clone(),
    dry_run: args.dry_run,
  };
  let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(runtime) => runtime,
    Err(why) => return refused(&why.to_string()),
  };

  match runtime.block_on(bmo_testbed_deploy::artifact_deploy::deploy_artifact(&ask, env, &|line| {
    println!("{line}")
  })) {
    Ok(line) => {
      println!("{line}");
      0
    }
    Err(why) => {
      eprintln!("{why}");
      1
    }
  }
}
