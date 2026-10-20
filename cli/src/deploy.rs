//! `deploy` and `backups` (docs/boundary.md, 「The verbs」; docs/deploy.md): the deploy run started in a
//! transient unit of its own and run there, its gate answered by the suite crate; and the backups sent away
//! and rotated, one run at a time. The runs themselves are the deploy crate's; this is the verbs' side.

use std::path::Path;
use std::time::Instant;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::deploy_steps::DeployStep as OwnStep;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::{LockWait, take_lock};
use bmo_testbed_core::turn::{Env, FRONT_VARIABLE, NOT_RUN_EXIT_CODE, harness_pid, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_deploy::deployment::{
  DeployAsk, DeployPlaces, Gate, GateAsk, GateOutcome, GateStep, Machine, Passed, deploy as deploy_run,
};
use bmo_testbed_deploy::progress::DEPLOY_PROGRESS_VARIABLE;
use bmo_testbed_deploy::unit::{
  DeployUnitStart, deploy_unit_name, deploy_unit_of_process, hand_off, this_again,
};
use bmo_testbed_deploy::upload::{
  AwsHands, BackupNames, Remote, send_backups, upload_failed_detail, upload_failed_summary,
};
use bmo_testbed_suite::once_more::run_judged;
use bmo_testbed_suite::run::{SuiteAsk, SuiteWords, ask_of};
use bmo_testbed_suite::static_check::{check_steps, run_static_check, spawn_step};

use crate::recorded::suite_record;
use crate::suite::repository;
use crate::words::{BINARY, DeployArgs};

/// The gate a deployment passes: the declared static check over the range from what was live, then the
/// suite over the same range, what failed run once more.
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
    }
  }
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

  for name in ["BMO_UNIT", "BMO_TESTBED_EVENT_HOOK", "BMO_CARGO_TARGET_DIR", "BMO_TESTBED_LOCK"] {
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

/// `deploy`: outside a deployment's unit it starts one and ends; inside it, the run. A refusal and a dry
/// run stay in the caller's process.
pub fn deploy(args: &DeployArgs) -> u8 {
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
    dry_run: args.dry_run,
    no_check: args.no_check,
    before_start: args.before_start.clone(),
    unit,
    upload: Some(upload),
    record: Some(data_root()),
  };
  let suite = SuiteGate {
    repo: &repo,
    hook: &hook,
  };
  let gate: &dyn Gate = if args.no_check { &Passed } else { &suite };
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
  let wait = LockWait {
    holder: said.holder.clone(),
    ..LockWait::default()
  };
  let Some(_held) = take_lock(&lock, said.wait_seconds as f64, wait) else {
    eprintln!("{}", fill_marks(&said.locked, &[("seconds", &said.wait_seconds.to_string())]));
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
