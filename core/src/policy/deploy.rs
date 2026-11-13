//! What a deployment says where its caller reads it (docs/deploy.md), each line with the `{name}` marks
//! its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackWords {
  pub restart_refused: String,
  pub not_answering: String,
  pub died_after_start: String,
  pub step_failed: String,
  pub answers: String,
  pub silent: String,
  pub first: String,
  pub before_start: String,
  pub tree: String,
  pub bot_down: String,
  pub cause: String,
  pub reason: String,
  pub step_reason: String,
  pub console_refused: String,
  pub console_reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefusalWords {
  pub check_failed: String,
  pub suite_failed: String,
  pub files: String,
  pub failed: String,
  pub locked: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
  pub dirty_shown: usize,
  pub quoted_chars: usize,
  pub command_chars: usize,
  pub releases_told_above: usize,
  pub health_poll_ms: u64,
  pub unit_check_ms: u64,
  pub journal_lines: usize,
  pub quiet_poll_ms: u64,
  pub quiet_unanswered_asks: u32,
  pub passed_suite_hours: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunWords {
  pub refused: String,
  pub no_deploy: String,
  pub harness: String,
  pub data_relative: String,
  pub data_in_tree: String,
  pub data_missing: String,
  pub backup_in_data: String,
  pub backup_in_tree: String,
  pub env_missing: String,
  pub fetch_failed: String,
  pub no_commit: String,
  pub commit_no_deploy: String,
  pub commit_elsewhere: String,
  pub remote: String,
  pub deploying: String,
  pub again: String,
  pub major: String,
  pub unit_faults: String,
  pub dry: String,
  pub nothing_live: String,
  pub adopted: String,
  pub adopt_failed: String,
  pub gate_left_out: String,
  pub gate_failed: String,
  pub gate_check: String,
  pub gate_suite: String,
  pub suite_left_out: String,
  pub on: String,
  pub flaky: String,
  pub backup_none: String,
  pub backup_unnamed: String,
  pub backup_taken: String,
  pub backup_failed: String,
  pub prepare_failed: String,
  pub prepared: String,
  pub made: String,
  pub taken: String,
  pub setup_failed: String,
  pub build_failed: String,
  pub metadata_failed: String,
  pub place_failed: String,
  pub placed: String,
  pub before_failed: String,
  pub unrecorded: String,
  pub recorded: String,
  pub unresolved: String,
  pub flip_failed: String,
  pub flipped: String,
  pub relinked: String,
  pub dangling: String,
  pub set_aside: String,
  pub link_failed: String,
  pub linked: String,
  pub timer_failed: String,
  pub restarted: String,
  pub restart_failed: String,
  pub left_running: String,
  pub no_bot: String,
  pub stop_failed: String,
  pub before_start: String,
  pub before_start_step: String,
  pub before_start_failed: String,
  pub quiet_asking: String,
  pub quiet_live: String,
  pub quiet_forced: String,
  pub quiet_unanswered: String,
  pub quiet_failed: String,
  pub quiet_forced_unanswered: String,
  pub quiet_exit: String,
  pub quiet_no_exit: String,
  pub waiting: String,
  pub watching: String,
  pub stayed: String,
  pub retired: String,
  pub retired_plain: String,
  pub unretired: String,
  pub after_failed: String,
  pub pruned: String,
  pub kept_pinned: String,
  pub pinned_summary: String,
  pub retired_trees: String,
  pub warning: String,
  pub upload_started: String,
  pub upload_failed: String,
  pub live: String,
  pub live_alone: String,
  pub deployed_summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptWords {
  pub retired_earlier: String,
  pub a_file: String,
  pub no_worktree: String,
  pub not_built_from: String,
  pub adopting: String,
  pub elsewhere: String,
  pub not_exchanged: String,
  pub linked: String,
  pub retired: String,
  pub bin_not_taken: String,
  pub not_retired: String,
  pub not_repaired: String,
  pub submodule_not_pointed: String,
  pub not_removed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootWords {
  pub not_run: String,
  pub exit: String,
  pub exit_said: String,
  pub a_signal: String,
  pub unasked: String,
  pub unit_is: String,
  pub strain: String,
  pub unknown: String,
  pub waited: String,
  pub gave_up: String,
  pub unit_unasked: String,
  pub unit: String,
  pub process: String,
  pub machine: String,
  pub journal: String,
  pub journal_unread: String,
  pub fell: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionWords {
  pub not_placed: String,
  pub not_discarded: String,
  pub not_emptied: String,
  pub failed: String,
  pub no_addresses: String,
  pub not_pinned: String,
  pub trees_unread: String,
  pub tree_absent: String,
  pub tree_unlinked: String,
  pub target_in_use: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollingWords {
  pub before_start: String,
  pub first: String,
  pub back_to: String,
  pub would_not_stop: String,
  pub dangling: String,
  pub links_one: String,
  pub links_many: String,
  pub answers: String,
  pub silent: String,
  pub not_built: String,
  pub flip_failed: String,
  pub flipped: String,
  pub unrecorded: String,
  pub unlogged: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseWords {
  pub live_unbuilt: String,
  pub leftover: String,
  pub not_added: String,
  pub unread_processes: String,
  pub not_removed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeWords {
  pub dirty: String,
  pub more: String,
  pub no_commit: String,
  pub unpushed: String,
  pub link_gone: String,
  pub link_several: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitWords {
  pub notice: String,
  pub not_started: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveWords {
  pub inside: String,
  pub holds: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deploy {
  pub rollback: RollbackWords,
  pub refusal: RefusalWords,
  pub numbers: Numbers,
  pub run: RunWords,
  pub adopt: AdoptWords,
  pub boot: BootWords,
  pub expansion: ExpansionWords,
  pub rolling: RollingWords,
  pub release: ReleaseWords,
  pub tree: TreeWords,
  pub unit: UnitWords,
  pub remove: RemoveWords,
  pub restarts: RestartWords,
  pub console: ConsoleWords,
}

/// The console probe's words and the contract's routes, headers and statuses it asks by (docs/deploy.md,
/// 「The console probe」).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleWords {
  pub layer_package: String,
  pub address_key: String,
  pub console_token_key: String,
  pub token_key: String,
  pub layer_field: String,
  pub words_field: String,
  pub bundle_field: String,
  pub stylesheet_field: String,
  pub role_field: String,
  pub owner_key_field: String,
  pub share_reads_field: String,
  pub credential_header: String,
  pub credential: String,
  pub owner_key: String,
  pub wait_seconds: u64,
  pub answered: u16,
  pub no_role: u16,
  pub not_console: u16,
  pub probing: String,
  pub held: String,
  pub no_pin: String,
  pub no_shared: String,
  pub no_layer: String,
  pub refused: String,
  pub point_reach: String,
  pub point_release: String,
  pub point_manifest: String,
  pub point_reading: String,
  pub point_layer: String,
  pub point_words: String,
  pub point_no_role: String,
  pub point_role: String,
  pub point_own_token: String,
  pub manifest: String,
  pub words: String,
  pub module: String,
  pub stylesheet: String,
  pub signal: String,
  pub reach_failed: String,
  pub reach_unread: String,
  pub status: String,
  pub unanswered: String,
  pub not_object: String,
  pub no_release: String,
  pub reading_failed: String,
  pub no_field: String,
  pub layer_differs: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestartWords {
  pub putting: String,
  pub rolling_back: String,
}

super::part!(pub DEPLOY: Deploy = "deploy.toml");
