//! A suite that failed, run once more before it is judged (docs/suite.md, 「What failed runs once
//! more」): the one step a close and a deployment both take, and the record of what passed
//! only the second time, so a test failing now and then is mended rather than forgotten.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use serde_json::json;

use crate::log::{
  FailedTest, failed_files_of, failed_tests_of, failures_of, unstarted_of, verdict_of, warnings_of,
};
use crate::run::{SuiteAsk, run_saying};
use crate::workspace::git_lines;

/// A suite's run: its exit code, and what its log says of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SuiteRun {
  pub code: i64,
  /// The suite's own last line, where it wrote one.
  pub verdict: Option<String>,
  /// What failed as the log names it: each failing test, and the files the sweep names.
  pub failures: Vec<String>,
  /// Each failing test with the file or unit it stands in.
  pub failed_tests: Vec<FailedTest>,
  /// The test files and units that failed.
  pub failed_files: Vec<String>,
  /// The files and units that never started within the suite's guard.
  pub unstarted: Vec<String>,
  /// The warnings the tests printed.
  pub warnings: Vec<String>,
}

fn once(all: impl IntoIterator<Item = String>) -> Vec<String> {
  let mut seen: Vec<String> = Vec::new();

  for one in all {
    if !seen.contains(&one) {
      seen.push(one);
    }
  }

  seen
}

impl SuiteRun {
  /// A run read out of the output it wrote and the code it ended with.
  pub fn of_log(code: i64, log: &str) -> Self {
    Self {
      code,
      verdict: verdict_of(log),
      failures: failures_of(log),
      failed_tests: failed_tests_of(log),
      failed_files: failed_files_of(log),
      unstarted: unstarted_of(log),
      warnings: warnings_of(log),
    }
  }

  /// Whether it passed: it ended in nought and wrote its verdict, since a run that wrote none has no result
  /// whatever its code says.
  pub fn passed(&self) -> bool {
    self.code == 0 && self.verdict.is_some()
  }

  /// What a run once more takes: the files and units that failed, then those that never started, each once.
  pub fn again(&self) -> Vec<String> {
    once(self.failed_files.iter().chain(&self.unstarted).cloned())
  }
}

/// A first run, and the second where what failed or never started in it ran once more.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SuiteRuns {
  pub first: SuiteRun,
  pub again: Option<SuiteRun>,
}

impl SuiteRuns {
  /// The run the judgement stands on: the second where there is one.
  pub fn last(&self) -> &SuiteRun {
    self.again.as_ref().unwrap_or(&self.first)
  }

  pub fn passed(&self) -> bool {
    self.last().passed()
  }

  /// Each run, in the order it ran.
  pub fn each(&self) -> impl Iterator<Item = &SuiteRun> {
    std::iter::once(&self.first).chain(&self.again)
  }

  /// The files and units that failed in either run, each once.
  pub fn failed_files(&self) -> Vec<String> {
    once(self.each().flat_map(|run| run.failed_files.iter().cloned()))
  }

  /// What failed in either run as the logs name it, each once.
  pub fn failures(&self) -> Vec<String> {
    once(self.each().flat_map(|run| run.failures.iter().cloned()))
  }

  /// What passed only the second time: what the first run failed, where the run once more passed. One that
  /// only never started is not flaky, since nothing of it had run.
  pub fn flaky(&self) -> Flaky {
    if self.again.is_none() || !self.passed() {
      return Flaky::default();
    }

    Flaky {
      tests: self.first.failed_tests.clone(),
      files: self.first.failed_files.clone(),
    }
  }
}

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

/// The suite judged on a run once more where its first run failed or left files unstarted: `again` runs
/// the files and units it is handed, alone and by name, in the same tree and over the same range as the
/// first. It runs once more at most, and not at all where there is nothing to name — a first run that
/// passed, or one whose turn never came and started nothing.
pub fn run_once_more(first: SuiteRun, again: impl FnOnce(&[String]) -> SuiteRun) -> SuiteRuns {
  let named = first.again();

  if first.passed() || named.is_empty() {
    return SuiteRuns { first, again: None };
  }

  let second = again(&named);

  SuiteRuns {
    first,
    again: Some(second),
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

/// The suite run with `--again` (docs/suite.md, 「What failed runs once more」): run once, and where it failed
/// or left units unstarted, run once more over what it named, judged on that second run. What passed only
/// the second time is written down under the data root and sent to the hook as `suite-flaky`. Every line
/// both runs write goes to standard error as it comes; the exit code the judgement ends with.
pub fn run_judged(repo: &Repository, ask: &SuiteAsk, data_root: &Path, hook: &Hook) -> u8 {
  let said = &SUITE.again;
  let heard = Mutex::new(String::new());
  let say = |line: &str| {
    eprintln!("{line}");

    if let Ok(mut heard) = heard.lock() {
      heard.push_str(line);
      heard.push('\n');
    }
  };
  let taken = || heard.lock().map(|mut heard| std::mem::take(&mut *heard)).unwrap_or_default();
  let code = run_saying(repo, ask, &say);
  let first = SuiteRun::of_log(i64::from(code), &taken());
  let mut last = code;
  let runs = run_once_more(first, |named| {
    say(&format!(
      "\n{}",
      fill_marks(
        &said.once_more,
        &[
          ("mark", &SUITE.lines.mark),
          ("count", &named.len().to_string()),
          ("names", &named.join(&said.joiner)),
        ],
      )
    ));

    let again = SuiteAsk {
      picked: named.to_vec(),
      ..ask.clone()
    };

    last = run_saying(repo, &again, &say);
    SuiteRun::of_log(i64::from(last), &taken())
  });

  if runs.again.is_none() {
    return code;
  }

  if !runs.passed() {
    eprintln!(
      "{}",
      fill_marks(
        &said.either,
        &[("mark", &SUITE.lines.mark), ("names", &runs.failures().join(&said.joiner))]
      )
    );
    return last;
  }

  let flaky = runs.flaky();

  if !flaky.is_empty() {
    let head = git_lines(&repo.root, &["rev-parse", "HEAD"])
      .ok()
      .and_then(|lines| lines.into_iter().next());
    let log = own_log();
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

    let tests: Vec<_> =
      flaky.tests.iter().map(|one| json!({ "test": one.test, "unit": one.file })).collect();

    hook.send(&Event {
      name: "suite-flaky",
      commit: head,
      summary: fill_marks(
        &said.flaky_summary,
        &[("count", &flaky.names().len().to_string()), ("repository", &repo.name())],
      ),
      detail: json!({ "tests": tests, "units": flaky.files, "log": log }),
    });
  }

  last
}
