//! The archives (docs/backup.md, 「What is backed up」): the walk leaves out the excluded paths, the media
//! folders and the large files, and an archive packed reads back with as many entries as it holds.

use std::path::PathBuf;

use bmo_testbed_backup::pack::{Walk, digest, entries, pack};
use glob::Pattern;

use crate::stand::Ground;

#[test]
fn the_walk_leaves_out_exclusions_media_and_large_files() {
  let ground = Ground::new();

  ground.file("data/brain/notes.txt", 10);
  ground.file("data/brain/cache/held.lock", 10);
  ground.file("data/brain/big.bin", 5000);
  ground.file("data/song/one.wav", 10);
  ground.file("data/env/app.env", 10);

  let root = ground.path("root");
  let exclude = [Pattern::new("**/*.lock").unwrap()];
  let apart = [root.join("data/song")];
  let walked = Walk {
    root: &root,
    exclude: &exclude,
    apart: &apart,
    large_over: Some(1000),
  }
  .over(&[
    root.join("data/brain"),
    root.join("data/song"),
    root.join("data/env"),
    root.join("data/gone"),
  ]);
  let held: Vec<PathBuf> = [
    "data",
    "data/brain",
    "data/brain/cache",
    "data/brain/notes.txt",
    "data/env",
    "data/env/app.env",
  ]
  .iter()
  .map(PathBuf::from)
  .collect();

  assert_eq!(walked.held, held);
  assert_eq!(walked.large, [PathBuf::from("data/brain/big.bin")]);
  assert_eq!(walked.large_bytes, 5000);
  assert_eq!(walked.missing, [PathBuf::from("data/gone")]);
}

#[test]
fn an_archive_reads_back_with_as_many_entries_as_it_holds() {
  let ground = Ground::new();

  ground.file("a/one.txt", 100);
  ground.file("a/two.txt", 200);

  let root = ground.path("root");
  let walked = Walk {
    root: &root,
    exclude: &[],
    apart: &[],
    large_over: None,
  }
  .over(&[root.join("a")]);
  let archive = ground.path("a.tar.zst");

  pack(&root, &walked.held, &archive).unwrap();

  assert_eq!(entries(&archive).unwrap(), 3);

  let (size, sha256) = digest(&archive).unwrap();

  assert_eq!(size, std::fs::metadata(&archive).unwrap().len());
  assert_eq!(sha256.len(), 64);
}

#[test]
fn an_archive_that_is_not_whole_does_not_read_back() {
  let ground = Ground::new();
  let archive = ground.path("broken.tar.zst");

  std::fs::write(&archive, b"not an archive").unwrap();

  assert!(entries(&archive).is_err());
}
