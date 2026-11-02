//! One command run in the machine's turn and read into the testbed's one result (docs/testbed.md, 「The
//! result」).

use std::path::{Path, PathBuf};

use bmo_testbed_core::languages::{LANGUAGES, Language, PLAIN, Tally};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::{LockWait, take_lock};
use bmo_testbed_core::run::{RunAsk, TestbedResult, run_in_testbed};
use bmo_testbed_core::turn::{Env, Lane, NOT_RUN_EXIT_CODE, TurnAsk};
use bmo_testbed_core::witness::Unwitnessed;

fn quiet(_: &str) {}

fn ask<'a>(dir: &Path, command: &[&str], languages: &'a [&'static dyn Language]) -> RunAsk<'a> {
  let command: Vec<String> = command.iter().map(|word| (*word).to_owned()).collect();

  RunAsk {
    language: PLAIN,
    languages,
    log_dir: dir.join("logs"),
    cwd: dir.to_path_buf(),
    turn: TurnAsk {
      command: command.join(" "),
      lane: Lane::Wide,
      say: &quiet,
      path: Some(dir.join("testbed.lock")),
      env: Env::new(),
      checkout: dir.to_string_lossy().into_owned(),
      poll: None,
      notice_seconds: None,
    },
    command,
    witness: &Unwitnessed,
  }
}

// A harness's process that has ended.
fn gone_harness() -> String {
  let mut ended = std::process::Command::new("true").spawn().unwrap();
  let pid = ended.id();

  ended.wait().unwrap();
  pid.to_string()
}

fn run(ask: &RunAsk<'_>) -> TestbedResult {
  run_in_testbed(ask).expect("the command runs")
}

// The result line as the policy words it, the run's own seconds and log filled in.
fn line(result: &TestbedResult, verdict: &str, counts: &str) -> String {
  let log: PathBuf = result.log.clone().unwrap_or_default();

  fill_marks(
    &LANES.run.line,
    &[
      ("verdict", verdict),
      ("seconds", &result.seconds.round().to_string()),
      ("waited", &result.waited_seconds.round().to_string()),
      ("language", result.language),
      ("counts", counts),
      ("log", &log.to_string_lossy()),
    ],
  )
}

// "keeps the whole output in its log and ends on one line with the counts every language read"
#[test]
fn a_run_keeps_its_whole_output_in_its_log_and_ends_on_one_line_with_the_counts_every_language_read() {
  let dir = tempfile::tempdir().unwrap();
  let result = run(&ask(
    dir.path(),
    &["sh", "-c", "echo ' 3 pass'; echo ' 1 fail'; echo '(fail) adds'; exit 1"],
    &LANGUAGES,
  ));

  assert!(result.ran && !result.ok);
  assert_eq!((result.language, result.code), (PLAIN.name(), Some(1)));
  assert_eq!(
    result.tally,
    Some(Tally {
      pass: 3,
      fail: 1,
      skip: 0
    })
  );
  assert_eq!(result.failures, ["adds"]);
  assert!(std::fs::read_to_string(result.log.as_ref().unwrap()).unwrap().contains("(fail) adds"));

  let counts = fill_marks(&LANES.run.counts, &[("pass", "3"), ("fail", "1"), ("skip", "0")]);

  assert_eq!(result.line(), line(&result, &LANES.run.failed, &counts));
  assert_eq!(result.exit_code(), 1);
}

// "says a build's exit code where no language found counts, and the last lines where nothing named a failure"
#[test]
fn a_build_says_its_exit_code_and_the_last_lines_where_nothing_named_a_failure() {
  let dir = tempfile::tempdir().unwrap();
  let passed = run(&ask(dir.path(), &["sh", "-c", "echo built"], &LANGUAGES));
  let failed = run(&ask(dir.path(), &["sh", "-c", "echo first; echo broke; exit 3"], &LANGUAGES));

  assert_eq!(
    passed.line(),
    line(&passed, &LANES.run.ok, &fill_marks(&LANES.run.exit, &[("code", "0")]))
  );
  assert_eq!(passed.exit_code(), 0);
  assert_eq!(failed.failures, ["first", "broke"]);
  assert_eq!(failed.exit_code(), 3);
}

// "runs a command until it ends, however long that takes"
#[test]
fn a_command_runs_until_it_ends() {
  let dir = tempfile::tempdir().unwrap();
  let result = run(&ask(dir.path(), &["sh", "-c", "sleep 1 & wait"], &[]));

  assert!(result.ran && result.ok);
  assert!(result.seconds >= 1.0);
}

// "runs nothing where the harness it waits for goes away before the turn comes, and says so"
#[test]
fn nothing_runs_where_the_turn_does_not_come_and_it_says_so() {
  let dir = tempfile::tempdir().unwrap();
  let holder =
    take_lock(&dir.path().join("testbed.lock"), 5.0, LockWait::default()).expect("a free lock is had");
  let mut asked = ask(dir.path(), &["sh", "-c", "exit 0"], &[]);

  asked.turn.env = Env::from([("CLAUDE_PID".to_owned(), gone_harness())]);
  asked.turn.notice_seconds = Some(0.05);

  let result = run(&asked);

  assert!(!result.ran);
  assert!(!dir.path().join("logs").exists());
  assert_eq!(result.line(), fill_marks(&LANES.run.not_run, &[("turn", &result.turn)]));
  let queued = fill_marks(&LANES.settled.queued, &[("seconds", "0")]);

  assert!(result.turn.starts_with(queued.split("{ahead}").next().unwrap()));
  assert_eq!(result.exit_code(), NOT_RUN_EXIT_CODE);

  drop(holder);
}
