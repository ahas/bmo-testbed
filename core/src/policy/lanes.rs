//! What the machine's turn for heavy commands says and the numbers it keeps to (docs/testbed.md), each
//! line with the `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaneWords {
  pub wide: String,
  pub narrow: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
  pub notice_seconds: u64,
  pub poll_milliseconds: u64,
  pub watch_seconds: u64,
  pub failure_lines: usize,
  pub log_keep_days: u64,
  pub narrow_divisor: usize,
  pub seats: usize,
  pub seat_free_gigabytes: u64,
  pub silence_seconds: u64,
  pub silence_grace_seconds: u64,
  pub silence_look_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryLimitNumbers {
  pub settle_seconds: u64,
  pub needed_gigabytes: u64,
  pub poll_seconds: u64,
  pub notice_seconds: u64,
  pub runs: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitingWords {
  pub line: String,
  pub nobody: String,
  pub held_by: String,
  pub between: String,
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacedWords {
  pub local: String,
  pub failed: String,
  pub unpushed: String,
  pub uncopied: String,
  pub unread: String,
  pub no_result: String,
  pub running: String,
  pub unsettled: String,
  pub unreached: String,
  pub full: String,
  pub tree_uncopied: String,
  pub file_unlaid: String,
  pub unfetched: String,
  pub broken: String,
  pub setup_failed: String,
  pub no_registry: String,
  pub stale: String,
  pub unreleased: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimesWords {
  pub line: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SilenceWords {
  pub hung: String,
  pub piped: String,
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
  pub failures: String,
  pub failures_first: String,
  pub failure: String,
  pub unstarted: String,
  pub no_log: String,
  pub dropped: String,
  pub refused: String,
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
  pub close_line: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerbWords {
  pub no_language: String,
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
  pub killed: String,
  pub times: String,
  pub exit: String,
  pub signal: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lanes {
  pub lanes: LaneWords,
  pub numbers: Numbers,
  pub memory_limits: MemoryLimitNumbers,
  pub waiting: WaitingWords,
  pub entry: EntryWords,
  pub settled: SettledWords,
  pub placed: PlacedWords,
  pub times: TimesWords,
  pub silence: SilenceWords,
  pub run: RunWords,
  pub status: StatusWords,
  pub verb: VerbWords,
  pub memory: MemoryWords,
}

super::part!(pub LANES: Lanes = "lanes.toml");
