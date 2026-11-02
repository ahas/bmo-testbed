//! The machine's turn (docs/testbed.md, 「The turn」): the queue hands a lock out in the order its takers
//! asked, the front ahead of everyone and the next behind the front, a waiter waiting until the lock is let
//! go, and a command inside a turn its caller holds running at once.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::{LockWait, Rank, now_ms, queue_tickets, take_lock};
use bmo_testbed_core::turn::{
  Env, HELD_VARIABLE, Lane, TestbedTurn, TurnAsk, TurnEntry, inside_held_turn, lane_width, lock_path,
  read_queue, take_turn,
};

fn lock_in(dir: &Path) -> PathBuf {
  dir.join("testbed.lock")
}

fn waiting(rank: Rank, name: &str) -> LockWait<'static> {
  LockWait {
    holder: name.to_owned(),
    rank,
    poll: Some(Duration::from_millis(5)),
    ..LockWait::default()
  }
}

#[test]
fn a_lane_s_lock_stands_in_the_temporary_directory_and_the_narrow_lane_s_beside_it() {
  let named = Env::from([("BMO_TESTBED_LOCK".to_owned(), "/x/bmo-testbed.lock".to_owned())]);

  assert_eq!(lock_path(&named, Lane::Wide), PathBuf::from("/x/bmo-testbed.lock"));
  assert_eq!(lock_path(&named, Lane::Narrow), PathBuf::from("/x/bmo-testbed-narrow.lock"));
  assert_eq!(lock_path(&Env::new(), Lane::Wide), PathBuf::from("/tmp/bmo-testbed.lock"));
}

#[test]
fn a_taker_gets_the_lock_only_once_nobody_holds_it_and_lets_it_go_with_its_ticket() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let held = take_lock(&lock, 1.0, waiting(Rank::Ordinary, "first")).expect("a free lock is had");

  assert_eq!(
    queue_tickets(&lock).iter().map(|one| one.holder.as_str()).collect::<Vec<_>>(),
    ["first"]
  );
  assert!(take_lock(&lock, 0.05, waiting(Rank::Ordinary, "second")).is_none());

  drop(held);

  assert!(queue_tickets(&lock).is_empty());
  assert!(take_lock(&lock, 0.05, waiting(Rank::Ordinary, "second")).is_some());
}

#[test]
fn a_ticket_at_the_front_stands_ahead_of_every_ordinary_one_and_a_next_one_between_them() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let held = take_lock(&lock, 1.0, waiting(Rank::Ordinary, "holder")).unwrap();
  let queue = lock.with_extension("lock.queue");

  // Tickets stood by hand for processes that live as long as this one, in the order the ranks put them.
  for (name, holder) in [
    ("000000000000001-000001-", "ordinary"),
    ("0.000000000000002-000001-", "next"),
    ("0-000000000000003-000001-", "front"),
  ] {
    std::fs::write(queue.join(format!("{name}{}", std::process::id())), holder).unwrap();
  }

  let order: Vec<String> = queue_tickets(&lock).into_iter().map(|one| one.holder).collect();

  assert_eq!(order[..3], ["front", "next", "ordinary"]);
  drop(held);
}

#[test]
fn a_waiter_waits_until_the_holder_lets_go_however_long_that_takes() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let held = take_lock(&lock, 1.0, waiting(Rank::Ordinary, "holder")).unwrap();
  let waiter = {
    let lock = lock.clone();

    std::thread::spawn(move || {
      let mut wait_on = |_: Option<&str>| -> Option<i64> { Some(now_ms() + 10) };

      take_lock(
        &lock,
        0.01,
        LockWait {
          wait_on: Some(&mut wait_on),
          ..waiting(Rank::Ordinary, "waiter")
        },
      )
      .is_some()
    })
  };

  std::thread::sleep(Duration::from_millis(200));

  assert!(!waiter.is_finished());
  assert!(queue_tickets(&lock).iter().any(|one| one.holder == "waiter"));

  drop(held);

  assert!(waiter.join().unwrap());
}

fn ask(lock: &Path, env: Env) -> TurnAsk<'static> {
  TurnAsk {
    command: "a check".to_owned(),
    lane: Lane::Wide,
    say: &|_| {},
    path: Some(lock.to_path_buf()),
    env,
    checkout: "/checkout".to_owned(),
    poll: Some(Duration::from_millis(5)),
    notice_seconds: Some(0.05),
  }
}

#[test]
fn a_turn_held_writes_its_record_and_a_command_under_it_runs_inside() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let env = Env::from([("BMO_TESTBED_SCOPE".to_owned(), "/scopes/scope-3/".to_owned())]);
  let turn = take_turn(&ask(&lock, env));
  let (name, pid) = turn.held_variable().expect("an own turn hands its children its mark");
  let holder = read_queue(&lock).holder.expect("the holder writes its record");

  assert_eq!(name, HELD_VARIABLE);
  assert_eq!(
    (holder.scope.as_deref(), holder.command.as_str()),
    (Some("/scopes/scope-3/"), "a check")
  );
  assert!(inside_held_turn(&lock, &Env::from([(HELD_VARIABLE.to_owned(), pid.clone())])));
  assert!(matches!(
    take_turn(&ask(&lock, Env::from([(HELD_VARIABLE.to_owned(), pid)]))),
    TestbedTurn::Inside
  ));

  drop(turn);

  assert!(read_queue(&lock).holder.is_none());
}

// A harness's process that has ended, so a waiter running for it leaves the line.
fn gone_harness() -> String {
  let mut ended = std::process::Command::new("true").spawn().unwrap();
  let pid = ended.id();

  ended.wait().unwrap();
  pid.to_string()
}

#[test]
fn a_turn_not_had_because_its_harness_went_away_says_who_holds_it() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let first = take_turn(&ask(&lock, Env::from([("BMO_TESTBED_SCOPE".to_owned(), "place-3".to_owned())])));
  let second = take_turn(&ask(&lock, Env::from([("CLAUDE_PID".to_owned(), gone_harness())])));
  let holder = TurnEntry {
    scope: Some("place-3".to_owned()),
    checkout: "/checkout".to_owned(),
    command: "a check".to_owned(),
    ..TurnEntry::default()
  };

  assert!(first.had());
  assert!(!second.had());
  assert!(
    second
      .words()
      .contains(&fill_marks(&LANES.waiting.held_by, &[("who", &holder.words())]))
  );
}

// "the narrow lane runs beside it on a quarter of the width": the machine's width over the policy's
// divisor, never less than one
#[test]
fn the_narrow_lane_s_width_is_the_machine_s_over_the_policy_s_divisor_and_never_less_than_one() {
  let env = Env::new();
  let divisor = LANES.numbers.narrow_divisor;

  assert_eq!(lane_width(Lane::Wide, &env, 32), 32);
  assert_eq!(lane_width(Lane::Narrow, &env, 32), 32 / divisor);
  assert_eq!(lane_width(Lane::Narrow, &env, 1), 1);
}
