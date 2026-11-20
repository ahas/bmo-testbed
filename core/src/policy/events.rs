//! What the testbed says of the events it sends (docs/boundary.md, 「Events」), each line with the `{name}`
//! marks its file names, and what `notice` sends and says (「`notice`」).

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
pub struct NoticeLines {
  pub queued: String,
  pub unfound: String,
  pub unread: String,
  pub unreached: String,
  pub refused: String,
  pub roomless: String,
}

/// How `notice` queues an instruction in the owner's private room at the receiver.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notice {
  pub code: String,
  pub sort: String,
  pub publisher: String,
  pub owner_route: String,
  pub route: String,
  pub env_file: String,
  pub address_variable: String,
  pub token_variable: String,
  pub seconds: u64,
  pub quoted_chars: usize,
  pub lines: NoticeLines,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Events {
  pub version: u64,
  pub hook_seconds: u64,
  pub lines: EventLines,
  pub notice: Notice,
}

super::part!(pub EVENTS: Events = "events.toml");
