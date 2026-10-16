//! What every verb's answer is held to (docs/boundary.md): its ceiling in characters.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
  pub ceiling_chars: usize,
}

super::part!(pub ANSWER: Answer = "answer.toml");
