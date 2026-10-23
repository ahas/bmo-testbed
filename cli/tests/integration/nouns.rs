//! The nouns the binary declares to `bmo` (docs/boundary.md, 「The nouns `bmo` serves」): what `nouns`
//! prints, and each noun's words reaching the verb it names.

use std::ffi::OsString;

use bmo_testbed_cli::words::{
  CargoArgs, CargoWord, DeployArgs, DeployFlags, DeployWord, PruneArgs, SuiteArgs, TestArgs, TestWord,
  Verb, read,
};

use crate::stand::{ran, testbed};

fn read_words(given: &[&str]) -> Verb {
  let words = ["bmo-testbed"].iter().chain(given).map(OsString::from);

  read(words).map(|words| words.verb).unwrap_or_else(|why| panic!("{given:?}: {why}"))
}

#[test]
fn the_nouns_are_printed_one_a_line() {
  let dir = tempfile::tempdir().unwrap();
  let printed = ran(&mut testbed(dir.path(), dir.path(), &["nouns"]));

  assert_eq!(printed.code, 0, "{}", printed.err);
  assert_eq!(
    printed.out.lines().collect::<Vec<_>>(),
    [
      "test suite",
      "test count",
      "test log",
      "test template",
      "test full",
      "check",
      "imports",
      "cargo",
      "cargo prune",
      "deploy commit",
      "deploy backups",
      "backup snapshot",
      "backup media",
      "backup drill",
      "backup prune",
    ]
  );
}

#[test]
fn every_noun_hands_its_words_to_the_verb_it_names() {
  let dir = tempfile::tempdir().unwrap();

  for (noun, verb) in [
    (&["test", "suite"][..], "suite"),
    (&["test", "count"], "count"),
    (&["test", "log"], "log"),
    (&["test", "template"], "template"),
    (&["test", "full"], "full"),
    (&["check"], "check"),
    (&["imports"], "imports"),
    (&["cargo"], "cargo"),
    (&["cargo", "prune"], "prune"),
    (&["deploy", "commit"], "deploy"),
    (&["deploy", "backups"], "backups"),
    (&["backup", "snapshot"], "backup"),
    (&["backup", "media"], "backup"),
    (&["backup", "drill"], "backup"),
    (&["backup", "prune"], "backup"),
  ] {
    let words: Vec<&str> = noun.iter().copied().chain(["--help"]).collect();
    let usage = ran(&mut testbed(dir.path(), dir.path(), &words));

    assert_eq!(usage.code, 0, "{noun:?}: {}", usage.err);
    assert!(usage.out.starts_with(&format!("usage: bmo-testbed {verb}")), "{noun:?}: {}", usage.out);
  }
}

#[test]
fn the_compound_nouns_read_as_the_verbs_they_name() {
  assert!(matches!(
    read_words(&["test", "suite", "--since", "HEAD"]),
    Verb::Test(TestArgs {
      word: TestWord::Suite(SuiteArgs { since: Some(_), .. })
    })
  ));

  match read_words(&["cargo", "prune", "--all"]) {
    Verb::Cargo(CargoArgs {
      word: Some(CargoWord::Prune(PruneArgs { all: true, .. })),
      ..
    }) => {}
    other => panic!("{other:?}"),
  }

  match read_words(&["cargo", "build", "--release"]) {
    Verb::Cargo(CargoArgs { word: None, words }) => assert_eq!(words, ["build", "--release"]),
    other => panic!("{other:?}"),
  }

  match read_words(&["deploy", "commit", "--dry-run"]) {
    Verb::Deploy(DeployArgs {
      word: Some(DeployWord::Commit(DeployFlags { dry_run: true, .. })),
      ..
    }) => {}
    other => panic!("{other:?}"),
  }

  match read_words(&["deploy", "backups"]) {
    Verb::Deploy(DeployArgs {
      word: Some(DeployWord::Backups),
      ..
    }) => {}
    other => panic!("{other:?}"),
  }
}
