//! The test programs a build made, read from Cargo's JSON report (docs/suite.md, 「Heavy units run in a lane of
//! one」).

use std::path::{Path, PathBuf};

use bmo_testbed_suite::test_programs::TestPrograms;

// One line of Cargo's report of an artifact: the package's manifest, whether it was built as a test, and the
// program it made, where it made one.
fn artifact(manifest: &Path, test: bool, executable: Option<&Path>) -> String {
  serde_json::json!({
    "reason": "compiler-artifact",
    "manifest_path": manifest,
    "profile": { "test": test },
    "executable": executable,
  })
  .to_string()
}

// "Its programs are read from what Cargo reported of the build: each test program of the crate's package"
#[test]
fn each_package_s_test_programs_are_read_from_the_report_and_nothing_else() {
  let dir = tempfile::tempdir().unwrap();
  let one = dir.path().join("one");
  let other = dir.path().join("other");
  let unit = PathBuf::from("/build/deps/one-unit");
  let harness = PathBuf::from("/build/deps/main-harness");
  let report = [
    artifact(&one.join("Cargo.toml"), false, None),
    artifact(&one.join("Cargo.toml"), false, Some(Path::new("/build/one-binary"))),
    artifact(&one.join("Cargo.toml"), true, Some(&unit)),
    "   Compiling one v0.1.0".to_owned(),
    serde_json::json!({ "reason": "build-finished", "success": true }).to_string(),
    artifact(&one.join("Cargo.toml"), true, Some(&harness)),
    artifact(&one.join("Cargo.toml"), true, Some(&unit)),
  ]
  .join("\n");
  let programs = TestPrograms::reported(&report);

  assert_eq!(programs.of(&one), [unit, harness]);
  assert!(programs.of(&other).is_empty());
}
