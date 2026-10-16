//! What the test count says (docs/checks.md, 「The test count」), each line with the `{name}` marks its
//! file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Count {
  pub usage: String,
  pub tree: String,
  pub range: String,
  pub removed: String,
  pub open: String,
  pub failed: String,
  pub unaccounted: String,
  pub test: String,
  pub joiner: String,
  pub unreadable: String,
  pub start: String,
  pub head: String,
}

super::part!(pub COUNT: Count = "count.toml");
