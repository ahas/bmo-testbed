//! Which of a crate's tests a changed test file owes (docs/checks.md, 「The change check」): a target's own
//! source its target, a harness's module its filter, and a shared module, a fixture or any other path the
//! crate's tests whole; a Rust test file read for its tests, the module paths it walks and its `#[path]` files.

use std::path::Path;

use bmo_testbed_suite::crates::{CrateDeclaration, TestTarget};
use bmo_testbed_suite::test_targets::{TargetTests, owed_tests, read_test_file};

fn write(root: &Path, file: &str, text: &str) {
  let path = root.join(file);

  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
}

fn target(name: &str, path: &str) -> TestTarget {
  TestTarget {
    name: name.to_owned(),
    path: path.to_owned(),
  }
}

// A crate in `lab/` holding both layouts: a harness at `tests/main.rs` with a module of tests, a shared helper
// and a module using it; a test target of a file of its own; a target with a process to itself reaching a
// module of the harness by `#[path]`; and a fixture.
fn lab() -> (tempfile::TempDir, CrateDeclaration) {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path();

  for (path, text) in [
    ("lab/src/lib.rs", "pub fn one() -> u8 {\n  1\n}\n"),
    ("lab/tests/main.rs", "mod alpha;\nmod beta;\nmod migrators;\nmod support;\n"),
    ("lab/tests/alpha.rs", "#[test]\nfn one() {\n  assert_eq!(lab::one(), 1);\n}\n"),
    ("lab/tests/support.rs", "pub fn two() -> u8 {\n  2\n}\n"),
    (
      "lab/tests/beta.rs",
      "#[tokio::test]\nasync fn two() {\n  assert_eq!(crate::support::two(), 2);\n}\n",
    ),
    ("lab/tests/migrators.rs", "#[test]\nfn kept() {}\n\npub struct Kept;\n"),
    ("lab/tests/solo.rs", "#[test]\nfn alone() {}\n"),
    (
      "lab/tests/sweep/main.rs",
      "#[path = \"../migrators.rs\"]\nmod migrators;\n\n#[test]\nfn swept() {}\n",
    ),
    ("lab/tests/fixtures/case.json", "{}\n"),
  ] {
    write(root, path, text);
  }

  let krate = CrateDeclaration {
    dir: "lab".to_owned(),
    name: "lab".to_owned(),
    library: true,
    tests: vec![
      target("main", "lab/tests/main.rs"),
      target("solo", "lab/tests/solo.rs"),
      target("sweep", "lab/tests/sweep/main.rs"),
    ],
    ..CrateDeclaration::default()
  };

  (dir, krate)
}

fn owed(target: &str, filters: &[&str]) -> TargetTests {
  TargetTests {
    krate: "lab".to_owned(),
    target: target.to_owned(),
    filters: filters.iter().map(|filter| (*filter).to_owned()).collect(),
  }
}

fn changed(paths: &[&str]) -> Vec<String> {
  paths.iter().map(|path| (*path).to_owned()).collect()
}

// "a changed `.rs` file in the folder of a harness ... runs by the module's name filter ... the filters of one
// target's modules in one step"; "a changed file that is a test target's own source runs that target whole"
#[test]
fn a_harness_module_runs_by_its_filter_and_a_target_s_own_source_runs_it_whole() {
  let (dir, lab) = lab();
  let owed_for = |paths: &[&str]| owed_tests(dir.path(), &lab, &changed(paths));

  assert_eq!(owed_for(&["lab/tests/alpha.rs"]), Some(vec![owed("main", &["alpha::"])]));
  assert_eq!(
    owed_for(&["lab/tests/beta.rs", "lab/tests/alpha.rs"]),
    Some(vec![owed("main", &["alpha::", "beta::"])])
  );
  assert_eq!(owed_for(&["lab/tests/main.rs", "lab/tests/alpha.rs"]), Some(vec![owed("main", &[])]));
  assert_eq!(owed_for(&["lab/tests/solo.rs"]), Some(vec![owed("solo", &[])]));
  assert_eq!(owed_for(&["lab/tests/sweep/main.rs"]), Some(vec![owed("sweep", &[])]));
  assert_eq!(
    owed_for(&["lab/tests/solo.rs", "lab/tests/alpha.rs"]),
    Some(vec![owed("main", &["alpha::"]), owed("solo", &[])])
  );
}

// "a module whose files carry no test attribute ... or that another file under the crate's `tests/` names by a
// path through it or by a `#[path]` attribute, is shared, and runs the crate's tests whole; so does a changed
// file under `tests/` that is neither"
#[test]
fn a_shared_module_a_fixture_and_a_source_owe_the_crate_s_tests_whole() {
  let (dir, lab) = lab();
  let owed_for = |paths: &[&str]| owed_tests(dir.path(), &lab, &changed(paths));

  // Carries no test, and the beta module walks a path through it.
  assert_eq!(owed_for(&["lab/tests/support.rs"]), None);
  // Carries a test, but the target of its own reaches it by `#[path]`.
  assert_eq!(owed_for(&["lab/tests/migrators.rs"]), None);
  assert_eq!(owed_for(&["lab/tests/fixtures/case.json"]), None);
  assert_eq!(owed_for(&["lab/src/lib.rs"]), None);
  assert_eq!(owed_for(&["lab/tests/alpha.rs", "lab/src/lib.rs"]), None);
  assert_eq!(owed_for(&[]), None);
}

// "Inside a macro's arguments a path is a run of tokens"
#[test]
fn a_test_file_is_read_for_its_tests_its_module_paths_and_its_path_attributes() {
  let read = read_test_file(
    "use crate::{helpers, shared::Kept};\n\n#[path = \"../outside/kept.rs\"]\nmod kept;\n\n#[tokio::test]\nasync fn one() {\n  assert_eq!(support::one(), deep::inner::two());\n}\n",
    "lab/tests/beta.rs",
  );

  assert!(read.has_tests);
  assert_eq!(read.path_files, ["lab/outside/kept.rs"]);

  for segment in ["crate", "helpers", "shared", "support", "deep", "inner"] {
    assert!(
      read.path_segments.iter().any(|held| held == segment),
      "{segment}: {:?}",
      read.path_segments
    );
  }

  let plain = read_test_file("#[cfg(test)]\nfn helper() {}\n", "lab/tests/plain.rs");

  assert!(!plain.has_tests);
  assert!(plain.path_segments.is_empty());
}
