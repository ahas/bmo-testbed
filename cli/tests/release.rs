//! The `release` verb (docs/boundary.md, 「`release`」): a dry run over a set prints every change and what it
//! would do in git and changes nothing, a proposal prints the step, and words that name neither are refused.

use std::path::Path;

use crate::stand::{git, ran, testbed};

// A set holding one unit, its remote a bare repository beside it.
fn set_of_one(dir: &Path) -> std::path::PathBuf {
  let set = dir.join("set");
  let unit = set.join("services/one");
  let remote = dir.join("remotes/unit-one.git");

  std::fs::create_dir_all(&unit).unwrap();
  std::fs::create_dir_all(&remote).unwrap();
  git(&remote, &["init", "--quiet", "--bare", "--initial-branch=main"]);
  git(&unit, &["init", "--quiet", "--initial-branch=main"]);
  git(&unit, &["config", "user.name", "Tester"]);
  git(&unit, &["config", "user.email", "tester@example.com"]);
  std::fs::write(unit.join("VERSION"), "0.1.0-rev.2\n").unwrap();
  std::fs::write(unit.join("notes.txt"), "Shipped in 0.1.0-rev.2.\n").unwrap();
  git(&unit, &["add", "."]);
  git(&unit, &["commit", "--quiet", "-m", "Add the version"]);
  git(&unit, &["remote", "add", "origin", &remote.to_string_lossy()]);
  git(&unit, &["push", "--quiet", "origin", "main"]);
  set
}

// "--dry-run prints the same and changes nothing but the fetch"
#[test]
fn a_dry_run_prints_every_change_and_changes_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let set = set_of_one(dir.path());
  let data = dir.path().join("data");
  let remote = dir.path().join("remotes/unit-one.git");
  let head = git(&remote, &["rev-parse", "main"]);
  let done = ran(&mut testbed(
    dir.path(),
    &data,
    &["release", "--version", "1.0.0", "--set", "set", "--dry-run"],
  ));

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(done.out.contains("release 1.0.0 over 1 repositories of"), "{}", done.out);
  assert!(done.out.contains("  VERSION: version 0.1.0-rev.2 → 1.0.0"), "{}", done.out);
  assert!(
    done.out.contains("  left as they stand, naming an own version: notes.txt"),
    "{}",
    done.out
  );
  assert!(done.out.contains("release: a dry run"), "{}", done.out);
  assert_eq!(git(&remote, &["rev-parse", "main"]), head);
  assert_eq!(git(&remote, &["tag", "--list"]), "");
  assert_eq!(std::fs::read_to_string(set.join("services/one/VERSION")).unwrap(), "0.1.0-rev.2\n");
}

// "--propose prints the step the commits since the last release propose"; a version that is no semantic
// version above the last release is refused
#[test]
fn a_proposal_prints_the_step_and_a_version_not_above_the_last_release_is_refused() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");

  set_of_one(dir.path());

  let proposed = ran(&mut testbed(dir.path(), &data, &["release", "--propose", "--set", "set"]));

  assert_eq!(proposed.code, 0, "{}", proposed.err);
  assert!(
    proposed.out.contains("unit-one: 1 commits: 0 major, 1 minor, 0 patch"),
    "{}",
    proposed.out
  );
  assert!(proposed.out.contains("propose: a minor release, 0.1.0 → 0.2.0"), "{}", proposed.out);

  let refused = ran(&mut testbed(
    dir.path(),
    &data,
    &["release", "--version", "0.1.0", "--set", "set", "--dry-run"],
  ));

  assert_eq!(refused.code, 2, "{}", refused.out);
  assert!(
    refused.err.contains("0.1.0 is no plain semantic version above the last release, 0.1.0"),
    "{}",
    refused.err
  );

  let neither = ran(&mut testbed(dir.path(), &data, &["release", "--set", "set"]));

  assert_eq!(neither.code, 2);
}
