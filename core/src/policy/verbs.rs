//! The binary's own words (docs/boundary.md, 「The binary」): the usage each verb prints and what
//! `classify` answers, each line with the `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageWords {
  pub binary: String,
  pub run: String,
  pub status: String,
  pub classify: String,
  pub suite: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifyWords {
  pub light: String,
  pub heavy: String,
  pub instead: String,
  pub unread: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verbs {
  pub usage: UsageWords,
  pub classify: ClassifyWords,
}

super::part!(pub VERBS: Verbs = "verbs.toml");
