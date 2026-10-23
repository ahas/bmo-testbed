//! The stores a destination is reached through (docs/backup.md, 「The settings」): an S3 bucket through the
//! AWS command, and an rclone remote. Each answers the same four asks, so the runs above never know which
//! one they are sending to; a test stands a program in for either command.

use std::path::Path;
use std::process::{Command, Stdio};

use bmo_testbed_core::policy::fill_shown;
use serde_json::Value;

use crate::policy::BACKUP;

/// An object standing at a destination: its key and its size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
  pub key: String,
  pub size: u64,
}

/// What a destination is asked: every key under a prefix, a file put at a key, a key fetched into a file,
/// and a key taken away.
pub trait Store {
  fn list(&self, prefix: &str) -> Result<Vec<Object>, String>;
  fn put(&self, local: &Path, key: &str, sha256: &str) -> Result<(), String>;
  fn get(&self, key: &str, local: &Path) -> Result<(), String>;
  fn remove(&self, key: &str) -> Result<(), String>;
}

/// A text cut to the length the policy quotes.
pub fn quoted(text: &str) -> String {
  text.trim().chars().take(BACKUP.numbers.quoted_chars).collect()
}

/// A command run to its end: its standard output where it ended well, and otherwise its code and what it
/// said, unless its code is one of `fine`, which reads as empty output.
pub fn ran(mut command: Command, what: &str, fine: &[i32]) -> Result<String, String> {
  let done = command
    .stdin(Stdio::null())
    .output()
    .map_err(|why| fill_shown(&BACKUP.words.unstarted, &[("what", &what), ("why", &why)]))?;

  if done.status.success() {
    return Ok(String::from_utf8_lossy(&done.stdout).into_owned());
  }

  if done.status.code().is_some_and(|code| fine.contains(&code)) {
    return Ok(String::new());
  }

  let err = String::from_utf8_lossy(&done.stderr);
  let out = String::from_utf8_lossy(&done.stdout);
  let told = if err.trim().is_empty() { out } else { err };

  Err(fill_shown(
    &BACKUP.words.exited,
    &[("what", &what), ("code", &done.status.code().unwrap_or(-1)), ("said", &quoted(&told))],
  ))
}

/// An S3 bucket, through the AWS command with the profile that holds its credentials and endpoint.
pub struct S3 {
  pub command: String,
  pub bucket: String,
  pub profile: Option<String>,
  pub endpoint: Option<String>,
}

impl S3 {
  fn aws(&self, words: &[&str]) -> Command {
    let mut command = Command::new(&self.command);

    if let Some(profile) = &self.profile {
      command.args(["--profile", profile]);
    }

    if let Some(endpoint) = &self.endpoint {
      command.args(["--endpoint-url", endpoint]);
    }

    command.args(words);
    command
  }

  fn address(&self, key: &str) -> String {
    format!("s3://{}/{key}", self.bucket)
  }
}

impl Store for S3 {
  fn list(&self, prefix: &str) -> Result<Vec<Object>, String> {
    let listed = ran(
      self.aws(&[
        "s3api",
        "list-objects-v2",
        "--bucket",
        &self.bucket,
        "--prefix",
        prefix,
        "--output",
        "json",
      ]),
      "aws s3api list-objects-v2",
      &[],
    )?;

    if listed.trim().is_empty() {
      return Ok(vec![]);
    }

    let read: Value = serde_json::from_str(&listed)
      .map_err(|why| fill_shown(&BACKUP.words.listing_unread, &[("why", &why)]))?;

    Ok(
      read
        .get("Contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|object| {
          Some(Object {
            key: object.get("Key")?.as_str()?.to_owned(),
            size: object.get("Size")?.as_u64()?,
          })
        })
        .collect(),
    )
  }

  fn put(&self, local: &Path, key: &str, sha256: &str) -> Result<(), String> {
    let metadata = format!("sha256={sha256}");

    ran(
      self.aws(&[
        "s3",
        "cp",
        &local.to_string_lossy(),
        &self.address(key),
        "--metadata",
        &metadata,
        "--only-show-errors",
      ]),
      "aws s3 cp",
      &[],
    )
    .map(|_| ())
  }

  fn get(&self, key: &str, local: &Path) -> Result<(), String> {
    ran(
      self.aws(&["s3", "cp", &self.address(key), &local.to_string_lossy(), "--only-show-errors"]),
      "aws s3 cp",
      &[],
    )
    .map(|_| ())
  }

  fn remove(&self, key: &str) -> Result<(), String> {
    ran(self.aws(&["s3", "rm", &self.address(key), "--only-show-errors"]), "aws s3 rm", &[]).map(|_| ())
  }
}

/// An rclone remote and the folder under it, `remote:folder`, its credentials rclone's own.
pub struct Rclone {
  pub command: String,
  pub remote: String,
}

impl Rclone {
  fn address(&self, key: &str) -> String {
    let remote = self.remote.trim_end_matches('/');

    if remote.ends_with(':') { format!("{remote}{key}") } else { format!("{remote}/{key}") }
  }

  fn rclone(&self, words: &[&str]) -> Command {
    let mut command = Command::new(&self.command);

    command.args(words);
    command
  }
}

// What rclone ends with where the folder listed does not exist: nothing stands there.
const RCLONE_NOT_FOUND: i32 = 3;

impl Store for Rclone {
  fn list(&self, prefix: &str) -> Result<Vec<Object>, String> {
    let folder = prefix.trim_end_matches('/');
    let listed = ran(
      self.rclone(&[
        "lsjson",
        "--recursive",
        "--files-only",
        "--no-mimetype",
        "--no-modtime",
        &self.address(folder),
      ]),
      "rclone lsjson",
      &[RCLONE_NOT_FOUND],
    )?;

    if listed.trim().is_empty() {
      return Ok(vec![]);
    }

    let read: Value = serde_json::from_str(&listed)
      .map_err(|why| fill_shown(&BACKUP.words.listing_unread, &[("why", &why)]))?;

    Ok(
      read
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|object| {
          Some(Object {
            key: format!("{folder}/{}", object.get("Path")?.as_str()?),
            size: object.get("Size")?.as_u64()?,
          })
        })
        .collect(),
    )
  }

  fn put(&self, local: &Path, key: &str, _sha256: &str) -> Result<(), String> {
    ran(
      self.rclone(&["copyto", &local.to_string_lossy(), &self.address(key)]),
      "rclone copyto",
      &[],
    )
    .map(|_| ())
  }

  fn get(&self, key: &str, local: &Path) -> Result<(), String> {
    ran(
      self.rclone(&["copyto", &self.address(key), &local.to_string_lossy()]),
      "rclone copyto",
      &[],
    )
    .map(|_| ())
  }

  fn remove(&self, key: &str) -> Result<(), String> {
    ran(self.rclone(&["deletefile", &self.address(key)]), "rclone deletefile", &[]).map(|_| ())
  }
}

/// The bytes free on the disk a path stands on, or none where it cannot be read.
pub fn free_bytes(path: &Path) -> Option<u64> {
  use std::os::unix::ffi::OsStrExt as _;

  let name = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
  // SAFETY: statvfs is plain data, for which all zeroes is a valid value.
  let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
  // SAFETY: the name is a valid C string and the buffer a statvfs the call writes into.
  let read = unsafe { libc::statvfs(name.as_ptr(), &mut stat) };

  (read == 0).then(|| stat.f_bavail.saturating_mul(stat.f_frsize))
}
