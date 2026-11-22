//! A deployment asked again (docs/deploy.md, 「The deployment line」): a repository and commit stand once, read as
//! commits whatever name they were asked by, and a commit whose gate failed is refused while main still stands
//! on it.

use std::path::Path;
use std::time::{Duration, Instant};

use bmo_testbed_client::{Deployment, DeploymentState, StopCode};
use bmo_testbed_core::queue::now_ms;
use bmo_testbed_deploy::again::{AskAgain, Asked, judged};
use bmo_testbed_deploy::line::{DeployTicket, RootTurn, standing};
use bmo_testbed_record::Record;

const WHOLE: &str = "d9354126cf6753f09f6485d0f78fa3311abbf340";
const OTHER: &str = "2880be22e026a1b2c3d4e5f60718293a4b5c6d7e";

// The whole commit a name names, as git would answer it of a checkout holding the two commits above.
fn resolve(named: &str) -> Option<String> {
  [WHOLE, OTHER].into_iter().find(|whole| whole.starts_with(named)).map(str::to_owned)
}

// A call of the whole commit above of `app` against the turn and record under the folder given, starting its own
// run of nothing, unmarked, main standing on the commit.
fn asking<'a>(dir: &'a Path, turn: &'a Path) -> AskAgain<'a> {
  AskAgain {
    repository: "app",
    commit: WHOLE,
    turn,
    data_root: dir,
    own_run: false,
    no_check: false,
    first: false,
    resolve: &resolve,
    main_head: &|| Some(WHOLE.to_owned()),
  }
}

// A deploy root's turn under the folder given, looked at every few milliseconds.
fn turn_in(dir: &Path) -> RootTurn {
  RootTurn {
    lock: dir.join("root/deploy-run.lock"),
    say_every: Duration::from_millis(5),
    poll: Some(Duration::from_millis(5)),
  }
}

// A deployment of the commit above that ended now in the record under the folder given, stopped in the stage
// given; the record keeps only recent deployments.
fn ended(dir: &Path, stopped_for: StopCode) {
  let now = now_ms();
  let deployment = Deployment {
    id: 0,
    repository_name: Some("app".to_owned()),
    unit_name: Some("bmo-deploy-1".to_owned()),
    pid: 1,
    commit: Some(WHOLE.to_owned()),
    state: DeploymentState::Failed,
    plan: vec![],
    stages: vec![],
    steps: vec![],
    rolled_back: false,
    stopped_for: Some(stopped_for),
    reason: Some("the suite ended with exit 1".to_owned()),
    started_at: now,
    ended_at: Some(now),
  };

  Record::open(dir).unwrap().write_deployment(&deployment).unwrap();
}

// "reads the commit each of those tickets names the same way, so `d935412` and `d9354126cf6753f0…` are one
// commit"; "the call attaches to it and starts nothing"; "An ask that brings what the standing one lacks stands on
// its own"
#[test]
fn a_deployment_of_the_commit_waiting_under_a_shorter_name_is_attached_to_unless_the_call_brings_its_own_run()
 {
  let dir = tempfile::tempdir().unwrap();
  let turn = turn_in(dir.path());
  let holder = turn.take(&DeployTicket::asked("app", OTHER, Some("bmo-deploy-1")), &|_| {}).unwrap();

  std::thread::scope(|scope| {
    scope.spawn(|| turn.take(&DeployTicket::asked("app", "d935412", Some("bmo-deploy-2")), &|_| {}));

    let given_up = Instant::now() + Duration::from_secs(60);

    while standing(&turn.lock).waiting.is_empty() && Instant::now() < given_up {
      std::thread::sleep(Duration::from_millis(5));
    }

    let attached = judged(&asking(dir.path(), &turn.lock));
    let own = judged(&AskAgain {
      own_run: true,
      ..asking(dir.path(), &turn.lock)
    });

    drop(holder);

    match attached {
      Asked::Standing(ticket) => assert_eq!(ticket.unit.as_deref(), Some("bmo-deploy-2")),
      other => panic!("attached to the waiting deployment, not {other:?}"),
    }
    assert_eq!(own, Asked::Start);
  });
}

// "Where that one failed in its gate ... and the repository's main branch ... still stands on that very commit,
// the call refuses"; "A main that has moved since ... reopens it, and so does an ask with `--first` ... an ask with
// `--no-check` runs no gate and is not refused for one"
#[test]
fn a_commit_whose_gate_failed_is_refused_while_main_stands_on_it_and_reopened_by_a_moved_main_first_or_no_gate()
 {
  let dir = tempfile::tempdir().unwrap();
  let turn = dir.path().join("root/deploy-run.lock");

  ended(dir.path(), StopCode::Check);

  match judged(&asking(dir.path(), &turn)) {
    Asked::FailedUnmoved(failed) => assert_eq!(failed.unit_name.as_deref(), Some("bmo-deploy-1")),
    other => panic!("refused for the failed gate, not {other:?}"),
  }

  let moved = || Some(OTHER.to_owned());

  assert_eq!(
    judged(&AskAgain {
      main_head: &moved,
      ..asking(dir.path(), &turn)
    }),
    Asked::Start
  );
  assert_eq!(
    judged(&AskAgain {
      first: true,
      ..asking(dir.path(), &turn)
    }),
    Asked::Start
  );
  assert_eq!(
    judged(&AskAgain {
      no_check: true,
      own_run: true,
      ..asking(dir.path(), &turn)
    }),
    Asked::Start
  );
}

// "Where that one failed in its gate (the `check` stage)": a failure in any later stage refuses nothing.
#[test]
fn a_commit_that_failed_past_its_gate_is_not_refused() {
  let dir = tempfile::tempdir().unwrap();
  let turn = dir.path().join("root/deploy-run.lock");

  ended(dir.path(), StopCode::Build);

  assert_eq!(judged(&asking(dir.path(), &turn)), Asked::Start);
}
