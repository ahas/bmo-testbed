//! The data backup's own numbers, names and words (docs/backup.md), read through core's policy loader from
//! `policy/data-backup.toml`, each line with the `{name}` marks its file names.

use std::sync::LazyLock;

use serde::Deserialize;

/// The defaults a settings file may set otherwise (docs/backup.md, 「The settings」).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
  pub large_file_bytes: u64,
  pub server: String,
  pub staging: String,
  pub prefix: String,
  pub hourly_slots: u64,
  pub daily_days: u64,
  pub releases_kept: usize,
  pub drill_max_age_hours: u64,
  pub room_floor_bytes: u64,
  pub s3_command: String,
  pub rclone_command: String,
}

/// The names the backup's own files, keys and databases go by.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Names {
  pub settings_file: String,
  pub manifest: String,
  pub data_archive: String,
  pub dump_suffix: String,
  pub archive_suffix: String,
  pub large_files: String,
  pub drill_database: String,
  pub hourly: String,
  pub daily: String,
  pub media: String,
  pub releases: String,
  pub next: String,
  pub latest: String,
  pub drill: String,
}

/// The numbers the dump, the pack and the restore run by.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
  pub manifest_version: u32,
  pub pack_level: u32,
  pub dump_compress: String,
  pub restore_jobs: u32,
  pub drop_tries: u32,
  pub drop_pause_milliseconds: u64,
  pub quoted_chars: usize,
}

/// What the verb says as it runs, and what a failure says.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Words {
  pub no_settings: String,
  pub settings_unread: String,
  pub settings_wrong: String,
  pub locked: String,
  pub snapshot_holder: String,
  pub media_holder: String,
  pub drill_holder: String,
  pub prune_holder: String,
  pub dumped: String,
  pub packed: String,
  pub packed_media: String,
  pub left_out: String,
  pub missing_folder: String,
  pub sent: String,
  pub release_sent: String,
  pub no_room: String,
  pub removed: String,
  pub would_remove: String,
  pub nothing_to_remove: String,
  pub snapshot_done: String,
  pub media_done: String,
  pub drill_from: String,
  pub drill_database: String,
  pub drill_data: String,
  pub drill_passed: String,
  pub drill_failed: String,
  pub no_snapshot: String,
  pub too_old: String,
  pub sha_mismatch: String,
  pub size_mismatch: String,
  pub not_listed: String,
  pub count_mismatch: String,
  pub restore_complained: String,
  pub table_missing: String,
  pub not_dropped: String,
  pub table_extra: String,
  pub entries_mismatch: String,
  pub unstarted: String,
  pub exited: String,
  pub listing_unread: String,
  pub manifest_unread: String,
  pub failed_summary: String,
  pub drill_failed_summary: String,
  pub failed_event: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupPolicy {
  pub defaults: Defaults,
  pub names: Names,
  pub numbers: Numbers,
  pub words: Words,
}

pub static BACKUP: LazyLock<BackupPolicy> = LazyLock::new(|| {
  bmo_testbed_core::policy::read(
    "data-backup.toml",
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../policy/data-backup.toml")),
  )
});
