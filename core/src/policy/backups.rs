//! What the `backups` verb says (docs/deploy.md, 「Sending backups away」), each line with the `{name}`
//! marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backups {
  pub upload_usage: Vec<String>,
  pub upload_failed_note: String,
}

super::part!(pub BACKUPS: Backups = "backups.toml");
