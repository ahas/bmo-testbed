//! What no build stands on any more in the shared Cargo build directory, and its removal (docs/deploy.md,
//! 「The Cargo build directory」). A unit is everything Cargo names by one hash: its fingerprint, its outputs and
//! its build script's directory. A unit of a checkout's own crates goes once no live checkout's record names it
//! and it was built more than the grace ago; any other once none of its files has been read or changed for the
//! span. A unit that stays keeps every unit it was built from, and a unit a live checkout's target directory
//! holds a hard link to stays. The build locks are taken at the front of each lock's line by
//! [`take_prune_locks`], which the caller holds while it plans and removes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{DirEntry, Metadata};
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::queue::{HeldLock, LockWait, Rank, take_lock};
use serde_json::Value;

use crate::crates::{CHECKOUT_RECORD_NAME, checkout_target_name};
use crate::superseded::{
  BUILT_UNITS_NAME, CARGO_BUILD_LOCK_NAME, PRUNE_DAYS, UNIT_DIRS, read_record, recorded_units, unit_hash,
};

/// The span a prune must exceed, in days: a relatime disk moves a file's access time on a read only once it is
/// a day old, so a shorter span would take a unit read hours ago for one unread.
pub const RELATIME_DAYS: i64 = 1;

pub const HOUR_MS: i64 = 60 * 60 * 1000;

pub const DAY_MS: i64 = 24 * HOUR_MS;

/// The span incremental state is left alone for when none is named, in hours: a session is written as its build
/// ends and never again, so its modification time says when it was made, and no access time is needed to judge
/// it.
pub const INCREMENTAL_HOURS: i64 = 3;

/// How long a unit of a checkout's own crates that no record names is left alone after it was built: a build
/// that failed leaves such units, and its job mends it and builds again within that.
pub const OWN_GRACE_MS: i64 = 12 * HOUR_MS;

/// The lock a Cargo run holds in a profile directory of its target directory for its whole build, where it has
/// no build directory apart from it.
pub const CARGO_LOCK_NAME: &str = ".cargo-lock";

/// What the build directory holds in its normal standing, apart from the checkouts: every dependency compiled
/// once in each profile, the deployment's release build, and the projects of their own.
pub const SHARED_CEILING_BYTES: u64 = 40_000_000_000;

/// What one live checkout's own crates hold in the build directory in their normal standing: one generation,
/// its builds, tests and lints.
pub const CHECKOUT_CEILING_BYTES: u64 = 8_000_000_000;

// The directories of a profile directory a build hands nothing back into, only works in.
const WORK_DIRS: [&str; 4] = [".fingerprint", "deps", "build", "incremental"];

/// One unit a prune removes: its hash and every path in the build directory that carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrunedUnit {
  pub hash: String,
  pub paths: Vec<PathBuf>,
}

/// What a prune removes from the build directory.
#[derive(Clone, Debug, Default)]
pub struct PrunePlan {
  pub units: Vec<PrunedUnit>,
  /// A crate's incremental sessions and their locks, which carry no unit hash, or the crate's whole incremental
  /// directory where every one of them goes.
  pub incremental: Vec<PathBuf>,
  /// A file a build handed back into a profile directory of the build directory itself.
  pub loose: Vec<PathBuf>,
  /// The target directory of a checkout that no longer exists.
  pub checkouts: Vec<PathBuf>,
  /// Everything else a clear of the whole directory removes; a prune by age leaves it empty.
  pub cleared: Vec<PathBuf>,
  /// What a build typed by hand on a commit whose configuration names no shared build directory left in a live
  /// checkout's own `target/`: its compiled units, fingerprints, build scripts' output and incremental state.
  pub hand_built: Vec<PathBuf>,
  /// About how much disk they hold: each file's allocated size, counted once however many links it has.
  pub bytes: u64,
}

impl PrunePlan {
  /// Every path the plan removes.
  pub fn paths(&self) -> Vec<&Path> {
    self
      .checkouts
      .iter()
      .chain(self.units.iter().flat_map(|unit| unit.paths.iter()))
      .chain(&self.incremental)
      .chain(&self.loose)
      .chain(&self.cleared)
      .chain(&self.hand_built)
      .map(PathBuf::as_path)
      .collect()
  }

  /// The plan carried out; a path already gone or one that cannot be removed is passed over.
  pub fn apply(&self) {
    for path in self.paths() {
      let removed = match std::fs::symlink_metadata(path) {
        Ok(held) if held.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
      };

      drop(removed);
    }
  }

  // The plan with the disk its paths hold counted.
  fn measured(mut self) -> Self {
    let mut counted = HashSet::new();
    let mut bytes = 0;

    for path in self.paths() {
      walk(path, &mut |_, held| {
        if held.is_file() && counted.insert(held.ino()) {
          bytes += held.blocks() * 512;
        }
      });
    }

    self.bytes = bytes;
    self
  }
}

fn entries_of(dir: &Path) -> Vec<DirEntry> {
  std::fs::read_dir(dir).map(|read| read.flatten().collect()).unwrap_or_default()
}

fn is_dir(entry: &DirEntry) -> bool {
  entry.file_type().is_ok_and(|kind| kind.is_dir())
}

fn name_of(entry: &DirEntry) -> String {
  entry.file_name().to_string_lossy().into_owned()
}

// Every path at and under `path`, with what lstat says of it; a link is never followed.
fn walk(path: &Path, visit: &mut dyn FnMut(&Path, &Metadata)) {
  let Ok(held) = std::fs::symlink_metadata(path) else { return };

  visit(path, &held);

  if held.is_dir() {
    for entry in entries_of(path) {
      walk(&entry.path(), visit);
    }
  }
}

fn modified_ms(held: &Metadata) -> i64 {
  held.mtime() * 1000 + held.mtime_nsec() / 1_000_000
}

fn accessed_ms(held: &Metadata) -> i64 {
  held.atime() * 1000 + held.atime_nsec() / 1_000_000
}

/// When anything at or under `path` was last read or changed, in milliseconds since the epoch. A directory
/// counts by its change time alone, since the listing a prune reads it by moves its access time.
pub fn last_use(path: &Path) -> i64 {
  let mut newest = i64::MIN;

  walk(path, &mut |_, held| {
    newest = newest.max(modified_ms(held));

    if !held.is_dir() {
      newest = newest.max(accessed_ms(held));
    }
  });

  newest
}

/// When anything at or under `path` was last changed, by modification time alone.
pub fn last_change(path: &Path) -> i64 {
  let mut newest = i64::MIN;

  walk(path, &mut |_, held| newest = newest.max(modified_ms(held)));

  newest
}

// A file read without moving its access time, so a prune never makes what it read look used; a plain read
// where the kernel refuses that, as it does for a file of another owner.
fn read_quietly(path: &Path) -> std::io::Result<String> {
  let quiet = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOATIME).open(path);
  let mut file = match quiet {
    Err(why) if why.raw_os_error() == Some(libc::EPERM) => std::fs::File::open(path)?,
    opened => opened?,
  };
  let mut text = String::new();

  file.read_to_string(&mut text)?;

  Ok(text)
}

/// What one fingerprint directory says: the fingerprint values its unit stands under, and those of the units
/// it was built from, each as the decimal the unit's JSON names it by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitFingerprint {
  pub values: Vec<String>,
  pub dependencies: Vec<String>,
}

// A fingerprint value as Cargo writes it into a unit's hash file: the 64-bit number's bytes, least significant
// first, in hex.
fn fingerprint_value(hex: &str) -> Option<String> {
  if hex.len() != 16 || !hex.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
    return None;
  }

  let number = u64::from_str_radix(hex, 16).ok()?;

  Some(u64::from_le_bytes(number.to_be_bytes()).to_string())
}

/// A unit's fingerprint directory read: beside each kind of output Cargo keeps a hash file and its JSON, whose
/// `deps` name every unit it was built from by that unit's fingerprint value, fourth.
pub fn read_fingerprint(dir: &Path) -> UnitFingerprint {
  let mut read = UnitFingerprint::default();

  for entry in entries_of(dir) {
    let name = name_of(&entry);
    let Some(stem) = name.strip_suffix(".json") else { continue };

    if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
      continue;
    }

    // A file Cargo is not writing any more but left half made: its unit is judged by time alone.
    let Ok(hash) = read_quietly(&dir.join(stem)) else { continue };
    let Some(recorded) = read_quietly(&entry.path())
      .ok()
      .and_then(|text| serde_json::from_str::<Value>(&text).ok())
    else {
      continue;
    };

    read.values.extend(fingerprint_value(hash.trim()));

    for dependency in recorded["deps"].as_array().into_iter().flatten() {
      match &dependency[3] {
        Value::Number(value) => read.dependencies.push(value.to_string()),
        Value::String(value) => read.dependencies.push(value.clone()),
        _ => {}
      }
    }
  }

  read
}

// The checkout path a checkout's target directory records, if it can be read.
fn recorded_checkout(dir: &Path) -> Option<String> {
  read_quietly(&dir.join(CHECKOUT_RECORD_NAME)).ok()
}

// Whether a recorded path names a checkout that exists and whose target directory is the one named `dir_name`.
fn records_live(recorded: &str, dir_name: &str) -> bool {
  Path::new(recorded).exists() && checkout_target_name(Path::new(recorded)) == dir_name
}

// Whether a checkout's target directory belongs to a checkout that no longer exists: its recorded path is gone
// or names another directory, or, with no record, nothing in it moved within the span.
fn checkout_gone(dir: &Path, cutoff: i64) -> bool {
  let name = dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();

  match recorded_checkout(dir) {
    Some(recorded) => !records_live(&recorded, &name),
    None => last_use(dir) < cutoff,
  }
}

// A live checkout's own `target/`, by the path its target directory under `root` records.
fn hand_targets(root: &Path) -> Vec<PathBuf> {
  entries_of(&root.join("checkouts"))
    .iter()
    .filter_map(|entry| {
      let recorded = recorded_checkout(&entry.path())?;

      records_live(&recorded, &name_of(entry)).then(|| Path::new(&recorded).join("target"))
    })
    .collect()
}

/// The profile directories a build typed by hand made in a live checkout's own `target/`, each holding
/// fingerprints: where its lock is taken and its work directories go.
pub fn hand_profile_dirs(root: &Path) -> Vec<PathBuf> {
  let mut dirs: Vec<PathBuf> = hand_targets(root).iter().flat_map(|target| profile_dirs(target)).collect();

  dirs.sort();
  dirs
}

// A hand-built profile's work directories, those unused within the span or, with no span, all of them.
fn plan_hand_built(root: &Path, cutoff: Option<i64>, plan: &mut PrunePlan) {
  for profile in hand_profile_dirs(root) {
    for dir in WORK_DIRS {
      let path = profile.join(dir);

      if path.exists() && cutoff.is_none_or(|cutoff| last_use(&path) < cutoff) {
        plan.hand_built.push(path);
      }
    }
  }
}

/// The profile directories of the build directory: each one holding fingerprints, outside `checkouts/`.
pub fn profile_dirs(root: &Path) -> Vec<PathBuf> {
  let mut dirs: Vec<PathBuf> = entries_of(root)
    .iter()
    .filter(|entry| {
      is_dir(entry) && name_of(entry) != "checkouts" && entry.path().join(".fingerprint").exists()
    })
    .map(DirEntry::path)
    .collect();

  dirs.sort();
  dirs
}

struct UnitFiles {
  paths: Vec<PathBuf>,
  used: i64,
  built: i64,
  // Held by a hard link in a live checkout's target directory.
  live: bool,
}

/// What a prune judges every profile directory by, apart from its spans.
struct Standing {
  // The inodes a live checkout's target directories hold.
  live_inodes: HashSet<u64>,
  // The units live checkouts' records name, as `<profile>:<hash>`.
  recorded: HashSet<String>,
  // When a unit of a checkout's own crates no record names must have been built before, to go.
  own_cutoff: i64,
}

fn plan_profile(
  profile: &Path,
  cutoff: i64,
  incremental_cutoff: i64,
  standing: &Standing,
  plan: &mut PrunePlan,
) {
  let mut units: BTreeMap<String, UnitFiles> = BTreeMap::new();
  let mut fingerprints: HashMap<String, UnitFingerprint> = HashMap::new();
  let mut packages: HashMap<String, String> = HashMap::new();
  let mut add = |hash: &str, path: PathBuf| {
    let unit = units.entry(hash.to_owned()).or_insert(UnitFiles {
      paths: Vec::new(),
      used: i64::MIN,
      built: i64::MIN,
      live: false,
    });
    let held = std::fs::symlink_metadata(&path).ok();

    unit.used = unit.used.max(last_use(&path));
    unit.built = unit.built.max(last_change(&path));
    unit.live |= held.is_some_and(|held| held.is_file() && standing.live_inodes.contains(&held.ino()));
    unit.paths.push(path);
  };

  for dir in UNIT_DIRS {
    for entry in entries_of(&profile.join(dir)) {
      let name = name_of(&entry);
      let Some(hash) = unit_hash(&name) else { continue };

      add(hash, entry.path());

      if dir == ".fingerprint" && is_dir(&entry) {
        fingerprints.insert(hash.to_owned(), read_fingerprint(&entry.path()));
        packages.insert(hash.to_owned(), name[..name.len() - hash.len() - 1].to_owned());
      }
    }
  }

  for entry in entries_of(profile) {
    let name = name_of(&entry);

    if is_dir(&entry) || name.starts_with('.') {
      continue;
    }

    match unit_hash(&name) {
      Some(hash) => add(hash, entry.path()),
      None if last_use(&entry.path()) < cutoff => plan.loose.push(entry.path()),
      None => {}
    }
  }

  // No profile of the workspace keeps incremental state, so what stands there is an older commit's build or one
  // run by hand: a session and its lock go once nothing in them changed for the incremental span, judged by
  // modification time alone, since a session is written once and only read after; a crate whose every session
  // goes goes whole.
  for krate in entries_of(&profile.join("incremental")) {
    if !is_dir(&krate) {
      continue;
    }

    let sessions: Vec<PathBuf> = entries_of(&krate.path()).iter().map(DirEntry::path).collect();
    let stale: Vec<PathBuf> =
      sessions.iter().filter(|path| last_change(path) < incremental_cutoff).cloned().collect();

    if stale.len() == sessions.len() {
      plan.incremental.push(krate.path());
    } else {
      plan.incremental.extend(stale);
    }
  }

  // A checkout's own crates are the packages the records' units belong to. Their units stay while a record
  // names them or for the grace after they were built, however lately they were read; any other unit while it
  // was used within the span.
  let profile_name =
    profile.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
  let named = |hash: &str| standing.recorded.contains(&format!("{profile_name}:{hash}"));
  let own: HashSet<&String> =
    packages.iter().filter(|(hash, _)| named(hash)).map(|(_, package)| package).collect();
  let stays = |hash: &str, unit: &UnitFiles| {
    if unit.live {
      return true;
    }

    if packages.get(hash).is_some_and(|package| own.contains(package)) {
      named(hash) || unit.built >= standing.own_cutoff
    } else {
      unit.used >= cutoff
    }
  };

  // What stays, with every unit those were built from, followed through the fingerprint values their
  // fingerprints name.
  let by_value: HashMap<&str, &str> = fingerprints
    .iter()
    .flat_map(|(hash, fingerprint)| {
      fingerprint.values.iter().map(move |value| (value.as_str(), hash.as_str()))
    })
    .collect();
  let mut kept: HashSet<&str> = HashSet::new();
  let mut pending: Vec<&str> = units
    .iter()
    .filter(|(hash, unit)| stays(hash, unit))
    .map(|(hash, _)| hash.as_str())
    .collect();

  while let Some(hash) = pending.pop() {
    if !kept.insert(hash) {
      continue;
    }

    for value in fingerprints
      .get(hash)
      .map(|fingerprint| fingerprint.dependencies.as_slice())
      .unwrap_or_default()
    {
      if let Some(dependency) = by_value.get(value.as_str())
        && !kept.contains(dependency)
      {
        pending.push(dependency);
      }
    }
  }

  let gone: Vec<PrunedUnit> = units
    .iter()
    .filter(|(hash, _)| !kept.contains(hash.as_str()))
    .map(|(hash, unit)| {
      let mut paths = unit.paths.clone();

      paths.sort();

      PrunedUnit {
        hash: hash.clone(),
        paths,
      }
    })
    .collect();

  plan.units.extend(gone);
}

/// What a prune of the build directory at `root` removes, leaving alone what was read or changed within
/// `older_than_ms` of `now`, and incremental state changed within `incremental_older_than_ms`.
pub fn plan_prune(root: &Path, older_than_ms: i64, now: i64, incremental_older_than_ms: i64) -> PrunePlan {
  let cutoff = now - older_than_ms;
  let mut plan = PrunePlan::default();
  let mut live_inodes = HashSet::new();
  let mut recorded = HashSet::new();
  let mut hold_links = |dir: &Path| {
    walk(dir, &mut |_, held| {
      if held.is_file() {
        live_inodes.insert(held.ino());
      }
    });
  };

  for entry in entries_of(&root.join("checkouts")) {
    if !is_dir(&entry) {
      continue;
    }

    let dir = entry.path();

    if checkout_gone(&dir, cutoff) {
      plan.checkouts.push(dir);
    } else {
      hold_links(&dir);

      // A record of an earlier form names its units with no places; they are as much what it stands on.
      let read = read_record(&dir.join(BUILT_UNITS_NAME));

      recorded.extend(recorded_units(&read.record));
      recorded.extend(read.stale);
    }
  }

  // What a build typed by hand handed back into a live checkout's own `target/`.
  for target in hand_targets(root) {
    hold_links(&target);
  }

  let standing = Standing {
    live_inodes,
    recorded,
    own_cutoff: now - OWN_GRACE_MS,
  };

  for profile in profile_dirs(root) {
    plan_profile(&profile, cutoff, now - incremental_older_than_ms, &standing, &mut plan);
  }

  plan_hand_built(root, Some(cutoff), &mut plan);
  plan.measured()
}

// The lock files Cargo keeps in a profile directory, which a clear leaves standing so a build waiting on one
// meets the same file once the clear lets it go.
fn is_cargo_lock(name: &str) -> bool {
  name.strip_prefix(".cargo-").is_some_and(|rest| rest.contains("lock"))
}

/// Everything in the build directory at `root` but the locks of its profile directories: every compiled unit,
/// every checkout's target directory and whatever else stands there. Every checkout builds afresh after it, so
/// it is run only when asked.
pub fn plan_clear(root: &Path) -> PrunePlan {
  let mut plan = PrunePlan::default();

  // Read before the checkouts' target directories, whose records name them, are planned away.
  plan_hand_built(root, None, &mut plan);

  let profiles: HashSet<PathBuf> = profile_dirs(root).into_iter().collect();

  for entry in entries_of(root) {
    let path = entry.path();

    if name_of(&entry) == "checkouts" {
      plan.checkouts.extend(entries_of(&path).iter().map(DirEntry::path));
    } else if profiles.contains(&path) {
      plan.cleared.extend(
        entries_of(&path)
          .iter()
          .filter(|inside| !is_cargo_lock(&name_of(inside)))
          .map(DirEntry::path),
      );
    } else {
      plan.cleared.push(path);
    }
  }

  plan.measured()
}

/// Whether the disk the build directory stands on keeps access times, which the whole prune is judged by.
pub fn keeps_access_times(root: &Path) -> bool {
  let asked = Command::new("findmnt")
    .args(["--noheadings", "--output", "OPTIONS", "--target"])
    .arg(root)
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output();

  match asked {
    Ok(asked) if asked.status.success() => !String::from_utf8_lossy(&asked.stdout)
      .trim()
      .split(',')
      .any(|option| option == "noatime"),
    _ => true,
  }
}

/// The build directory's ceiling: what it holds in its normal standing, with the checkouts whose records say
/// they build here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ceiling {
  pub bytes: u64,
  pub checkouts: u64,
}

/// The ceiling of the build directory at `root`, for the live checkouts that keep a record of their builds.
pub fn ceiling(root: &Path, now: i64) -> Ceiling {
  let checkouts = entries_of(&root.join("checkouts"))
    .iter()
    .filter(|entry| {
      let dir = entry.path();

      is_dir(entry)
        && dir.join(BUILT_UNITS_NAME).exists()
        && !checkout_gone(&dir, now - PRUNE_DAYS * DAY_MS)
    })
    .count() as u64;

  Ceiling {
    bytes: SHARED_CEILING_BYTES + CHECKOUT_CEILING_BYTES * checkouts,
    checkouts,
  }
}

/// About how much disk everything under `root` holds: each file's allocated size, counted once however many
/// links it has.
pub fn directory_bytes(root: &Path) -> u64 {
  let mut counted = HashSet::new();
  let mut bytes = 0;

  walk(root, &mut |_, held| {
    if held.is_file() && counted.insert(held.ino()) {
      bytes += held.blocks() * 512;
    }
  });

  bytes
}

/// Cargo's build lock in each profile directory of the build directory at `root`, and the lock a build typed by
/// hand holds in a live checkout's own `target/` where such a build left work there, each taken at the front of
/// its line and waited on for `wait_seconds`; the profile directory whose lock did not come where one did not.
pub fn take_prune_locks(root: &Path, wait_seconds: f64) -> Result<Vec<HeldLock>, PathBuf> {
  let paths = profile_dirs(root)
    .into_iter()
    .map(|profile| profile.join(CARGO_BUILD_LOCK_NAME))
    .chain(hand_profile_dirs(root).into_iter().map(|profile| profile.join(CARGO_LOCK_NAME)));
  let mut held = Vec::new();

  for path in paths {
    let wait = LockWait {
      holder: "cargo prune".to_owned(),
      rank: Rank::Front,
      ..LockWait::default()
    };

    match take_lock(&path, wait_seconds, wait) {
      Some(lock) => held.push(lock),
      None => return Err(path.parent().map(Path::to_path_buf).unwrap_or(path)),
    }
  }

  Ok(held)
}

/// What the disk a path stands on has free for this account, in bytes; nought where it cannot be read.
pub fn disk_free_bytes(path: &Path) -> u64 {
  let Ok(named) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
    return 0;
  };
  // SAFETY: an all-zero `statvfs` is a valid value of the plain C struct the call writes into.
  let mut read: libc::statvfs = unsafe { std::mem::zeroed() };

  // SAFETY: the path is a NUL-terminated string that lives through the call, and the struct is ours.
  if unsafe { libc::statvfs(named.as_ptr(), &raw mut read) } != 0 {
    return 0;
  }

  read.f_bavail.saturating_mul(read.f_frsize)
}
