//! The backup a deployment takes (docs/deploy.md, 「Backups」): the declared snapshot staged, the pack of it
//! and the data root read back as it is written, the check, and what a failure leaves.

use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use bmo_testbed_core::policy::backups::BACKUPS;
use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_deploy::backup::{
  BackupPack, BackupPlan, BackupTaken, Left, Packed, PackedBackup, SNAPSHOT_DIR, file_sha256, left_out,
  pack_backup, take_backup,
};
use bmo_testbed_deploy::progress::Part;
use bmo_testbed_deploy::remove::remove_tree;

// A staged snapshot beside a data root with a status log large enough to be read over many records.
struct LiveRoot {
  _dir: tempfile::TempDir,
  part: PathBuf,
  stage: PathBuf,
  data: PathBuf,
  scratch: PathBuf,
  status: PathBuf,
}

fn live_root() -> LiveRoot {
  let dir = tempfile::tempdir().unwrap();
  let stage = dir.path().join("stage");
  let data = dir.path().join("data");
  let scratch = dir.path().join("scratch");
  let status = data.join("logs").join("status.jsonl");

  std::fs::create_dir_all(stage.join(SNAPSHOT_DIR)).unwrap();
  std::fs::create_dir_all(status.parent().unwrap()).unwrap();
  std::fs::create_dir_all(&scratch).unwrap();
  std::fs::write(stage.join(SNAPSHOT_DIR).join("copy"), "snapshot").unwrap();
  std::fs::write(stage.join(SNAPSHOT_DIR).join("manifest.json"), "{}\n").unwrap();
  std::fs::write(&status, "{\"at\":1,\"payload\":{}}\n".repeat(10_000)).unwrap();

  LiveRoot {
    part: dir.path().join("backup.tar.zst"),
    _dir: dir,
    stage,
    data,
    scratch,
    status,
  }
}

fn pack(root: &LiveRoot, tar: &[&str], left_out: &[String]) -> PackedBackup {
  pack_backup(&BackupPack {
    part: &root.part,
    stage: &root.stage,
    data_root: &root.data,
    scratch: &root.scratch,
    left_out,
    tar,
  })
}

// A tar whose checkpoint action runs at every record it writes, so the status log moves under it every
// time rather than when a race happens to land; its words in the C locale.
fn moving_tar(action: &str) -> [String; 5] {
  [
    "env".to_owned(),
    "LC_ALL=C".to_owned(),
    "tar".to_owned(),
    "--checkpoint=1".to_owned(),
    format!("--checkpoint-action=exec={action}"),
  ]
}

// Every name a pack holds.
fn listing(archive: &Path) -> Vec<String> {
  let listed = Command::new("bash")
    .arg("-c")
    .arg("zstd -d -q -c \"$1\" | tar -tf -")
    .arg("bash")
    .arg(archive)
    .output()
    .unwrap();

  String::from_utf8_lossy(&listed.stdout).lines().map(str::to_owned).collect()
}

fn names_in(folder: &Path) -> Vec<String> {
  let mut names: Vec<String> = std::fs::read_dir(folder)
    .unwrap()
    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
    .collect();

  names.sort();
  names
}

#[test]
fn a_file_that_grows_while_it_is_read_is_a_backup_taken_with_tars_warning_kept() {
  let root = live_root();
  let tar = moving_tar(&format!("printf x >> '{}'", root.status.display()));
  let packed = pack(&root, &tar.iter().map(String::as_str).collect::<Vec<_>>(), &[]);

  assert_eq!(packed.packed, Packed::Moved, "{}", packed.warnings);
  assert!(
    packed.warnings.contains("./logs/status.jsonl: file changed as we read it"),
    "{}",
    packed.warnings
  );
  assert_eq!(packed.read_back, Ok(()));

  let listed = listing(&root.part);

  for name in ["./.backup-snapshot/copy", "./.backup-snapshot/manifest.json", "./logs/status.jsonl"] {
    assert!(listed.iter().any(|one| one == name), "{name} in {listed:?}");
  }
}

#[test]
fn a_file_rewritten_from_empty_while_it_is_read_is_a_backup_taken_too() {
  let root = live_root();
  let tar = moving_tar(&format!(": > '{}'", root.status.display()));
  let packed = pack(&root, &tar.iter().map(String::as_str).collect::<Vec<_>>(), &[]);

  assert_eq!(packed.packed, Packed::Moved, "{}", packed.warnings);
  assert!(packed.warnings.contains("File shrank"), "{}", packed.warnings);
}

#[test]
fn a_data_root_nobody_wrote_to_packs_whole_read_back_into_the_scratch_folder_and_hashed() {
  let root = live_root();
  let packed = pack(&root, &["tar"], &[]);

  assert_eq!(
    packed,
    PackedBackup {
      packed: Packed::Whole,
      warnings: String::new(),
      read_back: Ok(()),
      sha256: file_sha256(&root.part),
    }
  );
  assert_eq!(packed.sha256.len(), 64);
  assert_eq!(names_in(&root.scratch), [SNAPSHOT_DIR]);
  assert_eq!(names_in(&root.scratch.join(SNAPSHOT_DIR)), ["copy", "manifest.json"]);
  assert_eq!(
    std::fs::read_to_string(root.scratch.join(SNAPSHOT_DIR).join("copy")).unwrap(),
    "snapshot"
  );
}

#[test]
fn what_the_plan_leaves_out_is_left_out_of_the_pack() {
  let root = live_root();
  let backups = root.data.parent().unwrap().join("backups");

  for folder in ["cache", "tmp", "kept"] {
    std::fs::create_dir_all(root.data.join(folder)).unwrap();
    std::fs::write(root.data.join(folder).join("file"), folder).unwrap();
  }

  let exclude = [PathBuf::from("cache"), root.data.join("tmp/"), PathBuf::from("/elsewhere/tmp")];
  let plan = BackupPlan {
    data_root: &root.data,
    backup_dir: &backups,
    name: "app.tar.zst",
    snapshot: None,
    verify: None,
    exclude: &exclude,
    cwd: &root.data,
  };
  let leaving = left_out(&plan);

  assert_eq!(leaving, ["cache", "tmp"]);

  let packed = pack(&root, &["tar"], &leaving);

  assert_eq!(packed.packed, Packed::Whole, "{}", packed.warnings);

  let listed = listing(&root.part);

  assert!(listed.iter().any(|one| one == "./kept/file"), "{listed:?}");
  assert!(listed.iter().any(|one| one == "./.backup-snapshot/copy"), "{listed:?}");
  assert_eq!(
    listed
      .iter()
      .filter(|one| ["cache", "tmp"].iter().any(|word| one.contains(word)))
      .collect::<Vec<_>>(),
    Vec::<&String>::new()
  );
}

#[test]
fn a_tar_that_could_not_pack_fails() {
  let mut root = live_root();

  root.data = root.data.join("missing");

  assert_eq!(pack(&root, &["tar"], &[]).packed, Packed::Failed);
}

#[test]
fn a_stream_the_snapshot_cannot_be_taken_back_out_of_does_not_read_back_whole() {
  let root = live_root();
  let packed = pack(&root, &["tar", "--exclude=./.backup-snapshot"], &[]);

  assert_eq!(packed.packed, Packed::Whole, "{}", packed.warnings);
  assert!(
    packed.read_back.as_ref().is_err_and(|said| said.contains(SNAPSHOT_DIR)),
    "{:?}",
    packed.read_back
  );
}

// A data root with one file, and a backup directory beside it.
struct Stand {
  dir: tempfile::TempDir,
  data: PathBuf,
  backups: PathBuf,
}

fn stand() -> Stand {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let backups = dir.path().join("backups");

  std::fs::create_dir_all(&data).unwrap();
  std::fs::write(data.join("kept.txt"), "kept").unwrap();

  Stand { dir, data, backups }
}

const SNAPSHOT: &str =
  "echo copied > \"$BMO_BACKUP_SNAPSHOT_DIR/copy\"; echo '{}' > \"$BMO_BACKUP_SNAPSHOT_DIR/manifest.json\"";
const VERIFY: &str = "test \"$(cat \"$BMO_BACKUP_SNAPSHOT_DIR/copy\")\" = copied";

// A backup taken by the plan, with every part it timed.
fn taken(stand: &Stand, snapshot: Option<&str>, verify: Option<&str>) -> (BackupTaken, Vec<(Part, bool)>) {
  let parts = RefCell::new(vec![]);
  let taken = take_backup(
    &BackupPlan {
      data_root: &stand.data,
      backup_dir: &stand.backups,
      name: "app-20261017-010000.tar.zst",
      snapshot,
      verify,
      exclude: &[],
      cwd: stand.dir.path(),
    },
    &|path| {
      remove_tree(path, &stand.data).unwrap();
    },
    &|part, seconds, ok| {
      assert!(seconds >= 0.0);
      parts.borrow_mut().push((part, ok));
    },
  );

  (taken, parts.into_inner())
}

#[test]
fn a_backup_holds_the_snapshot_and_the_data_root_and_leaves_only_itself() {
  let stand = stand();
  let (taken, parts) = taken(&stand, Some(SNAPSHOT), Some(VERIFY));
  let BackupTaken::Taken {
    path,
    bytes,
    moved,
    warnings,
  } = taken
  else {
    panic!("{taken:?}");
  };

  assert_eq!(parts, [(Part::Snapshot, true), (Part::Pack, true), (Part::Check, true)]);
  assert_eq!(path, stand.backups.join("app-20261017-010000.tar.zst"));
  assert_eq!(bytes, std::fs::metadata(&path).unwrap().len());
  assert!(!moved && warnings.is_empty(), "{warnings}");
  assert_eq!(names_in(&stand.backups), ["app-20261017-010000.tar.zst"]);
  assert_eq!(std::fs::metadata(&stand.backups).unwrap().permissions().mode() & 0o777, 0o700);

  let listed = listing(&path);

  for name in ["./.backup-snapshot/copy", "./.backup-snapshot/manifest.json", "./kept.txt"] {
    assert!(listed.iter().any(|one| one == name), "{name} in {listed:?}");
  }
}

#[test]
fn a_backup_with_no_snapshot_declared_packs_the_data_root_alone() {
  let stand = stand();
  let (taken, parts) = taken(&stand, None, None);

  assert!(matches!(taken, BackupTaken::Taken { .. }), "{taken:?}");
  assert_eq!(parts, [(Part::Snapshot, true), (Part::Pack, true), (Part::Check, true)]);
  assert!(
    listing(&stand.backups.join("app-20261017-010000.tar.zst"))
      .iter()
      .any(|one| one == "./kept.txt")
  );
}

#[test]
fn a_snapshot_that_fails_changes_nothing() {
  let stand = stand();
  let snapshot = "echo 'no database' >&2; exit 4";
  let (taken, parts) = taken(&stand, Some(snapshot), Some(VERIFY));
  let words = &BACKUPS.backup;
  let exited = fill_shown(&words.exited, &[("line", &snapshot), ("code", &4), ("said", &"no database")]);

  assert_eq!(
    taken,
    BackupTaken::Failed {
      failure: fill_shown(&words.no_snapshot, &[("why", &exited)]),
      left: Left::NothingChanged,
      warnings: String::new(),
    }
  );
  assert_eq!(parts, [(Part::Snapshot, false)]);
  assert_eq!(names_in(&stand.backups), Vec::<String>::new());
  assert_eq!(Left::NothingChanged.said(), words.nothing_changed);
  assert_eq!(Left::BackupRemoved.said(), words.backup_removed);
}

#[test]
fn a_backup_that_does_not_restore_is_removed() {
  let stand = stand();
  let verify = "echo 'manifest does not match'; exit 1";
  let (taken, parts) = taken(&stand, Some(SNAPSHOT), Some(verify));
  let BackupTaken::Failed { failure, left, .. } = taken else {
    panic!("{taken:?}");
  };
  let exited = fill_shown(
    &BACKUPS.backup.exited,
    &[("line", &verify), ("code", &1), ("said", &"manifest does not match")],
  );

  assert_eq!(left, Left::BackupRemoved);
  assert_eq!(failure, fill_shown(&BACKUPS.backup.not_restored, &[("why", &exited)]));
  assert_eq!(parts, [(Part::Snapshot, true), (Part::Pack, true), (Part::Check, false)]);
  assert_eq!(names_in(&stand.backups), Vec::<String>::new());
}

#[test]
fn a_data_root_that_is_not_there_is_no_backup() {
  let stand = stand();

  std::fs::remove_dir_all(&stand.data).unwrap();

  let (taken, parts) = taken(&stand, None, None);

  assert!(
    matches!(&taken, BackupTaken::Failed { failure, left: Left::NothingChanged, .. } if failure == "tar could not write the backup"),
    "{taken:?}"
  );
  assert_eq!(parts, [(Part::Snapshot, true), (Part::Pack, false)]);
  assert_eq!(names_in(&stand.backups), Vec::<String>::new());
}
