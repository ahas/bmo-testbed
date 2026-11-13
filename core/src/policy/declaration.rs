//! A repository's declaration (docs/declaration.md): the file it is read from, the default each key it
//! leaves out takes, and what a declaration that does not read is refused with, each line with the
//! `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestDefaults {
  pub memory_gb: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseDefaults {
  pub max_files: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectsDefaults {
  pub check_script: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolsDefaults {
  pub manifest: String,
  pub check_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckDefaults {
  pub main: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildDefaults {
  pub cargo_profile: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployDefaults {
  pub link: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupDefaults {
  pub keep: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
  pub test: TestDefaults,
  pub database: DatabaseDefaults,
  pub projects: ProjectsDefaults,
  pub tools: ToolsDefaults,
  pub check: CheckDefaults,
  pub build: BuildDefaults,
  pub deploy: DeployDefaults,
  pub backup: BackupDefaults,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhyWords {
  pub unknown_language: String,
  pub needs_input: String,
  pub needs_data_root: String,
  pub half_database: String,
  pub no_silence: String,
  pub duplicate_step: String,
  pub no_reason: String,
  pub empty_exception: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefusalWords {
  pub unreadable: String,
  pub top_level: String,
  pub other_major: String,
  pub breaks: String,
  pub why: WhyWords,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
  pub file: String,
  pub defaults: Defaults,
  pub refusal: RefusalWords,
}

super::part!(pub DECLARATION: Declaration = "declaration.toml");
