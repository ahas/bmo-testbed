//! What the nightly round holds to and says (docs/suite.md, 「The nightly round」), each line with the
//! `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunWords {
  pub unit_failed: String,
  pub started: String,
  pub refused: String,
  pub not_due: String,
  pub started_since: String,
  pub window_passed: String,
  pub bad_at: String,
  pub registry_unread: String,
  pub begins: String,
  pub skip: String,
  pub would_run: String,
  pub running: String,
  pub passed: String,
  pub failed: String,
  pub not_run: String,
  pub busy: String,
  pub unread: String,
  pub passed_over: String,
  pub no_suite: String,
  pub ended: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetWords {
  pub no_remote: String,
  pub second_clone: String,
  pub no_declaration: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountWords {
  pub summary: String,
  pub entry: String,
  pub at: String,
  pub files: String,
  pub file: String,
  pub more: String,
  pub log: String,
  pub joiner: String,
  pub between: String,
  pub outside: String,
  pub unread: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nightly {
  pub at: String,
  pub window_hours: u32,
  pub folder: String,
  pub lock: String,
  pub files_shown: usize,
  pub run: RunWords,
  pub set: SetWords,
  pub account: AccountWords,
}

super::part!(pub NIGHTLY: Nightly = "nightly.toml");
