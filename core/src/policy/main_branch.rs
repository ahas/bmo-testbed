//! A repository's main branch and its head (docs/declaration.md, 「The main branch's head」), each line with the
//! `{name}` marks its file names.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MainBranchPolicy {
  pub remote: String,
  pub stall_seconds: u64,
  pub absent: String,
  pub parted: String,
}

super::part!(pub MAIN_BRANCH: MainBranchPolicy = "main-branch.toml");
