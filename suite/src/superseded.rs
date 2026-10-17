//! What a checkout's earlier builds left, removed once its next build has passed (docs/deploy.md, 「The
//! Cargo build directory」). Cargo names, in its JSON messages, every unit a build stands on; the
//! checkout's own target directory records, for each shape of build, the unit of the checkout's own crates its
//! last build stood on at each place, with the generation it was built in. A unit the shape's own entry named
//! and the new build does not goes, and so does one an entry of an earlier generation names at a place the new
//! build filled with another, unless any entry still names it. Dependencies' units, shared by every checkout,
//! are the prune's.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::suite::SUITE;
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::crates::{cargo_target_root, checkout_target_dir, resolved, workspace_crates};

/// The message format a build that is swept asks Cargo for: its JSON on standard output, its diagnostics on
/// standard error as a person reads them.
pub const REPORTED_FORMAT: &str = "--message-format=json-render-diagnostics";

/// The file in a checkout's own target directory naming the units each shape of its builds last stood on.
pub const BUILT_UNITS_NAME: &str = "built-units.json";

/// The lock every Cargo run holds in a profile directory of its build directory for its whole build.
pub const CARGO_BUILD_LOCK_NAME: &str = ".cargo-build-lock";

/// The directories of a profile directory whose entries Cargo names by a unit's hash.
pub const UNIT_DIRS: [&str; 3] = [".fingerprint", "deps", "build"];

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

// The option naming how Cargo reports, which the report a removal asks for stands in place of.
const FORMAT_OPTION: &str = "--message-format";

// The files at a checkout's root that every own unit's hash is made of beside the manifests.
const GENERATION_FILES: [&str; 3] = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml"];

/// The hash a name Cargo gives a unit's file or directory carries: anything, a hyphen and the unit's
/// sixteen hex digits, then an extension or nothing.
pub fn unit_hash(name: &str) -> Option<&str> {
  static PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^.]*-([0-9a-f]{16})(?:\.|$)").expect("the unit hash pattern"));

  PATTERN.captures(name).and_then(|found| found.get(1)).map(|hash| hash.as_str())
}

/// Where a checkout's builds stand: the shared build directory and the checkout's own target directory.
#[derive(Clone, Debug)]
pub struct Builds {
  pub root: PathBuf,
  pub build_root: PathBuf,
  pub target_dir: PathBuf,
}

impl Builds {
  /// The checkout at `root`, where the verbs build it.
  pub fn of(root: &Path) -> Self {
    Self {
      root: root.to_path_buf(),
      build_root: cargo_target_root(),
      target_dir: checkout_target_dir(root),
    }
  }

  /// The record naming the units each shape of its builds last stood on.
  pub fn record_path(&self) -> PathBuf {
    self.target_dir.join(BUILT_UNITS_NAME)
  }
}

/// One unit a build stood on: the place it fills in a build's shape, and the unit itself, its profile
/// directory and its hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltUnit {
  pub place: String,
  pub unit: String,
}

/// What a checkout's record holds for one shape: when it was last swept, the wrapper and generation its last
/// build ran in, and the unit it stood on at each place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeRecord {
  pub at: i64,
  pub wrapper: String,
  pub generation: String,
  pub places: BTreeMap<String, String>,
}

pub type BuiltRecord = BTreeMap<String, ShapeRecord>;

/// One passed build as the removal reads it: its shape, the wrapper it ran under and its generation.
#[derive(Clone, Debug)]
pub struct BuildIdentity {
  pub shape: String,
  pub wrapper: String,
  pub generation: String,
}

/// The words that ask Cargo, after a passed command, what that command's build stood on: the same words with
/// the JSON report in place of any format the command named, a test or bench with `--no-run`, and a run as a
/// build. None for a command that builds nothing a removal follows.
pub fn reporting_args(args: &[String]) -> Option<Vec<String>> {
  let split = args.iter().position(|arg| arg == "--").unwrap_or(args.len());
  let (named, after) = args.split_at(split);
  let verb = match named.first().map(String::as_str)? {
    "build" | "b" | "check" | "c" | "clippy" | "rustc" => "build",
    "test" | "t" | "bench" => "test",
    "run" | "r" => "run",
    _ => return None,
  };
  let before: Vec<String> = named
    .iter()
    .enumerate()
    .filter(|(at, arg)| {
      !arg.starts_with(&format!("{FORMAT_OPTION}="))
        && *arg != FORMAT_OPTION
        && (*at == 0 || named[at - 1] != FORMAT_OPTION)
    })
    .map(|(_, arg)| arg.clone())
    .collect();
  let mut asked = match verb {
    "run" => std::iter::once("build".to_owned()).chain(before.into_iter().skip(1)).collect(),
    _ => before,
  };

  if verb == "test" && !asked.iter().any(|arg| arg == "--no-run") {
    asked.push("--no-run".to_owned());
  }

  asked.push(REPORTED_FORMAT.to_owned());

  if verb != "run" {
    asked.extend(after.iter().cloned());
  }

  Some(asked)
}

// The package a unit belongs to where it is one of the checkout's own: its path under the checkout, with the
// name Cargo adds where it differs from the directory's, and never the version, which a generation may move;
// none for any other package.
fn own_package(root: &Path, package_id: &str) -> Option<String> {
  let url = url::Url::parse(package_id).ok()?;

  if url.scheme() != "path+file" {
    return None;
  }

  let path = percent_encoding::percent_decode_str(url.path()).decode_utf8().ok()?;
  let under = Path::new(path.as_ref()).strip_prefix(resolved(root)).ok()?;
  let named = url
    .fragment()
    .and_then(|fragment| fragment.rfind('@').map(|at| format!("#{}", &fragment[..at])))
    .unwrap_or_default();
  let parts: Vec<String> =
    under.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect();

  Some(format!("{}{named}", parts.join("/")))
}

fn entries_of(dir: &Path) -> Vec<std::fs::DirEntry> {
  std::fs::read_dir(dir).map(|read| read.flatten().collect()).unwrap_or_default()
}

fn joined(parts: &[String]) -> String {
  parts.join("/")
}

fn parts_of(path: &Path) -> Vec<String> {
  path.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect()
}

// The unit an output file names: in the build directory, its profile directory and the hash its name carries;
// in the checkout's own target directory, where a build hands a binary back as a hard link, the unit in the
// build directory holding the same file.
fn unit_of_file(builds: &Builds, path: &Path, crate_name: &str) -> Option<String> {
  if let Ok(in_build) = path.strip_prefix(&builds.build_root) {
    let parts = parts_of(in_build);
    let at = parts.iter().position(|part| UNIT_DIRS.contains(&part.as_str()));

    if let Some(at) = at.filter(|at| *at > 0)
      && let Some(hash) = parts.get(at + 1).and_then(|name| unit_hash(name))
      && !parts.iter().any(|part| part == "checkouts")
    {
      return Some(format!("{}:{hash}", joined(&parts[..at])));
    }
  }

  let in_target = path.strip_prefix(&builds.target_dir).ok()?;
  let profile = in_target.parent().unwrap_or(Path::new(""));
  let handed = std::fs::symlink_metadata(path).ok()?;
  let deps = builds.build_root.join(profile).join("deps");

  entries_of(&deps).into_iter().find_map(|entry| {
    let name = entry.file_name().to_string_lossy().into_owned();
    let hash = unit_hash(&name)?;
    let same = entry.metadata().is_ok_and(|held| held.ino() == handed.ino());

    (name.starts_with(&format!("{crate_name}-")) && same)
      .then(|| format!("{}:{hash}", joined(&parts_of(profile))))
  })
}

// Every unit Cargo's JSON report names as compiled or found fresh; any other line is passed over.
fn artifacts_of(output: &str) -> Vec<Value> {
  output
    .lines()
    .filter(|line| line.starts_with('{'))
    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    .filter(|message| message["reason"] == "compiler-artifact" && message["package_id"].is_string())
    .collect()
}

fn strings(value: &Value) -> Vec<String> {
  value
    .as_array()
    .map(|list| list.iter().filter_map(|one| one.as_str().map(str::to_owned)).collect())
    .unwrap_or_default()
}

/// The units of the checkout's own crates a build stood on, out of what Cargo printed on standard output in its
/// JSON report: one for each unit compiled or found fresh, by the place it fills in the build's shape.
pub fn built_units(builds: &Builds, output: &str) -> Vec<BuiltUnit> {
  artifacts_of(output)
    .into_iter()
    .filter_map(|message| {
      let owner = own_package(&builds.root, message["package_id"].as_str()?)?;
      let mut files = strings(&message["filenames"]);

      if let Some(executable) = message["executable"].as_str() {
        files.push(executable.to_owned());
      }

      let name = message["target"]["name"].as_str().unwrap_or_default();
      let crate_name = name.replace('-', "_");
      let unit = files.iter().find_map(|file| unit_of_file(builds, Path::new(file), &crate_name))?;
      let profile = &unit[..unit.rfind(':')?];
      let mode = if files.iter().all(|file| file.ends_with(".rmeta")) { "check" } else { "build" };
      let test = if message["profile"]["test"] == true { "test" } else { "plain" };
      let kinds = strings(&message["target"]["kind"]).join(",");

      Some(BuiltUnit {
        place: [profile, &owner, &kinds, name, test, mode].join("|"),
        unit,
      })
    })
    .collect()
}

fn hex16(hash: Sha256) -> String {
  format!("{:x}", hash.finalize())[..16].to_owned()
}

/// A build's shape: the wrapper it ran under and the places of its own units, whatever units fill them.
pub fn shape_of(wrapper: &str, units: &[BuiltUnit]) -> String {
  let places: BTreeSet<&str> = units.iter().map(|one| one.place.as_str()).collect();
  let mut hash = Sha256::new();

  hash.update(std::iter::once(wrapper).chain(places).collect::<Vec<_>>().join("\n"));
  hex16(hash)
}

/// The generation the checkout at `root` builds in: what the hashes of its own units are made of beside their
/// source — the lockfile, Cargo's configuration and the manifests of every crate of the workspace, whichever of
/// them a build names, so the builds of one commit share one generation however each selects its crates.
pub fn build_generation(root: &Path) -> Result<String, String> {
  let manifests = workspace_crates(root)?.into_iter().map(|one| format!("{}/Cargo.toml", one.dir));
  let paths: BTreeSet<String> =
    GENERATION_FILES.iter().map(|path| (*path).to_owned()).chain(manifests).collect();
  let mut hash = Sha256::new();

  for path in paths {
    let content = std::fs::read_to_string(root.join(&path)).unwrap_or_default();

    hash.update(format!("{path}\n{content}\n"));
  }

  Ok(hex16(hash))
}

/// A checkout's record as read, with the units an entry of an earlier form named: an entry naming its units
/// with no places reads as an entry of no generation whose units are all superseded.
#[derive(Debug, Default)]
pub struct ReadRecord {
  pub record: BuiltRecord,
  pub stale: Vec<String>,
}

/// The record at `path`, empty where there is none or it cannot be read.
pub fn read_record(path: &Path) -> ReadRecord {
  let mut read = ReadRecord::default();
  let Some(Value::Object(shapes)) =
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok())
  else {
    return read;
  };

  for (shape, value) in shapes {
    let entry = (|| {
      Some(ShapeRecord {
        at: value["at"].as_f64()? as i64,
        wrapper: value["wrapper"].as_str()?.to_owned(),
        generation: value["generation"].as_str()?.to_owned(),
        places: value["places"]
          .as_object()?
          .iter()
          .filter_map(|(place, unit)| Some((place.clone(), unit.as_str()?.to_owned())))
          .collect(),
      })
    })();

    match entry {
      Some(entry) => {
        read.record.insert(shape, entry);
      }
      None => read.stale.extend(strings(&value["units"])),
    }
  }

  read
}

/// Every unit a record names.
pub fn recorded_units(record: &BuiltRecord) -> Vec<String> {
  record.values().flat_map(|entry| entry.places.values().cloned()).collect()
}

// The record written whole by a rename, so a removal reading it never meets it half written, each entry with
// its units listed as well, the form every checkout's removal reads whatever version it runs.
fn write_record(path: &Path, record: &BuiltRecord) -> std::io::Result<()> {
  let written: serde_json::Map<String, Value> = record
    .iter()
    .map(|(shape, entry)| {
      let units: BTreeSet<&String> = entry.places.values().collect();

      (
        shape.clone(),
        json!({
          "at": entry.at,
          "wrapper": entry.wrapper,
          "generation": entry.generation,
          "places": entry.places,
          "units": units,
        }),
      )
    })
    .collect();
  let made = path.with_extension(format!("json.{}", std::process::id()));

  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir)?;
  }

  std::fs::write(&made, format!("{}\n", Value::Object(written)))?;
  std::fs::rename(&made, path)
}

/// What a removal takes, and the record it leaves the checkout once it has.
#[derive(Debug)]
pub struct SupersededPlan {
  pub gone: Vec<String>,
  pub record: BuiltRecord,
}

/// What the removal in the checkout takes after the build `build` stood on `units`: what the shape's own entry
/// named and the build does not; what an entry of the same wrapper and an earlier generation names at a place
/// the build filled with another unit, which that entry forgets; and what an entry of an earlier form named —
/// each unless an entry, of this checkout or any other, still names it. With it, the checkout's record with
/// the shape's entry replaced and the entries unswept for the prune's span gone.
pub fn plan_superseded(
  builds: &Builds,
  build: &BuildIdentity,
  units: &[BuiltUnit],
  now_ms: i64,
) -> SupersededPlan {
  let own = builds.record_path();
  let current = read_record(&own);
  let standing: BTreeMap<String, String> =
    units.iter().map(|one| (one.place.clone(), one.unit.clone())).collect();
  let cutoff = now_ms - PRUNE.older_than_days * DAY_MS;
  let mut candidates: BTreeSet<String> = current.stale.iter().cloned().collect();
  let mut record = BuiltRecord::new();

  if let Some(entry) = current.record.get(&build.shape) {
    candidates.extend(entry.places.values().cloned());
  }

  for (shape, entry) in &current.record {
    if *shape == build.shape || entry.at < cutoff {
      continue;
    }

    let replaced = entry.wrapper == build.wrapper && entry.generation != build.generation;
    let mut kept = BTreeMap::new();

    for (place, unit) in &entry.places {
      if replaced && standing.get(place).is_some_and(|filled| filled != unit) {
        candidates.insert(unit.clone());
      } else {
        kept.insert(place.clone(), unit.clone());
      }
    }

    if !kept.is_empty() {
      record.insert(
        shape.clone(),
        ShapeRecord {
          places: kept,
          ..entry.clone()
        },
      );
    }
  }

  record.insert(
    build.shape.clone(),
    ShapeRecord {
      at: now_ms,
      wrapper: build.wrapper.clone(),
      generation: build.generation.clone(),
      places: standing,
    },
  );

  let mut held: HashSet<String> = recorded_units(&record).into_iter().collect();

  for entry in entries_of(&builds.build_root.join("checkouts")) {
    let path = entry.path().join(BUILT_UNITS_NAME);

    if path == own {
      continue;
    }

    let read = read_record(&path);

    held.extend(recorded_units(&read.record));
    held.extend(read.stale);
  }

  SupersededPlan {
    gone: candidates.into_iter().filter(|unit| !held.contains(unit)).collect(),
    record,
  }
}

// A unit's profile directory and hash.
fn split_unit(unit: &str) -> (&str, &str) {
  unit.rsplit_once(':').unwrap_or(("", unit))
}

/// Every path in the build directory carrying one of `units`' hashes in its profile directory.
pub fn swept_paths(build_root: &Path, units: &[String]) -> Vec<PathBuf> {
  let mut by_profile: BTreeMap<&str, HashSet<&str>> = BTreeMap::new();

  for unit in units {
    let (profile, hash) = split_unit(unit);

    by_profile.entry(profile).or_default().insert(hash);
  }

  let mut paths: Vec<PathBuf> = by_profile
    .iter()
    .flat_map(|(profile, hashes)| {
      UNIT_DIRS.iter().flat_map(move |dir| {
        entries_of(&build_root.join(profile).join(dir)).into_iter().filter_map(move |entry| {
          let name = entry.file_name().to_string_lossy().into_owned();

          unit_hash(&name).is_some_and(|hash| hashes.contains(hash)).then(|| entry.path())
        })
      })
    })
    .collect();

  paths.sort();
  paths
}

// About how much disk the paths hold: each file's allocated size, counted once however many links it has.
fn bytes_of(paths: &[PathBuf]) -> u64 {
  fn visit(path: &Path, counted: &mut HashSet<u64>) -> u64 {
    let Ok(held) = std::fs::symlink_metadata(path) else { return 0 };

    if held.is_dir() {
      entries_of(path).iter().map(|entry| visit(&entry.path(), counted)).sum()
    } else if counted.insert(held.ino()) {
      held.blocks() * 512
    } else {
      0
    }
  }

  let mut counted = HashSet::new();

  paths.iter().map(|path| visit(path, &mut counted)).sum()
}

// One lock file held by flock on this process's own descriptor, let go when dropped.
struct HeldBuildLock(File);

impl Drop for HeldBuildLock {
  fn drop(&mut self) {
    use std::os::fd::AsRawFd as _;

    // SAFETY: the descriptor is the open file's own.
    unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
  }
}

fn try_flock(path: &Path) -> Option<HeldBuildLock> {
  use std::os::fd::AsRawFd as _;

  let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
  // SAFETY: the descriptor is the open file's own, and flock reads nothing else.
  let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;

  taken.then_some(HeldBuildLock(file))
}

// Cargo's build lock of every profile directory the units stand in, and of the host's profile directory a
// cross build's stands beside, taken in one order; none where one is not had in time, the removal then leaving
// what it found to the next one.
fn take_build_locks(build_root: &Path, units: &[String]) -> Option<Vec<HeldBuildLock>> {
  let numbers = &SUITE.numbers;
  let profiles: BTreeSet<&str> = units
    .iter()
    .flat_map(|unit| {
      let profile = split_unit(unit).0;

      [profile, profile.rsplit('/').next().unwrap_or(profile)]
    })
    .collect();
  let mut held = Vec::new();

  for profile in profiles {
    let path = build_root.join(profile).join(CARGO_BUILD_LOCK_NAME);
    let until = Instant::now() + Duration::from_secs(numbers.superseded_lock_wait_seconds);

    loop {
      if let Some(lock) = try_flock(&path) {
        held.push(lock);
        break;
      }

      if Instant::now() >= until {
        return None;
      }

      std::thread::sleep(Duration::from_millis(numbers.superseded_lock_poll_ms));
    }
  }

  Some(held)
}

fn now_ms() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

fn target_name(builds: &Builds) -> String {
  builds
    .target_dir
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_default()
}

/// The removal after a passed build of the checkout run under `wrapper`, out of what Cargo printed on standard
/// output in its JSON report. It never fails: what it could not do it says on a line through `say`, and the
/// build's own result stands.
pub fn remove_superseded(builds: &Builds, wrapper: &str, output: &str, say: &mut dyn FnMut(&str)) {
  let units = built_units(builds, output);

  if units.is_empty() {
    return;
  }

  let generation = match build_generation(&builds.root) {
    Ok(generation) => generation,
    Err(why) => return say(&format!("cargo: superseded units not removed: {why}")),
  };
  let build = BuildIdentity {
    shape: shape_of(wrapper, &units),
    wrapper: wrapper.to_owned(),
    generation,
  };
  let plan = plan_superseded(builds, &build, &units, now_ms());
  let record = builds.record_path();
  let written = |say: &mut dyn FnMut(&str)| {
    if let Err(why) = write_record(&record, &plan.record) {
      say(&format!("cargo: superseded units not removed: {why}"));
    }
  };

  if plan.gone.is_empty() {
    return written(say);
  }

  // What it would have removed is named by no entry once the record is written, which makes it the prune's.
  let Some(locks) = take_build_locks(&builds.build_root, &plan.gone) else {
    written(say);

    return say(&format!(
      "cargo: another build holds the build lock; {} superseded units of {} are left to the prune",
      plan.gone.len(),
      target_name(builds)
    ));
  };
  let paths = swept_paths(&builds.build_root, &plan.gone);
  let bytes = bytes_of(&paths);

  for path in &paths {
    let _ = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
  }

  written(say);
  drop(locks);
  say(&format!(
    "cargo: removed {} superseded units of {}, {} MB",
    plan.gone.len(),
    target_name(builds),
    (bytes as f64 / 1e6).round()
  ));
}
