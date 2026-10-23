//! What a run of the backup came to (docs/backup.md): each step that failed, at which destination or
//! database, and what was said, carried by the run's event.

use serde::Serialize;
use serde_json::{Value, json};

/// One step that failed: the step, what it was for, and what was said.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Failure {
  pub step: String,
  pub target: String,
  pub said: String,
}

/// The steps that failed in one run, in the order they failed.
#[derive(Debug, Default)]
pub struct Outcome {
  pub failed: Vec<Failure>,
}

impl Outcome {
  pub fn fail(&mut self, step: &str, target: &str, said: impl Into<String>) {
    self.failed.push(Failure {
      step: step.to_owned(),
      target: target.to_owned(),
      said: said.into(),
    });
  }

  pub fn passed(&self) -> bool {
    self.failed.is_empty()
  }

  /// The event's detail: the verb and each failure.
  pub fn detail(&self, verb: &str) -> Value {
    json!({ "verb": verb, "failures": self.failed })
  }
}
