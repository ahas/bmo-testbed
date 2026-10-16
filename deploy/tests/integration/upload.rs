//! The backups' upload (docs/deploy.md, 「Sending backups away」), the backup directory and the remote both
//! stood in by maps: nothing is read from disk and nothing leaves the machine.

use std::collections::{BTreeMap, BTreeSet};

use bmo_testbed_deploy::upload::{
  BackupNames, Digest, Remote, UploadFailure, UploadHands, UploadOutcome, UploadStep, receipt_name,
  send_backups, upload_failed_detail, upload_failed_summary, upload_usage,
};

const KEEP: usize = 5;

fn backup(minute: u32) -> String {
  format!("app-20261006-10{minute:02}00.tar.zst")
}

#[derive(Default)]
struct World {
  disk: BTreeMap<String, String>,
  remote: BTreeMap<String, String>,
  // Backups whose upload lands something other than what was sent.
  torn: BTreeSet<String>,
  // Steps that fail, by `<step>:<backup>`, or `list`.
  breaks: BTreeSet<String>,
  uploads: Vec<String>,
  said: Vec<String>,
}

fn digest(body: &str) -> Digest {
  Digest {
    size: body.len() as u64,
    sha256: format!("sha-{body}"),
  }
}

impl World {
  fn broken(&self, step: &str) -> Result<(), String> {
    if self.breaks.contains(step) { Err(format!("{step} refused")) } else { Ok(()) }
  }

  fn send(&mut self) -> UploadOutcome {
    send_backups(self, &BackupNames::new("app"), KEEP)
  }
}

impl UploadHands for World {
  fn local(&mut self) -> Vec<String> {
    self.disk.keys().cloned().collect()
  }

  fn receipt(&mut self, name: &str) -> Option<String> {
    self.disk.get(&receipt_name(name)).cloned()
  }

  fn write_receipt(&mut self, name: &str, sha: &str) -> Result<(), String> {
    self.disk.insert(receipt_name(name), sha.to_owned());
    Ok(())
  }

  fn remove_local(&mut self, name: &str) -> Result<(), String> {
    self.disk.remove(name);
    self.disk.remove(&receipt_name(name));
    Ok(())
  }

  fn digest(&mut self, name: &str) -> Result<Digest, String> {
    Ok(digest(&self.disk[name]))
  }

  fn remote(&mut self) -> Result<Vec<String>, String> {
    self.broken("list")?;
    Ok(self.remote.keys().cloned().collect())
  }

  fn upload(&mut self, name: &str, _sha: &str) -> Result<(), String> {
    self.broken(&format!("upload:{name}"))?;
    self.uploads.push(name.to_owned());

    let body = if self.torn.contains(name) { "torn".to_owned() } else { self.disk[name].clone() };

    self.remote.insert(name.to_owned(), body);
    Ok(())
  }

  fn read_back(&mut self, name: &str) -> Result<Digest, String> {
    self.broken(&format!("read-back:{name}"))?;
    Ok(digest(&self.remote[name]))
  }

  fn remove_remote(&mut self, name: &str) -> Result<(), String> {
    self.remote.remove(name);
    Ok(())
  }

  fn say(&mut self, line: &str) {
    self.said.push(line.to_owned());
  }
}

fn backups(minutes: &[u32]) -> BTreeMap<String, String> {
  minutes
    .iter()
    .map(|minute| (backup(*minute), format!("body of {}", backup(*minute))))
    .collect()
}

fn keys(map: &BTreeMap<String, String>) -> Vec<String> {
  map.keys().cloned().collect()
}

#[test]
fn the_new_one_is_sent_read_back_and_the_one_before_it_leaves_the_disk() {
  let mut world = World {
    disk: backups(&[1, 2]),
    ..World::default()
  };

  world.disk.insert(receipt_name(&backup(1)), format!("sha-body of {}", backup(1)));
  world.remote.insert(backup(1), format!("body of {}", backup(1)));

  let outcome = world.send();

  assert_eq!(outcome.failed, []);
  assert_eq!(outcome.sent, [backup(2)]);
  assert_eq!(keys(&world.disk), [backup(2), receipt_name(&backup(2))]);
  assert_eq!(keys(&world.remote), [backup(1), backup(2)]);
  assert_eq!(
    world.said,
    [
      format!(
        "{} stands remotely, {} bytes, sha256 sha-body of {}",
        backup(2),
        format!("body of {}", backup(2)).len(),
        backup(2)
      ),
      format!("{} removed from the local disk", backup(1)),
    ]
  );
}

#[test]
fn the_remote_keeps_the_declared_number_of_newest() {
  let mut world = World {
    disk: backups(&[7]),
    remote: backups(&[1, 2, 3, 4, 5, 6]),
    ..World::default()
  };
  let outcome = world.send();
  let mut removed = outcome.removed_remote.clone();

  removed.sort();
  assert_eq!(removed, [backup(1), backup(2)]);
  assert_eq!(keys(&world.remote), [3, 4, 5, 6, 7].map(backup));
  assert_eq!(world.remote.len(), KEEP);
}

#[test]
fn a_copy_that_reads_back_otherwise_is_taken_off_the_remote() {
  let mut world = World {
    disk: backups(&[1, 2]),
    torn: [backup(1)].into(),
    ..World::default()
  };
  let outcome = world.send();

  assert_eq!(
    outcome
      .failed
      .iter()
      .map(|failure| (failure.name.clone(), failure.step))
      .collect::<Vec<_>>(),
    [(Some(backup(1)), UploadStep::ReadBack)]
  );
  assert!(!world.remote.contains_key(&backup(1)));
  assert!(world.disk.contains_key(&backup(1)));
  assert!(!world.disk.contains_key(&receipt_name(&backup(1))));
  assert_eq!(outcome.sent, [backup(2)]);
}

#[test]
fn an_upload_that_fails_keeps_the_backup_until_a_later_run_confirms_it() {
  let mut world = World {
    disk: backups(&[1, 2]),
    breaks: [format!("upload:{}", backup(1))].into(),
    ..World::default()
  };
  let first = world.send();

  assert_eq!(
    first.failed.iter().map(|failure| failure.step).collect::<Vec<_>>(),
    [UploadStep::Upload]
  );
  assert!(world.disk.contains_key(&backup(1)));

  world.breaks.clear();

  let second = world.send();

  assert_eq!(second.sent, [backup(1)]);
  assert_eq!(second.removed_local, [backup(1)]);
}

#[test]
fn an_object_already_whole_remotely_is_confirmed_without_uploading_it_again() {
  let mut world = World {
    disk: backups(&[1, 2]),
    remote: backups(&[1]),
    ..World::default()
  };
  let outcome = world.send();
  let mut sent = outcome.sent.clone();

  sent.sort();
  assert_eq!(world.uploads, [backup(2)]);
  assert_eq!(sent, [backup(1), backup(2)]);
  assert!(!world.disk.contains_key(&backup(1)));
}

#[test]
fn a_backup_as_many_newer_copies_as_are_kept_supersede_is_not_sent_and_leaves_the_disk() {
  let mut world = World {
    disk: backups(&[1, 7]),
    remote: backups(&[2, 3, 4, 5, 6]),
    ..World::default()
  };
  let outcome = world.send();

  assert_eq!(world.uploads, [backup(7)]);
  assert_eq!(outcome.removed_local, [backup(1)]);
  assert!(!world.remote.contains_key(&backup(1)));
}

#[test]
fn with_no_listing_nothing_is_sent_or_rotated() {
  let mut world = World {
    disk: backups(&[1, 2, 3]),
    remote: backups(&[1]),
    breaks: ["list".to_owned()].into(),
    ..World::default()
  };

  world.disk.insert(receipt_name(&backup(1)), "sha".into());

  let outcome = world.send();

  assert!(world.uploads.is_empty());
  assert_eq!(
    outcome
      .failed
      .iter()
      .map(|failure| (failure.name.clone(), failure.step))
      .collect::<Vec<_>>(),
    [(None, UploadStep::List)]
  );
  assert_eq!(outcome.removed_local, [backup(1)]);
  assert_eq!(keys(&world.disk), [backup(2), backup(3)]);
}

#[test]
fn the_newest_stays_on_disk_and_a_receipt_left_alone_goes() {
  let mut world = World {
    disk: backups(&[1]),
    ..World::default()
  };

  world.disk.insert(receipt_name(&backup(0)), "sha".into());
  world.send();
  assert_eq!(keys(&world.disk), [backup(1), receipt_name(&backup(1))]);
}

#[test]
fn what_is_not_a_backup_is_never_touched() {
  let mut world = World {
    disk: backups(&[1, 2]),
    ..World::default()
  };

  for name in [
    "notes.txt".to_owned(),
    "notes.txt.sent".to_owned(),
    format!("{}.part", backup(3)),
    "other-20261006-100000.tar.zst".to_owned(),
  ] {
    world.disk.insert(name, "mine".into());
  }

  world.send();

  for name in [
    "notes.txt",
    "notes.txt.sent",
    &format!("{}.part", backup(3)),
    "other-20261006-100000.tar.zst",
  ] {
    assert!(world.disk.contains_key(name), "{name}");
    assert!(!world.remote.contains_key(name), "{name}");
  }
}

#[test]
fn backup_names_are_the_declared_name_a_second_and_an_ordinal() {
  let names = BackupNames::new("app.v2");

  assert!(names.is_backup("app.v2-20261006-100000.tar.zst"));
  assert!(names.is_backup("app.v2-20261006-100000-02.tar.zst"));
  assert!(!names.is_backup("appxv2-20261006-100000.tar.zst"));
  assert!(!names.is_backup("app.v2-20261006-100000.tar.zst.part"));
  assert_eq!(
    names.newest_first(&[
      "app.v2-20261006-100000.tar.zst",
      "notes.txt",
      "app.v2-20261006-100000-02.tar.zst",
      "app.v2-20261007-090000.tar.zst",
    ]),
    [
      "app.v2-20261007-090000.tar.zst",
      "app.v2-20261006-100000-02.tar.zst",
      "app.v2-20261006-100000.tar.zst",
    ]
  );
}

#[test]
fn the_event_carries_each_failure_with_its_backup_and_step() {
  let outcome = UploadOutcome {
    failed: vec![UploadFailure {
      name: Some(backup(1)),
      step: UploadStep::ReadBack,
      said: "no route".into(),
    }],
    ..UploadOutcome::default()
  };
  let detail = upload_failed_detail(&outcome);

  assert_eq!(detail["backups"][0]["name"], backup(1));
  assert_eq!(detail["backups"][0]["step"], "read-back");
  assert_eq!(detail["backups"][0]["said"], "no route");

  let summary = upload_failed_summary(&outcome);

  assert!(summary.starts_with("1 step(s) of the backup upload failed. "), "{summary}");
  assert!(summary.contains("stays on the local disk"), "{summary}");
}

#[test]
fn the_usage_names_the_remote_and_the_number_kept() {
  let usage = upload_usage("s3://backups/app/", 5);

  assert_eq!(usage[0], "usage: bmo-testbed backups");
  assert_eq!(usage[1], "");
  assert!(usage[2].contains("s3://backups/app/"), "{usage:?}");
  assert!(usage[3].contains("the newest 5 remotely"), "{usage:?}");
}

#[test]
fn a_remote_is_an_s3_bucket_with_a_prefix_under_it() {
  assert_eq!(
    Remote::parse("s3://backups/app"),
    Ok(Remote {
      bucket: "backups".to_owned(),
      prefix: "app/".to_owned(),
    })
  );
  assert_eq!(
    Remote::parse("s3://backups/app/deploy/"),
    Ok(Remote {
      bucket: "backups".to_owned(),
      prefix: "app/deploy/".to_owned(),
    })
  );
  assert_eq!(
    Remote::parse("s3://backups"),
    Ok(Remote {
      bucket: "backups".to_owned(),
      prefix: String::new(),
    })
  );
  assert!(Remote::parse("https://backups/app").is_err());
  assert!(Remote::parse("backups/app").is_err());
}
