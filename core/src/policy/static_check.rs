//! What the static check says of its steps (docs/checks.md, 「The static check」), each line with the
//! `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticCheck {
  pub step: String,
  pub ended: String,
  pub not_run: String,
  pub not_had: String,
  pub failed: String,
}

super::part!(pub STATIC_CHECK: StaticCheck = "static-check.toml");
