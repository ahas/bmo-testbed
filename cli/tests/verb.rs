//! The binary's `run` and `status` verbs (docs/boundary.md, 「The verbs」), and its words: what only the
//! running binary shows, its output and its exit.

use std::path::Path;

use bmo_testbed_core::placement::LOADED_VARIABLE;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::{LockWait, take_lock};
use bmo_testbed_core::turn::{
  Env, Lane, NOT_RUN_EXIT_CODE, SEAT_FREE_VARIABLE, SEATS_VARIABLE, TurnAsk, lane_width, machine_threads,
  process_env, take_turn,
};

use crate::stand::{Ran, checkout, ran, testbed};

fn run_in(dir: &Path, args: &[&str]) -> Ran {
  ran(&mut testbed(dir, dir, args))
}

// The status with nobody holding either lane or the deployment line, the wide lane's waiting lines after it.
fn lanes(waiting: &[String]) -> Vec<String> {
  let said = &LANES.status;
  let env = process_env();
  let nobody = fill_marks(&said.holding, &[("who", &said.nobody)]);

  Lane::ALL
    .iter()
    .enumerate()
    .flat_map(|(at, lane)| {
      let width = lane_width(*lane, &env, machine_threads()).to_string();

      [fill_marks(&said.lane, &[("lane", lane.words()), ("width", &width)]), nobody.clone()]
        .into_iter()
        .chain(if at == 0 { waiting.to_vec() } else { Vec::new() })
    })
    .chain([said.deploy_line.clone(), nobody.clone()])
    .collect()
}

fn lines(text: &str) -> Vec<String> {
  text.lines().map(str::to_owned).collect()
}

// "runs the command after -- and ends on its result line, with the command's own code"
#[test]
fn the_command_after_the_dashes_runs_and_ends_on_its_result_line_with_its_own_code() {
  let dir = tempfile::tempdir().unwrap();
  let passed = run_in(dir.path(), &["run", "--", "sh", "-c", "echo ' 2 pass'"]);
  let failed = run_in(dir.path(), &["run", "--", "sh", "-c", "echo broke; exit 4"]);
  let run = &LANES.run;
  let opening = fill_marks(run.line.split("{seconds}").next().unwrap(), &[("verdict", &run.ok)]);
  let counts = fill_marks(&run.counts, &[("pass", "2"), ("fail", "0"), ("skip", "0")]);

  assert_eq!(passed.code, 0, "{}", passed.err);
  assert_eq!(passed.out.lines().count(), 1);
  assert!(passed.out.starts_with(&opening) && passed.out.contains(&counts), "{}", passed.out);
  assert_eq!(failed.code, 4);
  assert!(
    failed
      .out
      .contains(&format!("{}\n{}\n", run.failures, fill_marks(&run.failure, &[("line", "broke")]))),
    "{}",
    failed.out
  );
}

// "says who holds the turn, and runs nothing where its harness goes away before the turn comes"
#[test]
fn the_status_says_who_holds_the_turn_and_nothing_runs_where_the_turn_does_not_come() {
  let dir = tempfile::tempdir().unwrap();

  assert_eq!(lines(&run_in(dir.path(), &["status"]).out), lanes(&[]));

  let holder = take_lock(
    &dir.path().join("testbed.lock"),
    5.0,
    LockWait {
      holder: r#"{"place":"place-3","checkout":"/repo","command":"the close","asked":0,"pid":0}"#
        .to_owned(),
      ..LockWait::default()
    },
  )
  .expect("a free lock is had");
  // The run waits for its turn until the harness it runs for has gone, reaped as it ends so it is gone.
  let mut harness = std::process::Command::new("sleep").arg("1").spawn().unwrap();
  let pid = harness.id().to_string();
  let reaped = std::thread::spawn(move || harness.wait().unwrap());
  let queued =
    ran(testbed(dir.path(), dir.path(), &["run", "--", "sh", "-c", "exit 0"]).env("CLAUDE_PID", pid));

  reaped.join().unwrap();
  let not_run = fill_marks(&LANES.run.not_run, &[("turn", &LANES.settled.queued)]);

  assert_eq!(queued.code, i32::from(NOT_RUN_EXIT_CODE));
  assert!(queued.out.starts_with(not_run.split("{seconds}").next().unwrap()), "{}", queued.out);

  let who = ["place-3", "/repo", "the close"].join(&LANES.entry.joiner);
  let waiting = fill_marks(&LANES.status.waiting, &[("at", "1"), ("who", &who)]);

  assert_eq!(lines(&run_in(dir.path(), &["status"]).out), lanes(&[waiting]));

  drop(holder);
}

// "a holding it: line for each seat held, naming its holder", and in JSON `holders`, one for each seat held,
// with `holder` the first of them
#[test]
fn the_status_names_the_holder_of_each_seat_held() {
  let dir = tempfile::tempdir().unwrap();
  let seat = |command: &str| {
    take_turn(&TurnAsk {
      command: command.to_owned(),
      lane: Lane::Wide,
      say: &|_| {},
      path: Some(dir.path().join("testbed.lock")),
      env: Env::from([
        (SEATS_VARIABLE.to_owned(), "2".to_owned()),
        (SEAT_FREE_VARIABLE.to_owned(), "0".to_owned()),
      ]),
      checkout: "/repo".to_owned(),
      poll: None,
      notice_seconds: None,
    })
  };
  let (first, second) = (seat("a check"), seat("a build"));

  assert!(first.had() && second.had());

  let out = run_in(dir.path(), &["status"]).out;
  let holding = |command: &str| {
    let who = ["/repo", command].join(&LANES.entry.joiner);

    fill_marks(&LANES.status.holding, &[("who", &who)])
  };
  let wide: Vec<&str> = out.lines().skip(1).take(2).collect();

  assert!(wide[0].starts_with(&holding("a check")), "{out}");
  assert!(wide[1].starts_with(&holding("a build")), "{out}");

  let json: serde_json::Value =
    serde_json::from_str(&run_in(dir.path(), &["status", "--json"]).out).expect("the status reads as JSON");
  let lane = &json["queues"]["items"][0];
  let commands: Vec<&str> = lane["holders"]
    .as_array()
    .unwrap()
    .iter()
    .map(|one| one["command"].as_str().unwrap())
    .collect();

  assert_eq!(commands, ["a check", "a build"]);
  assert_eq!(lane["holder"], lane["holders"][0]);
}

// "prints nothing and exits 0 where this machine takes a run of its wide lane now: a seat free, nobody waiting for
// one, and the machine not loaded; else it prints one line saying why and exits 1"
#[test]
fn the_status_says_by_its_code_whether_the_machine_takes_a_wide_run_and_why_not() {
  let dir = tempfile::tempdir().unwrap();
  let asked = |seats: &str, share: &str| {
    let mut command = testbed(dir.path(), dir.path(), &["status", "--free"]);

    ran(
      command
        .env(SEATS_VARIABLE, seats)
        .env(SEAT_FREE_VARIABLE, "0")
        .env(LOADED_VARIABLE, share),
    )
  };
  let free = |seats: &str| asked(seats, "1000000");
  let idle = free("2");
  let loaded = asked("2", "0");
  let loaded_head = LANES.placed.loaded.split("{load}").next().unwrap();

  assert_eq!(loaded.code, 1, "{}", loaded.err);
  assert!(loaded.out.starts_with(loaded_head) && loaded.out.lines().count() == 1, "{}", loaded.out);

  assert_eq!((idle.code, idle.out.as_str()), (0, ""), "{}", idle.err);

  let held = take_turn(&TurnAsk {
    command: "a check".to_owned(),
    lane: Lane::Wide,
    say: &|_| {},
    path: Some(dir.path().join("testbed.lock")),
    env: Env::from([
      (SEATS_VARIABLE.to_owned(), "2".to_owned()),
      (SEAT_FREE_VARIABLE.to_owned(), "0".to_owned()),
    ]),
    checkout: "/repo".to_owned(),
    poll: None,
    notice_seconds: None,
  });

  assert!(held.had());
  assert_eq!(free("2").code, 0, "one of two seats is still free");

  let full = free("1");

  assert_eq!(
    (full.code, full.out.trim_end()),
    (1, LANES.placed.seats_taken.as_str()),
    "the one seat is held"
  );
}

// "prints the close line of the repository it is run in, and none outside one"
#[test]
fn the_status_prints_the_close_line_of_the_repository_it_is_run_in() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");

  checkout(&repo, "testbed = 0\n");

  let inside = ran(&mut testbed(&repo, dir.path(), &["status"]));
  let json: serde_json::Value =
    serde_json::from_str(&ran(&mut testbed(&repo, dir.path(), &["status", "--json"])).out)
      .expect("the status reads as JSON");
  let outside: serde_json::Value =
    serde_json::from_str(&run_in(dir.path(), &["status", "--json"]).out).expect("the status reads as JSON");

  assert!(inside.out.contains(&LANES.status.close_line), "{}", inside.out);
  assert_eq!(json["closeLine"], serde_json::json!({ "holder": null, "waiting": [] }));
  assert_eq!(json["queues"]["items"][0]["name"], "wide");
  assert_eq!(json["queues"]["items"][1]["name"], "narrow");
  assert!(json["queues"]["items"][0]["waiting"].is_array());
  assert!(outside["closeLine"].is_null());
}

// "words that do not read name the one thing wrong and end with 2; asking for the usage ends with 0"
#[test]
fn words_that_do_not_read_are_refused_and_the_usage_is_printed_on_asking() {
  let dir = tempfile::tempdir().unwrap();

  for words in [
    &["run"][..],
    &["run", "sh", "-c", "true"],
    &["run", "--fast", "--", "true"],
    &["run", "--guard"],
    &["walk"],
    &[],
  ] {
    let refused = run_in(dir.path(), words);

    assert_eq!(refused.code, 2, "{words:?}: {}", refused.err);
    assert!(refused.out.is_empty() || words.is_empty(), "{words:?}: {}", refused.out);
  }

  let unknown = run_in(dir.path(), &["run", "--adapter", "cobol", "--", "true"]);

  assert_eq!(unknown.code, 2);
  assert!(unknown.err.contains(&fill_marks(
    &LANES.verb.no_language,
    &[("name", "cobol"), ("names", "browser, bun, cargo, node, plain")]
  )));

  for words in [&["--help"][..], &["-h"], &["run", "--help"], &["status", "-h"], &["suite", "--help"]] {
    let usage = run_in(dir.path(), words);

    assert_eq!(usage.code, 0, "{words:?}: {}", usage.err);
    assert!(usage.out.starts_with("usage: bmo-testbed"), "{words:?}: {}", usage.out);
  }
}
