//! The manifest a snapshot carries (docs/backup.md, 「What is backed up」): when it was taken and into
//! which slot, each file's size and SHA-256, each database's row counts, and the data archive's entries.
//! It names no row and no file's contents.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A table's rows, by `schema.table`.
pub type Tables = BTreeMap<String, i64>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
  pub name: String,
  pub size: u64,
  pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseEntry {
  pub name: String,
  pub file: String,
  pub tables: Tables,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataEntry {
  pub file: String,
  pub entries: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
  pub version: u32,
  /// RFC 3339, the machine's local offset.
  pub taken_at: String,
  /// Epoch milliseconds.
  pub taken_ms: i64,
  pub slot: u64,
  pub files: Vec<FileEntry>,
  pub databases: Vec<DatabaseEntry>,
  pub data: Option<DataEntry>,
}

impl Manifest {
  /// Every file's bytes together.
  pub fn bytes(&self) -> u64 {
    self.files.iter().map(|file| file.size).sum()
  }
}
