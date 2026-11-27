//! `deploy history`: the machine's deployments as the testbed's record keeps them, newest first, one a line
//! (docs/boundary.md, `deploy`): when each started, its repository, its commit, how it stands and the sentence
//! beside its outcome.

use bmo_testbed_client::{Deployment, DeploymentState, PageQuery};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::verb::data_root;
use bmo_testbed_record::Record;
use chrono::{DateTime, Local};

use crate::words::HistoryArgs;

// The most lines one call prints, and how many it prints where none is asked.
const MOST: usize = 200;
const SOME: usize = 20;

/// The deployments of the repository named, or of every repository, newest first; 0 where the record was read, 2
/// where it could not be.
pub fn history(args: &HistoryArgs) -> u8 {
  let said = &VERBS.history;
  let unread = |why: &str| {
    eprintln!("{}", fill_marks(&said.unread, &[("why", why)]));
    2
  };
  let record = match Record::open(&data_root()) {
    Ok(record) => record,
    Err(why) => return unread(&why),
  };
  let wanted = args.limit.unwrap_or(SOME).clamp(1, MOST);
  let named = args.repository.as_deref().map(|name| name.trim_start_matches("bmo-").to_owned());
  let mut lines = Vec::new();
  let mut after = None;

  // The record pages newest first; a repository named is picked out of the pages until enough stand.
  while lines.len() < wanted {
    let page = match record.deployments(&PageQuery {
      after,
      limit: Some(100),
    }) {
      Ok(page) => page,
      Err(why) => return unread(&why),
    };
    let Some(last) = page.items.last().map(|deployment| deployment.id) else { break };

    lines.extend(
      page
        .items
        .iter()
        .filter(|deployment| named.as_deref().is_none_or(|name| repository_of(deployment) == name))
        .map(line_of),
    );

    if !page.more {
      break;
    }

    after = Some(last);
  }

  lines.truncate(wanted);

  if lines.is_empty() {
    let of = named.map(|name| fill_marks(&said.of, &[("repository", &name)])).unwrap_or_default();

    println!("{}", fill_marks(&said.none, &[("of", &of)]));
  }

  for line in lines {
    println!("{line}");
  }

  0
}

// A deployment's repository as the record names it, without the set's prefix.
fn repository_of(deployment: &Deployment) -> &str {
  deployment.repository_name.as_deref().unwrap_or("?").trim_start_matches("bmo-")
}

/// One deployment as a line: when it started, its repository, its commit, how it ended, and why.
pub fn line_of(deployment: &Deployment) -> String {
  let started = DateTime::from_timestamp_millis(deployment.started_at)
    .map(|at| at.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
    .unwrap_or_default();
  let commit: String = deployment.commit.as_deref().unwrap_or("?").chars().take(9).collect();
  let said = &VERBS.history;
  let state = match deployment.state {
    DeploymentState::Running => &said.running,
    DeploymentState::Live => &said.live,
    DeploymentState::RolledBack => &said.rolled_back,
    DeploymentState::Failed => &said.failed,
    DeploymentState::Stopped => &said.stopped,
    DeploymentState::Gone => &said.gone,
  };
  let reason = deployment
    .reason
    .as_deref()
    .filter(|reason| !reason.is_empty())
    .map(|reason| format!(" · {reason}"))
    .unwrap_or_default();

  format!("{started} {} {commit} {state}{reason}", repository_of(deployment))
}
