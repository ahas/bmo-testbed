//! What the backup and the `backups` verb say and count by (docs/deploy.md, 「Backups」 and 「Sending
//! backups away」), each line with the `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
  pub pack_level: u32,
  pub kept_locally: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backups {
  pub upload_usage: Vec<String>,
  pub upload_failed_note: String,
  pub upload_failed: String,
  pub numbers: Numbers,
}

super::part!(pub BACKUPS: Backups = "backups.toml");
