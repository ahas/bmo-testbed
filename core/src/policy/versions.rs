//! The set's versions (docs/deploy.md, 「The deployed version」): the platform's day, the release tag, the
//! nightly's pre-release, the alpha's word, and the files a tree declares its version in.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Versions {
  pub platform_offset_minutes: i32,
  pub tag_prefix: String,
  pub nightly: String,
  pub alpha: String,
  pub version_file: String,
  pub cargo_manifest: String,
  pub package_manifest: String,
}

super::part!(pub VERSIONS: Versions = "versions.toml");
