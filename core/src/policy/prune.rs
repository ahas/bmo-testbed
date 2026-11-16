//! The numbers of the prune of the shared Cargo build directory (docs/deploy.md, 「The Cargo build
//! directory」): its spans, its ceiling, the trim run after it and the placed machine's disk floor.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prune {
  pub older_than_days: i64,
  pub relatime_days: i64,
  pub incremental_hours: i64,
  pub own_grace_hours: i64,
  pub deploy_build_days: i64,
  pub ceiling_gigabytes: u64,
  pub ceiling_room_gigabytes: u64,
  pub deployment_ceiling_gigabytes: u64,
  pub trim_command: Vec<String>,
  pub placed_disk_free_gigabytes: f64,
}

super::part!(pub PRUNE: Prune = "prune.toml");
