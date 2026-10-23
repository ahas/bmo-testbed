//! The archives (docs/backup.md, 「What is backed up」): the paths under the root a set holds, walked with
//! the exclusions, the media folders and the large files left out, packed as one tar stream through zstd;
//! an archive read through and counted; a file's size and SHA-256.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::policy::fill_shown;
use glob::{MatchOptions, Pattern};
use sha2::{Digest as _, Sha256};

use crate::destination::quoted;
use crate::policy::BACKUP;

/// What one walk leaves out, and the size over which a file is large.
pub struct Walk<'a> {
  pub root: &'a Path,
  pub exclude: &'a [Pattern],
  /// Folders whose whole tree is left out: the media sets'.
  pub apart: &'a [PathBuf],
  /// A file over this is not held but named among the large; none holds every file.
  pub large_over: Option<u64>,
}

/// What a walk found: the paths to hold, read from the root, every folder ahead of what it holds; the
/// large files left out, the same way; and the declared paths that do not exist.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Walked {
  pub held: Vec<PathBuf>,
  pub large: Vec<PathBuf>,
  pub large_bytes: u64,
  pub missing: Vec<PathBuf>,
}

const MATCH: MatchOptions = MatchOptions {
  case_sensitive: true,
  require_literal_separator: true,
  require_literal_leading_dot: false,
};

impl Walk<'_> {
  fn left_out(&self, path: &Path, relative: &Path) -> bool {
    self.apart.iter().any(|apart| path.starts_with(apart))
      || self.exclude.iter().any(|glob| glob.matches_path_with(relative, MATCH))
  }

  fn visit(&self, path: &Path, walked: &mut Walked) {
    let Ok(relative) = path.strip_prefix(self.root) else {
      return;
    };

    if self.left_out(path, relative) {
      return;
    }

    let Ok(meta) = std::fs::symlink_metadata(path) else {
      walked.missing.push(relative.to_path_buf());
      return;
    };

    if meta.is_dir() {
      walked.held.push(relative.to_path_buf());

      let mut children: Vec<PathBuf> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();

      children.sort();

      for child in children {
        self.visit(&child, walked);
      }
    } else if meta.is_file() && self.large_over.is_some_and(|over| meta.len() > over) {
      walked.large.push(relative.to_path_buf());
      walked.large_bytes += meta.len();
    } else {
      walked.held.push(relative.to_path_buf());
    }
  }

  /// The paths under the root each of the folders reaches.
  pub fn over(&self, folders: &[PathBuf]) -> Walked {
    let mut walked = Walked::default();

    for folder in folders {
      // The folders above a declared one go in as folders alone, so the archive restores their modes.
      if let Ok(relative) = folder.strip_prefix(self.root) {
        for above in relative.ancestors().skip(1).filter(|one| !one.as_os_str().is_empty()) {
          if !walked.held.iter().any(|held| held == above) {
            walked.held.push(above.to_path_buf());
          }
        }
      }

      self.visit(folder, &mut walked);
    }

    walked
  }
}

/// The paths packed from the root into the archive, as one tar stream through zstd. A file that changes
/// or goes while tar reads it goes in as it stood, or not at all.
pub fn pack(root: &Path, paths: &[PathBuf], archive: &Path) -> Result<(), String> {
  use std::os::unix::ffi::OsStrExt as _;

  let compress = format!("zstd -T0 -{} -q", BACKUP.numbers.pack_level);
  let mut tar = Command::new("tar")
    .arg("--create")
    .arg("--file")
    .arg(archive)
    .arg("--use-compress-program")
    .arg(&compress)
    .arg("--directory")
    .arg(root)
    .args(["--no-recursion", "--null", "--ignore-failed-read", "--warning=no-file-changed"])
    .args(["--warning=no-file-removed", "--warning=no-file-shrank", "--files-from=-"])
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|why| fill_shown(&BACKUP.words.unstarted, &[("what", &"tar"), ("why", &why)]))?;

  let mut list: Vec<u8> = Vec::new();

  for path in paths {
    list.extend_from_slice(path.as_os_str().as_bytes());
    list.push(0);
  }

  // The list is written from a thread of its own, so what tar says meanwhile never fills its pipe.
  let writer = tar.stdin.take().map(|mut stdin| std::thread::spawn(move || stdin.write_all(&list)));
  let mut err = String::new();

  if let Some(mut stream) = tar.stderr.take() {
    let _ = stream.read_to_string(&mut err);
  }

  if let Some(writer) = writer {
    let _ = writer.join();
  }

  let status = tar.wait().map_err(|why| why.to_string())?;

  // tar ends 1 where a file changed as it was read, which went in as it stood.
  match status.code() {
    Some(0 | 1) => Ok(()),
    code => Err(fill_shown(
      &BACKUP.words.exited,
      &[("what", &"tar"), ("code", &code.unwrap_or(-1)), ("said", &quoted(&err))],
    )),
  }
}

/// How many entries an archive holds, read through whole, which fails where it does not decompress or
/// read as tar to its end.
pub fn entries(archive: &Path) -> Result<u64, String> {
  let listed = Command::new("tar")
    .arg("--list")
    .arg("--file")
    .arg(archive)
    .arg("--use-compress-program")
    .arg("zstd -d -q")
    .stdin(Stdio::null())
    .output()
    .map_err(|why| fill_shown(&BACKUP.words.unstarted, &[("what", &"tar"), ("why", &why)]))?;

  if !listed.status.success() {
    return Err(fill_shown(
      &BACKUP.words.exited,
      &[
        ("what", &"tar --list"),
        ("code", &listed.status.code().unwrap_or(-1)),
        ("said", &quoted(&String::from_utf8_lossy(&listed.stderr))),
      ],
    ));
  }

  // tar writes one name to a line, a line break inside a name escaped.
  Ok(listed.stdout.iter().filter(|byte| **byte == b'\n').count() as u64)
}

/// A file's size and SHA-256, read whole.
pub fn digest(path: &Path) -> Result<(u64, String), String> {
  let mut file = std::fs::File::open(path).map_err(|why| format!("{}: {why}", path.display()))?;
  let mut hasher = Sha256::new();
  let mut size = 0u64;
  let mut chunk = vec![0u8; 1 << 20];

  loop {
    let read = file.read(&mut chunk).map_err(|why| format!("{}: {why}", path.display()))?;

    if read == 0 {
      break;
    }

    hasher.update(&chunk[..read]);
    size += read as u64;
  }

  Ok((size, format!("{:x}", hasher.finalize())))
}
