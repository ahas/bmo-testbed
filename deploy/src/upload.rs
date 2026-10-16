//! The backups' upload to remote storage and their rotation (docs/deploy.md, 「Sending backups away」):
//! every backup not yet confirmed is sent and read back, the remote keeps the declared number of newest,
//! and the local directory keeps the newest beside any whose copy is not confirmed. The flow works through
//! hands, so a test drives it with nothing leaving the machine.

use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use bmo_testbed_core::policy::backups::BACKUPS;
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_marks;
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::unit::first_chars;

/// What a confirmed backup's receipt is called beside it.
pub fn receipt_name(backup: &str) -> String {
  format!("{backup}.sent")
}

/// The names a repository's backups go by: its declared name, the local second the backup was taken, and
/// an ordinal where a second holds two (`crate::tree::backup_name`).
#[derive(Debug, Clone)]
pub struct BackupNames {
  pattern: Regex,
}

impl BackupNames {
  pub fn new(name: &str) -> Self {
    let pattern = Regex::new(&format!(r"^{}-\d{{8}}-\d{{6}}(?:-\d{{2}})?\.tar\.zst$", regex::escape(name)))
      .expect("an escaped name makes a pattern that reads");

    Self { pattern }
  }

  /// Whether a file name is one of the backups.
  pub fn is_backup(&self, name: &str) -> bool {
    self.pattern.is_match(name)
  }

  /// The backups among the names, newest first; anything that is not a backup name is left out. Every
  /// name shares the declared name ahead of its time, so the names sort as their times do.
  pub fn newest_first<S: AsRef<str>>(&self, names: &[S]) -> Vec<String> {
    let mut backups: Vec<String> = names
      .iter()
      .map(AsRef::as_ref)
      .filter(|name| self.is_backup(name))
      .map(str::to_owned)
      .collect();

    backups.sort_by(|one, other| stamp_of(other).cmp(stamp_of(one)));
    backups
  }
}

fn stamp_of(backup: &str) -> &str {
  backup.strip_suffix(".tar.zst").unwrap_or(backup)
}

/// A file's length and SHA-256, as read whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Digest {
  pub size: u64,
  pub sha256: String,
}

/// What the upload works through: the backup directory, the remote storage, and what is said.
pub trait UploadHands {
  /// Every file name in the backup directory.
  fn local(&mut self) -> Vec<String>;
  /// The checksum a receipt holds, or none where the backup has none.
  fn receipt(&mut self, backup: &str) -> Option<String>;
  fn write_receipt(&mut self, backup: &str, sha256: &str) -> Result<(), String>;
  /// The backup and its receipt removed from the local directory.
  fn remove_local(&mut self, backup: &str) -> Result<(), String>;
  /// The local file read whole.
  fn digest(&mut self, backup: &str) -> Result<Digest, String>;
  /// The names standing remotely.
  fn remote(&mut self) -> Result<Vec<String>, String>;
  fn upload(&mut self, backup: &str, sha256: &str) -> Result<(), String>;
  /// The object read back whole from the remote.
  fn read_back(&mut self, backup: &str) -> Result<Digest, String>;
  fn remove_remote(&mut self, backup: &str) -> Result<(), String>;
  fn say(&mut self, line: &str);
}

/// The step an upload failed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UploadStep {
  List,
  Digest,
  Upload,
  ReadBack,
  RemoveRemote,
  RemoveLocal,
}

impl UploadStep {
  fn name(self) -> &'static str {
    match self {
      Self::List => "list",
      Self::Digest => "digest",
      Self::Upload => "upload",
      Self::ReadBack => "read-back",
      Self::RemoveRemote => "remove-remote",
      Self::RemoveLocal => "remove-local",
    }
  }
}

/// One step that failed: the backup it was for, or none for the listing, and what was said.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UploadFailure {
  pub name: Option<String>,
  pub step: UploadStep,
  pub said: String,
}

/// What one run came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UploadOutcome {
  pub sent: Vec<String>,
  pub removed_remote: Vec<String>,
  pub removed_local: Vec<String>,
  pub failed: Vec<UploadFailure>,
}

fn said(why: &str) -> String {
  first_chars(why.trim(), DEPLOY.numbers.quoted_chars)
}

// A backup's copy confirmed remotely: uploaded where no whole one stands there, read back and held to the
// local file. The step it stopped at, and why, where it was not.
fn confirm(
  hands: &mut impl UploadHands,
  backup: &str,
  present: &mut BTreeSet<String>,
) -> Result<Digest, (UploadStep, String)> {
  let local = hands.digest(backup).map_err(|why| (UploadStep::Digest, why))?;
  // An object of its name may already be whole there, uploaded by a run killed before its receipt or put
  // back by hand; one that does not read back whole is uploaded again.
  let whole = present.contains(backup) && hands.read_back(backup).is_ok_and(|back| back == local);

  if !whole {
    hands.upload(backup, &local.sha256).map_err(|why| (UploadStep::Upload, why))?;
    present.insert(backup.to_owned());

    let back = hands.read_back(backup).map_err(|why| (UploadStep::ReadBack, why))?;

    if back != local {
      hands.remove_remote(backup).map_err(|why| (UploadStep::RemoveRemote, why))?;
      present.remove(backup);

      return Err((
        UploadStep::ReadBack,
        format!(
          "read back {} bytes, sha256 {}, where the local file has {} bytes, sha256 {}",
          back.size, back.sha256, local.size, local.sha256
        ),
      ));
    }
  }

  hands.write_receipt(backup, &local.sha256).map_err(|why| (UploadStep::ReadBack, why))?;

  Ok(local)
}

/// One run, in order: confirm what is not, rotate the remote down to `keep`, then thin the local
/// directory. A failing step is recorded and the run goes on.
pub fn send_backups(hands: &mut impl UploadHands, names: &BackupNames, keep: usize) -> UploadOutcome {
  let mut outcome = UploadOutcome::default();
  let files = hands.local();
  let locals = names.newest_first(&files);

  // A receipt whose backup is gone is what a run killed between the two removals left.
  for file in &files {
    if let Some(backup) = file
      .strip_suffix(".sent")
      .filter(|backup| names.is_backup(backup) && !locals.iter().any(|one| one == backup))
    {
      let _ = hands.remove_local(backup);
    }
  }

  let listed = match hands.remote() {
    Ok(standing) => Some(names.newest_first(&standing)),
    Err(why) => {
      // Nothing can be confirmed or rotated without the listing, so only what is already confirmed leaves
      // the local directory.
      outcome.failed.push(UploadFailure {
        name: None,
        step: UploadStep::List,
        said: said(&why),
      });
      None
    }
  };
  let mut present: BTreeSet<String> = listed.iter().flatten().cloned().collect();
  let newer_remotely = |present: &BTreeSet<String>, backup: &str| {
    present.iter().filter(|other| stamp_of(other) > stamp_of(backup)).count()
  };

  if listed.is_some() {
    for backup in &locals {
      if hands.receipt(backup).is_some() || newer_remotely(&present, backup) >= keep {
        continue;
      }

      match confirm(hands, backup, &mut present) {
        Ok(local) => {
          outcome.sent.push(backup.clone());
          hands.say(&format!("{backup} stands remotely, {} bytes, sha256 {}", local.size, local.sha256));
        }
        Err((step, why)) => {
          let why = said(&why);

          hands.say(&format!("{backup} was not confirmed remotely ({}): {why}", step.name()));
          outcome.failed.push(UploadFailure {
            name: Some(backup.clone()),
            step,
            said: why,
          });
        }
      }
    }

    let standing: Vec<String> = present.iter().cloned().collect();

    for backup in names.newest_first(&standing).into_iter().skip(keep) {
      match hands.remove_remote(&backup) {
        Ok(()) => {
          present.remove(&backup);
          hands.say(&format!("{backup} rotated off the remote"));
          outcome.removed_remote.push(backup);
        }
        Err(why) => outcome.failed.push(UploadFailure {
          name: Some(backup),
          step: UploadStep::RemoveRemote,
          said: said(&why),
        }),
      }
    }
  }

  for backup in locals.iter().skip(BACKUPS.numbers.kept_locally) {
    if hands.receipt(backup).is_none() && newer_remotely(&present, backup) < keep {
      continue;
    }

    match hands.remove_local(backup) {
      Ok(()) => {
        hands.say(&format!("{backup} removed from the local disk"));
        outcome.removed_local.push(backup.clone());
      }
      Err(why) => outcome.failed.push(UploadFailure {
        name: Some(backup.clone()),
        step: UploadStep::RemoveLocal,
        said: said(&why),
      }),
    }
  }

  outcome
}

/// The detail of the `backup-upload-failed` event (docs/boundary.md): each failed step with its backup.
pub fn upload_failed_detail(outcome: &UploadOutcome) -> Value {
  json!({ "backups": outcome.failed })
}

/// The summary line of the `backup-upload-failed` event.
pub fn upload_failed_summary(outcome: &UploadOutcome) -> String {
  format!(
    "{} {}",
    fill_marks(&BACKUPS.upload_failed, &[("count", &outcome.failed.len().to_string())]),
    BACKUPS.upload_failed_note
  )
}

/// The usage of the `backups` verb, for the remote and the number kept there.
pub fn upload_usage(remote: &str, keep: u64) -> Vec<String> {
  let lines = BACKUPS
    .upload_usage
    .iter()
    .map(|line| fill_marks(line, &[("remote", remote), ("keep", &keep.to_string())]));

  ["usage: bmo-testbed backups".to_owned(), String::new()]
    .into_iter()
    .chain(lines)
    .collect()
}

// A stream read whole into its length and SHA-256.
fn digest_of(mut stream: impl std::io::Read) -> std::io::Result<Digest> {
  let mut hasher = Sha256::new();
  let mut size = 0u64;
  let mut chunk = vec![0u8; 1 << 20];

  loop {
    let read = stream.read(&mut chunk)?;

    if read == 0 {
      break;
    }

    hasher.update(&chunk[..read]);
    size += read as u64;
  }

  Ok(Digest {
    size,
    sha256: format!("{:x}", hasher.finalize()),
  })
}

/// Where the backups stand remotely: an S3 bucket and the prefix under it, read off the declared address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
  pub bucket: String,
  /// Empty, or ending in `/`.
  pub prefix: String,
}

impl Remote {
  /// The declared `s3://bucket/prefix` address read; why it does not read as one where it does not.
  pub fn parse(address: &str) -> Result<Self, String> {
    let read =
      url::Url::parse(address).map_err(|why| format!("the remote {address} does not read: {why}"))?;
    let bucket = read.host_str().filter(|_| read.scheme() == "s3").unwrap_or_default();

    if bucket.is_empty() {
      return Err(format!("the remote {address} is no s3://bucket address"));
    }

    let prefix = read.path().trim_matches('/');

    Ok(Self {
      bucket: bucket.to_owned(),
      prefix: if prefix.is_empty() { String::new() } else { format!("{prefix}/") },
    })
  }

  fn object(&self, backup: &str) -> String {
    format!("s3://{}/{}{backup}", self.bucket, self.prefix)
  }
}

/// The machine's own hands: the backup directory on disk and the remote through the AWS command with the
/// declared profile.
pub struct AwsHands {
  pub dir: PathBuf,
  pub remote: Remote,
  pub profile: Option<String>,
  /// Where each line said goes.
  pub say: Box<dyn FnMut(&str)>,
}

impl AwsHands {
  fn aws(&self, words: &[&str]) -> Command {
    let mut command = Command::new("aws");

    if let Some(profile) = &self.profile {
      command.args(["--profile", profile]);
    }

    command.args(words).stdin(Stdio::null());
    command
  }
}

// A command run to its end, its standard output where it ended well.
fn ran(mut command: Command, what: &str) -> Result<String, String> {
  let done = command.output().map_err(|why| format!("{what} could not be started: {why}"))?;

  if done.status.success() {
    return Ok(String::from_utf8_lossy(&done.stdout).into_owned());
  }

  let err = String::from_utf8_lossy(&done.stderr);
  let out = String::from_utf8_lossy(&done.stdout);
  let told = if err.trim().is_empty() { out.trim() } else { err.trim() };

  Err(format!("{what} exited {}: {}", done.status.code().unwrap_or(-1), said(told)))
}

impl UploadHands for AwsHands {
  fn local(&mut self) -> Vec<String> {
    std::fs::read_dir(&self.dir)
      .into_iter()
      .flatten()
      .flatten()
      .map(|entry| entry.file_name().to_string_lossy().into_owned())
      .collect()
  }

  fn receipt(&mut self, backup: &str) -> Option<String> {
    std::fs::read_to_string(self.dir.join(receipt_name(backup)))
      .ok()
      .map(|text| text.trim().to_owned())
  }

  fn write_receipt(&mut self, backup: &str, sha256: &str) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    std::fs::OpenOptions::new()
      .write(true)
      .create(true)
      .truncate(true)
      .mode(0o600)
      .open(self.dir.join(receipt_name(backup)))
      .and_then(|mut file| writeln!(file, "{sha256}"))
      .map_err(|why| why.to_string())
  }

  fn remove_local(&mut self, backup: &str) -> Result<(), String> {
    for path in [self.dir.join(backup), self.dir.join(receipt_name(backup))] {
      match std::fs::remove_file(&path) {
        Err(why) if why.kind() != std::io::ErrorKind::NotFound => {
          return Err(format!("{}: {why}", path.display()));
        }
        _ => {}
      }
    }

    Ok(())
  }

  fn digest(&mut self, backup: &str) -> Result<Digest, String> {
    let path = self.dir.join(backup);
    let digest = std::fs::File::open(&path).and_then(digest_of).map_err(|why| why.to_string())?;
    let size = std::fs::metadata(&path).map_err(|why| why.to_string())?.len();

    if digest.size != size {
      return Err("the file changed while it was read".into());
    }

    Ok(digest)
  }

  fn remote(&mut self) -> Result<Vec<String>, String> {
    let listed = ran(
      self.aws(&[
        "s3api",
        "list-objects-v2",
        "--bucket",
        &self.remote.bucket,
        "--prefix",
        &self.remote.prefix,
        "--output",
        "json",
      ]),
      "aws s3api list-objects-v2",
    )?;

    if listed.trim().is_empty() {
      return Ok(vec![]);
    }

    let read: Value =
      serde_json::from_str(&listed).map_err(|why| format!("the listing could not be read: {why}"))?;

    Ok(
      read
        .get("Contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|object| object.get("Key").and_then(Value::as_str))
        .map(|key| key.strip_prefix(&self.remote.prefix).unwrap_or(key).to_owned())
        .collect(),
    )
  }

  fn upload(&mut self, backup: &str, sha256: &str) -> Result<(), String> {
    let local = self.dir.join(backup);
    let metadata = format!("sha256={sha256}");

    ran(
      self.aws(&[
        "s3",
        "cp",
        &local.to_string_lossy(),
        &self.remote.object(backup),
        "--metadata",
        &metadata,
        "--only-show-errors",
      ]),
      "aws s3 cp",
    )
    .map(|_| ())
  }

  fn read_back(&mut self, backup: &str) -> Result<Digest, String> {
    let object = self.remote.object(backup);
    let mut reading = self
      .aws(&["s3", "cp", &object, "-"])
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn()
      .map_err(|why| format!("aws could not be started: {why}"))?;
    let digest = reading.stdout.take().map(digest_of);
    let mut err = String::new();

    if let Some(mut stream) = reading.stderr.take() {
      let _ = stream.read_to_string(&mut err);
    }

    let status = reading.wait().map_err(|why| why.to_string())?;

    if !status.success() {
      return Err(format!("reading {object} back exited {}: {}", status.code().unwrap_or(-1), said(&err)));
    }

    match digest {
      Some(Ok(digest)) => Ok(digest),
      Some(Err(why)) => Err(why.to_string()),
      None => Err(format!("reading {object} back gave no output")),
    }
  }

  fn remove_remote(&mut self, backup: &str) -> Result<(), String> {
    ran(self.aws(&["s3", "rm", &self.remote.object(backup), "--only-show-errors"]), "aws s3 rm").map(|_| ())
  }

  fn say(&mut self, line: &str) {
    (self.say)(line);
  }
}
