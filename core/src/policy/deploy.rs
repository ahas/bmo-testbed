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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deploy {
  pub rollback: RollbackWords,
  pub refusal: RefusalWords,
  pub numbers: Numbers,
}

super::part!(pub DEPLOY: Deploy = "deploy.toml");
