//! The `backup` verb (docs/boundary.md, 「The verbs」; docs/backup.md): a data root with no settings or
//! settings that do not read is refused with exit 2, another run of the same word holding the lock ends
//! with 75, and the words read as the word they name.

use std::ffi::OsString;

use bmo_testbed_cli::words::{BackupPruneArgs, BackupWord, Verb, read};

use crate::stand::{ran, testbed};

#[test]
fn a_data_root_with_no_settings_is_refused() {
  let dir = tempfile::tempdir().unwrap();
  let printed = ran(&mut testbed(dir.path(), dir.path(), &["backup", "snapshot"]));

  assert_eq!(printed.code, 2, "{}", printed.err);
  assert!(printed.err.contains("no settings at"), "{}", printed.err);
}

#[test]
fn settings_that_do_not_read_are_refused() {
  let dir = tempfile::tempdir().unwrap();

  std::fs::create_dir_all(dir.path().join("env")).unwrap();
  std::fs::write(dir.path().join("env/backup.toml"), "root = 3").unwrap();

  let printed = ran(&mut testbed(dir.path(), dir.path(), &["backup", "drill"]));

  assert_eq!(printed.code, 2, "{}", printed.err);
  assert!(printed.err.contains("do not read"), "{}", printed.err);
}

#[test]
fn a_prune_with_nothing_to_remove_says_so() {
  let dir = tempfile::tempdir().unwrap();
  let store = dir.path().join("store");
  let settings = format!(
    "root = \"{root}\"\nfolders = []\ndatabases = []\nstaging = \"{root}/staging\"\n\
     [[destination]]\nname = \"local\"\nkind = \"rclone\"\nremote = \"{store}\"\ncommand = \"true\"\n",
    root = dir.path().display(),
    store = store.display(),
  );

  std::fs::create_dir_all(dir.path().join("env")).unwrap();
  std::fs::write(dir.path().join("env/backup.toml"), settings).unwrap();

  let printed = ran(&mut testbed(dir.path(), dir.path(), &["backup", "prune", "--dry-run"]));

  assert_eq!(printed.code, 0, "{}", printed.err);
  assert_eq!(printed.out.trim(), "nothing to remove");
}

#[test]
fn the_words_read_as_the_word_they_name() {
  let words = |given: &[&str]| {
    read(["bmo-testbed"].iter().chain(given).map(OsString::from))
      .map(|words| words.verb)
      .unwrap_or_else(|why| panic!("{given:?}: {why}"))
  };

  assert!(
    matches!(words(&["backup", "snapshot"]), Verb::Backup(args) if matches!(args.word, BackupWord::Snapshot))
  );
  assert!(matches!(
    words(&["backup", "prune", "--dry-run"]),
    Verb::Backup(args) if matches!(args.word, BackupWord::Prune(BackupPruneArgs { dry_run: true }))
  ));
  assert!(read(["bmo-testbed", "backup"].map(OsString::from)).is_err());
}
