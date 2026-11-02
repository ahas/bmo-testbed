//! The repository's close line (docs/testbed.md, 「The close line」), and the testbed's next place its
//! closes stand in (「Next after the front」). The first test stands in for several harnesses at once:
//! this test binary runs itself again as each close and each job running heavy commands by hand, all on one
//! testbed lock and one close line of the test's own, holding each turn for milliseconds where the machine
//! holds for minutes.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use bmo_testbed_core::close_line::{
  CloseLine, CloseLineAsk, WideStretch, read_close_line, take_close_line,
};
use bmo_testbed_core::policy::close_line::CLOSE_LINE;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::queue::{now_ms, queue_tickets};
use bmo_testbed_core::turn::{Env, FRONT_VARIABLE, Lane, NEXT_VARIABLE, TurnAsk, read_queue, take_turn};
use serde_json::{Value, json};

const ROLE: &str = "BMO_CLOSE_LINE_ROLE";
const SETTING: &str = "BMO_CLOSE_LINE_SETTING";
const POLL: Duration = Duration::from_millis(10);

fn quiet(_: &str) {}

fn sleep_ms(ms: u64) {
  std::thread::sleep(Duration::from_millis(ms));
}

// A bounded wait on a condition.
fn until(condition: impl Fn() -> bool) {
  let began = Instant::now();

  while !condition() && began.elapsed() < Duration::from_secs(10) {
    std::thread::sleep(POLL);
  }
}

// One close: it stands in the line, reads main, holds the wide lane through each part of its check standing
// next, reads main again and moves it one on, and prints what it saw as one line.
fn close(setting: &Value) -> Value {
  let asked = now_ms();
  let turn = take_close_line(&CloseLineAsk {
    path: PathBuf::from(setting["line"].as_str().unwrap()),
    env: Env::new(),
    checkout: String::new(),
    say: &quiet,
    notice_seconds: None,
    poll: Some(POLL),
    watch: false,
  })
  .expect("a close whose harness lives is had its turn");
  let lined = now_ms();
  let main = PathBuf::from(setting["main"].as_str().unwrap());
  let read_main = || std::fs::read_to_string(&main).unwrap().trim().parse::<i64>().unwrap();
  let base = read_main();
  let next = Env::from([
    ("BMO_TESTBED_LOCK".to_owned(), setting["testbed"].as_str().unwrap().to_owned()),
    (NEXT_VARIABLE.to_owned(), "1".to_owned()),
  ]);
  let mut stretch = WideStretch::new(next, String::new(), &quiet);
  let mut steps = Vec::new();

  for step in setting["steps"].as_array().unwrap() {
    let step_asked = now_ms();

    assert!(stretch.ready());

    let began = now_ms();

    sleep_ms(step["holdMs"].as_u64().unwrap());
    stretch.release();
    steps.push(json!({ "command": step["command"], "asked": step_asked, "began": began }));
    sleep_ms(step["gapMs"].as_u64().unwrap());
  }

  let before = read_main();

  std::fs::write(&main, (before + 1).to_string()).unwrap();
  drop(turn);
  json!({ "name": setting["name"], "asked": asked, "lined": lined, "landed": now_ms(), "base": base, "before": before, "steps": steps })
}

// One job running heavy commands by hand: ordinary turns one after another until the stop file stands.
fn hand(setting: &Value) -> Value {
  let stop = PathBuf::from(setting["stop"].as_str().unwrap());
  let env = Env::from([("BMO_TESTBED_LOCK".to_owned(), setting["testbed"].as_str().unwrap().to_owned())]);
  let mut began = Vec::new();

  while !stop.exists() {
    let held = take_turn(&TurnAsk {
      command: format!("{} by hand", setting["name"].as_str().unwrap()),
      lane: Lane::Wide,
      say: &quiet,
      path: None,
      env: env.clone(),
      checkout: String::new(),
      poll: Some(POLL),
      notice_seconds: None,
    });

    if held.held_variable().is_none() {
      continue;
    }

    began.push(now_ms());
    sleep_ms(setting["holdMs"].as_u64().unwrap());
    drop(held);
  }

  json!({ "name": setting["name"], "began": began })
}

// The stand-in a run of this binary plays where the test that started it names a role, its answer on a line
// of its own; every other run passes over it.
#[test]
fn close_line_stand_in() {
  let (Ok(role), Ok(setting)) = (std::env::var(ROLE), std::env::var(SETTING)) else {
    return;
  };
  let setting: Value = serde_json::from_str(&setting).unwrap();
  let said = if role == "close" { close(&setting) } else { hand(&setting) };

  println!("stand-in: {said}");
}

fn start(role: &str, setting: &Value) -> Child {
  Command::new(std::env::current_exe().unwrap())
    .args(["--exact", "close_line::close_line_stand_in", "--nocapture", "--test-threads", "1"])
    .env(ROLE, role)
    .env(SETTING, setting.to_string())
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap()
}

fn answer(child: Child) -> Value {
  let done = child.wait_with_output().unwrap();

  assert!(done.status.success());

  let text = String::from_utf8_lossy(&done.stdout).into_owned();
  // The test runner writes its own word on the same line ahead of the answer.
  let line = text
    .lines()
    .find_map(|line| line.find("stand-in: ").map(|at| &line[at + "stand-in: ".len()..]))
    .expect("the stand-in answered");

  serde_json::from_str(line).unwrap()
}

// "takes five closes at once in the order they asked, each on the main the one before it landed, each
// step behind one command at most": five harnesses closing at once beside two harnesses running heavy commands by
// hand, where no close may lose its place in the line to the runs by hand.
#[test]
fn takes_five_closes_at_once_in_the_order_they_asked() {
  let dir = tempfile::tempdir().unwrap();
  let at = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
  let shared = json!({ "line": at("close.lock"), "testbed": at("testbed.lock"), "main": at("main"), "stop": at("stop") });
  let with = |more: Value| {
    let mut setting = shared.clone();

    setting.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
    setting
  };
  // The two parts of a close's check, with the lane let go between them.
  let steps = json!([{ "command": "the static check", "holdMs": 40, "gapMs": 20 }, { "command": "the selected suite", "holdMs": 80, "gapMs": 0 }]);
  let names = ["w2", "w4", "w5", "w6", "w7"];

  std::fs::write(at("main"), "0").unwrap();

  let hands: Vec<Child> = ["w5", "w7"]
    .map(|name| start("hand", &with(json!({ "name": name, "holdMs": 60 }))))
    .into_iter()
    .collect();
  let line = PathBuf::from(at("close.lock"));
  let mut closes: Vec<Child> = Vec::new();

  // Each close stands in the line before the next is started, so the order they asked is the order here.
  for name in names {
    closes.push(start("close", &with(json!({ "name": name, "steps": steps }))));

    let began = Instant::now();

    while standing(&line, &mut closes) < closes.len() && began.elapsed() < Duration::from_secs(5) {
      std::thread::sleep(POLL);
    }
  }

  let closed: Vec<Value> = closes.into_iter().map(answer).collect();

  std::fs::write(at("stop"), "").unwrap();

  let by_hand: Vec<i64> = hands
    .into_iter()
    .flat_map(|child| {
      answer(child)["began"]
        .as_array()
        .unwrap()
        .iter()
        .map(|at| at.as_i64().unwrap())
        .collect::<Vec<_>>()
    })
    .collect();
  let mut landed: Vec<(i64, &str)> = closed
    .iter()
    .map(|one| (one["landed"].as_i64().unwrap(), one["name"].as_str().unwrap()))
    .collect();

  landed.sort();

  assert_eq!(landed.iter().map(|(_, name)| *name).collect::<Vec<_>>(), names);

  // Main never moved under a close, so none checked again: each landed on what the one before it landed.
  for (at, one) in closed.iter().enumerate() {
    assert_eq!((one["base"].as_i64(), one["before"].as_i64()), (Some(at as i64), Some(at as i64)));
  }

  for (at, one) in closed.iter().enumerate().skip(1) {
    assert!(one["lined"].as_i64().unwrap() >= closed[at - 1]["landed"].as_i64().unwrap());
  }

  // The hand runs went on beside, and each step of a close waited for at most the one command holding the lane.
  assert!(!by_hand.is_empty());

  for step in closed.iter().flat_map(|one| one["steps"].as_array().unwrap().clone()) {
    let (asked, began) = (step["asked"].as_i64().unwrap(), step["began"].as_i64().unwrap());

    assert!(by_hand.iter().filter(|at| **at > asked && **at < began).count() <= 1);
  }
}

// How many of the closes stand in the line or have already ended.
fn standing(line: &Path, closes: &mut [Child]) -> usize {
  queue_tickets(line).len() + closes.iter_mut().filter_map(|close| close.try_wait().ok().flatten()).count()
}

// "holds the wide lane through a part of the check, and lets a deployment waiting at the front in before
// the next"
#[test]
fn holds_the_wide_lane_and_lets_a_deployment_at_the_front_in_before_the_next() {
  let dir = tempfile::tempdir().unwrap();
  let lock = dir.path().join("testbed.lock");
  let env = Env::from([
    ("BMO_TESTBED_LOCK".to_owned(), lock.to_string_lossy().into_owned()),
    (NEXT_VARIABLE.to_owned(), "1".to_owned()),
  ]);
  let said = std::sync::Mutex::new(Vec::<String>::new());
  let keep = |line: &str| said.lock().unwrap().push(line.to_owned());
  let mut stretch = WideStretch::new(env, String::new(), &keep).on(lock.clone(), POLL);

  assert!(stretch.ready());

  let holder = read_queue(&lock).holder.unwrap();

  assert_eq!((holder.command.as_str(), holder.next), (CLOSE_LINE.stretch.command.as_str(), true));

  let order = std::sync::Mutex::new(Vec::<&str>::new());
  let front = Env::from([
    ("BMO_TESTBED_LOCK".to_owned(), lock.to_string_lossy().into_owned()),
    (FRONT_VARIABLE.to_owned(), "1".to_owned()),
  ]);

  std::thread::scope(|scope| {
    let deployment = scope.spawn(|| {
      let turn = take_turn(&TurnAsk {
        command: "the deployment's check".to_owned(),
        lane: Lane::Wide,
        say: &quiet,
        path: Some(lock.clone()),
        env: front.clone(),
        checkout: String::new(),
        poll: Some(POLL),
        notice_seconds: None,
      });

      order.lock().unwrap().push("deployment");
      sleep_ms(30);
      drop(turn);
    });

    until(|| queue_tickets(&lock).iter().any(|one| one.front));
    assert!(stretch.ready());
    order.lock().unwrap().push("close");
    deployment.join().unwrap();
  });

  assert_eq!(*order.lock().unwrap(), ["deployment", "close"]);
  assert!(said.lock().unwrap().contains(&CLOSE_LINE.stretch.front_first));

  stretch.release();
  assert_eq!(read_queue(&lock).holder, None);
  assert!(queue_tickets(&lock).is_empty());
}

// "leaves the line once its harness is gone, saying where it stood while it waited"
#[test]
fn leaves_the_line_once_its_harness_is_gone() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("close.lock");
  let holder = take_close_line(&CloseLineAsk {
    path: path.clone(),
    env: Env::from([("BMO_TESTBED_SCOPE".to_owned(), "place-4".to_owned())]),
    checkout: "/checkout".to_owned(),
    say: &quiet,
    notice_seconds: None,
    poll: Some(POLL),
    watch: false,
  })
  .unwrap();
  let mut job = Command::new("sleep").arg("30").stdin(Stdio::null()).spawn().unwrap();
  let harness_pid = job.id().to_string();
  let lines = std::sync::Mutex::new(Vec::<String>::new());
  let keep = |line: &str| lines.lock().unwrap().push(line.to_owned());
  let held_by = fill_marks(&CLOSE_LINE.line.held_by, &[("who", "place-4 · /checkout")]);

  std::thread::scope(|scope| {
    let waiting = scope.spawn(|| {
      take_close_line(&CloseLineAsk {
        path: path.clone(),
        env: Env::from([("CLAUDE_PID".to_owned(), harness_pid.clone())]),
        checkout: String::new(),
        say: &keep,
        notice_seconds: Some(0.05),
        poll: Some(POLL),
        watch: false,
      })
      .is_none()
    });

    until(|| lines.lock().unwrap().len() >= 2);
    assert!(lines.lock().unwrap()[0].ends_with(&held_by));

    let _ = job.kill();
    let _ = job.wait();

    assert!(waiting.join().unwrap());
  });

  assert_eq!(queue_tickets(&path).len(), 1);
  drop(holder);
  assert_eq!(read_close_line(&path), CloseLine::default());
}
