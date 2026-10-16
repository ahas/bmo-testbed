//! A run's log read back by the binary's own readers (docs/suite.md, 「Reading the output」): the verdict,
//! what failed and what never started, read out of the log the suite wrote.

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_suite::log::{
  Tally, failed_files_of, failures_of, tally_of, unstarted_of, verdict_of, warnings_of,
};
use bmo_testbed_suite::run::verdict_line;

// A sweep's closing lines as the run writes them, over a failing test under its file's header.
fn closing() -> String {
  let said = &SUITE.ending;

  [
    "server/one.test.ts:".to_owned(),
    "(fail) one thing went wrong [2.19ms]".to_owned(),
    fill_marks(
      &SUITE.file.heading,
      &[("mark", &SUITE.file.cut), ("seconds", "90.0"), ("file", "client/browser/b.test.ts")],
    ),
    fill_marks(&said.cut_files, &[("guard", "90"), ("files", "client/browser/b.test.ts")]),
    fill_marks(&said.failed_files, &[("files", "server/one.test.ts, server/two.test.ts")]),
    fill_marks(&said.unstarted_files, &[("guard", "240"), ("files", "server/three.test.ts")]),
    verdict_line(false, 334.0, "1 of 4 files passed"),
  ]
  .join("\n")
}

#[test]
fn the_close_reads_the_verdict_the_run_wrote_last_and_none_where_it_wrote_none() {
  assert_eq!(verdict_of(&closing()), Some(verdict_line(false, 334.0, "1 of 4 files passed")));
  assert_eq!(verdict_of("bun test v1.4.2\n(pass) one [0.1ms]"), None);
}

#[test]
fn the_close_reads_what_failed_each_once_and_never_a_file_that_did_not_start() {
  assert_eq!(
    failures_of(&closing()),
    [
      "one thing went wrong",
      "client/browser/b.test.ts",
      "server/one.test.ts",
      "server/two.test.ts"
    ]
  );
  assert_eq!(
    failed_files_of(&closing()),
    ["server/one.test.ts", "client/browser/b.test.ts", "server/two.test.ts"]
  );
  assert_eq!(unstarted_of(&closing()), ["server/three.test.ts"]);
}

// The totals of a run are the sum of every summary block, carried from repo/repo/suite-run.test.ts.
#[test]
fn the_totals_are_the_sum_of_every_block_bun_and_cargo_alike_with_commas_read_and_none_without_a_summary() {
  let blocks = [
    "server/a.test.ts:",
    "(pass) one [0.1ms]",
    "",
    " 12 pass",
    " 0 fail",
    "Ran 12 tests across 1 file. [1.0s]",
    "server/b.test.ts:",
    "",
    " 8 pass",
    " 2 fail",
    "Ran 10 tests across 1 file. [1.0s]",
  ]
  .join("\n");

  assert_eq!(
    tally_of(&blocks),
    Tally {
      pass: 20,
      fail: 2,
      skip: 0,
      todo: 0,
      files: 2
    }
  );
  assert_eq!(tally_of(" 1,557 pass\n 0 fail\nRan 1,557 tests across 1 file.").pass, 1557);

  let cargo = [
    " 4 pass",
    " 0 fail",
    "test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s",
    "test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s",
  ]
  .join("\n");

  assert_eq!(
    tally_of(&cargo),
    Tally {
      pass: 9,
      fail: 1,
      skip: 1,
      todo: 0,
      files: 0
    }
  );
  assert_eq!(
    tally_of(" 3 pass\n 1 skip\n 2 todo\n 0 fail"),
    Tally {
      pass: 3,
      fail: 0,
      skip: 1,
      todo: 2,
      files: 0
    }
  );
  // A word about passing in the middle of a line is not a count.
  assert_eq!(tally_of("bun test v1.3.14\n(pass) one [0.1ms]\nerror: it broke"), Tally::default());
}
#[test]
fn the_deployment_reads_each_tools_warnings_by_their_shape_and_nothing_else_that_names_one() {
  let log = [
    " \u{1b}[33mWARN\u{1b}[39m  \u{1b}[33m[PLUGIN_TIMINGS]\u{1b}[0m Plugin hooks ran for 6.7s of this 7.2s build (93%).",
    " WARN  [plugin builtin:vite-reporter] ",
    "",
    "(!) Some chunks are larger than 500 kB after minification.",
    "...pkg/case.py:669: ResourceWarning: unclosed <socket.socket fd=3>",
    "(node:4242) DeprecationWarning: an old call",
    "warning: unused variable: `spare`",
    "warning: `sample` (lib test) generated 1 warning",
    "(pass) a warning that could not be sent is said again [0.1ms]",
    "test keepalive::the_warning_names_the_account ... ok",
  ]
  .join("\n");

  assert_eq!(
    warnings_of(&log),
    [
      "WARN [PLUGIN_TIMINGS] Plugin hooks ran for 6.7s of this 7.2s build (93%).",
      "WARN [plugin builtin:vite-reporter] (!) Some chunks are larger than 500 kB after minification.",
      "pkg/case.py:669: ResourceWarning: unclosed <socket.socket fd=3>",
      "DeprecationWarning: an old call",
      "warning: unused variable: `spare`",
    ]
  );
}
