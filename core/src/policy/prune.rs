//! The numbers of the prune of the shared Cargo build directory (docs/deploy.md, 「The Cargo build
//! directory」): its spans, its ceiling and its wait for the build locks.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prune {
  pub older_than_days: i64,
  pub relatime_days: i64,
  pub incremental_hours: i64,
  pub own_grace_hours: i64,
  pub shared_ceiling_bytes: u64,
  pub checkout_ceiling_bytes: u64,
}

super::part!(pub PRUNE: Prune = "prune.toml");
