//! What a run of the suite says (docs/suite.md), each line with the `{name}` marks its file names.

use serde::Deserialize;

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
pub struct ArgumentWords {
  pub no_value: String,
  pub not_seconds: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineWords {
  pub mark: String,
  pub turn: String,
  pub guards: String,
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
  pub cut: String,
  pub ok: String,
  pub failed: String,
  pub heading: String,
  pub no_check: String,
  pub no_tool_check: String,
  pub unread_manifest: String,
  pub held: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndingWords {
  pub summary: String,
  pub cut_files: String,
  pub failed_files: String,
  pub unstarted_files: String,
  pub verdict: String,
  pub ok: String,
  pub failed: String,
  pub not_run: String,
  pub all_passed: String,
  pub passed: String,
  pub failed_count: String,
  pub cut_count: String,
  pub unstarted_count: String,
  pub joiner: String,
  pub only_unstarted: String,
  pub unbuilt: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
  pub run: RunWords,
  pub reasons: ReasonWords,
  pub selection: SelectionWords,
  pub arguments: ArgumentWords,
  pub lines: LineWords,
  pub file: FileWords,
  pub ending: EndingWords,
}

super::part!(pub SUITE: Suite = "suite.toml");
