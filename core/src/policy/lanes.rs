//! What the machine's turn for heavy commands says (docs/testbed.md), each line with the `{name}` marks
//! its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaneWords {
  pub wide: String,
  pub narrow: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitingWords {
  pub line: String,
  pub nobody: String,
  pub held_by: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryWords {
  pub joiner: String,
  pub pid: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettledWords {
  pub inside: String,
  pub own: String,
  pub queued: String,
  pub nobody: String,
  pub kept: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacedWords {
  pub local: String,
  pub failed: String,
  pub unpushed: String,
  pub uncopied: String,
  pub running: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimesWords {
  pub line: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunWords {
  pub not_run: String,
  pub line: String,
  pub ok: String,
  pub failed: String,
  pub counts: String,
  pub exit: String,
  pub by_signal: String,
  pub cut: String,
  pub failures: String,
  pub failures_first: String,
  pub failure: String,
  pub unstarted: String,
  pub no_log: String,
  pub dropped: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusWords {
  pub lane: String,
  pub holding: String,
  pub nobody: String,
  pub waiting: String,
  pub age: String,
  pub front: String,
  pub next: String,
  pub kept: String,
  pub close_line: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerbWords {
  pub usage: String,
  pub hint: String,
  pub takes_value: String,
  pub seconds: String,
  pub flag_as_command: String,
  pub no_command: String,
  pub no_language: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefusalWords {
  pub heavy: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryWords {
  pub mark: String,
  pub not_run: String,
  pub took: String,
  pub unread: String,
  pub freed: String,
  pub short: String,
  pub fenced: String,
  pub spent: String,
  pub again: String,
  pub times: String,
  pub exit: String,
  pub signal: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lanes {
  pub lanes: LaneWords,
  pub waiting: WaitingWords,
  pub entry: EntryWords,
  pub settled: SettledWords,
  pub placed: PlacedWords,
  pub times: TimesWords,
  pub run: RunWords,
  pub status: StatusWords,
  pub verb: VerbWords,
  pub refusal: RefusalWords,
  pub memory: MemoryWords,
}

super::part!(pub LANES: Lanes = "lanes.toml");
