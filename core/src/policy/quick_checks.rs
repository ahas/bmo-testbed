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
  pub language: LanguageWords,
  pub cargo_profile: CargoProfileWords,
  pub shared_versions: SharedVersionsWords,
  pub layout: Layout,
  pub dependencies: Dependencies,
  pub personal_data: PersonalDataWords,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalDataWords {
  pub clean: String,
  pub hit: String,
  pub mail_kind: String,
  pub home_kind: String,
  pub telephone_kind: String,
  pub network_kind: String,
  pub documentation_domains: Vec<String>,
  pub documentation_suffixes: Vec<String>,
  pub neutral_homes: Vec<String>,
  pub unit_suffixes: Vec<String>,
  pub reserved_blocks: Vec<[u8; 2]>,
  pub forms: Vec<PersonalDataForm>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalDataForm {
  pub kind: String,
  pub pattern: String,
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
pub struct LanguageWords {
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedVersionsWords {
  pub file: String,
  pub lines: String,
  pub snapshot: String,
  pub packages: String,
  pub compatible: String,
  pub compatible_unpinned: String,
  pub package_manifest: String,
  pub package_lists: Vec<String>,
  pub file_prefix: String,
  pub lock: String,
  pub registry_sources: Vec<String>,
  pub clean: String,
  pub unread_registry: String,
  pub no_checkout: String,
  pub two_checkouts: String,
  pub not_shared: String,
  pub not_on_main: String,
  pub not_manifest: String,
  pub not_lock: String,
  pub not_package_manifest: String,
  pub not_object: String,
  pub package: String,
  pub package_version: String,
  pub package_other_file: String,
  pub package_missing: String,
  pub package_other_bytes: String,
  pub range_joiner: String,
  pub line: String,
  pub inherited: String,
  pub inherit: String,
  pub inherit_optional: String,
  pub inherit_keys: Vec<String>,
  pub locked: String,
  pub not_listed: String,
  pub same_line: String,
  pub not_locked: String,
  pub bad_line: String,
  pub report_clean: String,
  pub report_differs: String,
  pub report_lines: String,
  pub report_locked: String,
  pub report_version: String,
  pub report_others: String,
  pub report_joiner: String,
  pub report_list_joiner: String,
  pub report_unread: String,
  pub report_package: String,
  pub report_missing_file: String,
  pub report_other_bytes: String,
  pub report_total: String,
  pub report_unit: String,
  pub report_how: String,
}

super::part!(pub QUICK_CHECKS: QuickChecks = "quick-checks.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CargoProfileWords {
  pub manifest: String,
  pub clean: String,
  pub differs: String,
  pub missing: String,
  pub not_manifest: String,
  /// Each profile's settings by key, as the manifest writes them.
  pub wanted: std::collections::BTreeMap<String, std::collections::BTreeMap<String, toml::Value>>,
}
