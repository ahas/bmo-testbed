//! The stores (docs/backup.md, 「The settings」): an S3 bucket and an rclone remote each put, list, get and
//! remove a key the same way, reached through the programs that stand in for them.

use bmo_testbed_backup::destination::{Rclone, S3, Store};

use crate::stand::Ground;

fn round_trip(ground: &Ground, store: &dyn Store) {
  let local = ground.path("local.bin");
  let back = ground.path("back.bin");

  std::fs::write(&local, b"hello there").unwrap();

  assert_eq!(store.list("data-backup/").unwrap(), []);

  store.put(&local, "data-backup/hourly/0/one.bin", "sha").unwrap();

  let listed = store.list("data-backup/").unwrap();

  assert_eq!(listed.len(), 1);
  assert_eq!(listed[0].key, "data-backup/hourly/0/one.bin");
  assert_eq!(listed[0].size, 11);

  store.get("data-backup/hourly/0/one.bin", &back).unwrap();

  assert_eq!(std::fs::read(&back).unwrap(), b"hello there");

  store.remove("data-backup/hourly/0/one.bin").unwrap();

  assert_eq!(store.list("data-backup/hourly/").unwrap(), []);
}

#[test]
fn an_s3_bucket_puts_lists_gets_and_removes() {
  let ground = Ground::new();
  let store = S3 {
    command: ground.path("bin/aws").to_string_lossy().into_owned(),
    bucket: "bucket".to_owned(),
    profile: Some("stand-in".to_owned()),
    endpoint: None,
  };

  round_trip(&ground, &store);
}

#[test]
fn an_rclone_remote_puts_lists_gets_and_removes() {
  let ground = Ground::new();
  let store = Rclone {
    command: ground.path("bin/rclone").to_string_lossy().into_owned(),
    remote: "remote:kept".to_owned(),
  };

  round_trip(&ground, &store);
}

#[test]
fn a_store_whose_program_fails_says_so() {
  let ground = Ground::new();
  let store = S3 {
    command: ground.path("bin/aws").to_string_lossy().into_owned(),
    bucket: "missing".to_owned(),
    profile: None,
    endpoint: None,
  };
  let said = store.list("data-backup/").err().unwrap();

  assert!(said.contains("exited 254"), "{said}");
}
