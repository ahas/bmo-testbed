//! The HTTP service's address, its readings' pace and its sentences (docs/boundary.md, 「The HTTP
//! service」).

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceErrors {
  pub unauthorized: String,
  pub no_route: String,
  pub not_found: String,
  pub bad_query: String,
  pub unknown_kind: String,
  pub gone: String,
  pub not_ready: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceLines {
  pub listening: String,
  pub no_token: String,
  pub unbound: String,
  pub no_record: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServicePolicy {
  pub listen: String,
  pub queue_poll_milliseconds: u64,
  pub event_poll_milliseconds: u64,
  pub keep_alive_seconds: u64,
  pub errors: ServiceErrors,
  pub lines: ServiceLines,
}

super::part!(pub SERVICE: ServicePolicy = "service.toml");
