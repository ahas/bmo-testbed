//! The rotation (docs/backup.md, 「What is removed」): only keys of the shapes it names, under the prefix,
//! past what is kept; nothing outside the prefix, and nothing of another shape inside it.

use bmo_testbed_backup::destination::Object;
use bmo_testbed_backup::rotation::{Rotation, rotate};
use chrono::NaiveDate;

use crate::stand::Ground;

fn objects(keys: &[&str]) -> Vec<Object> {
  keys
    .iter()
    .map(|key| Object {
      key: (*key).to_owned(),
      size: 1,
    })
    .collect()
}

#[test]
fn only_the_named_shapes_past_what_is_kept_are_removed() {
  let media = ["song".to_owned(), "large-files".to_owned()];
  let rotation = Rotation {
    prefix: "data-backup",
    hourly_slots: 4,
    today: NaiveDate::from_ymd_opt(2026, 10, 24).unwrap(),
    daily_days: 7,
    releases_kept: 2,
    media: &media,
  };
  let standing = objects(&[
    "hourly/00/bmo.pgdump",
    "hourly/23/manifest.json",
    "deploy/app-20260101-000000.tar.zst",
    "data-backup/hourly/0/app.pgdump",
    "data-backup/hourly/3/manifest.json",
    "data-backup/hourly/4/app.pgdump",
    "data-backup/hourly/12/manifest.json",
    "data-backup/daily/2026-10-24/manifest.json",
    "data-backup/daily/2026-10-18/manifest.json",
    "data-backup/daily/2026-10-17/manifest.json",
    "data-backup/daily/2026-10-01/app.pgdump",
    "data-backup/daily/not-a-date/app.pgdump",
    "data-backup/deploy/app/app-20261020-010000.tar.zst",
    "data-backup/deploy/app/app-20261022-010000.tar.zst",
    "data-backup/deploy/app/app-20261021-010000.tar.zst",
    "data-backup/deploy/other/other-20261001-010000.tar.zst",
    "data-backup/media/song.tar.zst",
    "data-backup/media/large-files.tar.zst",
    "data-backup/media/voice.tar.zst",
    "data-backup/notes.txt",
    "data-backup-old/hourly/9/app.pgdump",
  ]);

  assert_eq!(
    rotation.doomed(&standing),
    [
      "data-backup/daily/2026-10-01/app.pgdump",
      "data-backup/daily/2026-10-17/manifest.json",
      "data-backup/deploy/app/app-20261020-010000.tar.zst",
      "data-backup/hourly/12/manifest.json",
      "data-backup/hourly/4/app.pgdump",
      "data-backup/media/voice.tar.zst",
    ]
  );
}

#[test]
fn a_dry_run_removes_nothing_and_a_run_removes_what_it_said() {
  let ground = Ground::new();
  let plan = ground.plan(&ground.settings("folders = []\ndatabases = []"));

  for key in [
    "data-backup/hourly/9/app.pgdump",
    "data-backup/hourly/1/app.pgdump",
    "hourly/9/app.pgdump",
  ] {
    let path = ground.path("s3/bucket").join(key);

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
  }

  let media: Vec<String> = vec![];
  let today = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
  let rotation = Rotation::of(&plan, today, &media);
  let mut said = Vec::new();
  let dry = rotate(&plan, &rotation, true, &mut |line| said.push(line.to_owned()));

  assert!(dry.passed(), "{:?}", dry.failed);
  assert!(
    said
      .iter()
      .any(|line| line.contains("would remove from bucket: data-backup/hourly/9/app.pgdump")),
    "{said:?}"
  );
  assert_eq!(ground.in_bucket().len(), 3);

  let ran = rotate(&plan, &rotation, false, &mut |_| {});

  assert!(ran.passed(), "{:?}", ran.failed);
  assert_eq!(ground.in_bucket(), ["data-backup/hourly/1/app.pgdump", "hourly/9/app.pgdump"]);
}
