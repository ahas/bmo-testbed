//! Where a heavy run goes before it asks for its turn (docs/boundary.md, 「Placement」): here, or on another
//! machine, whose output and exit code come back. The common type asks one thing of a placement; the local
//! placement, always here, is the default every caller with no other stands on.

use std::path::PathBuf;

use crate::policy::lanes::LANES;
use crate::turn::{Env, HELD_VARIABLE, Lane, harness_pid};

/// The variable a deployment's or a whole run's unit is named by inside it.
pub const DEPLOY_UNIT_VARIABLE: &str = "BMO_DEPLOY_UNIT";

/// A run asked about before its turn: its words and where its caller stands.
pub struct PlaceAsk<'a> {
  pub command: &'a [String],
  pub lane: Lane,
  /// The caller's wait for the turn and its guard, in seconds.
  pub wait_seconds: u64,
  pub guard_seconds: u64,
  /// The directory the run was started in.
  pub cwd: PathBuf,
  pub env: &'a Env,
}

impl PlaceAsk<'_> {
  /// The caller's wait: the one it gave, or a harness's call's or a unit's.
  pub fn wait_of(given: Option<f64>, env: &Env) -> u64 {
    let numbers = &LANES.numbers;

    given.map_or(
      if harness_pid(env).is_some() {
        numbers.harness_wait_seconds
      } else {
        numbers.unit_wait_seconds
      },
      |seconds| seconds as u64,
    )
  }

  /// The caller's guard: the one it gave, or the run's own.
  pub fn guard_of(given: Option<f64>) -> u64 {
    given.map_or(LANES.numbers.guard_seconds, |seconds| seconds as u64)
  }
}

/// Where a run goes.
pub trait Placement {
  /// The exit code the run ended with where it ran elsewhere, or none where it runs here.
  fn place(&self, ask: &PlaceAsk<'_>) -> Option<u8>;
}

/// The placement that keeps every run where it was started.
pub struct Local;

impl Placement for Local {
  fn place(&self, _: &PlaceAsk<'_>) -> Option<u8> {
    None
  }
}

/// Whether a run goes where it stands whatever a placement would answer: one started inside a turn its
/// caller holds, a placed machine's own run among them, and one inside a deployment's or a whole run's unit.
pub fn runs_here(env: &Env) -> bool {
  [HELD_VARIABLE, DEPLOY_UNIT_VARIABLE]
    .iter()
    .any(|name| env.get(*name).is_some_and(|value| !value.is_empty()))
}
