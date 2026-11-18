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
  pub panic_lines: usize,
  pub log_keep_days: u64,
  pub narrow_divisor: usize,
  pub seats: usize,
  pub seat_free_gigabytes: u64,
  pub silence_seconds: u64,
  pub silence_grace_seconds: u64,
  pub silence_look_seconds: u64,
  pub hung_named_processes: usize,
  pub hung_process_chars: usize,
  pub unreached_pause_seconds: u64,
  pub unreached_wait_minutes: u64,
  pub placed_lock_seconds: u64,
  pub status_connect_seconds: u64,
  pub placed_standing_seconds: u64,
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
  pub still_running: String,
  pub unsettled: String,
  pub negative_floor: String,
  pub here: String,
  pub order_empty: String,
  pub order_twice: String,
  pub order_unknown: String,
  pub unreached: String,
  pub full: String,
  pub tree_uncopied: String,
  pub file_unlaid: String,
  pub unfetched: String,
  pub broken: String,
  pub broke: String,
  pub ran_there: String,
  pub stale_removed: String,
  pub stale_unremoved: String,
  pub trees_gone: String,
  pub trees_unread: String,
  pub setup_failed: String,
  pub no_registry: String,
  pub no_dir: String,
  pub stale: String,
  pub unreleased: String,
  pub testbed_unreached: String,
  pub testbed_unwritten: String,
  pub testbed_unread: String,
  pub testbed_other: String,
  pub testbed_unrun: String,
  pub lock_unheld: String,
  pub brought_live: String,
  pub unbrought: String,
  pub live_uncarried: String,
  pub release_failed: String,
  pub unreached_waiting: String,
  pub unreached_gave_up: String,
  pub unreachable_event: String,
  pub recovered_event: String,
  pub release_failed_event: String,
  pub kept: String,
  pub ignored_dir: String,
  pub set_one_checkout: String,
  pub set_no_machine: String,
  pub set_kept: String,
  pub turn_came: String,
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
  pub standing: String,
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
  pub placed: String,
  pub placed_waiting: String,
  pub placed_running: String,
  pub machine: String,
  pub machine_current: String,
  pub machine_behind: String,
  pub machine_unreleased: String,
  pub machine_uncompared: String,
  pub machine_unreached: String,
  pub machine_outage: String,
  pub close_line: String,
  pub deploy_line: String,
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
