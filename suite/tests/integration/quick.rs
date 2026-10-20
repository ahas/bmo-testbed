//! The quick checks over a checkout (docs/checks.md, 「The quick checks」): the lines they print, the checks
//! a declaration enables, and the files they read. What each check finds is its own module's test.

use std::path::Path;
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::turn::Env;
use bmo_testbed_suite::quick::files::{checkout_files_where, is_recorded, is_test_file};
use bmo_testbed_suite::quick::{enabled_checks, report};

/// A scratch checkout holding the files given, each written at its path, with git's own settings kept
/// out.
pub fn checkout(files: &[(&str, &str)]) -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();

  for (path, text) in files {
    write(dir.path(), path, text);
  }

  let done = Command::new("git")
    .args(["init", "-q"])
    .current_dir(dir.path())
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_SYSTEM", "/dev/null")
    .stdin(Stdio::null())
    .output()
    .unwrap();

  assert!(done.status.success());
  dir
}

/// A file of a scratch checkout written at its path.
pub fn write(root: &Path, path: &str, text: &str) {
  let full = root.join(path);

  std::fs::create_dir_all(full.parent().unwrap()).unwrap();
  std::fs::write(full, text).unwrap();
}

/// The repository a scratch checkout stands for, as its declaration reads.
pub fn repository(dir: &tempfile::TempDir) -> Repository {
  Repository::read(dir.path()).unwrap()
}

/// The environment a check is run under, a home named and nothing else.
pub fn env() -> Env {
  Env::from([("HOME".to_owned(), "/nonexistent".to_owned())])
}

// "a check says what it read where it is clean and counts its faults where it is not"
#[test]
fn a_check_says_what_it_read_where_it_is_clean_and_counts_its_faults_where_it_is_not() {
  assert_eq!(
    report("wording", &[], "clean over 3 files"),
    ["quick check: wording: clean over 3 files"]
  );
  assert_eq!(
    report("wording", &["a.md:1: x".to_owned(), "b.md:2: y".to_owned()], "clean"),
    ["quick check: wording: 2 to fix", "  a.md:1: x", "  b.md:2: y"]
  );
}

// "the enabled checks run in the order the declaration names them, each naming the input it read"
#[test]
fn the_enabled_checks_run_in_the_order_the_declaration_names_them_each_naming_the_input_it_read() {
  let dir = checkout(&[
    (
      "testbed.toml",
      "testbed = 0\n[check]\nquick = [\"language\", \"dependencies\", \"wording\", \"cargo-profile\"]\nlanguage_ledger = \"ledger.json\"\nwording_document = \"words.md\"\n",
    ),
    ("words.md", "## Banned terms\n\n- en \u{b7} tool strip \u{2192} toolbar\n"),
    ("notes.txt", "the tool strip\n"),
  ]);
  let checks = enabled_checks(&repository(&dir), &env());
  let heads: Vec<&str> = checks.iter().map(|check| check.lines[0].as_str()).collect();

  assert_eq!(
    heads,
    [
      "quick check: language (ledger.json): 0 files within the ledger",
      "quick check: dependencies: 0 dependencies, each allowed",
      "quick check: wording (words.md): 1 to fix",
      "quick check: cargo-profile: 0 profile settings keep every build small",
    ]
  );
  assert_eq!(
    checks.iter().map(|check| check.faulted).collect::<Vec<_>>(),
    [false, false, true, false]
  );
  assert_eq!(checks[2].lines[1], "  notes.txt:1: \"tool strip\" \u{2192} toolbar");
}

// "a declaration enabling nothing runs nothing"
#[test]
fn a_declaration_enabling_nothing_runs_nothing() {
  let dir = checkout(&[("testbed.toml", "testbed = 0\n"), ("a.rs", "\"\u{2014}\"")]);

  assert!(enabled_checks(&repository(&dir), &env()).is_empty());
}

// "a test file is made up, recorded input and installed packages are somebody else's"
#[test]
fn a_test_file_is_made_up_and_recorded_input_and_installed_packages_are_somebody_elses() {
  for path in ["src/a.test.ts", "core/tests/flow.rs", "src/test/a.kt", "src/tests.rs"] {
    assert!(is_test_file(path), "{path}");
  }

  for path in ["src/tests_of_mine.rs", "src/latest.rs", "tests.md/x", "src/fixtures.ts"] {
    assert!(!is_test_file(path), "{path}");
  }

  assert!(is_recorded("site/fixtures/page.html") && is_recorded("node_modules/a/index.js"));
  assert!(!is_recorded("src/fixtures.ts"));
}

// "the checks read the tracked and the new files, never recorded input or an ignored file"
#[test]
fn the_checks_read_the_tracked_and_the_new_files_never_recorded_input_or_an_ignored_file() {
  let dir = checkout(&[
    (".gitignore", "ignored.ts\n"),
    ("new.ts", ""),
    ("ignored.ts", ""),
    ("site/fixtures/page.ts", ""),
  ]);
  let mut files = checkout_files_where(dir.path(), |path| path.ends_with(".ts"));

  files.sort();
  assert_eq!(files, ["new.ts"]);
}
