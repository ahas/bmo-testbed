//! The machine's turn (docs/testbed.md, 「The turn」): the queue hands a lock out in the order its takers
//! asked, the front ahead of everyone and the next behind the front, a waiter waiting until the lock is let
//! go, and a command inside a turn its caller holds running at once; a lane's seats handed out from its one
//! queue, a deployment's own seat beside them, and a seat beside a held one only while memory is spare.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::{
  LockWait, Rank, SeatWait, WaitOn, now_ms, queue_tickets, take_lock, take_seats,
};
use bmo_testbed_core::turn::{
  Env, FRONT_VARIABLE, HELD_VARIABLE, Lane, SEAT_FREE_VARIABLE, SEATS_VARIABLE, Seats, TestbedTurn,
  TurnAsk, TurnEntry, inside_held_turn, lane_width, lock_path, read_queue, seat_paths, take_turn,
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
  let holder = read_queue(&lock).holder().cloned().expect("the holder writes its record");

  assert_eq!(name, HELD_VARIABLE);
  assert_eq!(
    (holder.scope.as_deref(), holder.command.as_str()),
    (Some("/scopes/scope-3/"), "a check")
  );
  assert!(inside_held_turn(&lock, &Env::from([(HELD_VARIABLE.to_owned(), pid.clone())])));
  assert!(matches!(
    take_turn(&ask(&lock, Env::from([(HELD_VARIABLE.to_owned(), pid)]))),
    TestbedTurn::Inside { .. }
  ));

  drop(turn);

  assert!(read_queue(&lock).holder().is_none());
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
  let one_seat = (SEATS_VARIABLE.to_owned(), "1".to_owned());
  let first = take_turn(&ask(
    &lock,
    Env::from([("BMO_TESTBED_SCOPE".to_owned(), "place-3".to_owned()), one_seat.clone()]),
  ));
  let second = take_turn(&ask(&lock, Env::from([("CLAUDE_PID".to_owned(), gone_harness()), one_seat])));
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

// The variables of a wide lane of `seats` seats whose memory gate asks for `free` gigabytes.
fn seated(seats: usize, free: u64) -> Env {
  Env::from([
    (SEATS_VARIABLE.to_owned(), seats.to_string()),
    (SEAT_FREE_VARIABLE.to_owned(), free.to_string()),
  ])
}

// A turn asked for on a thread of its own, answered once it is had.
fn aside(lock: &Path, env: Env) -> JoinHandle<TestbedTurn> {
  let lock = lock.to_path_buf();

  std::thread::spawn(move || take_turn(&ask(&lock, env)))
}

fn until(what: &str, holds: impl Fn() -> bool) {
  let began = Instant::now();

  while !holds() {
    assert!(began.elapsed() < Duration::from_secs(10), "waited too long for {what}");
    std::thread::sleep(Duration::from_millis(5));
  }
}

// Long enough for a waiter that could take a seat to have taken it.
fn a_while() {
  std::thread::sleep(Duration::from_millis(200));
}

// "a command takes one seat": with two seats, two takers hold one each at once, each on half the width, and a
// third waits until one lets go
#[test]
fn two_takers_hold_a_seat_each_at_once_and_a_third_waits_until_one_lets_go() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let first = take_turn(&ask(&lock, seated(2, 0)));
  let second = take_turn(&ask(&lock, seated(2, 0)));

  assert!(first.had() && second.had());
  assert_eq!(read_queue(&lock).holders.len(), 2);
  assert_eq!((first.width(24), second.width(24)), (12, 12));

  let third = aside(&lock, seated(2, 0));

  a_while();
  assert!(!third.is_finished());

  drop(first);

  assert!(third.join().unwrap().had());
  drop(second);
}

fn spare() -> bool {
  true
}

fn seat_wait<'a>(holder: &str, wait_on: Option<WaitOn<'a>>) -> SeatWait<'a> {
  SeatWait {
    lock: LockWait {
      holder: holder.to_owned(),
      wait_on,
      poll: Some(Duration::from_millis(5)),
      ..LockWait::default()
    },
    deployment_seat: None,
    front: false,
    spare: &spare,
  }
}

// "only the first ticket not yet seated may take a seat": a seated ticket stands ahead of nobody, and the
// unseated ones behind it take the seats as they free in the order they asked
#[test]
fn a_seated_ticket_stands_ahead_of_nobody_and_the_unseated_ones_keep_their_order() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let seats = seat_paths(&lock, 2);
  let first = take_seats(&seats, 1.0, seat_wait("first", None)).expect("a free seat is had");
  let second = take_seats(&seats, 0.05, seat_wait("second", None)).expect("a seated ticket blocks nobody");
  let (told, taken) = mpsc::channel::<String>();
  let waiter = |name: &'static str| {
    let (seats, told) = (seats.clone(), told.clone());

    std::thread::spawn(move || {
      let mut wait_on = |_: Option<&str>| -> Option<i64> { Some(now_ms() + 10) };
      let held = take_seats(&seats, 0.01, seat_wait(name, Some(&mut wait_on)));

      told.send(name.to_owned()).unwrap();
      held
    })
  };
  let third = waiter("third");

  until("the third ticket", || queue_tickets(&lock).len() == 3);

  let fourth = waiter("fourth");

  until("the fourth ticket", || queue_tickets(&lock).len() == 4);
  assert_eq!(
    queue_tickets(&lock).iter().map(|one| one.seated).collect::<Vec<_>>(),
    [true, true, false, false]
  );

  drop(first);

  assert_eq!(taken.recv_timeout(Duration::from_secs(10)).unwrap(), "third");
  a_while();
  assert!(taken.try_recv().is_err(), "the fourth waits while both seats are held");

  drop(second);

  assert_eq!(taken.recv_timeout(Duration::from_secs(10)).unwrap(), "fourth");
  assert!(third.join().unwrap().is_some() && fourth.join().unwrap().is_some());
}

// A wide lane of `seats` seats, as `seated` names it, for a caller at the front.
fn at_front(seats: usize, free: u64) -> Env {
  let mut env = seated(seats, free);

  env.insert(FRONT_VARIABLE.to_owned(), "1".to_owned());
  env
}

// "it takes the lane's deployment's seat first, which no other caller takes ... Where another deployment holds
// it, it takes the next of the lane's own seats that frees, ahead of everyone"
#[test]
fn a_deployment_runs_at_once_in_its_own_seat_and_a_second_takes_the_next_seat_ahead_of_everyone() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let first = take_turn(&ask(&lock, seated(2, 0)));
  let second = take_turn(&ask(&lock, seated(2, 0)));
  let behind = aside(&lock, seated(2, 0));

  until("the ordinary ticket", || queue_tickets(&lock).len() == 3);

  let deployment = take_turn(&ask(&lock, at_front(2, 0)));

  assert!(deployment.had(), "every seat of the lane held and a ticket waiting, it runs at once");
  assert_eq!(deployment.width(24), 12, "one seat's share");
  assert_eq!(read_queue(&lock).holders.len(), 3, "beside both seats' holders");

  let other = aside(&lock, at_front(2, 0));

  until("the second deployment's ticket", || {
    queue_tickets(&lock).iter().filter(|one| one.front).count() == 2
  });
  a_while();
  assert!(
    !other.is_finished() && !behind.is_finished(),
    "its seat held, the second waits for a lane's seat"
  );

  drop(first);

  let other = other.join().unwrap();

  assert!(other.had());
  a_while();
  assert!(!behind.is_finished(), "the ordinary ticket asked first still waits behind the front");

  drop(second);

  assert!(behind.join().unwrap().had());
  drop((deployment, other));
}

// "a command that finds that number holding any seat of its lane takes the turn as its own": the deployment's
// seat among them, so a deployment's step runs its builds inside the seat it took
#[test]
fn a_command_under_a_deployment_s_seat_runs_inside_it() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let deployment = take_turn(&ask(&lock, at_front(2, 0)));
  let (_, pid) = deployment.held_variable().unwrap();
  let mut under = seated(2, 0);

  under.insert(HELD_VARIABLE.to_owned(), pid);

  assert!(matches!(
    take_turn(&ask(&lock, under)),
    TestbedTurn::Inside {
      seats: Seats { held: 1, of: 2 }
    }
  ));
}

// "the memory gate holds for its seat as for any other"
#[test]
fn short_of_memory_a_deployment_waits_for_a_held_seat_to_free() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let lone = take_turn(&ask(&lock, seated(2, 1_000_000)));
  let deployment = aside(&lock, at_front(2, 1_000_000));

  a_while();
  assert!(!deployment.is_finished());

  drop(lone);

  assert!(deployment.join().unwrap().had());
}

// "a seat beside a held one is taken only while the machine has memory to spare ... or where no seat of the
// lane is held at all"
#[test]
fn short_of_memory_a_seat_is_taken_only_where_no_seat_is_held() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let short = seated(2, 1_000_000);
  let lone = take_turn(&ask(&lock, short.clone()));

  assert!(lone.had(), "a lone command always runs");

  let next = aside(&lock, short);

  a_while();
  assert!(!next.is_finished(), "the free seat waits for memory");

  drop(lone);

  assert!(next.join().unwrap().had());
}

// "a command that finds that number holding any seat of its lane takes the turn as its own, in the holder's
// seats", however full the line is: here a taker waits for the lane's one seat it holds
#[test]
fn a_command_inside_a_held_seat_runs_at_once_on_its_holder_s_share() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let holder = take_turn(&ask(&lock, seated(1, 0)));
  let waiter = aside(&lock, seated(1, 0));

  until("the waiter's ticket", || queue_tickets(&lock).len() == 2);

  let (_, pid) = holder.held_variable().unwrap();
  let mut under = seated(1, 0);

  under.insert(HELD_VARIABLE.to_owned(), pid);

  let inside = take_turn(&ask(&lock, under));

  assert!(matches!(
    inside,
    TestbedTurn::Inside {
      seats: Seats { held: 1, of: 1 }
    }
  ));
  assert_eq!(inside.width(24), 24);

  drop(holder);
  assert!(waiter.join().unwrap().had());
}

const HOLD: &str = "BMO_TESTBED_TURN_TEST_HOLD";

// The stand-in a run of this binary plays where the test that started it names a lock: it holds the lane's
// one seat until it is killed. Every other run passes over it.
#[test]
fn seat_holder_stand_in() {
  let Ok(lock) = std::env::var(HOLD) else { return };

  let held = take_turn(&ask(Path::new(&lock), seated(1, 0)));

  assert!(held.had());
  std::thread::sleep(Duration::from_secs(60));
  drop(held);
}

fn holding_process(lock: &Path) -> Child {
  Command::new(std::env::current_exe().unwrap())
    .args(["--exact", "turn::seat_holder_stand_in", "--nocapture", "--test-threads", "1"])
    .env(HOLD, lock)
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap()
}

// "a holder process that dies lets its seats go with it ... and the next in line takes it"
#[test]
fn a_seat_whose_holder_dies_is_taken_by_the_next() {
  let dir = tempfile::tempdir().unwrap();
  let lock = lock_in(dir.path());
  let mut holding = holding_process(&lock);
  let pid = i64::from(holding.id());

  until("the stand-in's seat", || read_queue(&lock).holder().is_some_and(|one| one.pid == pid));

  let next = aside(&lock, seated(1, 0));

  a_while();
  assert!(!next.is_finished());

  holding.kill().unwrap();
  holding.wait().unwrap();

  assert!(next.join().unwrap().had());
}
