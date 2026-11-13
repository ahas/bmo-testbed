//! The `backup` verb (docs/boundary.md, 「The verbs」; docs/backup.md): a data root with no settings or
//! settings that do not read is refused with exit 2, another run of the same word holding the lock ends
//! with 75, the words read as the word they name, and a failure is written at the journal's error priority
//! only where the output is the journal.

use std::ffi::OsString;

use bmo_testbed_cli::backup::{failure_lines, output_is_journal};
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
fn a_failure_carries_the_error_priority_on_every_line_only_where_the_output_is_the_journal() {
  assert_eq!(failure_lines("pack failed\nno room", true), "<3>pack failed\n<3>no room");
  assert_eq!(failure_lines("pack failed\nno room", false), "pack failed\nno room");
  assert!(!output_is_journal(None));
  assert!(!output_is_journal(Some("not a stream")));
  assert!(!output_is_journal(Some("0:0")));
}

#[test]
fn a_drill_that_fails_ends_with_its_event_and_no_priority_off_the_journal() {
  let dir = tempfile::tempdir().unwrap();
  let settings = format!(
    "root = \"{root}\"\nfolders = []\ndatabases = []\nstaging = \"{root}/staging\"\n\
     [[destination]]\nname = \"local\"\nkind = \"rclone\"\nremote = \"{root}/store\"\ncommand = \"true\"\n",
    root = dir.path().display(),
  );

  std::fs::create_dir_all(dir.path().join("env")).unwrap();
  std::fs::write(dir.path().join("env/backup.toml"), settings).unwrap();

  // No hook of the machine's is sent the failure.
  let printed =
    ran(testbed(dir.path(), dir.path(), &["backup", "drill"]).env_remove("BMO_TESTBED_EVENT_HOOK"));

  assert_eq!(printed.code, 1, "{}", printed.err);
  assert!(!printed.out.contains("<3>"), "{}", printed.out);
  assert!(
    printed
      .out
      .trim_end()
      .ends_with("restore-drill-failed: The restore drill failed: 1 mismatch(es) or step(s)."),
    "{}",
    printed.out
  );
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
