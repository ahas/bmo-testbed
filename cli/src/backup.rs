//! `backup` (docs/boundary.md, 「The verbs」; docs/backup.md): the settings read from the data root, one run
//! of each word at a time, and an event where any step failed, its failures written at the journal's error
//! priority. The runs themselves are the backup crate's; this is the verb's side.

use std::os::fd::AsFd as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;

use bmo_testbed_backup::drill::drill;
use bmo_testbed_backup::media::media;
use bmo_testbed_backup::outcome::{Outcome, Voice};
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
  let mut say = Printed {
    journal: output_is_journal(env.get("JOURNAL_STREAM").map(String::as_str)),
  };
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
  let summary = fill_marks(summary, &[("count", &outcome.failed.len().to_string())]);

  say.failure(&fill_marks(&words.failed_event, &[("event", event), ("summary", &summary)]));
  Hook::of(repo.as_ref(), &env).send(&Event {
    name: event,
    commit: None,
    summary,
    detail: outcome.detail(verb),
  });

  1
}

/// The journal's error priority, as the prefix a line written to it carries (`sd-daemon(3)`).
const ERROR_PRIORITY: &str = "<3>";

/// The verb's voice: every line to standard output, a failure's at the journal's error priority where the
/// output is the journal.
pub struct Printed {
  pub journal: bool,
}

impl Voice for Printed {
  fn line(&mut self, line: &str) {
    println!("{line}");
  }

  fn failure(&mut self, line: &str) {
    println!("{}", failure_lines(line, self.journal));
  }
}

/// A failure as it is written: each of its lines carrying the error priority where the output is the
/// journal, which reads a prefix on every line it is handed, and as it is otherwise.
pub fn failure_lines(line: &str, journal: bool) -> String {
  if !journal {
    return line.to_owned();
  }

  line.lines().map(|one| format!("{ERROR_PRIORITY}{one}")).collect::<Vec<_>>().join("\n")
}

/// Whether standard output is the journal: the service manager names the device and inode of the stream it
/// connected in `JOURNAL_STREAM`, which standard output is where its own are the same (`systemd.exec(5)`).
/// The value is two decimal numbers joined by a colon, read as such.
pub fn output_is_journal(stream: Option<&str>) -> bool {
  let Some((device, inode)) = stream.and_then(|named| named.split_once(':')) else {
    return false;
  };
  let Ok(own) = std::io::stdout().as_fd().try_clone_to_owned() else {
    return false;
  };
  let Ok(meta) = std::fs::File::from(own).metadata() else {
    return false;
  };

  device.parse() == Ok(meta.dev()) && inode.parse() == Ok(meta.ino())
}
