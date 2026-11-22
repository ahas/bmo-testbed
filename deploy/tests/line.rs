//! The deployment line's first marks (docs/deploy.md, 「The deployment line」): a deployment marked first goes
//! ahead of every one waiting, never ahead of the one holding the line, those marked first in the order they
//! were marked; a waiting one is raised to first by its unit; and every mark names who made it and why where
//! the line's tickets are listed.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::{LockWait, now_ms, take_lock};
use bmo_testbed_deploy::line::{
  DeployLine, DeployTicket, FIRST_MARKS, FirstMark, RaiseAsk, RootTurn, Standing, WaitsFor, raise_first,
  served, standing, status_lines,
};

// A line under the folder given, looking and saying where it stands every few milliseconds.
fn line_in(dir: &Path) -> DeployLine {
  DeployLine {
    lock: dir.join("locks/deploy-line.lock"),
    bound: Duration::from_secs(3600),
    say_every: Duration::from_millis(5),
    poll: Some(Duration::from_millis(5)),
  }
}

// The ticket of a deployment named `name`, in the unit of its name, marked first where a mark is given.
fn ticket(name: &str, first: Option<FirstMark>) -> DeployTicket {
  DeployTicket {
    first,
    ..DeployTicket::asked("app", name, Some(&format!("bmo-deploy-{name}")))
  }
}

// The words of each ticket, in the order given.
fn words(tickets: &[DeployTicket]) -> Vec<&str> {
  tickets.iter().map(|ticket| ticket.commit.as_deref().unwrap_or_default()).collect()
}

// A deployment standing in the line on a thread of the scope: each line it says sent on the channel returned,
// and its name sent on `taken` once its turn comes, the line let go at once.
fn waiter<'scope>(
  scope: &'scope std::thread::Scope<'scope, '_>,
  line: &'scope DeployLine,
  ticket: DeployTicket,
  taken: Sender<String>,
) -> Receiver<String> {
  let (told, heard) = channel();

  scope.spawn(move || {
    let say = |said: &str| {
      let _ = told.send(said.to_owned());
    };
    let held = line.take(&ticket, &say);

    let _ = taken.send(ticket.commit.clone().unwrap_or_default());
    drop(held);
  });
  heard
}

// Until the waiter says a line holding the fixed words of the policy line given, or a minute has gone.
fn until_said(heard: &Receiver<String>, line: &str) {
  let words = line.split('{').next().unwrap_or_default();
  let given_up = Instant::now() + Duration::from_secs(60);
  let found =
    std::iter::from_fn(|| heard.recv_timeout(given_up.saturating_duration_since(Instant::now())).ok())
      .any(|said| said.contains(words));

  assert!(found, "the waiter said {words:?}");
}

// The line as it stands once it holds what `holds` asks for, looked at every few milliseconds for a minute at
// most: a waiter moves its own ticket just after it says so.
fn standing_until(lock: &Path, holds: impl Fn(&Standing) -> bool) -> Standing {
  let given_up = Instant::now() + Duration::from_secs(60);

  loop {
    let now = standing(lock);

    if holds(&now) || Instant::now() >= given_up {
      return now;
    }

    std::thread::sleep(Duration::from_millis(5));
  }
}

// The names the deployments' turns came in, as many as asked, waiting a minute at most for each.
fn turns(taken: &Receiver<String>, count: usize) -> Vec<String> {
  (0..count).filter_map(|_| taken.recv_timeout(Duration::from_secs(60)).ok()).collect()
}

// "goes ahead of every deployment waiting in the line, never ahead of the one holding it"; "Deployments
// marked first stand among themselves in the order they were marked"
#[test]
fn deployments_marked_first_go_ahead_of_those_waiting_in_the_order_marked_and_never_ahead_of_the_holder() {
  let dir = tempfile::tempdir().unwrap();
  let line = line_in(dir.path());
  let holder = line.take(&ticket("holder", None), &|_| {}).unwrap();
  let (taken_by, taken) = channel();

  std::thread::scope(|scope| {
    let plain = waiter(scope, &line, ticket("plain", None), taken_by.clone());

    until_said(&plain, &DEPLOY.run.line_waiting);

    let one = waiter(
      scope,
      &line,
      ticket("one", Some(FirstMark::now("ops", "the paint fix"))),
      taken_by.clone(),
    );

    until_said(&one, &DEPLOY.run.line_waiting);

    let two = waiter(
      scope,
      &line,
      ticket("two", Some(FirstMark::now("ops", "the queue fix"))),
      taken_by.clone(),
    );

    until_said(&two, &DEPLOY.run.line_waiting);

    let now = standing(&line.lock);

    assert_eq!(now.holder.as_ref().and_then(|holder| holder.commit.as_deref()), Some("holder"));
    assert_eq!(words(&now.waiting), ["one", "two", "plain"]);
    assert!(taken.try_recv().is_err(), "nobody takes the line while its holder holds it");

    drop(holder);

    assert_eq!(turns(&taken, 3), ["one", "two", "plain"]);
  });
}

// "A deployment already waiting is raised to first ... moves itself ahead at once, marked first in the order
// of the raise"; "its waiting lines name them, and `status` shows them beside its ticket"
#[test]
fn a_raise_moves_a_waiting_deployment_after_those_marked_before_it_and_its_ticket_names_who_and_why() {
  let dir = tempfile::tempdir().unwrap();
  let line = line_in(dir.path());
  let holder = line.take(&ticket("holder", None), &|_| {}).unwrap();
  let (taken_by, taken) = channel();

  std::thread::scope(|scope| {
    let early = waiter(scope, &line, ticket("early", None), taken_by.clone());

    until_said(&early, &DEPLOY.run.line_waiting);

    let late = waiter(scope, &line, ticket("late", None), taken_by.clone());

    until_said(&late, &DEPLOY.run.line_waiting);

    let marked = waiter(
      scope,
      &line,
      ticket("marked", Some(FirstMark::now("ops", "the queue fix"))),
      taken_by.clone(),
    );

    until_said(&marked, &DEPLOY.run.line_waiting);

    let raised = raise_first(&RaiseAsk {
      line: &line.lock,
      unit: "bmo-deploy-late",
      mark: FirstMark::now("chat", "the paint fix is urgent"),
      running: &|_| true,
      recorded: &|_| Ok(false),
    });

    assert!(raised.as_ref().is_ok_and(|said| said.contains("bmo-deploy-late")), "{raised:?}");
    until_said(&late, &DEPLOY.run.first_raised);

    let now = standing_until(&line.lock, |now| {
      now
        .waiting
        .iter()
        .any(|ticket| ticket.first.as_ref().is_some_and(|first| first.by == "chat"))
    });
    let listed = status_lines(&line.lock, now_ms()).join("\n");
    let shown = serde_json::to_value(&now).unwrap();

    assert_eq!(words(&now.waiting), ["marked", "late", "early"], "{shown}");
    assert!(
      listed.contains("app late (bmo-deploy-late), first by chat: the paint fix is urgent"),
      "{listed}"
    );
    assert!(listed.contains("holding it: app holder"), "{listed}");
    assert_eq!(shown["waiting"][1]["first"]["by"], "chat");
    assert_eq!(shown["waiting"][1]["first"]["why"], "the paint fix is urgent");
    assert!(shown["waiting"][1]["askedAt"].is_i64(), "{shown}");
    assert!(!dir.path().join("locks").join(FIRST_MARKS).join("bmo-deploy-late.json").exists());

    drop(holder);

    assert_eq!(turns(&taken, 3), ["marked", "late", "early"]);
  });
}

// "A raise is refused, with nothing left behind, for a unit that is not a running deployment unit, one the
// testbed's record already holds ..., the one holding the line, and one already marked first"
#[test]
fn a_raise_of_the_holder_of_one_marked_first_or_of_one_not_waiting_is_refused_and_leaves_no_mark() {
  let dir = tempfile::tempdir().unwrap();
  let line = line_in(dir.path());
  let holder = line.take(&ticket("holder", None), &|_| {}).unwrap();
  let (taken_by, taken) = channel();
  let said = &DEPLOY.run;

  std::thread::scope(|scope| {
    let marked =
      waiter(scope, &line, ticket("marked", Some(FirstMark::now("ops", "the queue fix"))), taken_by);

    until_said(&marked, &said.line_waiting);

    let running = |unit: &str| unit != "bmo-deploy-gone";
    let recorded = |unit: &str| Ok(unit == "bmo-deploy-past");
    let raise = |unit: &str| {
      raise_first(&RaiseAsk {
        line: &line.lock,
        unit,
        mark: FirstMark::now("chat", "the paint fix"),
        running: &running,
        recorded: &recorded,
      })
      .unwrap_err()
    };

    for (unit, refusal) in [
      ("bmo-deploy-holder", &said.raise_holding),
      ("bmo-deploy-marked", &said.raise_already),
      ("bmo-deploy-gone", &said.raise_not_running),
      ("bmo-deploy-past", &said.raise_past),
      ("../bmo-deploy-gone", &said.raise_unit),
    ] {
      assert_eq!(raise(unit), refusal.replace("{unit}", unit));
    }

    let marks = dir.path().join("locks").join(FIRST_MARKS);

    assert!(
      std::fs::read_dir(&marks).map_or(true, |mut left| left.next().is_none()),
      "no mark is left"
    );

    drop(holder);

    assert_eq!(turns(&taken, 1), ["marked"]);
  });
}

// "A ticket a testbed of an earlier revision stood carries its `words` and `pid` alone": a line held under an
// earlier revision's words is listed with that holder, and a deployment marked first still waits for it.
#[test]
fn a_line_held_under_an_earlier_revisions_words_names_that_holder_and_a_first_one_waits_for_it() {
  let dir = tempfile::tempdir().unwrap();
  let line = line_in(dir.path());
  let earlier = take_lock(
    &line.lock,
    0.0,
    LockWait {
      holder: "app 0123456789ab (bmo-deploy-1)".to_owned(),
      ..LockWait::default()
    },
  )
  .unwrap();
  let (taken_by, taken) = channel();

  std::thread::scope(|scope| {
    let first =
      waiter(scope, &line, ticket("first", Some(FirstMark::now("ops", "the paint fix"))), taken_by);

    until_said(&first, &DEPLOY.run.line_waiting);

    let now = standing(&line.lock);

    assert_eq!(
      now.holder.as_ref().map(|holder| holder.words.as_str()),
      Some("app 0123456789ab (bmo-deploy-1)")
    );
    assert_eq!(now.holder.as_ref().and_then(|holder| holder.repository.as_deref()), None);
    assert_eq!(words(&now.waiting), ["first"]);
    assert!(taken.try_recv().is_err(), "nobody takes the line while the earlier holder holds it");

    drop(earlier);

    assert_eq!(turns(&taken, 1), ["first"]);
  });
}

// A deploy root's turn under the folder given, named `name`, enrolled beside the line and looked at every few
// milliseconds.
fn root_in(dir: &Path, line: &DeployLine, name: &str) -> RootTurn {
  let turn = RootTurn {
    lock: dir.join(name).join("deploy-run.lock"),
    say_every: Duration::from_millis(5),
    poll: Some(Duration::from_millis(5)),
  };

  line.enrol(&turn.lock);
  turn
}

// A deployment waiting for a deploy root's turn on a thread of the scope, until `ahead` of them wait there.
fn root_waiter<'scope>(
  scope: &'scope std::thread::Scope<'scope, '_>,
  turn: &'scope RootTurn,
  name: &str,
  ahead: usize,
) {
  let stood = ticket(name, None);

  scope.spawn(move || drop(turn.take(&stood, &|_| {})));

  let given_up = Instant::now() + Duration::from_secs(60);

  while standing(&turn.lock).waiting.len() <= ahead && Instant::now() < given_up {
    std::thread::sleep(Duration::from_millis(5));
  }
}

// The names of the deployments the line will serve, those waiting for their deploy root's turn marked `root:`.
fn served_names(lock: &Path) -> Vec<String> {
  served(lock)
    .waiting
    .iter()
    .map(|waiting| {
      let name = waiting.ticket.commit.clone().unwrap_or_default();

      match waiting.waits_for {
        WaitsFor::Line => name,
        WaitsFor::Root => format!("root:{name}"),
      }
    })
    .collect()
}

// "Those in the line come first"; "Those waiting for their deploy root's turn come after ... in rounds ... those
// marked first, by `--first` or by a raise not yet read, stand ahead ... the rest ... by that deployment's place in
// the line"; "The deploy roots are found through the line"
#[test]
fn the_line_is_served_after_its_own_by_those_waiting_for_their_deploy_roots_turn_in_rounds_marked_first_ahead()
 {
  let dir = tempfile::tempdir().unwrap();
  let line = line_in(dir.path());
  let (a, b) = (root_in(dir.path(), &line, "a"), root_in(dir.path(), &line, "b"));
  let a_turn = a.take(&ticket("a1", None), &|_| {}).unwrap();
  let b_turn = b.take(&ticket("b1", None), &|_| {}).unwrap();
  let a_line = line.take(&ticket("a1", None), &|_| {}).unwrap();
  let (taken_by, _taken) = channel();

  std::thread::scope(|scope| {
    let b1 = waiter(scope, &line, ticket("b1", None), taken_by);

    until_said(&b1, &DEPLOY.run.line_waiting);
    root_waiter(scope, &a, "a2", 0);
    root_waiter(scope, &a, "a3", 1);
    root_waiter(scope, &b, "b2", 0);

    let before = served_names(&line.lock);
    let raised = raise_first(&RaiseAsk {
      line: &line.lock,
      unit: "bmo-deploy-b2",
      mark: FirstMark::now("ops", "the paint fix"),
      running: &|_| true,
      recorded: &|_| Ok(false),
    });
    let after = served_names(&line.lock);
    let listed = status_lines(&line.lock, now_ms()).join("\n");
    let holder = served(&line.lock).holder.and_then(|holder| holder.commit);

    drop((a_line, a_turn, b_turn));

    assert!(raised.is_ok(), "{raised:?}");
    assert_eq!(holder.as_deref(), Some("a1"));
    assert_eq!(before, ["b1", "root:a2", "root:b2", "root:a3"]);
    assert_eq!(after, ["b1", "root:b2", "root:a2", "root:a3"]);
    assert!(
      listed
        .lines()
        .any(|line| line.contains("app a3 (bmo-deploy-a3)") && line.ends_with(&LANES.status.for_root)),
      "{listed}"
    );
  });
}
