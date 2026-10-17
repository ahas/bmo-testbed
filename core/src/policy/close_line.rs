//! What the repository's close line says and the number it keeps to (docs/testbed.md, 「The close
//! line」), each line with the `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineWords {
  pub waiting: String,
  pub nobody: String,
  pub held_by: String,
  pub joiner: String,
  pub pid: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineNumbers {
  pub notice_seconds: u64,
  pub lock_name_chars: usize,
  pub lock_digest_chars: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StretchWords {
  pub command: String,
  pub front_first: String,
  pub held: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseLine {
  pub line: LineWords,
  pub numbers: LineNumbers,
  pub stretch: StretchWords,
}

super::part!(pub CLOSE_LINE: CloseLine = "close-line.toml");
