//! What a test run tells whoever keeps the record of it (docs/boundary.md, 「The record」): that it has
//! begun, once its turn came, and how it ended. `core` keeps no record itself; its consumer hands a run the
//! witness it keeps one through, and the default keeps none.

use std::path::Path;

use crate::turn::Env;

/// A test run as it begins: its words, where it runs, its log, when and in what environment.
pub struct RunBegun<'a> {
  /// The words run, as one line.
  pub command: &'a str,
  pub checkout: &'a Path,
  pub log: &'a Path,
  /// Epoch milliseconds.
  pub started_at: i64,
  /// The environment the run was started in, which names the caller's scope and the unit it runs in.
  pub env: &'a Env,
}

/// What a test run came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunVerdict {
  Passed,
  Failed,
  /// It had its turn, and ended not run all the same.
  NotRun,
}

/// How a test run ended: its verdict and its counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunEnded {
  pub verdict: RunVerdict,
  pub passed: u64,
  pub failed: u64,
  pub skipped: u64,
  /// Epoch milliseconds.
  pub ended_at: i64,
}

/// Whoever keeps the record of test runs. Nothing it answers changes the run.
pub trait RunWitness {
  /// The run has its turn and begins; the id it is kept under, where it was kept.
  fn began(&self, run: &RunBegun<'_>) -> Option<i64>;

  /// The run kept under the id has ended.
  fn ended(&self, id: i64, run: &RunEnded);
}

/// The witness that keeps no record.
pub struct Unwitnessed;

impl RunWitness for Unwitnessed {
  fn began(&self, _: &RunBegun<'_>) -> Option<i64> {
    None
  }

  fn ended(&self, _: i64, _: &RunEnded) {}
}
