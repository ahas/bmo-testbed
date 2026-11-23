//! A unit's state as the supervisor keeps it and the stand-ins show it (docs/supervise.md, 「What it keeps」),
//! in systemd's own words, so a reader of `systemctl show` reads the supervisor as it reads systemd.

use serde::{Deserialize, Serialize};

/// Whether a bot unit's repository answers its declared health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Health {
  /// Not asked: the unit declares none, or does not run.
  #[default]
  Unknown,
  Healthy,
  Unhealthy,
}

/// One unit's state, its keys named as systemd names its properties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitState {
  pub id: String,
  pub description: String,
  /// `loaded`, `not-found` or `error`.
  pub load_state: String,
  /// `active`, `activating`, `deactivating`, `inactive` or `failed`.
  pub active_state: String,
  /// `running`, `exited`, `dead`, `failed`, `start-pre`, `start`, `auto-restart`, `stop-sigterm` or `waiting`.
  pub sub_state: String,
  /// `success`, `exit-code`, `signal`, `core-dump`, `start-limit-hit`, `resources` or `condition`.
  pub result: String,
  /// The main process's id, 0 where none runs.
  #[serde(rename = "mainPID")]
  pub main_pid: u32,
  /// How many times the supervisor started the unit again after its process ended.
  pub n_restarts: u32,
  /// The main process's last exit code, or the signal that ended it.
  pub exec_main_status: i32,
  /// `enabled`, `disabled` or `transient`.
  pub unit_file_state: String,
  /// The deploy mark the unit carries, empty where it carries none.
  pub mark: String,
  pub health: Health,
  /// When the unit last became active, in epoch milliseconds, 0 where it never did.
  pub active_since_ms: i64,
  /// The keys the unit names that the supervisor reads and does not apply.
  pub not_applied: Vec<String>,
}

impl UnitState {
  /// A unit loaded and never started.
  pub fn loaded(id: &str, description: &str, enabled: bool) -> Self {
    Self {
      id: id.to_owned(),
      description: description.to_owned(),
      load_state: "loaded".to_owned(),
      active_state: "inactive".to_owned(),
      sub_state: "dead".to_owned(),
      result: "success".to_owned(),
      main_pid: 0,
      n_restarts: 0,
      exec_main_status: 0,
      unit_file_state: if enabled { "enabled" } else { "disabled" }.to_owned(),
      mark: String::new(),
      health: Health::Unknown,
      active_since_ms: 0,
      not_applied: Vec::new(),
    }
  }

  /// A unit the unit directory does not hold.
  pub fn not_found(id: &str) -> Self {
    Self {
      load_state: "not-found".to_owned(),
      unit_file_state: String::new(),
      ..Self::loaded(id, "", false)
    }
  }

  pub fn set(&mut self, active: &str, sub: &str) {
    self.active_state = active.to_owned();
    self.sub_state = sub.to_owned();
  }

  pub fn is_active(&self) -> bool {
    matches!(self.active_state.as_str(), "active" | "activating" | "deactivating")
  }

  /// The unit's properties as `systemctl show` prints them, in its own names.
  pub fn properties(&self) -> Vec<(&'static str, String)> {
    vec![
      ("Id", self.id.clone()),
      ("Description", self.description.clone()),
      ("LoadState", self.load_state.clone()),
      ("ActiveState", self.active_state.clone()),
      ("SubState", self.sub_state.clone()),
      ("Result", self.result.clone()),
      ("MainPID", self.main_pid.to_string()),
      ("NRestarts", self.n_restarts.to_string()),
      ("ExecMainStatus", self.exec_main_status.to_string()),
      ("UnitFileState", self.unit_file_state.clone()),
    ]
  }
}

/// What the supervisor writes into its state file: every unit's state, and when each timer last fired.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
  /// The supervisor's process, 0 once it has ended.
  pub pid: u32,
  pub units: Vec<UnitState>,
  /// Each timer's last elapse, in epoch milliseconds, which a persistent timer reads as the supervisor starts.
  pub fired: Vec<(String, i64)>,
}
