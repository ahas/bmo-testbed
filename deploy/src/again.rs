//! A deployment asked again (docs/deploy.md, 「The deployment line」): a repository and commit stand once for their
//! deploy root, read as commits rather than as the names they were asked by, and a commit whose gate failed is
//! not run through that gate again while nothing it stands on has moved.

use std::path::Path;

use bmo_testbed_client::{Deployment, DeploymentState, StopCode};
use bmo_testbed_record::Record;

use crate::line::{DeployTicket, standing};

/// Whether a deployment ended failing in its gate, the `check` stage.
pub fn failed_in_gate(deployment: &Deployment) -> bool {
  deployment.state == DeploymentState::Failed && deployment.stopped_for == Some(StopCode::Check)
}

/// The deployment of the repository's whole commit standing for its deploy root's turn under the lock given,
/// holding it or waiting for it, in a unit a caller can follow; the commit each ticket names is read as a commit
/// by `resolve`, which answers a name's whole commit. None where no such deployment stands.
pub fn standing_for(
  turn: &Path,
  repository: &str,
  commit: &str,
  resolve: &dyn Fn(&str) -> Option<String>,
) -> Option<DeployTicket> {
  let root = standing(turn);
  let of_commit = |named: &str| named == commit || resolve(named).as_deref() == Some(commit);

  root
    .holder
    .into_iter()
    .chain(root.waiting)
    .filter(|ticket| ticket.repository.as_deref() == Some(repository) && ticket.unit.is_some())
    .find(|ticket| ticket.commit.as_deref().is_some_and(of_commit))
}

/// The newest ended deployment of the repository's whole commit in the testbed's record under the data root,
/// where it failed in its gate and ended no earlier than `since`, in epoch milliseconds; none otherwise, and none
/// where the record cannot be read, since a record that cannot be read refuses nothing.
pub fn failed_gate(data_root: &Path, repository: &str, commit: &str, since: i64) -> Option<Deployment> {
  Record::open(data_root)
    .and_then(|record| record.newest_ended_deployment(repository, commit))
    .ok()
    .flatten()
    .filter(|deployment| {
      failed_in_gate(deployment) && deployment.ended_at.unwrap_or(deployment.started_at) >= since
    })
}

/// A call that would start a deployment of a commit, as it is judged against the deployments before it.
pub struct AskAgain<'a> {
  pub repository: &'a str,
  /// The whole commit asked for.
  pub commit: &'a str,
  /// The lock of the deploy root's turn.
  pub turn: &'a Path,
  /// The testbed's data root, whose record holds the deployments that ended.
  pub data_root: &'a Path,
  /// It brings what a standing deployment lacks: a command to run before its start, no gate, or going on past a
  /// `quiet` at once.
  pub own_run: bool,
  /// It runs no gate.
  pub no_check: bool,
  /// It is marked first.
  pub first: bool,
  /// The whole commit a name names, none where it names none.
  pub resolve: &'a dyn Fn(&str) -> Option<String>,
  /// The commit the repository's main branch stands on, none where it cannot be read.
  pub main_head: &'a dyn Fn() -> Option<String>,
}

/// What a call that would start a deployment finds before it.
#[derive(Debug, Clone, PartialEq)]
pub enum Asked {
  /// A deployment of the commit standing for the deploy root, which the call attaches to.
  Standing(DeployTicket),
  /// The commit's newest ended deployment, failed in its gate while main still stands on the commit, for which
  /// the call is refused.
  FailedUnmoved(Deployment),
  /// Nothing stands in its way, so the call starts its own.
  Start,
}

/// A call judged against the deployments before it (docs/deploy.md, 「The deployment line」): one of the same
/// commit standing for the deploy root is attached to unless the call brings a run of its own; else a commit
/// whose newest ended deployment failed in its gate is refused while main still stands on it, unless the call is
/// marked first or runs no gate.
pub fn judged(ask: &AskAgain<'_>) -> Asked {
  if !ask.own_run
    && let Some(standing) = standing_for(ask.turn, ask.repository, ask.commit, ask.resolve)
  {
    return Asked::Standing(standing);
  }

  if ask.no_check || ask.first {
    return Asked::Start;
  }

  match failed_gate(ask.data_root, ask.repository, ask.commit, 0) {
    Some(failed) if (ask.main_head)().as_deref() == Some(ask.commit) => Asked::FailedUnmoved(failed),
    _ => Asked::Start,
  }
}
