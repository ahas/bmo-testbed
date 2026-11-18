//! What the harness plugin says of itself, of each tool and in the OpenAPI document (docs/boundary.md, 「The harness plugin」).

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDescriptions {
  pub testbed_queues: String,
  pub testbed_test_runs: String,
  pub testbed_test_run: String,
  pub testbed_deployments: String,
  pub testbed_deployment: String,
  pub testbed_run: String,
  pub testbed_check: String,
  pub testbed_suite: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginLines {
  pub bad_arguments: String,
  pub no_command: String,
  pub since_with_checkouts: String,
  pub no_tool: String,
  pub unstarted: String,
  pub unread: String,
  pub unserved: String,
  pub ended: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentWords {
  pub title: String,
  pub description: String,
  pub server: String,
  pub bearer: String,
  pub entry: String,
  pub health: String,
  pub events: String,
  pub queues: String,
  pub test_runs: String,
  pub test_run: String,
  pub deployments: String,
  pub deployment: String,
  pub log_lines: String,
  pub restarts: String,
  pub error: String,
  pub after: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPolicy {
  pub instructions: String,
  pub document: DocumentWords,
  pub tools: ToolDescriptions,
  pub lines: PluginLines,
}

super::part!(pub PLUGIN: PluginPolicy = "plugin.toml");
