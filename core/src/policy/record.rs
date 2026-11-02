//! The record's file, what it keeps and how many items a page answers (docs/boundary.md, 「The record」).

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordLines {
  pub unwritten: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordPolicy {
  pub dir: String,
  pub file: String,
  pub keep_days: i64,
  pub event_window_days: i64,
  pub default_limit: i64,
  pub limit_ceiling: i64,
  pub lines: RecordLines,
}

super::part!(pub RECORD: RecordPolicy = "record.toml");
