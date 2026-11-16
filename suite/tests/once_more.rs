//! What failed runs once more (docs/suite.md, 「What failed runs once more」): a unit's first account passed
//! over by the readers of the log once it ran once more, and the record of what passed only the second time.

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_suite::log::{FailedTest, failed_files_of, failed_tests_of, failures_of, tally_of};
use bmo_testbed_suite::once_more::{FlakySeen, flaky_of, flaky_record_path, write_flaky};
use bmo_testbed_suite::run::{UnitOnceMore, verdict_line};

const FLAKE: &str = "server/flake.test.ts";
const STEADY: &str = "server/steady.test.ts";

fn heading(mark: &str, file: &str) -> String {
  fill_marks(&SUITE.file.heading, &[("mark", mark), ("seconds", "1.2"), ("file", file)])
}

fn flake() -> FailedTest {
  FailedTest {
    file: FLAKE.to_owned(),
    test: "the reply comes back > within its wait".to_owned(),
  }
}

// A sweep in which one file failed, ran once more at once and passed, and another file passed.
fn flake_passed_once_more() -> String {
  [
    heading(&SUITE.file.ok, STEADY),
    " 3 pass".to_owned(),
    heading(&SUITE.file.redo, FLAKE),
    format!("{FLAKE}:"),
    "(fail) the reply comes back > within its wait [30001.00ms]".to_owned(),
    " 1 pass".to_owned(),
    " 1 fail".to_owned(),
    fill_marks(
      &SUITE.again.once_more,
      &[("mark", &SUITE.lines.mark), ("unit", FLAKE), ("what", &SUITE.again.whole)],
    ),
    heading(&SUITE.file.ok, FLAKE),
    " 2 pass".to_owned(),
    fill_marks(&SUITE.ending.summary, &[("files", "2"), ("passed", "2"), ("failed", "0")]),
    verdict_line(true, 40.0, "2 of 2 files passed"),
  ]
  .join("\n")
}

// "a unit's first account under the redo mark is passed over, so the log reads as the units stand"
#[test]
fn a_unit_s_first_account_under_the_redo_mark_is_passed_over() {
  let log = flake_passed_once_more();
  let tally = tally_of(&log);

  assert_eq!((tally.pass, tally.fail), (5, 0));
  assert!(failed_files_of(&log).is_empty(), "{log}");
  assert!(failed_tests_of(&log).is_empty(), "{log}");
  assert!(failures_of(&log).is_empty(), "{log}");
}

// "a unit that fails once more is read from its second account alone"
#[test]
fn a_unit_that_fails_once_more_is_read_from_its_second_account() {
  let log = [
    heading(&SUITE.file.redo, FLAKE),
    format!("{FLAKE}:"),
    "(fail) the first wait ran out [30001.00ms]".to_owned(),
    fill_marks(
      &SUITE.again.once_more,
      &[("mark", &SUITE.lines.mark), ("unit", FLAKE), ("what", &SUITE.again.whole)],
    ),
    heading(&SUITE.file.failed, FLAKE),
    format!("{FLAKE}:"),
    "(fail) another wait ran out [30001.00ms]".to_owned(),
    fill_marks(&SUITE.ending.failed_files, &[("files", FLAKE)]),
  ]
  .join("\n");

  assert_eq!(failed_files_of(&log), [FLAKE]);
  assert_eq!(failures_of(&log), ["another wait ran out", FLAKE]);
  assert_eq!(
    failed_tests_of(&log),
    [FailedTest {
      file: FLAKE.to_owned(),
      test: "another wait ran out".to_owned(),
    }]
  );
}

// "only a unit whose second run passed is flaky, with the tests its first run failed, and it is written down"
#[test]
fn only_a_unit_whose_second_run_passed_is_flaky_and_it_is_written_down() {
  let flaky = flaky_of(&[
    UnitOnceMore {
      unit: FLAKE.to_owned(),
      failed_tests: vec![flake()],
      passed: true,
    },
    UnitOnceMore {
      unit: STEADY.to_owned(),
      failed_tests: vec![FailedTest {
        file: STEADY.to_owned(),
        test: "fails every time".to_owned(),
      }],
      passed: false,
    },
  ]);

  assert_eq!(flaky.tests, [flake()]);
  assert_eq!(flaky.files, [FLAKE]);
  assert_eq!(flaky.names(), [format!("the reply comes back > within its wait ({FLAKE})")]);
  assert!(flaky_of(&[]).is_empty());

  let data = tempfile::tempdir().unwrap();

  write_flaky(
    data.path(),
    &FlakySeen {
      flaky: &flaky,
      head: "abc123",
      log: "/data/testbed/suite.log",
    },
  )
  .unwrap();

  let text = std::fs::read_to_string(flaky_record_path(data.path())).unwrap();
  let lines: Vec<serde_json::Value> =
    text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();

  assert_eq!(lines.len(), 1);
  assert_eq!(
    lines[0]["tests"],
    serde_json::json!([{ "file": FLAKE, "test": "the reply comes back > within its wait" }])
  );
  assert_eq!(
    (&lines[0]["head"], &lines[0]["files"], &lines[0]["log"]),
    (
      &serde_json::json!("abc123"),
      &serde_json::json!([FLAKE]),
      &serde_json::json!("/data/testbed/suite.log")
    )
  );
}

// "a crate's failed test is named under the unit it ran in, and a unit with none named stands as itself"
#[test]
fn a_crate_s_failed_test_is_named_under_its_unit() {
  let log = [
    heading(&SUITE.file.failed, "crates/bmo-suite/"),
    "test once_more::waits ... FAILED".to_owned(),
    "test once_more::holds ... ok".to_owned(),
    heading(&SUITE.file.failed, "crates/bmo-lang/"),
    "error: could not compile `bmo-lang`".to_owned(),
  ]
  .join("\n");

  assert_eq!(
    failed_tests_of(&log),
    [FailedTest {
      file: "crates/bmo-suite/".to_owned(),
      test: "once_more::waits".to_owned(),
    }]
  );

  let flaky = bmo_testbed_suite::once_more::Flaky {
    tests: failed_tests_of(&log),
    files: vec!["crates/bmo-suite/".to_owned(), "crates/bmo-lang/".to_owned()],
  };

  assert_eq!(flaky.names(), ["once_more::waits (crates/bmo-suite/)", "crates/bmo-lang/"]);
}

// "a single file's bun log gives each failure to the file bun printed it under"
#[test]
fn a_single_file_s_bun_log_gives_each_failure_to_its_file() {
  let log = "server/a.test.ts:\n(fail) one [1ms]\nserver/b.test.ts:\n(fail) two [2ms]";

  assert_eq!(
    failed_tests_of(log),
    [
      FailedTest {
        file: "server/a.test.ts".to_owned(),
        test: "one".to_owned(),
      },
      FailedTest {
        file: "server/b.test.ts".to_owned(),
        test: "two".to_owned(),
      },
    ]
  );
}
