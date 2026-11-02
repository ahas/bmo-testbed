//! What failed run once more before the suite is judged (docs/suite.md, 「What failed runs once more」):
//! the second run of what failed or never started, and the record of what passed only the second time.

use std::cell::RefCell;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_suite::log::{FailedTest, failed_tests_of};
use bmo_testbed_suite::once_more::{FlakySeen, SuiteRun, flaky_record_path, run_once_more, write_flaky};
use bmo_testbed_suite::run::verdict_line;

const FLAKE: &str = "server/flake.test.ts";
const STEADY: &str = "server/steady.test.ts";

fn heading(mark: &str, file: &str) -> String {
  fill_marks(&SUITE.file.heading, &[("mark", mark), ("seconds", "1.2"), ("file", file)])
}

// A sweep in which one test of one file failed and another file passed.
fn one_failed() -> String {
  [
    heading(&SUITE.file.ok, STEADY),
    "(pass) holds [0.4ms]".to_owned(),
    heading(&SUITE.file.failed, FLAKE),
    format!("{FLAKE}:"),
    "(fail) the reply comes back > within its wait [30001.00ms]".to_owned(),
    fill_marks(&SUITE.ending.failed_files, &[("files", FLAKE)]),
    verdict_line(false, 40.0, "1 of 2 files passed"),
  ]
  .join("\n")
}

fn passed(note: &str) -> String {
  verdict_line(true, 3.0, note)
}

fn flake() -> FailedTest {
  FailedTest {
    file: FLAKE.to_owned(),
    test: "the reply comes back > within its wait".to_owned(),
  }
}

// "a test that fails and then passes when run once more passes, written down by its name"
#[test]
fn a_test_that_fails_and_then_passes_when_run_once_more_passes_written_down_by_its_name() {
  let asked = RefCell::new(Vec::new());
  let runs = run_once_more(SuiteRun::of_log(1, &one_failed()), |named: &[String]| {
    asked.borrow_mut().push(named.to_vec());
    SuiteRun::of_log(0, &passed("1 of 1 files passed"))
  });
  let flaky = runs.flaky();

  assert_eq!(asked.into_inner(), [vec![FLAKE.to_owned()]]);
  assert!(runs.passed());
  assert_eq!(flaky.tests, [flake()]);
  assert_eq!(flaky.files, [FLAKE]);
  assert_eq!(flaky.names(), [format!("the reply comes back > within its wait ({FLAKE})")]);

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

// "a test that fails twice is refused, naming what failed in either run, and nothing is flaky"
#[test]
fn a_test_that_fails_twice_is_refused_and_nothing_is_flaky() {
  let again = [
    heading(&SUITE.file.failed, FLAKE),
    "(fail) another wait ran out [30001.00ms]".to_owned(),
    fill_marks(&SUITE.ending.failed_files, &[("files", FLAKE)]),
    verdict_line(false, 31.0, "0 of 1 files passed"),
  ]
  .join("\n");
  let runs = run_once_more(SuiteRun::of_log(1, &one_failed()), |_: &[String]| SuiteRun::of_log(1, &again));

  assert!(!runs.passed());
  assert!(runs.flaky().is_empty());
  assert_eq!(runs.failed_files(), [FLAKE]);
  assert_eq!(
    runs.failures(),
    ["the reply comes back > within its wait", FLAKE, "another wait ran out"]
  );
}

// "a run that passed, or one whose turn never came and started nothing, runs nothing once more"
#[test]
fn a_run_that_passed_or_started_nothing_runs_nothing_once_more() {
  let ran = RefCell::new(0);
  let not_run = verdict_line(false, 0.0, &SUITE.ending.not_run);

  for first in [SuiteRun::of_log(0, &passed("2 of 2 files passed")), SuiteRun::of_log(75, &not_run)] {
    let runs = run_once_more(first.clone(), |_: &[String]| {
      *ran.borrow_mut() += 1;
      SuiteRun::default()
    });

    assert_eq!(runs.again, None);
    assert_eq!(runs.passed(), first.passed());
    assert!(runs.flaky().is_empty());
  }

  assert_eq!(ran.into_inner(), 0);
}

// "what never started runs once more beside what failed, and only what failed is flaky"
#[test]
fn what_never_started_runs_once_more_beside_what_failed_and_only_what_failed_is_flaky() {
  // A log a run with a whole-run guard wrote, naming a unit it never started.
  let first = format!("{}\nnever started within 240s: {STEADY}", one_failed());
  let asked = RefCell::new(Vec::new());
  let runs = run_once_more(SuiteRun::of_log(1, &first), |named: &[String]| {
    asked.borrow_mut().push(named.to_vec());
    SuiteRun::of_log(0, &passed("2 of 2 files passed"))
  });

  assert_eq!(asked.into_inner(), [vec![FLAKE.to_owned(), STEADY.to_owned()]]);
  assert_eq!(runs.flaky().files, [FLAKE]);
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
