//! Where a heavy run goes before it asks for its turn (docs/boundary.md, 「Placement」): here, or on another
//! machine, whose output and exit code come back. The common type asks one thing of a placement; the local
//! placement, always here, is the default every caller with no other stands on.

use std::path::PathBuf;

use crate::turn::{Env, HELD_VARIABLE, Lane, lock_path, read_queue};

/// The variable a deployment's or a whole run's unit is named by inside it.
pub const DEPLOY_UNIT_VARIABLE: &str = "BMO_DEPLOY_UNIT";
/// The variable the `placed` verb marks the run it starts with: a run on a placed machine, never placed again.
pub const PLACED_VARIABLE: &str = "BMO_TESTBED_PLACED";

/// A run asked about before its turn: its words and where its caller stands.
pub struct PlaceAsk<'a> {
  pub command: &'a [String],
  /// The language the caller named to read the command, where it named one.
  pub adapter: Option<&'a str>,
  pub lane: Lane,
  /// The directory the run was started in.
  pub cwd: PathBuf,
  pub env: &'a Env,
  /// Whether it goes only to a seat free there now: a run placed because this machine is full, which would
  /// otherwise wait in that machine's line while this one's frees, waits in this one's instead.
  pub to_free_seat: bool,
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
/// caller holds, one on a placed machine, and one inside a deployment's or a whole run's unit.
pub fn runs_here(env: &Env) -> bool {
  [HELD_VARIABLE, DEPLOY_UNIT_VARIABLE, PLACED_VARIABLE]
    .iter()
    .any(|name| env.get(*name).is_some_and(|value| !value.is_empty()))
}

/// Whether this machine's wide lane has no seat for a run now: every seat held, or someone already waiting for
/// one. A run that may go either way goes elsewhere only then, so this machine's own seats are used first.
pub fn wide_lane_full(env: &Env) -> bool {
  let line = read_queue(&lock_path(env, Lane::Wide));

  line.holders.len() >= Lane::Wide.seats(env) || !line.waiting.is_empty()
}
