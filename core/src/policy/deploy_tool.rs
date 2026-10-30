//! What the hosted deploy tool reads and says (docs/deploy.md, 「The hosted deploy tool」): the entry it
//! answers, how it waits a deployment out, and the lines it answers with.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
  pub poll_seconds: u64,
  pub wait_seconds: u64,
  pub journal_lines: usize,
  pub commit_shortest: usize,
  pub commit_longest: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lines {
  pub live: String,
  pub previous: String,
  pub no_previous: String,
  pub message: String,
  pub unit: String,
  pub not_live: String,
  pub not_written: String,
  pub nothing_recorded: String,
  pub still_running: String,
  pub unasked: String,
  pub journal_unread: String,
  pub refused: String,
  pub failed: String,
  pub no_code: String,
  pub no_unit: String,
  pub not_started: String,
  pub said_unit: String,
  pub said_how: String,
  pub arguments: String,
  pub empty: String,
  pub commit: String,
  pub no_registry: String,
  pub registry: String,
  pub unknown: String,
  pub unread: String,
  pub no_names: String,
  pub ambiguous: String,
  pub no_deploy: String,
  pub request: String,
  pub protocol: String,
  pub other_entry: String,
  pub no_resources: String,
  pub other_kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployTool {
  pub entry: String,
  pub protocol: u32,
  pub numbers: Numbers,
  pub lines: Lines,
}

super::part!(pub DEPLOY_TOOL: DeployTool = "deploy-tool.toml");
