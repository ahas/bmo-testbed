//! The set's versions (docs/deploy.md, 「The deployed version」): the platform's day, the release tag, the
//! nightly's pre-release, the words a release tag's version carries, and the variable a build is handed it in.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Versions {
  pub platform_offset_minutes: i32,
  pub tag_prefix: String,
  pub nightly: String,
  pub alpha: String,
  pub releases: Vec<String>,
  pub first_major: u64,
  pub variable: String,
}

super::part!(pub VERSIONS: Versions = "versions.toml");
