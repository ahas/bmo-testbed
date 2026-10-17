//! The `log` verb (docs/boundary.md, 「The verbs」): what a run's log came to, read off the log it left. What
//! the verb has to do beyond counting is end non-zero on a failure, the ones a whole-suite log names only on
//! its closing lines included, and say so of a log with no summary rather than count it a pass.

use std::path::{Path, PathBuf};

use bmo_testbed_cli::suite::log;
use bmo_testbed_cli::words::LogArgs;
use bmo_testbed_core::policy::answer::ANSWER;
use serde_json::Value;

fn written(dir: &Path, name: &str, lines: &[&str]) -> PathBuf {
  std::fs::write(dir.join(name), format!("{}\n", lines.join("\n"))).unwrap();
  PathBuf::from(name)
}

fn read(dir: &Path, path: PathBuf, json: bool) -> (String, u8) {
  let said = log(&LogArgs { log: path, json }, dir);
  let mut all = said.out;

  all.extend(said.err);
  (all.join("\n"), said.code)
}

#[test]
fn a_run_that_passed_is_counted_and_ends_at_nought() {
  let dir = tempfile::tempdir().unwrap();
  let path = written(dir.path(), "ok.log", &[" 20 pass", " 0 fail", "Ran 20 tests across 2 files. [1.0s]"]);

  assert_eq!(read(dir.path(), path, false), ("pass 20 · fail 0 · files 2".to_owned(), 0));
}

#[test]
fn a_run_that_failed_names_what_failed_and_ends_non_zero() {
  let dir = tempfile::tempdir().unwrap();
  let path = written(
    dir.path(),
    "bad.log",
    &[
      "(fail) some test > a case [0.16ms]",
      " 19 pass",
      " 1 fail",
      "Ran 20 tests across 1 file. [1.0s]",
    ],
  );
  let (out, code) = read(dir.path(), path, false);

  assert_eq!(code, 1);
  assert!(out.contains("fail 1"));
  assert!(out.contains("1 failure(s) the log names:\n  some test > a case"), "{out}");
}

#[test]
fn the_whole_suite_runners_verdict_stands_ahead_of_the_counts() {
  let dir = tempfile::tempdir().unwrap();
  let verdict = "suite: ok in 66s \u{2014} 1 of 1 files passed";
  let path = written(dir.path(), "verdict.log", &[" 5 pass", " 0 fail", verdict]);
  let (out, code) = read(dir.path(), path, false);

  assert_eq!(code, 0);
  assert_eq!(out.lines().next(), Some(verdict));
}

// A file cut by its guard or never started wrote no failing block, so the counts alone read as a pass; the
// sweep's closing lines are what name it. A file never started was not tested, so it is named apart.
#[test]
fn a_file_the_sweep_cut_is_a_failure_and_one_never_started_is_named_apart() {
  let dir = tempfile::tempdir().unwrap();
  let path = written(
    dir.path(),
    "cut-file.log",
    &[
      " 100 pass",
      " 0 fail",
      "Ran 100 tests across 1 file. [1.0s]",
      "did not end within 90s: server/c.test.ts",
      "never started within 240s: server/d.test.ts",
      "suite: FAILED in 331s \u{2014} 1 of 3 files passed, 1 did not end within 90s, 1 never started within 240s",
    ],
  );
  let (out, code) = read(dir.path(), path, false);

  assert_eq!(code, 1);
  assert!(out.contains("1 failure(s) the log names:\n  server/c.test.ts\n"), "{out}");
  assert!(
    out.contains("1 unit(s) the run never started, so nothing of them was tested:\n  server/d.test.ts")
  );
}

// "--json gives the counts, the verdict, the units that failed and never started, and each failure with its
// unit"
#[test]
fn json_gives_the_counts_the_units_and_each_failure_with_its_unit() {
  let dir = tempfile::tempdir().unwrap();
  let path = written(
    dir.path(),
    "json.log",
    &[
      "fail 1.2s  server/a.test.ts",
      "(fail) one [0.1ms]",
      " 1 pass",
      " 1 fail",
      "failed: server/a.test.ts",
      "never started within 240s: server/b.test.ts",
    ],
  );
  let (out, code) = read(dir.path(), path, true);
  let answer: Value = serde_json::from_str(&out).unwrap();

  assert_eq!(code, 1);
  assert_eq!((answer["passed"].clone(), answer["failed"].clone()), (1.into(), 1.into()));
  assert_eq!(answer["skipped"], 0);
  assert_eq!(answer["unstarted_units"], serde_json::json!(["server/b.test.ts"]));
  assert_eq!(answer["failures"], serde_json::json!([{ "test": "one", "unit": "server/a.test.ts" }]));
  assert_eq!(answer["failed_units"], serde_json::json!(["server/a.test.ts"]));
  assert!(answer["verdict"].is_null());
}

// "a failing test the log names under no unit is a failure all the same, its unit null"
#[test]
fn json_keeps_a_failure_the_log_names_under_no_unit() {
  let dir = tempfile::tempdir().unwrap();
  let path = written(dir.path(), "unplaced.log", &["(fail) one [0.1ms]", " 3 pass", " 1 fail"]);
  let (out, code) = read(dir.path(), path, true);
  let answer: Value = serde_json::from_str(&out).unwrap();

  assert_eq!(code, 1);
  assert_eq!(answer["failures"], serde_json::json!([{ "test": "one", "unit": null }]));
}

// A log with no summary at all is a run that never got there, and a silent nought would read exactly like a
// run of no tests that passed.
#[test]
fn a_log_with_no_summary_is_named_as_one_rather_than_counted_as_a_pass() {
  let dir = tempfile::tempdir().unwrap();
  let path = written(dir.path(), "cut.log", &["bun test v1.3.14", "(pass) one [0.1ms]"]);
  let (out, code) = read(dir.path(), path, false);

  assert_eq!(code, 1);
  assert!(out.contains("has no summary"));
}

#[test]
fn a_log_that_is_not_there_ends_non_zero() {
  let dir = tempfile::tempdir().unwrap();

  assert_eq!(
    read(dir.path(), PathBuf::from("missing.log"), false),
    ("missing.log does not exist".to_owned(), 1)
  );
}

#[test]
fn a_list_of_names_past_the_ceiling_is_cut_to_whole_names_and_says_how_many_are_left_out() {
  let dir = tempfile::tempdir().unwrap();
  let names: Vec<String> =
    (0..2000).map(|at| format!("(fail) a failing test with a long name {at:04}")).collect();
  let mut lines: Vec<&str> = vec!["fail 9.0s  server/many.test.ts"];

  lines.extend(names.iter().map(String::as_str));
  lines.extend([" 0 pass", " 2000 fail"]);

  let path = written(dir.path(), "many.log", &lines);
  let (out, code) = read(dir.path(), path.clone(), false);

  assert_eq!(code, 1);
  assert!(out.chars().count() <= ANSWER.ceiling_chars + 200);
  assert!(out.contains("-- cut at the 30,000-character ceiling:"), "{}", &out[out.len() - 300..]);
  assert!(out.contains("read the log itself for the rest."));
  assert!(
    out
      .lines()
      .filter(|line| line.starts_with("  a failing test"))
      .all(|line| line.ends_with(char::is_numeric))
  );

  let said = log(
    &LogArgs {
      log: path,
      json: true,
    },
    dir.path(),
  );

  assert!(said.out[0].chars().count() <= ANSWER.ceiling_chars);
  assert!(serde_json::from_str::<Value>(&said.out[0]).is_ok());
  assert!(said.err[0].starts_with("-- cut at the 30,000-character ceiling:"), "{:?}", said.err);
}
