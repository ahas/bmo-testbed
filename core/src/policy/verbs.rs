//! The binary's own words (docs/boundary.md, 「The binary」): the usage each verb prints and what each verb
//! says of itself, each line with the `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageWords {
  pub binary: String,
  pub run: String,
  pub status: String,
  pub classify: String,
  pub suite: String,
  pub count: String,
  pub log: String,
  pub template: String,
  pub check: String,
  pub imports: String,
  pub cargo: String,
  pub prune: String,
  pub full: String,
  pub deploy: String,
  pub backups: String,
  pub backup: String,
  pub serve: String,
  pub mcp: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueWords {
  pub not_named: String,
  pub not_a_time: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupsWords {
  pub none_declared: String,
  pub local_only: String,
  pub holder: String,
  pub wait_seconds: u64,
  pub locked: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifyWords {
  pub light: String,
  pub heavy: String,
  pub instead: String,
  pub unread: String,
  pub depth_most: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnWords {
  pub unchecked: String,
  pub untouched: String,
  pub unstarted: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogWords {
  pub unstarted_divisor: usize,
  pub missing: String,
  pub no_summary: String,
  pub failures: String,
  pub unstarted: String,
  pub item: String,
  pub count: String,
  pub joiner: String,
  pub unit: String,
  pub how: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckWords {
  pub unread: String,
  pub server_down: String,
  pub not_enabled: String,
  pub settled: String,
  pub unsettled: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashesWords {
  pub file: String,
  pub before: String,
  pub after: String,
  pub none: String,
  pub written: String,
  pub can: String,
  pub missing: String,
  pub unreadable: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportsWords {
  pub to: String,
  pub missing: String,
  pub ambiguous: String,
  pub clean: String,
  pub not_files: String,
  pub not_checkout: String,
  pub refused: String,
  pub nothing_written: String,
  pub none_written: String,
  pub to_fix: String,
  pub fixed: String,
  pub unit: String,
  pub how: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PruneWords {
  pub gone: String,
  pub noatime: String,
  pub too_short: String,
  pub no_hours: String,
  pub locked: String,
  pub everything: String,
  pub spans: String,
  pub heading: String,
  pub removed: String,
  pub dry: String,
  pub counts: String,
  pub named: String,
  pub cleared: String,
  pub bytes: String,
  pub free: String,
  pub held: String,
  pub gigabytes: String,
  pub over: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verbs {
  pub usage: UsageWords,
  pub classify: ClassifyWords,
  pub own: OwnWords,
  pub log: LogWords,
  pub check: CheckWords,
  pub dashes: DashesWords,
  pub imports: ImportsWords,
  pub words: ValueWords,
  pub backups: BackupsWords,
  pub prune: PruneWords,
}

super::part!(pub VERBS: Verbs = "verbs.toml");
