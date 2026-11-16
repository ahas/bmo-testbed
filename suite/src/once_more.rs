//! The suite with a failed unit run once more as soon as it ends (docs/suite.md, 「What failed runs once
//! more」): the one step a close and a deployment both take, and the record of what passed only the second
//! time, so a test failing now and then is mended rather than forgotten.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use serde_json::json;

use crate::log::FailedTest;
use crate::run::{SuiteAsk, SuiteEnded, UnitOnceMore, run};
use crate::workspace::git_lines;

/// The tests and the files that failed and then passed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Flaky {
  pub tests: Vec<FailedTest>,
  pub files: Vec<String>,
}

impl Flaky {
  pub fn is_empty(&self) -> bool {
    self.tests.is_empty() && self.files.is_empty()
  }

  /// Each by name: every test with where it stands, then each file or unit that failed with no test of it
  /// named, such as a crate that did not build.
  pub fn names(&self) -> Vec<String> {
    self
      .tests
      .iter()
      .map(FailedTest::named)
      .chain(
        self
          .files
          .iter()
          .filter(|file| !self.tests.iter().any(|test| &test.file == *file))
          .cloned(),
      )
      .collect()
  }
}

/// Where flaky tests are written down, under the data root's `testbed/`.
pub fn flaky_record_path(data_root: &Path) -> PathBuf {
  data_root.join("testbed").join("suite-flaky.jsonl")
}

/// One sighting of flaky tests: what they were, the commit the suite ran on and its log.
#[derive(Debug, Clone, Copy)]
pub struct FlakySeen<'a> {
  pub flaky: &'a Flaky,
  pub head: &'a str,
  pub log: &'a str,
}

/// The sighting appended to the record as one line, with the time it was written.
pub fn write_flaky(data_root: &Path, seen: &FlakySeen<'_>) -> std::io::Result<()> {
  let path = flaky_record_path(data_root);
  let tests: Vec<_> = seen
    .flaky
    .tests
    .iter()
    .map(|one| json!({ "file": one.file, "test": one.test }))
    .collect();
  let line = json!({
    "at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
    "head": seen.head,
    "tests": tests,
    "files": seen.flaky.files,
    "log": seen.log,
  });

  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir)?;
  }

  let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;

  writeln!(file, "{line}")
}

/// Where the suite's own lines go: the file standard error is, where it is one, as the record names a log.
fn own_log() -> String {
  std::fs::read_link("/proc/self/fd/2")
    .ok()
    .filter(|path| path.is_file())
    .map(|path| path.to_string_lossy().into_owned())
    .unwrap_or_default()
}

/// What passed only the second time: each test the first run of a unit named as failing, and the unit, for
/// every unit whose run once more passed.
pub fn flaky_of(once_more: &[UnitOnceMore]) -> Flaky {
  let passed = || once_more.iter().filter(|one| one.passed);

  Flaky {
    tests: passed().flat_map(|one| one.failed_tests.iter().cloned()).collect(),
    files: passed().map(|one| one.unit.clone()).collect(),
  }
}

/// The suite run with `--again` (docs/suite.md, 「What failed runs once more」): each unit that ran and failed
/// runs once more as soon as it ends and stands on that second run. What passed only the second time is
/// written down under the data root and sent to the hook as `suite-flaky`. Every line the run writes goes to
/// standard error as it comes. What the run came to, and what was flaky.
pub fn run_judged(repo: &Repository, ask: &SuiteAsk, data_root: &Path, hook: &Hook) -> (SuiteEnded, Flaky) {
  let said = &SUITE.again;
  let ended = run(
    repo,
    &SuiteAsk {
      again: true,
      ..ask.clone()
    },
  );
  let flaky = flaky_of(&ended.once_more);

  if flaky.is_empty() {
    return (ended, flaky);
  }

  let head = git_lines(&repo.root, &["rev-parse", "HEAD"])
    .ok()
    .and_then(|lines| lines.into_iter().next());
  let log = ended.log.as_ref().map_or_else(own_log, |path| path.to_string_lossy().into_owned());
  let seen = FlakySeen {
    flaky: &flaky,
    head: head.as_deref().unwrap_or_default(),
    log: &log,
  };

  if let Err(why) = write_flaky(data_root, &seen) {
    eprintln!(
      "{}",
      fill_marks(&said.unrecorded, &[("mark", &SUITE.lines.mark), ("why", &why.to_string())])
    );
  }

  let tests: Vec<_> = flaky.tests.iter().map(|one| json!({ "test": one.test, "unit": one.file })).collect();

  hook.send(&Event {
    name: "suite-flaky",
    commit: head,
    summary: fill_marks(
      &said.flaky_summary,
      &[("count", &flaky.names().len().to_string()), ("repository", &repo.name())],
    ),
    detail: json!({ "tests": tests, "units": flaky.files, "log": log }),
  });

  (ended, flaky)
}
