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
pub struct BackupWords {
  pub not_made: String,
  pub no_fifo: String,
  pub no_hash: String,
  pub no_pipeline: String,
  pub read_back_exits: String,
  pub unstarted: String,
  pub exited: String,
  pub no_snapshot: String,
  pub unwritten: String,
  pub not_whole: String,
  pub not_placed: String,
  pub not_the_stream: String,
  pub not_restored: String,
  pub nothing_changed: String,
  pub backup_removed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadWords {
  pub stands: String,
  pub unconfirmed: String,
  pub rotated: String,
  pub removed_locally: String,
  pub mismatch: String,
  pub remote_unread: String,
  pub not_s3: String,
  pub unstarted: String,
  pub exited: String,
  pub changed: String,
  pub listing_unread: String,
  pub back_exited: String,
  pub back_empty: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backups {
  pub upload_heading: String,
  pub upload_usage: Vec<String>,
  pub upload_failed_note: String,
  pub upload_failed: String,
  pub numbers: Numbers,
  pub backup: BackupWords,
  pub upload: UploadWords,
}

super::part!(pub BACKUPS: Backups = "backups.toml");
