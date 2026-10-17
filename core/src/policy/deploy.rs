//! What a deployment says where its caller reads it (docs/deploy.md), each line with the `{name}` marks
//! its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackWords {
  pub restart_refused: String,
  pub not_answering: String,
  pub died_after_start: String,
  pub answers: String,
  pub silent: String,
  pub first: String,
  pub before_start: String,
  pub tree: String,
  pub bot_down: String,
  pub cause: String,
  pub reason: String,
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
  pub health_ask_ms: u64,
  pub health_poll_ms: u64,
  pub unit_check_ms: u64,
  pub stay_poll_ms: u64,
  pub journal_lines: usize,
  pub step_poll_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunWords {
  pub refused: String,
  pub no_deploy: String,
  pub job: String,
  pub data_relative: String,
  pub data_in_tree: String,
  pub data_missing: String,
  pub backup_in_data: String,
  pub backup_in_tree: String,
  pub env_missing: String,
  pub fetch_failed: String,
  pub no_commit: String,
  pub remote: String,
  pub deploying: String,
  pub again: String,
  pub unit_faults: String,
  pub dry: String,
  pub nothing_live: String,
  pub adopted: String,
  pub adopt_failed: String,
  pub gate_left_out: String,
  pub gate_failed: String,
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
  pub waiting: String,
  pub watching: String,
  pub stayed: String,
  pub retired: String,
  pub retired_plain: String,
  pub unretired: String,
  pub after_failed: String,
  pub after_overran: String,
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
pub struct Deploy {
  pub rollback: RollbackWords,
  pub refusal: RefusalWords,
  pub numbers: Numbers,
  pub run: RunWords,
}

super::part!(pub DEPLOY: Deploy = "deploy.toml");
