//! What a run of the suite says (docs/suite.md), each line with the `{name}` marks its file names, and the
//! numbers it and the change check hold to.

use serde::Deserialize;

/// The numbers a run and the change check hold to.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
  pub build_job_tenths_gb: u64,
  pub fence_swap_gb: u64,
  pub fence_tasks: u64,
  pub fence_move_poll_ms: u64,
  pub fence_move_tries: u32,
  pub server_start_poll_ms: u64,
  pub check_shown_lines: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunWords {
  pub whole: String,
  pub selected: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasonWords {
  pub owner: String,
  pub root: String,
  pub reads: String,
  pub unclaimed: String,
  pub runs: String,
  pub runs_loading: String,
  pub dependent: String,
  pub always: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionWords {
  pub heading: String,
  pub unmarked: String,
  pub marked: String,
  pub more: String,
  pub project_unmarked: String,
  pub project_marked: String,
  pub tool_unmarked: String,
  pub tool_marked: String,
  pub crate_unmarked: String,
  pub crate_marked: String,
  pub changed: String,
  pub no_packages: String,
  pub not_a_range: String,
  pub no_crates: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateWords {
  pub superseded_kept_seconds: u64,
  pub bad_name: String,
  pub no_migrate: String,
  pub migrate_failed: String,
  pub failed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgainWords {
  pub once_more: String,
  pub either: String,
  pub joiner: String,
  pub flaky_summary: String,
  pub unrecorded: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineWords {
  pub mark: String,
  pub turn: String,
  pub cargo_jobs: String,
  pub fence_none: String,
  pub fence_held: String,
  pub fence_refused: String,
  pub fence_not_moved: String,
  pub fence_killed: String,
  pub process: String,
  pub processes: String,
  pub gone: String,
  pub no_runtime: String,
  pub no_socket: String,
  pub server_down: String,
  pub on_postgres: String,
  pub template_stands: String,
  pub template_failed: String,
  pub building_tests: String,
  pub building: String,
  pub tests: String,
  pub library_tests: String,
  pub binaries: String,
  pub plan: String,
  pub no_browser: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileWords {
  pub ok: String,
  pub failed: String,
  pub heading: String,
  pub signalled: String,
  pub silent: String,
  pub test_overran: String,
  pub no_check: String,
  pub no_runner: String,
  pub tests_step: String,
  pub no_tool_check: String,
  pub unread_manifest: String,
  pub unbuilt_crate: String,
}

/// What the crate-tests verb says (docs/boundary.md, 「crate-tests」).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrateTestsWords {
  pub running: String,
  pub none: String,
  pub unread: String,
  pub unwritten: String,
  pub unstarted: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndingWords {
  pub summary: String,
  pub failed_files: String,
  pub verdict: String,
  pub ok: String,
  pub failed: String,
  pub not_run: String,
  pub all_passed: String,
  pub passed: String,
  pub failed_count: String,
  pub skipped_count: String,
  pub joiner: String,
  pub unbuilt: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckWords {
  pub heading: String,
  pub since: String,
  pub given: String,
  #[serde(rename = "where")]
  pub where_it_runs: String,
  pub step: String,
  pub ok: String,
  pub failed: String,
  pub logged: String,
  pub log: String,
  pub no_turn: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CargoWords {
  pub not_removed: String,
  pub unasked: String,
  pub no_code: String,
  pub locked: String,
  pub removed: String,
  pub unstarted: String,
  pub path_aligned: String,
  pub path_moved: String,
  pub path_absent: String,
}

/// What no test unit inherits: the deployment's own variables, which name the live data rather than the
/// test's (docs/suite.md, 「What a test unit is handed」).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
  pub stripped: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
  pub numbers: Numbers,
  pub environment: Environment,
  pub run: RunWords,
  pub reasons: ReasonWords,
  pub selection: SelectionWords,
  pub template: TemplateWords,
  pub again: AgainWords,
  pub lines: LineWords,
  pub file: FileWords,
  pub crate_tests: CrateTestsWords,
  pub ending: EndingWords,
  pub check: CheckWords,
  pub cargo: CargoWords,
}

super::part!(pub SUITE: Suite = "suite.toml");
