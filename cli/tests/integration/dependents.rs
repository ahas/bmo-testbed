//! The `dependents` verb (docs/suite.md, 「The dependents of a major change」): the checkouts whose own
//! declarations name this repository as an upstream, and with `--run` each one's suite run through the suite
//! runner.

use std::path::Path;

use crate::stand::{checkout, ran, testbed};

fn declaration(name: &str, upstream: &str) -> String {
  format!("testbed = 0\nname = \"{name}\"\nupstreams = [\"{upstream}\"]\n")
}

// A registry naming the consumer beside the upstream.
fn registry(dir: &Path, consumer: &Path) -> std::path::PathBuf {
  let at = dir.join("checkouts.toml");

  std::fs::write(&at, format!("checkouts = [\"{}\"]\n", consumer.display())).unwrap();
  at
}

// "the checkouts whose declaration names this repository among their upstreams"
#[test]
fn the_verb_lists_the_checkouts_that_name_the_repository() {
  let dir = tempfile::tempdir().unwrap();
  let upstream = dir.path().join("upstream");
  let consumer = dir.path().join("consumer");
  let other = dir.path().join("other");
  let data = dir.path().join("data");

  checkout(&upstream, "testbed = 0\nname = \"shared\"\n");
  checkout(&consumer, &declaration("consumer", "shared"));
  checkout(&other, "testbed = 0\nname = \"other\"\nupstreams = [\"elsewhere\"]\n");

  let at = dir.path().join("checkouts.toml");

  std::fs::write(&at, format!("checkouts = [\"{}\", \"{}\"]\n", consumer.display(), other.display()))
    .unwrap();

  let mut command = testbed(&upstream, &data, &["dependents"]);

  command.env("BMO_TESTBED_CHECKOUTS", &at);
  let done = ran(&mut command);

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(done.out.contains("consumer"), "{}", done.out);
  assert!(!done.out.contains("other"), "{}", done.out);
}

// "with --run, each consumer's suite runs through the suite runner"
#[test]
fn the_verb_runs_each_consumers_suite_with_the_run_flag() {
  let dir = tempfile::tempdir().unwrap();
  let upstream = dir.path().join("upstream");
  let consumer = dir.path().join("consumer");
  let data = dir.path().join("data");

  checkout(&upstream, "testbed = 0\nname = \"shared\"\n");
  checkout(&consumer, &declaration("consumer", "shared"));

  let at = registry(dir.path(), &consumer);
  let mut command = testbed(&upstream, &data, &["dependents", "--run"]);

  command.env("BMO_TESTBED_CHECKOUTS", &at);
  let done = ran(&mut command);

  assert_eq!(done.code, 0, "stdout: {}\nstderr: {}", done.out, done.err);
  assert!(done.out.contains("passed"), "{}", done.out);
  assert!(done.err.contains("suite:"), "{}", done.err);
}
