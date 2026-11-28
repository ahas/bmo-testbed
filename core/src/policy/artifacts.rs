//! What `pack` and a deployment from an artifact go by (docs/artifacts.md).

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifacts {
  pub floor: Floor,
  pub keep: Keep,
  pub refused: Refused,
  pub runtimes: BTreeMap<String, Runtime>,
  pub signing: Signing,
  pub said: Said,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signing {
  pub public_key: String,
  pub allow_unsigned: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Floor {
  pub glibc: String,
  pub arches: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keep {
  pub always: Vec<String>,
  pub never: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refused {
  pub words: Vec<String>,
}

/// One runtime an artifact may carry: its version, where its release stands, the digest it is checked against,
/// and the binary's path inside the release's archive.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
  pub version: String,
  pub url: String,
  pub sha256: String,
  pub binary: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Said {
  pub no_table: String,
  pub building_step: String,
  pub above_floor: String,
  pub unknown_runtime: String,
  pub runtime_failed: String,
  pub built: String,
  pub build_failed: String,
  pub no_manifest: String,
  pub digest_differs: String,
  pub unsigned: String,
  pub bad_signature: String,
  pub floor_above: String,
  pub other_repository: String,
  pub env_missing: String,
  pub laid: String,
  pub step_failed: String,
  pub flipped: String,
  pub unit_linked: String,
  pub restarted: String,
  pub health_failed: String,
  pub rolled_back: String,
  pub live: String,
  pub planned: String,
}

super::part!(pub ARTIFACTS: Artifacts = "artifacts.toml");
