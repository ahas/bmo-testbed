//! `deploy history` prints a deployment as one line: when it started, its repository without the set's prefix,
//! its commit cut to nine characters, how it stands in the policy's words, and the sentence beside its outcome
//! where one stands (docs/boundary.md, `deploy`).

use bmo_testbed_cli::deploy_history::line_of;
use bmo_testbed_client::{Deployment, DeploymentState};
use bmo_testbed_core::policy::verbs::VERBS;

fn deployment(state: DeploymentState, reason: Option<&str>) -> Deployment {
  Deployment {
    id: 7,
    repository_name: Some("bmo-unit-a".to_owned()),
    unit_name: Some("deploy-unit-a".to_owned()),
    pid: 1,
    commit: Some("0123456789abcdef".to_owned()),
    state,
    plan: vec![],
    stages: vec![],
    steps: vec![],
    rolled_back: false,
    stopped_for: None,
    reason: reason.map(str::to_owned),
    started_at: 0,
    ended_at: None,
  }
}

#[test]
fn a_deployment_is_one_line_of_its_repository_commit_state_and_reason() {
  let live = line_of(&deployment(DeploymentState::Live, None));
  let failed = line_of(&deployment(DeploymentState::Failed, Some("the suite ended with exit 1")));

  assert!(live.ends_with(&format!(" unit-a 012345678 {}", VERBS.history.live)), "{live}");
  assert!(
    failed.ends_with(&format!(" unit-a 012345678 {} · the suite ended with exit 1", VERBS.history.failed)),
    "{failed}"
  );
}
