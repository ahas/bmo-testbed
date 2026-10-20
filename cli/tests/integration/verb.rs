//! The binary's `run` and `status` verbs (docs/boundary.md, 「The verbs」), and its words: what only the
//! running binary shows, its output and its exit.

use std::path::Path;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::{LockWait, take_lock};
use bmo_testbed_core::turn::{Lane, NOT_RUN_EXIT_CODE, lane_width, machine_threads, process_env};

use crate::stand::{Ran, checkout, ran, testbed};

fn run_in(dir: &Path, args: &[&str]) -> Ran {
  ran(&mut testbed(dir, dir, args))
}

// The lanes as the status prints them with nobody holding either, the wide one's waiting lines after it.
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

// "says who holds the turn, and runs nothing where the turn does not come"
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
  let queued = run_in(dir.path(), &["run", "--wait", "1", "--", "sh", "-c", "exit 0"]);
  let not_run = fill_marks(&LANES.run.not_run, &[("turn", &LANES.settled.queued)]);

  assert_eq!(queued.code, i32::from(NOT_RUN_EXIT_CODE));
  assert!(queued.out.starts_with(not_run.split("{seconds}").next().unwrap()), "{}", queued.out);

  let who = ["place-3", "/repo", "the close"].join(&LANES.entry.joiner);
  let waiting = fill_marks(&LANES.status.waiting, &[("at", "1"), ("who", &who)]);

  assert_eq!(lines(&run_in(dir.path(), &["status"]).out), lanes(&[waiting]));

  drop(holder);
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
    &["run", "--wait", "0", "--", "true"],
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
