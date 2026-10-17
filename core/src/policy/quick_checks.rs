//! What the quick checks say and the fixed words they read the tree by (docs/checks.md, 「The quick
//! checks」), each line with the `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuickChecks {
  pub head: String,
  pub to_fix: String,
  pub named: String,
  pub no_input: String,
  pub unreadable: String,
  pub wording: Wording,
  pub dashes: Dashes,
  pub korean: Korean,
  pub layout: Layout,
  pub dependencies: Dependencies,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wording {
  pub clean: String,
  pub hit: String,
  pub stale: String,
  pub not_markdown: String,
  pub unknown_language: String,
  pub own_sense: String,
  pub banned_section: String,
  pub own_sense_section: String,
  pub allowed_section: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dashes {
  pub clean: String,
  pub hit: String,
  pub excerpt_reach: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Korean {
  pub clean: String,
  pub below: String,
  pub over: String,
  pub not_ledger: String,
  pub areas: String,
  pub files: String,
  pub row: String,
  pub kind: String,
  pub cut: String,
  pub code: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
  pub clean: String,
  pub readme: String,
  pub table_head: String,
  pub not_markdown: String,
  pub no_table: String,
  pub unnamed: String,
  pub gone: String,
  pub language: String,
  pub not_member: String,
  pub no_manifest: String,
  pub levels: usize,
  pub languages: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependencies {
  pub clean: String,
  pub foreign: String,
  pub foreign_dev: String,
  pub test_kit: Vec<String>,
}

super::part!(pub QUICK_CHECKS: QuickChecks = "quick-checks.toml");
