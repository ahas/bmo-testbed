//! `backup` (docs/boundary.md, 「The verbs」; docs/backup.md): the settings read from the data root, one run
//! of each word at a time, and an event where any step failed. The runs themselves are the backup crate's;
//! this is the verb's side.

use std::path::PathBuf;

use bmo_testbed_backup::drill::drill;
use bmo_testbed_backup::media::media;
use bmo_testbed_backup::outcome::Outcome;
use bmo_testbed_backup::policy::BACKUP;
use bmo_testbed_backup::rotation::{Rotation, rotate};
use bmo_testbed_backup::settings::{Plan, settings_path};
use bmo_testbed_backup::snapshot::{media_names, snapshot};
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::policy::{fill_marks, fill_shown};
use bmo_testbed_core::queue::{LockWait, take_lock};
use bmo_testbed_core::turn::{NOT_RUN_EXIT_CODE, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};

use crate::words::BackupWord;

/// `backup <word>`: 0 where every step passed, 1 where any failed, 2 where the settings do not read, 75
/// where another run of the same word holds its lock.
pub fn backup(word: &BackupWord) -> u8 {
  let words = &BACKUP.words;
  let env = process_env();
  let root = data_root();
  let path = settings_path(&root);

  if !path.exists() {
    eprintln!("{}", fill_shown(&words.no_settings, &[("path", &path.display())]));
    return REFUSED_EXIT_CODE;
  }

  let home = env.get("HOME").map(PathBuf::from).unwrap_or_default();
  let plan = match Plan::read(&path, &home) {
    Ok(plan) => plan,
    Err(why) => {
      eprintln!("{why}");
      return REFUSED_EXIT_CODE;
    }
  };
  // The prune takes the snapshot's lock, since a snapshot ends with the same rotation.
  let (holder, lock) = match word {
    BackupWord::Snapshot | BackupWord::Prune(_) => (&words.snapshot_holder, "backup-snapshot.lock"),
    BackupWord::Media => (&words.media_holder, "backup-media.lock"),
    BackupWord::Drill => (&words.drill_holder, "backup-drill.lock"),
  };
  let wait = LockWait {
    holder: holder.clone(),
    ..LockWait::default()
  };
  let _ = std::fs::create_dir_all(root.join("locks"));
  let Some(_held) = take_lock(&root.join("locks").join(lock), 0.0, wait) else {
    eprintln!("{}", fill_marks(&words.locked, &[("holder", holder)]));
    return NOT_RUN_EXIT_CODE;
  };
  let mut say = |line: &str| println!("{line}");
  let now = chrono::Local::now();
  let outcome: Outcome = match word {
    BackupWord::Snapshot => snapshot(&plan, now, &mut say),
    BackupWord::Media => media(&plan, &mut say),
    BackupWord::Drill => drill(&plan, now, &mut say),
    BackupWord::Prune(args) => {
      let media = media_names(&plan);

      rotate(&plan, &Rotation::of(&plan, now.date_naive(), &media), args.dry_run, &mut say)
    }
  };

  if outcome.passed() {
    return 0;
  }

  let (event, summary, verb) = match word {
    BackupWord::Drill => ("restore-drill-failed", &words.drill_failed_summary, "drill"),
    BackupWord::Snapshot => ("data-backup-failed", &words.failed_summary, "snapshot"),
    BackupWord::Media => ("data-backup-failed", &words.failed_summary, "media"),
    BackupWord::Prune(_) => ("data-backup-failed", &words.failed_summary, "prune"),
  };
  let repo = std::env::current_dir().ok().and_then(|cwd| Repository::read(&cwd).ok());

  Hook::of(repo.as_ref(), &env).send(&Event {
    name: event,
    commit: None,
    summary: fill_marks(summary, &[("count", &outcome.failed.len().to_string())]),
    detail: outcome.detail(verb),
  });

  1
}
