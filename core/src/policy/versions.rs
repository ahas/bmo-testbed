//! The set's versions (docs/deploy.md, 「The deployed version」 and 「The set's release」): the platform's day,
//! the release tag and the nightly's word, the file a tree declares its version in, and what the `release`
//! verb says, each line with the `{name}` marks its file names.

use serde::Deserialize;

/// The files the `release` verb reads, and what of them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileWords {
  pub base: String,
  pub revision: String,
  pub cargo_manifest: String,
  pub cargo_lock: String,
  pub package_manifest: String,
  pub pyproject: String,
  pub json_suffix: String,
  pub markdown_suffix: String,
  pub dependency_tables: Vec<String>,
  pub python_tables: Vec<Vec<String>>,
  pub unread_folders: Vec<String>,
  pub left_shown: usize,
}

/// How the `release` verb names what it sets in a file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhatWords {
  pub package: String,
  pub workspace: String,
  pub dependency: String,
  pub lock: String,
  pub project: String,
  pub version_file: String,
  pub member: String,
  pub unplaced: String,
  pub fixed_at: String,
}

/// What the `release` verb says.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseWords {
  pub subject: String,
  pub heading: String,
  pub before_first: String,
  pub dry: String,
  pub repository: String,
  pub change: String,
  pub locked: String,
  pub untouched: String,
  pub left: String,
  pub more: String,
  pub joiner: String,
  pub commit: String,
  pub tag_only: String,
  pub passed_over: String,
  pub second_clone: String,
  pub no_remote: String,
  pub unread: String,
  pub refused_version: String,
  pub tagged: String,
  pub no_set: String,
  pub no_set_folder: String,
  pub unreadable_files: String,
  pub fetch_failed: String,
  pub step_failed: String,
  pub landed: String,
  pub released: String,
  pub nothing: String,
  pub cut_unit: String,
  pub cut_how: String,
  pub files: FileWords,
  pub what: WhatWords,
}

/// The words a proposal reads a commit's subject by, and what it says.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeWords {
  pub breaking: Vec<String>,
  pub major: Vec<String>,
  pub minor: Vec<String>,
  pub minor_types: Vec<String>,
  pub heading: String,
  pub since_none: String,
  pub repository: String,
  pub example: String,
  pub step: String,
  pub major_word: String,
  pub minor_word: String,
  pub patch_word: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Versions {
  pub platform_offset_minutes: i32,
  pub tag_prefix: String,
  pub nightly: String,
  pub version_file: String,
  pub release: ReleaseWords,
  pub propose: ProposeWords,
}

super::part!(pub VERSIONS: Versions = "versions.toml");
