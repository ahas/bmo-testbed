//! `deploy` and `backups` (docs/boundary.md, 「The verbs」; docs/deploy.md): the deploy run started in a
//! transient unit of its own and run there, its gate answered by the suite crate; and the backups sent away
//! and rotated, one run at a time. The runs themselves are the deploy crate's; this is the verbs' side.

use std::path::Path;
use std::time::Instant;

use bmo_testbed_client::{TestRun, TestRunState};
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::deploy_steps::DeployStep as OwnStep;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::{LockWait, now_ms, take_lock};
use bmo_testbed_core::turn::{Env, FRONT_VARIABLE, NOT_RUN_EXIT_CODE, harness_pid, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_deploy::console::{LayerPin, PinnedLayer};
use bmo_testbed_deploy::deployment::{
  BuildAsk, DeployAsk, DeployPlaces, Gate, GateAsk, GateOutcome, GateStep, LaidUnits, Machine, Passed,
  SuitePassed, deploy as deploy_run,
};
use bmo_testbed_deploy::progress::DEPLOY_PROGRESS_VARIABLE;
use bmo_testbed_deploy::unit::{
  DEPLOY_SLICE, DeployUnitStart, deploy_unit_name, deploy_unit_of_process, hand_off, this_again,
};
use bmo_testbed_deploy::upload::{
  AwsHands, BackupNames, Remote, send_backups, upload_failed_detail, upload_failed_summary,
};
use bmo_testbed_suite::once_more::run_judged;
use bmo_testbed_suite::quick::shared_versions::find_shared;
use bmo_testbed_suite::run::{SuiteAsk, SuiteWords, ask_of};
use bmo_testbed_suite::static_check::{check_steps, run_static_check, spawn_step};

use crate::home_of;
use crate::placement::{BenchFor, MachinePlacement, PLACEMENT_VARIABLE, ReadyBench};
use crate::recorded::suite_record;
use crate::suite::repository;
use crate::words::{BINARY, DeployFlags};

/// The gate a deployment passes: the declared static check over the range from what was live, then the
/// suite over the same range, what failed run once more, unless the deployment left the suite out.
pub struct SuiteGate<'a> {
  pub repo: &'a Repository,
  pub hook: &'a Hook,
}

impl Gate for SuiteGate<'_> {
  fn pass(&self, ask: &GateAsk<'_>) -> GateOutcome {
    let mut steps = Vec::new();
    let failed = run_static_check(
      &self.repo.root,
      &check_steps(self.repo, ask.since),
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
      record: Some(suite_record(&data_root(), self.repo, command)),
      ..ask_of(&words, &self.repo.declaration.test)
    };
    let (ended, flaky) = run_judged(self.repo, &judged, &data_root(), self.hook);
    let code = ended.code;

    steps.push(GateStep {
      name: OwnStep::Tests.name().to_owned(),
      code: i64::from(code),
      waited_seconds: 0.0,
      ran_seconds: started.elapsed().as_secs_f64(),
    });

    GateOutcome {
      passed: code == 0,
      reason: fill_marks(&DEPLOY.run.gate_suite, &[("code", &code.to_string())]),
      flaky: flaky.tests.iter().map(|one| (one.test.clone(), one.file.clone())).collect(),
      steps,
      suite_passed: None,
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

/// The gate passed on the placed machine the settings name (docs/deploy.md, 「Where a deployment runs」): the
/// commit benched there with the checkout's outside trees, in the repository's own deployment bench, the
/// static check and the suite run there as that machine's own verbs, the suite left out where that machine's
/// record holds a passed run of the whole suite on the commit, and the Cargo release build run there and
/// read back. The gate here stands in where the machine cannot be reached.
pub struct PlacedGate<'a> {
  pub machine: MachinePlacement,
  /// The deployed repository's name, which its bench there stands under.
  pub repository: String,
  pub here: &'a dyn Gate,
  /// Whether the gate is passed at all; the build goes to the machine either way.
  pub checked: bool,
  /// Whether the testbed there is brought to a live release that carries one; not where the caller set the
  /// placement empty (docs/boundary.md, 「The environment」).
  pub brings_testbed: bool,
}

impl PlacedGate<'_> {
  fn testbed(&self) -> String {
    self.machine.settings.testbed.clone()
  }

  // One verb of the machine's testbed run in the bench, timed: its step.
  fn step(&self, bench: &ReadyBench, name: &str, words: &[String], setup: bool) -> GateStep {
    let started = Instant::now();
    let code = self.machine.run(bench, words, setup, None);

    GateStep {
      name: name.to_owned(),
      code: i64::from(code),
      waited_seconds: 0.0,
      ran_seconds: started.elapsed().as_secs_f64(),
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

    if !self.machine.reached() {
      return self.here.pass(ask);
    }

    let bench = match self.machine.ready(ask.root, ask.commit, BenchFor::Deployment(&self.repository)) {
      Ok(bench) => bench,
      Err(why) => {
        return GateOutcome {
          passed: false,
          reason: why,
          ..GateOutcome::default()
        };
      }
    };
    let since = ask.since.iter().flat_map(|since| ["--since".to_owned(), (*since).to_owned()]);
    let check = [self.testbed(), "check".to_owned(), "static".to_owned()]
      .into_iter()
      .chain(since.clone())
      .collect::<Vec<_>>();
    let mut steps = vec![self.step(&bench, "check", &check, true)];

    if steps[0].code != 0 {
      return GateOutcome {
        passed: false,
        reason: fill_marks(&DEPLOY.run.gate_check, &[("code", &steps[0].code.to_string())]),
        steps,
        ..GateOutcome::default()
      };
    }

    // Where the deploying machine's record held no passed run of the whole suite on the commit, the placed
    // machine's own record is asked, since a suite placed there is written there alone.
    let suite_passed = if ask.suite { self.passed_there(&bench, ask.commit) } else { None };

    if !ask.suite || suite_passed.is_some() {
      return GateOutcome {
        passed: true,
        steps,
        suite_passed,
        ..GateOutcome::default()
      };
    }

    let suite = [self.testbed(), "suite".to_owned(), "--again".to_owned()]
      .into_iter()
      .chain(since)
      .collect::<Vec<_>>();

    steps.push(self.step(&bench, OwnStep::Tests.name(), &suite, false));

    let code = steps[1].code;

    GateOutcome {
      passed: code == 0,
      reason: fill_marks(&DEPLOY.run.gate_suite, &[("code", &code.to_string())]),
      steps,
      ..GateOutcome::default()
    }
  }

  fn dependents(&self, ask: &GateAsk<'_>) -> GateOutcome {
    self.here.dependents(ask)
  }

  fn build(&self, ask: &BuildAsk<'_>) -> Option<Result<(), String>> {
    if !self.machine.reached() {
      return None;
    }

    let bench = match self.machine.ready(ask.root, ask.commit, BenchFor::Deployment(&self.repository)) {
      Ok(bench) => bench,
      Err(why) => return Some(Err(why)),
    };
    let build = [self.testbed(), "cargo".to_owned()]
      .into_iter()
      .chain(["build", "--workspace", "--locked", "--profile", ask.profile].map(str::to_owned))
      .collect::<Vec<_>>();
    let built = self.step(&bench, OwnStep::CargoRelease.name(), &build, true);

    if built.code != 0 {
      return Some(Err(fill_marks(&LANES.run.exit, &[("code", &built.code.to_string())])));
    }

    Some(self.machine.fetch_binaries(&bench, ask.profile, ask.binaries, ask.built))
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

fn refused(line: &str) -> u8 {
  eprintln!("{}", fill_marks(&DEPLOY.run.refused, &[("why", line)]));
  REFUSED_EXIT_CODE
}

// The release of the console layer the shared versions of a repository pin, which the console probe reads a
// manifest by and holds its layer to (docs/deploy.md, 「The console probe」); the deploy run asks it of the
// repository the deployed commit's declaration names, where that commit brings a console screen.
fn pinned_layer(repository: &str, env: &Env) -> PinnedLayer {
  match find_shared(repository, env) {
    Ok(found) => match found.packages.get(&DEPLOY.console.layer_package) {
      Some(pin) => PinnedLayer::Pinned(LayerPin {
        version: pin.version.clone(),
        file: pin.file.clone(),
        sha256: pin.sha256.clone(),
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
    let start = DeployUnitStart {
      unit: deploy_unit_name(bmo_testbed_core::queue::now_ms()),
      slice: DEPLOY_SLICE.to_owned(),
      cwd: repo.root.clone(),
      command: this_again(),
      env: handed(&places, &env),
    };

    return hand_off(&start, env.get("PATH").map_or("", String::as_str), &|line| println!("{line}"));
  }

  let hook = Hook::of(Some(&repo), &env);
  // The upload runs in a unit of its own, in the repository, aiming at the backups this run took.
  let upload: Vec<String> = std::iter::once(format!("--working-directory={}", repo.root.display()))
    .chain(
      handed(&places, &env)
        .into_iter()
        .map(|(name, value)| format!("--setenv={name}={value}")),
    )
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
