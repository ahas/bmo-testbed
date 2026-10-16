//! What a deployment says where its caller reads it (docs/deploy.md), each line with the `{name}` marks
//! its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackWords {
  pub restart_refused: String,
  pub not_answering: String,
  pub died_after_start: String,
  pub untouched: String,
  pub answers: String,
  pub silent: String,
  pub first: String,
  pub before_start: String,
  pub tree: String,
  pub bot_down: String,
  pub cause: String,
  pub output: String,
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
pub struct Deploy {
  pub rollback: RollbackWords,
  pub refusal: RefusalWords,
}

super::part!(pub DEPLOY: Deploy = "deploy.toml");
