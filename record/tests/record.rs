//! The record (docs/boundary.md, 「The record」): each member written, changed and read back in the shape the
//! client answers, its event written with it, the pages by their cursor, and the window past which events
//! are gone.

use std::path::Path;

use bmo_testbed_client::{
  Deployment, DeploymentState, EventKind, PageQuery, RestartsQuery, TestRun, TestRunKind, TestRunState,
  TestRunsQuery,
};
use bmo_testbed_record::{EventsRead, GateGround, GatePass, Record, TestRunEnd, record_path};

fn test_run(checkout: &str, pid: i64) -> TestRun {
  TestRun {
    id: 0,
    kind: TestRunKind::Command,
    checkout: checkout.to_owned(),
    repository_name: None,
    commit: None,
    whole: None,
    clean: None,
    command: "cargo test".to_owned(),
    scope: Some("scope-1".to_owned()),
    unit_name: None,
    pid,
    state: TestRunState::Passed,
    passed_count: 9,
    failed_count: 9,
    skipped_count: 9,
    log_path: "/logs/one.log".to_owned(),
    started_at: bmo_testbed_core::queue::now_ms(),
    ended_at: Some(1),
  }
}

fn deployment(pid: i64, started_at: i64, state: DeploymentState) -> Deployment {
  Deployment {
    id: 0,
    repository_name: Some("repository".to_owned()),
    unit_name: Some("deploy-unit".to_owned()),
    pid,
    commit: Some("abc123".to_owned()),
    state,
    plan: vec![],
    stages: vec![],
    steps: vec![],
    rolled_back: false,
    stopped_for: None,
    reason: None,
    started_at,
    ended_at: None,
  }
}

fn kinds(record: &Record, after: i64) -> Vec<EventKind> {
  match record.events(after, &[], None).unwrap() {
    EventsRead::Page(page) => page.items.iter().map(|event| event.kind).collect(),
    EventsRead::Gone(through) => panic!("events past {through} are gone"),
  }
}

fn own_pid() -> i64 {
  i64::from(std::process::id())
}

// "a test run is written running with its own id, and ended with its counts, an event for each"
#[test]
fn a_test_run_is_written_running_then_ended_with_an_event_for_each() {
  let dir = tempfile::tempdir().unwrap();
  let mut record = Record::open(dir.path()).unwrap();
  let begun = record.begin_test_run(&test_run("/checkout", own_pid())).unwrap();

  assert!(record_path(dir.path()).is_file());
  assert!(begun.id > 0);
  assert_eq!((begun.state, begun.ended_at), (TestRunState::Running, None));

  let end = TestRunEnd {
    state: TestRunState::Failed,
    passed_count: 3,
    failed_count: 1,
    skipped_count: 2,
    ended_at: begun.started_at + 10,
  };
  let ended = record.end_test_run(begun.id, &end).unwrap().expect("the run stands");

  assert_eq!(record.test_run(begun.id).unwrap(), Some(ended.clone()));
  assert_eq!((ended.passed_count, ended.failed_count, ended.skipped_count), (3, 1, 2));
  assert_eq!(ended.scope.as_deref(), Some("scope-1"));
  assert_eq!(kinds(&record, 0), vec![EventKind::TestRunCreated, EventKind::TestRunEnded]);
  assert_eq!(record.end_test_run(begun.id + 100, &end).unwrap(), None);
}

// "a run still running whose process is gone reads as gone"
#[test]
fn a_running_test_run_whose_process_is_gone_reads_as_gone() {
  let dir = tempfile::tempdir().unwrap();
  let mut record = Record::open(dir.path()).unwrap();
  let begun = record.begin_test_run(&test_run("/checkout", i64::from(i32::MAX))).unwrap();

  assert_eq!(record.test_run(begun.id).unwrap().unwrap().state, TestRunState::Gone);
}

// "test runs page newest first, narrowed by checkout, the cursor going on to older ids"
#[test]
fn test_runs_page_newest_first_by_checkout_and_cursor() {
  let dir = tempfile::tempdir().unwrap();
  let mut record = Record::open(dir.path()).unwrap();
  let ids: Vec<i64> = ["/a", "/b", "/a", "/a"]
    .iter()
    .map(|checkout| record.begin_test_run(&test_run(checkout, own_pid())).unwrap().id)
    .collect();
  let first = record
    .test_runs(&TestRunsQuery {
      checkout: Some("/a".to_owned()),
      limit: Some(2),
      ..TestRunsQuery::default()
    })
    .unwrap();
  let rest = record
    .test_runs(&TestRunsQuery {
      checkout: Some("/a".to_owned()),
      after: first.items.last().map(|run| run.id),
      limit: Some(2),
    })
    .unwrap();

  assert_eq!(first.items.iter().map(|run| run.id).collect::<Vec<_>>(), vec![ids[3], ids[2]]);
  assert!(first.more);
  assert_eq!(rest.items.iter().map(|run| run.id).collect::<Vec<_>>(), vec![ids[0]]);
  assert!(!rest.more);
}

// "a deployment is made once for its run and written over after, with its lines and restarts"
#[test]
fn a_deployment_is_made_once_then_changed_with_its_lines_and_restarts() {
  let dir = tempfile::tempdir().unwrap();
  let mut record = Record::open(dir.path()).unwrap();
  let now = bmo_testbed_core::queue::now_ms();
  let made = record.write_deployment(&deployment(own_pid(), now, DeploymentState::Running)).unwrap();
  let changed = record.write_deployment(&deployment(own_pid(), now, DeploymentState::Live)).unwrap();
  let other = record
    .write_deployment(&deployment(own_pid(), now + 1, DeploymentState::Running))
    .unwrap();

  assert_eq!(made.id, changed.id);
  assert_ne!(made.id, other.id);
  assert_eq!(record.deployment(made.id).unwrap().unwrap().state, DeploymentState::Live);
  assert_eq!(record.deployment_of(own_pid(), now).unwrap(), Some(made.id));
  assert_eq!(
    record
      .deployments(&PageQuery::default())
      .unwrap()
      .items
      .iter()
      .map(|one| one.id)
      .collect::<Vec<_>>(),
    vec![other.id, made.id]
  );

  for line in ["one", "two", "three"] {
    record.add_log_line(made.id, line).unwrap();
  }

  let lines = record
    .log_lines(
      made.id,
      &PageQuery {
        after: None,
        limit: Some(2),
      },
    )
    .unwrap()
    .unwrap();
  let after = record
    .log_lines(
      made.id,
      &PageQuery {
        after: lines.items.last().map(|line| line.id),
        limit: None,
      },
    )
    .unwrap()
    .unwrap();

  assert_eq!(
    lines.items.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
    vec!["one", "two"]
  );
  assert_eq!(after.items.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(), vec!["three"]);
  assert_eq!(record.log_lines(other.id + 100, &PageQuery::default()).unwrap(), None);

  record.add_restart("first.service", made.id, "deployment stops it").unwrap();
  record.add_restart("second.service", made.id, "deployment restarts it").unwrap();

  let first = record
    .restarts(&RestartsQuery {
      unit_name: Some("first.service".to_owned()),
      ..RestartsQuery::default()
    })
    .unwrap();

  assert_eq!(first.items.len(), 1);
  assert_eq!(
    (first.items[0].deployment_id, first.items[0].reason.as_str()),
    (made.id, "deployment stops it")
  );
  assert_eq!(
    kinds(&record, 0),
    vec![
      EventKind::DeploymentCreated,
      EventKind::DeploymentChanged,
      EventKind::DeploymentCreated,
      EventKind::RestartCreated,
      EventKind::RestartCreated,
    ]
  );
}

// Every event of the record aged past the window, as if written that long ago.
fn age_events(dir: &Path) {
  let connection = rusqlite::Connection::open(record_path(dir)).unwrap();

  connection.execute("UPDATE events SET at = 0", []).unwrap();
}

// "events narrow by kind, and a resume from before the window is gone"
#[test]
fn events_narrow_by_kind_and_a_resume_past_the_window_is_gone() {
  let dir = tempfile::tempdir().unwrap();
  let mut record = Record::open(dir.path()).unwrap();
  let first = record
    .post_event(EventKind::QueueChanged, &serde_json::json!({ "name": "wide" }))
    .unwrap();

  record.begin_test_run(&test_run("/checkout", own_pid())).unwrap();

  let queues = match record.events(0, &[EventKind::QueueChanged], None).unwrap() {
    EventsRead::Page(page) => page.items,
    EventsRead::Gone(_) => panic!("nothing is past the window yet"),
  };

  assert_eq!(queues.len(), 1);
  assert_eq!(queues[0].data, serde_json::json!({ "name": "wide" }));
  assert_eq!(record.window_start().unwrap(), first.id - 1);

  age_events(dir.path());

  let last = record.post_event(EventKind::QueueChanged, &serde_json::json!({})).unwrap();

  assert_eq!(record.last_event_id().unwrap(), last.id);
  assert!(
    matches!(record.events(first.id, &[], None).unwrap(), EventsRead::Gone(through) if through == last.id - 1)
  );
  assert!(
    matches!(record.events(last.id - 1, &[], None).unwrap(), EventsRead::Page(page) if page.items.len() == 1)
  );
}

// A run of the repository's suite on a commit, whole or not and from a clean tree or not, begun the given
// milliseconds before now and ended as asked; its id.
fn suite_run(
  record: &mut Record,
  commit: &str,
  whole: Option<bool>,
  clean: Option<bool>,
  ago: i64,
  state: TestRunState,
) -> i64 {
  let begun = record
    .begin_test_run(&TestRun {
      kind: TestRunKind::Suite,
      repository_name: Some("repository".to_owned()),
      commit: Some(commit.to_owned()),
      whole,
      clean,
      started_at: bmo_testbed_core::queue::now_ms() - ago,
      ..test_run("/checkout", own_pid())
    })
    .unwrap();
  let end = TestRunEnd {
    state,
    passed_count: 1,
    failed_count: 0,
    skipped_count: 0,
    ended_at: begun.started_at + 1,
  };

  record.end_test_run(begun.id, &end).unwrap();
  begun.id
}

// "the passed suite a deployment takes is the newest whole run of the repository on the commit from a clean
// tree since the moment asked, where it passed; a run whose wholeness or tree is not held is none"
#[test]
fn the_passed_suite_is_the_newest_whole_clean_run_on_the_commit_where_it_passed() {
  let dir = tempfile::tempdir().unwrap();
  let mut record = Record::open(dir.path()).unwrap();
  let now = bmo_testbed_core::queue::now_ms();
  let since = now - 10_000;
  let passed = |record: &Record, commit: &str| {
    record.passed_suite("repository", commit, since).unwrap().map(|run| run.id)
  };

  let first = suite_run(&mut record, "aaa", Some(true), Some(true), 1_000, TestRunState::Passed);

  assert_eq!(passed(&record, "aaa"), Some(first));
  assert_eq!(passed(&record, "bbb"), None);
  assert_eq!(record.passed_suite("another", "aaa", since).unwrap(), None);

  // Runs that do not count leave the passed one standing.
  suite_run(&mut record, "aaa", Some(false), Some(true), 500, TestRunState::Failed);
  suite_run(&mut record, "aaa", Some(true), Some(false), 500, TestRunState::Failed);
  suite_run(&mut record, "aaa", None, None, 500, TestRunState::Passed);
  assert_eq!(passed(&record, "aaa"), Some(first));

  // A run begun before the moment asked is none, and so is one held without its wholeness or its tree.
  suite_run(&mut record, "old", Some(true), Some(true), 60_000, TestRunState::Passed);
  suite_run(&mut record, "unknown", None, Some(true), 1_000, TestRunState::Passed);
  suite_run(&mut record, "unknown", Some(true), None, 1_000, TestRunState::Passed);
  assert_eq!(passed(&record, "old"), None);
  assert_eq!(passed(&record, "unknown"), None);

  // A newer whole clean run that failed outweighs the older one that passed.
  suite_run(&mut record, "aaa", Some(true), Some(true), 100, TestRunState::Failed);
  assert_eq!(passed(&record, "aaa"), None);
}

// "A gate's pass ... the repository, the commit, the commit the gate's range began at, the heads of the
// repositories laid beside the tree it judged": a pass is found on exactly its ground and from the moment asked.
#[test]
fn a_gates_pass_is_found_on_exactly_its_ground_and_no_earlier_than_asked() {
  let dir = tempfile::tempdir().unwrap();
  let mut record = Record::open(dir.path()).unwrap();
  let at = bmo_testbed_core::queue::now_ms();
  let ground = GateGround {
    repository: "repository".to_owned(),
    commit: "c".repeat(40),
    since: Some("a".repeat(40)),
    heads: vec!["1".repeat(40), "2".repeat(40)],
    toolchains: "cargo: rustc 1.0.0\n".to_owned(),
  };

  record
    .write_gate_pass(&GatePass {
      ground: ground.clone(),
      machine: None,
      at,
    })
    .unwrap();

  let found =
    |ground: &GateGround, since: i64| record.gate_passed(ground, since).unwrap().map(|pass| pass.at);

  assert_eq!(found(&ground, at), Some(at));
  assert_eq!(found(&ground, at + 1), None);
  assert_eq!(
    found(
      &GateGround {
        since: None,
        ..ground.clone()
      },
      0
    ),
    None
  );
  assert_eq!(
    found(
      &GateGround {
        heads: vec!["1".repeat(40)],
        ..ground.clone()
      },
      0
    ),
    None
  );
}

// The newest ended deployment of a commit: one still running is passed over, and another repository's or another
// commit's is none of it.
#[test]
fn the_newest_ended_deployment_of_a_commit_passes_over_one_still_running() {
  let dir = tempfile::tempdir().unwrap();
  let mut record = Record::open(dir.path()).unwrap();
  let alive = i64::from(std::process::id());
  // Recent moments, since the record keeps only recent deployments.
  let at = bmo_testbed_core::queue::now_ms();
  let ended = |started_at: i64, state: DeploymentState| Deployment {
    ended_at: Some(started_at + 1),
    ..deployment(alive, started_at, state)
  };

  record.write_deployment(&ended(at + 1, DeploymentState::Live)).unwrap();
  record.write_deployment(&ended(at + 2, DeploymentState::Failed)).unwrap();
  record.write_deployment(&deployment(alive, at + 3, DeploymentState::Running)).unwrap();
  record
    .write_deployment(&Deployment {
      commit: Some("def456".to_owned()),
      ..ended(at + 4, DeploymentState::Live)
    })
    .unwrap();

  let newest = record.newest_ended_deployment("repository", "abc123").unwrap();

  assert_eq!(
    newest.map(|deployment| (deployment.started_at, deployment.state)),
    Some((at + 2, DeploymentState::Failed))
  );
  assert_eq!(record.newest_ended_deployment("another", "abc123").unwrap(), None);
}
