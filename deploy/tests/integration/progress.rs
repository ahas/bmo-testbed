//! The progress record (docs/deploy.md, 「What a deployment says of itself as it runs」): begun by the
//! outermost run, carried on by a run in the same unit, written stage by stage and step by step, and ended
//! by the run that began it.

use std::path::Path;

use bmo_testbed_client::{DeploymentState, PageQuery, RestartsQuery, StageName, StepState, StopCode};
use bmo_testbed_deploy::progress::{
  DeployProgress, DeployStart, Outcome, Part, Stage, StepAhead, begin_deploy, read_deploy_progress,
};
use bmo_testbed_record::Record;

fn now() -> i64 {
  1_760_418_900_000
}

fn read(path: &Path) -> DeployProgress {
  read_deploy_progress(std::fs::read_to_string(path).ok().as_deref()).unwrap()
}

fn start(unit: Option<&str>, pid: i64, commit: Option<&str>) -> DeployStart {
  DeployStart {
    unit: unit.map(str::to_owned),
    pid,
    commit: commit.map(str::to_owned),
    plan: Stage::PLANNED.to_vec(),
    now: 1_760_418_000_000,
    repository: None,
    record: None,
  }
}

#[test]
fn a_run_writes_its_plan_its_stages_and_its_steps_and_ends() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("deploy-progress.json");
  let run = begin_deploy(&path, &start(Some("bmo-deploy-1"), 41, None), now);

  assert!(run.owner);
  assert_eq!(read(&path).plan, Stage::PLANNED);

  run.carries(&"c".repeat(40));
  run.stage(Stage::Check);
  run.stage(Stage::Check);
  run.passes_over(&[Stage::Build]);
  run.stage(Stage::Backup);
  run.steps_ahead(&[
    ("snapshot", Some(Part::Snapshot)),
    ("pack", Some(Part::Pack)),
    ("check", Some(Part::Check)),
  ]);
  run.began("snapshot", Some(Part::Snapshot));

  let record = read(&path);

  assert_eq!(record.commit.as_deref(), Some("c".repeat(40).as_str()));
  assert_eq!(
    record.stages.iter().map(|one| one.stage).collect::<Vec<_>>(),
    [Stage::Check, Stage::Backup]
  );
  assert!(!record.plan.contains(&Stage::Build));
  assert_eq!(record.running.as_ref().map(|step| step.name.as_str()), Some("snapshot"));
  assert_eq!(record.running.as_ref().and_then(|step| step.stage), Some(Stage::Backup));
  assert_eq!(
    record.ahead,
    [
      StepAhead {
        name: "pack".to_owned(),
        part: Some(Part::Pack),
      },
      StepAhead {
        name: "check".to_owned(),
        part: Some(Part::Check),
      },
    ]
  );

  run.step("snapshot", Some(Part::Snapshot), 0, 0.0, 1.5);
  run.stage(Stage::Tree);

  let record = read(&path);

  assert!(record.running.is_none() && record.ahead.is_empty());
  assert_eq!(record.steps.len(), 1);
  assert_eq!(record.steps[0].stage, Some(Stage::Backup));
  assert_eq!(record.steps[0].ran_seconds, 1.5);

  run.end(Outcome::Live, None);

  let ended = read(&path).ending.unwrap();

  assert_eq!(ended.outcome, Outcome::Live);
  assert_eq!(ended.at, now());

  // An ended record is written no more.
  run.stage(Stage::After);
  assert_eq!(read(&path).stages.len(), 3);
}

#[test]
fn a_run_in_the_same_unit_carries_the_record_on_and_the_first_ends_it() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("deploy-progress.json");
  let outer = begin_deploy(&path, &start(Some("bmo-deploy-1"), 41, None), now);

  outer.stage(Stage::Check);

  let inner = begin_deploy(&path, &start(Some("bmo-deploy-1"), 42, Some("abc")), now);

  assert!(!inner.owner);
  assert_eq!(read(&path).pid, 41);
  assert_eq!(read(&path).commit.as_deref(), Some("abc"));

  inner.stage(Stage::Restart);
  inner.rolled_back();
  inner.end(Outcome::Failed, Some("not answering"));
  assert!(read(&path).ending.is_none());

  outer.end(Outcome::Failed, Some("not answering"));

  let record = read(&path);

  assert_eq!(record.stages.len(), 2);
  assert!(record.rolled_back);
  assert_eq!(record.ending.unwrap().outcome, Outcome::RolledBack);
}

#[test]
fn a_run_in_another_unit_or_none_begins_afresh_and_the_old_run_writes_no_more() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("deploy-progress.json");
  let old = begin_deploy(&path, &start(Some("bmo-deploy-1"), 41, None), now);
  let new = begin_deploy(&path, &start(Some("bmo-deploy-2"), 43, None), now);

  assert!(new.owner);

  old.stage(Stage::Backup);
  assert!(read(&path).stages.is_empty());
  assert_eq!(read(&path).unit.as_deref(), Some("bmo-deploy-2"));

  let bare = begin_deploy(&path, &start(None, 44, None), now);

  assert!(bare.owner);
  assert_eq!(read(&path).pid, 44);
}

#[test]
fn a_record_that_does_not_read_is_none() {
  assert_eq!(read_deploy_progress(None), None);
  assert_eq!(read_deploy_progress(Some("not json")), None);
  assert_eq!(
    read_deploy_progress(Some(
      r#"{"unit":null,"pid":1,"commit":null,"startedAt":1,"stages":[{"stage":"lunch","at":1}]}"#
    )),
    None
  );

  let bare = read_deploy_progress(Some(r#"{"unit":null,"pid":1,"commit":null,"startedAt":1}"#)).unwrap();

  assert!(bare.plan.is_empty() && bare.ending.is_none() && !bare.rolled_back);
}

// "a run handed the testbed's data root is written there as a deployment, with its lines and restarts"
#[test]
fn a_run_handed_a_record_is_written_there_with_its_lines_and_restarts() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("deploy-progress.json");
  let started = DeployStart {
    repository: Some("repository".to_owned()),
    record: Some(dir.path().to_path_buf()),
    now: bmo_testbed_core::queue::now_ms(),
    ..start(Some("bmo-deploy-3"), i64::from(std::process::id()), Some("abc"))
  };
  let run = begin_deploy(&path, &started, now);
  let id = run.deployment_id().expect("the deployment is written as the run begins");

  run.stage(Stage::Check);
  run.steps_ahead(&[("types", None), ("tests", None)]);
  run.step("types", None, 0, 1.5, 2.0);
  run.said("the gate passed");
  run.stage(Stage::Restart);
  run.restarting("first.service", "deployment restarts it");

  let record = Record::open(dir.path()).unwrap();
  let under_way = record.deployment(id).unwrap().unwrap();

  assert_eq!(under_way.state, DeploymentState::Running);
  assert_eq!(under_way.repository_name.as_deref(), Some("repository"));
  assert_eq!(
    under_way.steps.iter().map(|step| (step.name.as_str(), step.state)).collect::<Vec<_>>(),
    vec![("types", StepState::Ended)]
  );
  assert_eq!((under_way.steps[0].waited_ms, under_way.steps[0].spent_ms), (Some(1500), Some(2000)));
  assert_eq!(under_way.stages.last().map(|at| at.stage), Some(StageName::Restart));

  run.end(Outcome::Failed, Some("the service did not come up"));

  let ended = record.deployment(id).unwrap().unwrap();
  let lines = record.log_lines(id, &PageQuery::default()).unwrap().unwrap();
  let restarts = record.restarts(&RestartsQuery::default()).unwrap();

  assert_eq!((ended.state, ended.stopped_for), (DeploymentState::Failed, Some(StopCode::Restart)));
  assert_eq!(ended.reason.as_deref(), Some("the service did not come up"));
  assert_eq!(
    lines.items.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
    vec!["the gate passed"]
  );
  assert_eq!(
    restarts
      .items
      .iter()
      .map(|one| (one.unit_name.as_str(), one.deployment_id))
      .collect::<Vec<_>>(),
    vec![("first.service", id)]
  );
}

// "a refusal ends the deployment stopped, its code the refusal's"
#[test]
fn a_refusal_ends_the_deployment_stopped_for_the_refusal() {
  let dir = tempfile::tempdir().unwrap();
  let started = DeployStart {
    record: Some(dir.path().to_path_buf()),
    now: bmo_testbed_core::queue::now_ms(),
    ..start(None, i64::from(std::process::id()), None)
  };
  let run = begin_deploy(&dir.path().join("deploy-progress.json"), &started, now);

  run.refuse("the data root stands inside a checkout");

  let ended = Record::open(dir.path())
    .unwrap()
    .deployment(run.deployment_id().unwrap())
    .unwrap()
    .unwrap();

  assert_eq!((ended.state, ended.stopped_for), (DeploymentState::Stopped, Some(StopCode::Refused)));
}
