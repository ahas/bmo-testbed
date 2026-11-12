//! What the testbed says of the events it sends (docs/boundary.md, 「Events」), each line with the `{name}`
//! marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventLines {
  pub unhooked: String,
  pub failed: String,
  pub overran: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Events {
  pub version: u64,
  pub hook_seconds: u64,
  pub lines: EventLines,
}

super::part!(pub EVENTS: Events = "events.toml");
