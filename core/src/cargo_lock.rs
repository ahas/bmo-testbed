//! A Cargo lock's path packages brought to their manifests before a command that holds the lock
//! (docs/deploy.md, 「The Cargo build directory」). A package read from a path carries in the lock the version
//! its manifest names, and a sibling repository moves that version with commits of its own, so `--locked`
//! would refuse a lock nothing but a sibling's commit made stale. Cargo resolves the workspace offline and
//! writes the lock; what it wrote is kept for the command only where the path packages alone moved, and the
//! lock is put back byte for byte otherwise, so every package from a registry or a git source stays held as
//! it stood. Once the command ends the lock is put back as it was found, so the checkout's tracked lock never
//! changes under it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use toml::{Table, Value};

use crate::policy::fill_marks;
use crate::policy::suite::SUITE;

// The words that make a Cargo command hold its lock as it stands.
const HOLDING: [&str; 2] = ["--locked", "--frozen"];

/// One path package the lock gave another entry: its name, and its version before and after, none where
/// the package was not in the lock on that side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathMove {
  pub name: String,
  pub from: Option<String>,
  pub to: Option<String>,
}

/// A lock whose path packages were brought to their manifests: where it stands, what moved, and its bytes
/// as they were found and as they were written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aligned {
  pub lock: PathBuf,
  pub moves: Vec<PathMove>,
  found: Vec<u8>,
  written: Vec<u8>,
}

impl Aligned {
  /// The lock put back as it was found, once the command it was aligned for has ended. A lock something
  /// else has written since is left as that left it.
  pub fn put_back(&self) {
    if std::fs::read(&self.lock).is_ok_and(|now| now == self.written) {
      let _ = std::fs::write(&self.lock, &self.found);
    }
  }

  /// The one line that says which path packages moved.
  pub fn said(&self) -> String {
    let words = &SUITE.cargo;
    let version = |side: &Option<String>| side.clone().unwrap_or_else(|| words.path_absent.clone());
    let moves: Vec<String> = self
      .moves
      .iter()
      .map(|one| {
        fill_marks(
          &words.path_moved,
          &[("name", &one.name), ("from", &version(&one.from)), ("to", &version(&one.to))],
        )
      })
      .collect();

    fill_marks(
      &words.path_aligned,
      &[("lock", &self.lock.to_string_lossy()), ("moves", &moves.join(", "))],
    )
  }
}

/// Whether a Cargo command holds its lock as it stands, by the words before any `--`.
pub fn holds_lock(args: &[String]) -> bool {
  args
    .iter()
    .take_while(|word| *word != "--")
    .any(|word| HOLDING.contains(&word.as_str()))
}

/// The path packages that moved between two texts of one lock, where nothing else moved: every package from
/// a registry or a git source, and every table but the packages, stand alike in both. None where anything
/// else moved or either text does not read as a lock.
pub fn path_moves(before: &str, after: &str) -> Option<Vec<PathMove>> {
  let before: Table = before.parse().ok()?;
  let after: Table = after.parse().ok()?;
  let rest = |lock: &Table| {
    let mut rest = lock.clone();

    rest.remove("package");
    rest
  };

  if rest(&before) != rest(&after) {
    return None;
  }

  let (before_sourced, before_paths) = split_packages(&before)?;
  let (after_sourced, after_paths) = split_packages(&after)?;

  if before_sourced != after_sourced {
    return None;
  }

  let names: BTreeSet<&String> =
    before_paths.iter().chain(after_paths.iter()).map(|(name, _)| name).collect();
  let version = |package: Option<&Table>| {
    package.and_then(|package| package.get("version")?.as_str().map(str::to_owned))
  };

  Some(
    names
      .into_iter()
      .filter_map(|name| {
        let was = entry(&before_paths, name);
        let is = entry(&after_paths, name);

        (was != is).then(|| PathMove {
          name: name.clone(),
          from: version(was),
          to: version(is),
        })
      })
      .collect(),
  )
}

// The path package of that name among a lock's path packages.
fn entry<'a>(paths: &[(String, &'a Table)], name: &str) -> Option<&'a Table> {
  paths.iter().find(|(one, _)| one == name).map(|(_, at)| *at)
}

// A lock's packages, those from a source in the lock's order and those read from a path by name. None where
// the packages do not read as tables.
type Packages<'a> = (Vec<&'a Table>, Vec<(String, &'a Table)>);

fn split_packages(lock: &Table) -> Option<Packages<'_>> {
  let listed = match lock.get("package") {
    Some(packages) => packages.as_array()?.as_slice(),
    None => &[],
  };
  let mut sourced = Vec::new();
  let mut paths = Vec::new();

  for package in listed {
    let package = package.as_table()?;

    if package.contains_key("source") {
      sourced.push(package);
    } else {
      let name = package.get("name").and_then(Value::as_str)?;

      paths.push((name.to_owned(), package));
    }
  }

  Some((sourced, paths))
}

// The lock of the workspace `dir` stands in, as Cargo places it.
fn lock_of(dir: &Path, cargo: &dyn Fn() -> Command) -> Option<PathBuf> {
  let located = cargo()
    .args(["locate-project", "--workspace", "--message-format", "plain", "--offline"])
    .current_dir(dir)
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output()
    .ok()
    .filter(|located| located.status.success())?;
  let manifest = PathBuf::from(String::from_utf8_lossy(&located.stdout).trim());

  Some(manifest.parent()?.join("Cargo.lock"))
}

/// Before the Cargo command `args` in `dir`: where it holds its lock, the lock's path packages brought to the
/// versions their manifests name, offline. `cargo` makes a Cargo command with the caller's program and
/// environment. Answers what moved where the lock was rewritten, for the caller to say and to put back once
/// the command has ended; none where nothing needed it, the command
/// names its manifest by `--manifest-path`, or Cargo's resolution would move more than path packages or did
/// not run, the lock then standing exactly as it stood.
pub fn align_path_packages(dir: &Path, args: &[String], cargo: &dyn Fn() -> Command) -> Option<Aligned> {
  let named_manifest = args
    .iter()
    .take_while(|word| *word != "--")
    .any(|word| word == "--manifest-path" || word.starts_with("--manifest-path="));

  if !holds_lock(args) || named_manifest {
    return None;
  }

  let lock = lock_of(dir, cargo)?;
  let before = std::fs::read(&lock).ok()?;
  let resolved = cargo()
    .args(["metadata", "--offline", "--format-version", "1"])
    .current_dir(dir)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|status| status.success());
  let after = std::fs::read(&lock).unwrap_or_default();

  if after == before {
    return None;
  }

  let moves = resolved
    .then(|| path_moves(&String::from_utf8_lossy(&before), &String::from_utf8_lossy(&after)))
    .flatten();

  match moves {
    Some(moves) => Some(Aligned {
      lock,
      moves,
      found: before,
      written: after,
    }),
    None => {
      let _ = std::fs::write(&lock, &before);

      None
    }
  }
}
