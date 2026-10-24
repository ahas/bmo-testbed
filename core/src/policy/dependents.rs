//! What the `dependents` verb reads and says (docs/suite.md, 「The dependents of a major change」): the
//! machine-level registry of checkouts and the lines it answers with.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineWords {
  pub heading: String,
  pub none: String,
  pub unread: String,
  pub declaration: String,
  pub entry: String,
  pub running: String,
  pub pass: String,
  pub fail: String,
  pub unstarted: String,
  pub passed_all: String,
  pub failed_some: String,
  pub unstarted_some: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependents {
  pub registry: String,
  pub lines: LineWords,
}

super::part!(pub DEPENDENTS: Dependents = "dependents.toml");
