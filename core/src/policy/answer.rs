//! What every verb's answer is held to (docs/boundary.md): its ceiling in characters, and the lines saying
//! what a cut left out.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
  pub ceiling_chars: usize,
  pub cut: String,
  pub alone: String,
}

super::part!(pub ANSWER: Answer = "answer.toml");
