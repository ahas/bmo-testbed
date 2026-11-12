//! The record as the binary's verbs keep it (docs/boundary.md, 「The record」): a test run written through
//! the record as its turn comes and ended with its counts, and a verb's outcome printed as one command run.
//! A record that cannot be written is said once and changes nothing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bmo_testbed_client::{CommandRun, TestRun, TestRunKind, TestRunState};
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::placement::DEPLOY_UNIT_VARIABLE;
use bmo_testbed_core::turn::scope_of;
use bmo_testbed_core::witness::{RunBegun, RunEnded, RunVerdict, RunWitness};
use bmo_testbed_record::{Record, TestRunEnd, say_unwritten};
use bmo_testbed_suite::run::SuiteRecord;
use bmo_testbed_suite::workspace::git_lines;

/// The witness keeping a verb's test runs in the record under a data root, as runs of one kind.
pub struct Recorded {
  pub data_root: PathBuf,
  pub kind: TestRunKind,
  pub repository_name: Option<String>,
}

impl Recorded {
  /// The command runs of the `run` verb.
  pub fn commands(data_root: &Path) -> Self {
    Self {
      data_root: data_root.to_path_buf(),
      kind: TestRunKind::Command,
      repository_name: None,
    }
  }

  fn written<T>(&self, write: impl FnOnce(&mut Record) -> Result<T, String>) -> Option<T> {
    match Record::open(&self.data_root).and_then(|mut record| write(&mut record)) {
      Ok(written) => Some(written),
      Err(why) => {
        say_unwritten(&why);
        None
      }
    }
  }
}

impl RunWitness for Recorded {
  fn began(&self, run: &RunBegun<'_>) -> Option<i64> {
    // What the run stands on as its turn comes: the checkout's commit, and whether its tracked files stand
    // unchanged against it, each none where git cannot tell.
    let commit = git_lines(run.checkout, &["rev-parse", "HEAD"])
      .ok()
      .and_then(|lines| lines.into_iter().next());
    let clean = git_lines(run.checkout, &["status", "--porcelain", "--untracked-files=no"])
      .ok()
      .map(|changed| changed.is_empty());
    let begun = TestRun {
      id: 0,
      kind: self.kind,
      checkout: run.checkout.to_string_lossy().into_owned(),
      repository_name: self.repository_name.clone(),
      commit,
      whole: Some(run.whole),
      clean,
      command: run.command.to_owned(),
      scope: scope_of(run.env),
      unit_name: run.env.get(DEPLOY_UNIT_VARIABLE).filter(|unit| !unit.is_empty()).cloned(),
      pid: i64::from(std::process::id()),
      state: TestRunState::Running,
      passed_count: 0,
      failed_count: 0,
      skipped_count: 0,
      log_path: run.log.to_string_lossy().into_owned(),
      started_at: run.started_at,
      ended_at: None,
    };

    self.written(|record| record.begin_test_run(&begun)).map(|run| run.id)
  }

  fn ended(&self, id: i64, run: &RunEnded) {
    let end = TestRunEnd {
      state: match run.verdict {
        RunVerdict::Passed => TestRunState::Passed,
        RunVerdict::Failed => TestRunState::Failed,
        RunVerdict::NotRun => TestRunState::NotRun,
      },
      passed_count: run.passed as i64,
      failed_count: run.failed as i64,
      skipped_count: run.skipped as i64,
      ended_at: run.ended_at,
    };

    self.written(|record| record.end_test_run(id, &end));
  }
}

/// Where a suite run of the repository keeps its own log and its test run: under the data root, its words
/// as given, and the repository's name.
pub fn suite_record(data_root: &Path, repo: &Repository, command: String) -> SuiteRecord {
  SuiteRecord {
    log_dir: data_root.join("testbed"),
    command,
    witness: Arc::new(Recorded {
      data_root: data_root.to_path_buf(),
      kind: TestRunKind::Suite,
      repository_name: Some(repo.name()),
    }),
  }
}

/// A verb's outcome as the one command run its `--json` prints: its code, its log, its wait and its time.
pub fn command_run(
  code: u8,
  log: Option<&Path>,
  waited_seconds: f64,
  seconds: f64,
  test_run: Option<i64>,
) -> String {
  let run = CommandRun {
    exit_code: i64::from(code),
    log_path: log.map(|log| log.to_string_lossy().into_owned()).unwrap_or_default(),
    waited_ms: (waited_seconds * 1000.0).round() as i64,
    spent_ms: (seconds * 1000.0).round() as i64,
    test_run_id: test_run,
  };

  serde_json::to_string(&run).unwrap_or_default()
}
