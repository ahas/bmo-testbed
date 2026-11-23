//! What the supervisor says and the numbers it keeps (docs/supervise.md): its usage, the stand-ins' answers,
//! the defaults systemd gives a unit that names none, and its own paces.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Supervise {
  pub usage: String,
  pub numbers: Numbers,
  pub places: Places,
  pub said: Said,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
  pub default_restart_seconds: f64,
  pub default_stop_seconds: u64,
  pub default_start_limit_seconds: u64,
  pub default_start_limit_burst: u32,
  pub health_every_seconds: u64,
  pub health_bound_seconds: u64,
  pub log_cap_bytes: u64,
  pub journal_lines: usize,
  pub follow_poll_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Places {
  pub folder: String,
  pub socket: String,
  pub state: String,
  pub lock: String,
  pub logs: String,
  pub stand_ins: String,
  pub runtime: String,
  pub variable: String,
  pub names: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Said {
  pub started: String,
  pub stopping: String,
  pub ended: String,
  pub systemd_stands: String,
  pub held: String,
  pub no_socket: String,
  pub unreachable: String,
  pub not_found: String,
  pub unreadable: String,
  pub not_applied: String,
  pub start_failed: String,
  pub start_limit: String,
  pub no_command: String,
  pub no_verb: String,
  pub transient_exists: String,
  pub enable_link: String,
  pub disable_link: String,
  pub words_refused: String,
}

super::part!(pub SUPERVISE: Supervise = "supervise.toml");
