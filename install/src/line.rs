//! One step's line (docs/install.md, 「What it prints」): the step, its status, one English sentence of detail
//! and what the agent does next.

use serde::Serialize;

/// How a step stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
  /// Done, now or before.
  Ok,
  /// A dry run's step that would act.
  Planned,
  /// A value is missing.
  NeedsInput,
  /// It could not be done.
  Failed,
}

/// One printed line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Line {
  pub step: String,
  pub status: Status,
  pub detail: String,
  pub next: String,
}

impl Line {
  pub fn new(
    step: impl Into<String>,
    status: Status,
    detail: impl Into<String>,
    next: impl Into<String>,
  ) -> Self {
    Self {
      step: step.into(),
      status,
      detail: detail.into(),
      next: next.into(),
    }
  }

  /// The line as it is printed: one JSON object.
  pub fn json(&self) -> String {
    serde_json::to_string(self).unwrap_or_default()
  }
}
