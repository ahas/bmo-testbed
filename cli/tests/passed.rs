//! What a run of the suite writes of what it ran on, and the `passed` verb reading it back (docs/boundary.md,
//! 「The record」 and 「`passed`」): a whole run from a clean checkout is the passed run a deployment's gate
//! takes, and a run from a checkout with uncommitted changes to tracked files, or of part of the suite, is
//! none.

use std::path::Path;

use bmo_testbed_client::TestRun;

use crate::stand::{Ran, checkout, git, ran, testbed};

fn suite(repo: &Path, data: &Path, args: &[&str]) -> Ran {
  ran(&mut testbed(repo, data, &[&["suite"][..], args].concat()))
}

fn passed(repo: &Path, data: &Path) -> Ran {
  ran(&mut testbed(repo, data, &["passed", "--json"]))
}

// "a whole run of the suite from a clean checkout is written with its commit, whole and clean, and `passed`
// answers it"
#[test]
fn a_whole_suite_from_a_clean_checkout_is_the_passed_run() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let data = dir.path().join("data");

  checkout(&repo, "testbed = 0\nname = \"sample\"\n");
  std::fs::create_dir_all(&data).unwrap();

  let none = passed(&repo, &data);

  assert_eq!(none.code, 1, "{}\n{}", none.out, none.err);
  assert!(none.out.contains("no run of the whole suite"), "{}", none.out);

  let run = suite(&repo, &data, &[]);

  assert_eq!(run.code, 0, "{}", run.err);

  let found = passed(&repo, &data);

  assert_eq!(found.code, 0, "{}\n{}", found.out, found.err);

  let answered: TestRun = serde_json::from_str(found.out.trim()).expect("one test run");

  assert_eq!(answered.commit.as_deref(), Some(git(&repo, &["rev-parse", "HEAD"]).as_str()));
  assert_eq!((answered.whole, answered.clean), (Some(true), Some(true)));

  let named = ran(&mut testbed(&repo, &data, &["passed"]));

  assert_eq!(named.code, 0, "{}", named.err);
  assert!(
    named.out.contains(&format!("passed: test run {} passed the whole suite", answered.id)),
    "{}",
    named.out
  );
}

// "a run from a checkout with an uncommitted change to a tracked file, or a run of part of the suite, is no
// passed run"
#[test]
fn a_dirty_or_partial_run_is_no_passed_run() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let data = dir.path().join("data");

  checkout(&repo, "testbed = 0\nname = \"sample\"\n");
  std::fs::create_dir_all(&data).unwrap();

  let partial = suite(&repo, &data, &["--since", "HEAD"]);

  assert_eq!(partial.code, 0, "{}", partial.err);
  assert_eq!(passed(&repo, &data).code, 1);

  std::fs::write(repo.join("testbed.toml"), "testbed = 0\nname = \"sample\"\n\n").unwrap();

  let dirty = suite(&repo, &data, &[]);

  assert_eq!(dirty.code, 0, "{}", dirty.err);
  assert_eq!(passed(&repo, &data).code, 1);

  let refused = ran(&mut testbed(&repo, &data, &["passed", "--commit", "no-such-commit"]));

  assert_eq!(refused.code, 2, "{}", refused.err);
}
