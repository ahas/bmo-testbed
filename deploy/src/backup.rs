//! The backup a deployment takes before it changes anything (docs/deploy.md, 「Backups」): the declared
//! snapshot, the pack of it and the rest of the data root read back as it is written, and the check that
//! the file on disk is the stream that read back and that the snapshot taken back out holds to its
//! manifest. Each of the three is timed apart and told as it ends; nothing here decides what the
//! deployment does next.

use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use bmo_testbed_core::policy::backups::BACKUPS;
use bmo_testbed_core::policy::deploy::DEPLOY;

use crate::progress::Part;
use crate::run::{command, shell_words};
use crate::unit::first_chars;

/// The folder the snapshot stands in, in the staging folder, in the archive and in the read-back.
pub const SNAPSHOT_DIR: &str = ".backup-snapshot";

/// The variable naming the folder the declared snapshot command writes into and the verify command reads.
pub const SNAPSHOT_VARIABLE: &str = "BMO_BACKUP_SNAPSHOT_DIR";

/// What a backup's tar run came to. `Moved` is a pack finished while some file of the data root changed,
/// shrank or vanished under it: tar says so with exit code 1 and writes the file as it stood, which is a
/// backup taken, since what must be exact comes from the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packed {
  Whole,
  Moved,
  Failed,
}

/// What one pack came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedBackup {
  pub packed: Packed,
  /// What the pack said: tar of a file that moved under it, or any process of it why it failed.
  pub warnings: String,
  /// Whether the stream decompressed whole as it was written, its tar taking the snapshot out; what the
  /// reading side said where it did not.
  pub read_back: Result<(), String>,
  /// The SHA-256 of every byte written, in hex; empty where it could not be taken.
  pub sha256: String,
}

// The SHA-256 a `sha256sum` run printed, ahead of the name it read; empty where it printed none.
fn digest_said(out: &str) -> String {
  let digest = out.split_whitespace().next().unwrap_or_default();

  if digest.len() == 64 && digest.chars().all(|one| one.is_ascii_hexdigit() && !one.is_ascii_uppercase()) {
    digest.to_owned()
  } else {
    String::new()
  }
}

// The pack as one pipeline the shell joins, since it has two links that carry the whole data root
// uncompressed. Its arguments are the archive, the FIFO, the scratch folder, the snapshot folder's name,
// the level, then the packing tar's whole command, and its exit codes come in pipeline order: tar, zstd,
// tee, the reading zstd, the reading tar.
const PIPELINE: &str = "part=$1 stream=$2 scratch=$3 snapshot=$4 level=$5
shift 5
\"$@\" | zstd -q -T0 \"-$level\" -c | tee -p \"$part\" \"$stream\" | zstd -d -q -c | tar -xf - -C \"$scratch\" \"$snapshot\"
codes=\"${PIPESTATUS[*]}\"
printf '\\n%s\\n' \"$codes\"";

/// What the pack is handed.
pub struct BackupPack<'a> {
  /// The archive to write.
  pub part: &'a Path,
  /// The staging folder holding the snapshot folder.
  pub stage: &'a Path,
  pub data_root: &'a Path,
  /// An empty folder the read-back takes the snapshot out into.
  pub scratch: &'a Path,
  /// What the pack of the data root leaves out, as paths under it.
  pub left_out: &'a [String],
  /// The packing tar ahead of what the pack names: the program, and any options of its own.
  pub tar: &'a [&'a str],
}

/// Packs the snapshot and the rest of the data root into one archive and reads it back on the way. tar's
/// stream goes through zstd on every core into `tee`, which writes the archive, hands every byte to
/// `sha256sum` through a FIFO beside it, and passes the stream on to a second zstd that decompresses it
/// into a second tar, which reads to the end of the stream and takes the snapshot out.
pub fn pack_backup(pack: &BackupPack<'_>) -> PackedBackup {
  let snapshot = format!("./{SNAPSHOT_DIR}");
  let stream = PathBuf::from(format!("{}.stream", pack.part.display()));
  let failed = |warnings: String| PackedBackup {
    packed: Packed::Failed,
    warnings,
    read_back: Err(String::new()),
    sha256: String::new(),
  };

  let _ = std::fs::remove_file(&stream);

  if !Command::new("mkfifo")
    .args(["-m", "600"])
    .arg(&stream)
    .status()
    .is_ok_and(|status| status.success())
  {
    return failed(format!("mkfifo {} failed", stream.display()));
  }

  let hashing = Command::new("sha256sum")
    .arg(&stream)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn();
  let Ok(hashing) = hashing else {
    let _ = std::fs::remove_file(&stream);

    return failed("sha256sum could not be started".to_owned());
  };
  let mut tar: Vec<String> = pack.tar.iter().map(|word| (*word).to_owned()).collect();

  tar.extend([
    "-cf".to_owned(),
    "-".to_owned(),
    "-C".to_owned(),
    pack.stage.to_string_lossy().into_owned(),
    snapshot.clone(),
    "-C".to_owned(),
    pack.data_root.to_string_lossy().into_owned(),
  ]);
  tar.extend(pack.left_out.iter().map(|path| format!("--exclude=./{path}")));
  tar.push(".".to_owned());

  let piped = Command::new("bash")
    .arg("-c")
    .arg(PIPELINE)
    .arg("bash")
    .arg(pack.part)
    .arg(&stream)
    .arg(pack.scratch)
    .arg(&snapshot)
    .arg(BACKUPS.numbers.pack_level.to_string())
    .args(&tar)
    .stdin(Stdio::null())
    .output();
  let hashed = match &piped {
    Ok(_) => hashing.wait_with_output(),
    Err(_) => {
      // Nothing will open the FIFO for writing, so the hash waiting on it is let go.
      let mut hashing = hashing;
      let _ = hashing.kill();

      hashing.wait_with_output()
    }
  };

  let _ = std::fs::remove_file(&stream);

  let Ok(piped) = piped else {
    return failed("the pack's pipeline could not be started".to_owned());
  };
  let out = String::from_utf8_lossy(&piped.stdout);
  // The pipeline's last line is the exit codes the shell gathered, one space between each.
  let codes: Vec<i32> = out
    .trim_end()
    .lines()
    .last()
    .unwrap_or_default()
    .split(' ')
    .filter_map(|code| code.parse().ok())
    .collect();
  let [tar_code, compress, tee, decompress, extract] =
    codes.as_slice().try_into().unwrap_or([2, 2, 2, 2, 2]);
  let (sha256, hash_said) = match hashed {
    Ok(done) if done.status.success() => (
      digest_said(&String::from_utf8_lossy(&done.stdout)),
      String::from_utf8_lossy(&done.stderr).into_owned(),
    ),
    Ok(done) => (String::new(), String::from_utf8_lossy(&done.stderr).into_owned()),
    Err(why) => (String::new(), why.to_string()),
  };
  let warnings = [String::from_utf8_lossy(&piped.stderr).trim().to_owned(), hash_said.trim().to_owned()]
    .into_iter()
    .filter(|text| !text.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
  let packed = if tar_code > 1 || compress != 0 || tee != 0 || sha256.is_empty() {
    Packed::Failed
  } else if tar_code == 1 {
    Packed::Moved
  } else {
    Packed::Whole
  };
  let read_back = if decompress == 0 && extract == 0 {
    Ok(())
  } else if warnings.is_empty() {
    Err(format!("zstd -d exit {decompress}, tar -x exit {extract}"))
  } else {
    Err(warnings.clone())
  };

  PackedBackup {
    packed,
    warnings,
    read_back,
    sha256,
  }
}

/// A file's SHA-256, in hex, read from the disk; empty where it could not be read.
pub fn file_sha256(path: &Path) -> String {
  match Command::new("sha256sum").arg(path).stdin(Stdio::null()).stderr(Stdio::null()).output() {
    Ok(done) if done.status.success() => digest_said(&String::from_utf8_lossy(&done.stdout)),
    _ => String::new(),
  }
}

/// What the backup is taken of and where it goes.
pub struct BackupPlan<'a> {
  pub data_root: &'a Path,
  pub backup_dir: &'a Path,
  /// The backup's file name in the backup directory.
  pub name: &'a str,
  /// The declared snapshot command, run with the snapshot folder named.
  pub snapshot: Option<&'a str>,
  /// The declared verify command, run with the snapshot taken back out named.
  pub verify: Option<&'a str>,
  /// The declared paths under the data root the pack leaves out.
  pub exclude: &'a [PathBuf],
  /// Where the declared commands run.
  pub cwd: &'a Path,
}

/// What a failed backup left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Left {
  NothingChanged,
  BackupRemoved,
}

impl Left {
  /// What was left, as the deploy output says it.
  pub fn said(self) -> &'static str {
    match self {
      Self::NothingChanged => "nothing was changed.",
      Self::BackupRemoved => "the backup was removed and nothing else was changed.",
    }
  }
}

/// What the backup came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupTaken {
  Taken {
    path: PathBuf,
    bytes: u64,
    moved: bool,
    warnings: String,
  },
  Failed {
    failure: String,
    left: Left,
    warnings: String,
  },
}

/// What the pack leaves out, as paths under the data root: the declared exclusions, an absolute one taken
/// as its part under the data root. A backup directory never stands inside the data root, which the
/// deployment refuses before it takes a backup.
pub fn left_out(plan: &BackupPlan<'_>) -> Vec<String> {
  let under = |path: &Path| {
    let path = if path.is_absolute() { path.strip_prefix(plan.data_root).ok()? } else { path };
    let path = path.to_string_lossy().trim_end_matches('/').to_owned();

    (!path.is_empty()).then_some(path)
  };

  plan.exclude.iter().filter_map(|path| under(path)).collect()
}

// A declared command run to its end in the plan's place with the snapshot folder named; what it said
// where it did not end well.
fn declared(line: &str, cwd: &Path, folder: &Path) -> Result<(), String> {
  let done = command(&shell_words(line), Some(cwd))
    .env(SNAPSHOT_VARIABLE, folder)
    .output()
    .map_err(|why| format!("`{line}` could not be started: {why}"))?;

  if done.status.success() {
    return Ok(());
  }

  let said = format!("{}{}", String::from_utf8_lossy(&done.stdout), String::from_utf8_lossy(&done.stderr));

  Err(format!(
    "`{line}` exited {}: {}",
    done.status.code().unwrap_or(-1),
    first_chars(said.trim(), DEPLOY.numbers.quoted_chars)
  ))
}

/// The deployment's backup, in its three parts: the declared snapshot written into a staging folder; the
/// pack, read back as it is written; and the check — the file on disk hashed again against the stream
/// that read back, and the snapshot taken out held to its manifest by the declared verify command. A
/// backup that fails any of them is removed with everything it made. `remove` takes a path away by the
/// guarded removal; `timed` is told each part as it ends, with its seconds and whether it did what it is
/// for.
pub fn take_backup(
  plan: &BackupPlan<'_>,
  remove: &dyn Fn(&Path),
  timed: &dyn Fn(Part, f64, bool),
) -> BackupTaken {
  let pid = std::process::id();
  let stage = plan.backup_dir.join(format!(".staging-{pid}"));
  let scratch = plan.backup_dir.join(format!(".verify-{pid}"));
  let part = plan.backup_dir.join(format!("{}.part", plan.name));
  let finished = plan.backup_dir.join(plan.name);
  let failed = |failure: String, left: Left, warnings: String| {
    remove(&stage);
    remove(&scratch);
    remove(&part);

    if left == Left::BackupRemoved {
      remove(&finished);
    }

    BackupTaken::Failed {
      failure,
      left,
      warnings,
    }
  };
  let private = |folder: &Path| std::fs::DirBuilder::new().recursive(true).mode(0o700).create(folder);

  // The backup directory holds the env files the data root carries, so it is the user's alone.
  if let Err(why) = private(plan.backup_dir) {
    return failed(
      format!("{} could not be made: {why}", plan.backup_dir.display()),
      Left::NothingChanged,
      String::new(),
    );
  }

  remove(&stage);
  remove(&scratch);

  for folder in [&stage.join(SNAPSHOT_DIR), &scratch] {
    if let Err(why) = private(folder) {
      return failed(
        format!("{} could not be made: {why}", folder.display()),
        Left::NothingChanged,
        String::new(),
      );
    }
  }

  let started = Instant::now();

  if let Some(line) = plan.snapshot
    && let Err(why) = declared(line, plan.cwd, &stage.join(SNAPSHOT_DIR))
  {
    timed(Part::Snapshot, started.elapsed().as_secs_f64(), false);

    return failed(format!("the snapshot could not be taken: {why}"), Left::NothingChanged, String::new());
  }

  timed(Part::Snapshot, started.elapsed().as_secs_f64(), true);

  let started = Instant::now();
  let packed = pack_backup(&BackupPack {
    part: &part,
    stage: &stage,
    data_root: plan.data_root,
    scratch: &scratch,
    left_out: &left_out(plan),
    tar: &["tar"],
  });

  // The staging copy has served its purpose the moment the pack is written, and it goes now: a whole
  // uncompressed snapshot left behind would be one nothing ever takes away.
  remove(&stage);
  timed(
    Part::Pack,
    started.elapsed().as_secs_f64(),
    packed.packed != Packed::Failed && packed.read_back.is_ok(),
  );

  if packed.packed == Packed::Failed {
    return failed("tar could not write the backup".to_owned(), Left::NothingChanged, packed.warnings);
  }

  if let Err(said) = &packed.read_back {
    return failed(
      format!(
        "the backup {} did not read back whole: {}",
        plan.name,
        first_chars(said, DEPLOY.numbers.quoted_chars)
      ),
      Left::BackupRemoved,
      packed.warnings,
    );
  }

  if let Err(why) = std::fs::rename(&part, &finished) {
    return failed(
      format!("the backup could not be put in place: {why}"),
      Left::BackupRemoved,
      packed.warnings,
    );
  }

  let started = Instant::now();
  let checked = if file_sha256(&finished) != packed.sha256 {
    Err("the file on disk is not the stream that read back".to_owned())
  } else if let Some(line) = plan.verify {
    declared(line, plan.cwd, &scratch.join(SNAPSHOT_DIR))
  } else {
    Ok(())
  };

  if let Err(why) = checked {
    timed(Part::Check, started.elapsed().as_secs_f64(), false);

    return failed(format!("the backup did not restore: {why}"), Left::BackupRemoved, packed.warnings);
  }

  remove(&scratch);
  timed(Part::Check, started.elapsed().as_secs_f64(), true);

  BackupTaken::Taken {
    bytes: std::fs::metadata(&finished).map(|meta| meta.len()).unwrap_or(0),
    path: finished,
    moved: packed.packed == Packed::Moved,
    warnings: packed.warnings,
  }
}
