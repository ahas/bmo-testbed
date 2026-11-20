//! What the static check says of its steps (docs/checks.md, 「The static check」), each line with the
//! `{name}` marks its file names, and the ceilings of the fence each step runs in.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticCheck {
  pub step: String,
  pub ended: String,
  pub left_out: String,
  pub not_run: String,
  pub not_had: String,
  pub failed: String,
  pub fence_none: String,
  pub fence_unjoined: String,
  pub fence: StepFenceCeilings,
}

/// What a step's fence holds it to.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepFenceCeilings {
  pub memory_gb: u64,
  pub swap_gb: u64,
  pub tasks: u64,
}

super::part!(pub STATIC_CHECK: StaticCheck = "static-check.toml");
