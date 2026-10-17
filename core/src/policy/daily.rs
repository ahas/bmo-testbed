//! What the daily whole run says (docs/suite.md, 「The daily whole run」), each line with the `{name}`
//! marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeWords {
  pub not_moved: String,
  pub not_made: String,
  pub setup_failed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunWords {
  pub unit_failed: String,
  pub started: String,
  pub refused: String,
  pub not_due: String,
  pub no_head: String,
  pub no_head_verdict: String,
  pub begins: String,
  pub failed: String,
  pub again: String,
  pub no_verdict: String,
  pub pass: String,
  pub setup_step: String,
  pub joiner: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountWords {
  pub summary: String,
  pub at_head: String,
  pub no_head: String,
  pub files: String,
  pub file: String,
  pub joiner: String,
  pub outside: String,
  pub outside_listed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Daily {
  pub tree: TreeWords,
  pub run: RunWords,
  pub account: AccountWords,
}

super::part!(pub DAILY: Daily = "daily.toml");
