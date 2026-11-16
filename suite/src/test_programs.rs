//! The test programs one Cargo build of the crates tested made, read from Cargo's own JSON report of that build
//! (docs/suite.md, 「Heavy units run in a lane of one」), so a crate's unit runs them without compiling again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::crates::resolved;

// The fields of one line of Cargo's report a test program is read from: an artifact built, the manifest of the
// package it is for, whether it was built as a test, and the program it made.
#[derive(Deserialize)]
struct Reported {
  reason: String,
  #[serde(default)]
  manifest_path: Option<PathBuf>,
  #[serde(default)]
  profile: Option<ReportedProfile>,
  #[serde(default)]
  executable: Option<PathBuf>,
}

#[derive(Deserialize)]
struct ReportedProfile {
  test: bool,
}

/// The test programs a build made, by the folder of the package each is for, each package's in the order Cargo
/// reported them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestPrograms(BTreeMap<PathBuf, Vec<PathBuf>>);

impl TestPrograms {
  /// The test programs Cargo's report names: every artifact built as a test that made a program. A line that is
  /// not one of Cargo's JSON messages is passed over.
  pub fn reported(report: &str) -> Self {
    let mut programs: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();

    for line in report.lines() {
      let Ok(reported) = serde_json::from_str::<Reported>(line) else { continue };

      if reported.reason != "compiler-artifact" || !reported.profile.is_some_and(|profile| profile.test) {
        continue;
      }

      let (Some(manifest), Some(program)) = (reported.manifest_path, reported.executable) else {
        continue;
      };
      let Some(folder) = manifest.parent() else { continue };
      let made = programs.entry(resolved(folder)).or_default();

      if !made.contains(&program) {
        made.push(program);
      }
    }

    Self(programs)
  }

  /// The test programs of the package standing in `folder`; none where the build made none for it.
  pub fn of(&self, folder: &Path) -> &[PathBuf] {
    self.0.get(&resolved(folder)).map_or(&[], Vec::as_slice)
  }
}
