//! The weekly media (docs/backup.md, 「How the media run runs」): each media set and the large files packed
//! under a name that stays the same, sent to every destination, removed locally once sent; and a
//! destination whose disk would fall under the floor is sent nothing.

use bmo_testbed_backup::media::media;

use crate::stand::Ground;

#[test]
fn each_media_set_and_the_large_files_go_out_under_their_own_names() {
  let ground = Ground::new();

  ground.file("data/song/one.wav", 10);
  ground.file("data/brain/big.bin", 5000);
  ground.file("data/brain/small.txt", 10);

  let plan = ground.plan(&ground.settings(
    "folders = [\"data/brain\"]\ndatabases = []\nlarge_file_bytes = 1000\n\
     [[media]]\nname = \"song\"\nfolders = [\"data/song\"]",
  ));
  let mut said = Vec::new();
  let outcome = media(&plan, &mut |line| said.push(line.to_owned()));

  assert!(outcome.passed(), "{:?} {said:?}", outcome.failed);
  assert_eq!(
    ground.in_bucket(),
    ["data-backup/media/large-files.tar.zst", "data-backup/media/song.tar.zst"]
  );
  assert_eq!(
    ground.on_remote(),
    ["data-backup/media/large-files.tar.zst", "data-backup/media/song.tar.zst"]
  );
  assert_eq!(std::fs::read_dir(ground.path("staging/media")).unwrap().count(), 0);
}

#[test]
fn a_destination_whose_disk_would_fall_under_the_floor_is_sent_nothing() {
  let ground = Ground::new();

  ground.file("data/song/one.wav", 10);

  let text = ground
    .settings("folders = []\ndatabases = []\nroom_floor_bytes = 9223372036854775807\n[[media]]\nname = \"song\"\nfolders = [\"data/song\"]")
    .replacen("kind = \"s3\"", &format!("kind = \"s3\"\nroom = \"{}\"", ground.path("s3").display()), 1);
  let plan = ground.plan(&text);
  let outcome = media(&plan, &mut |_| {});

  assert_eq!(outcome.failed.len(), 1, "{:?}", outcome.failed);
  assert_eq!(outcome.failed[0].step, "room");
  assert!(ground.in_bucket().is_empty());
  assert_eq!(ground.on_remote(), ["data-backup/media/song.tar.zst"]);
}
