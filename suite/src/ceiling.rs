//! The shared Cargo build directory's ceiling and its enforcement (docs/deploy.md, 「The Cargo build
//! directory」). The ceiling is one size the environment sets, never more than the filesystem holding the
//! directory leaves it with the policy's room free. Once the prune's rules have run, the enforcement removes
//! rebuildable output by three rules in order, each taking its own by least recent read, until the directory
//! holds no more than its ceiling: never in a profile directory whose build lock the prune does not hold, never a
//! lock file, never what a running deployment builds in or a running program was built in. What it may remove is
//! read before the prune removes anything, so a dry run orders it as the run does.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::HeldLock;
use bmo_testbed_core::turn::{Env, leading_number};

use crate::crates::checkout_target_name;
use crate::prune::{
  entries_of, is_cargo_lock, is_dir, name_of, processes, profile_dirs, remove_path, unit_entries, used_at,
  walk_where,
};

/// The variable naming the ceiling, in gigabytes of a thousand million bytes.
pub const CEILING_VARIABLE: &str = "BMO_CARGO_TARGET_CEILING_GB";

const GIGABYTE: u64 = 1_000_000_000;

/// The size the ceiling is set to: the gigabytes `BMO_CARGO_TARGET_CEILING_GB` names, or the policy's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CeilingSetting {
  pub bytes: u64,
  /// Whether the environment named it.
  pub named: bool,
}

impl CeilingSetting {
  /// The setting `env` names: a positive whole number of gigabytes, anything else meaning the policy's.
  pub fn of(env: &Env) -> Self {
    match env
      .get(CEILING_VARIABLE)
      .and_then(|given| leading_number(given))
      .filter(|given| *given > 0)
    {
      Some(given) => Self {
        bytes: (given as u64).saturating_mul(GIGABYTE),
        named: true,
      },
      None => Self {
        bytes: PRUNE.ceiling_gigabytes.saturating_mul(GIGABYTE),
        named: false,
      },
    }
  }
}

/// The build directory's ceiling and how it came to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ceiling {
  pub bytes: u64,
  pub setting: CeilingSetting,
  /// What the filesystem holding the directory leaves it with the room still free.
  pub fits: u64,
  /// The free space the ceiling always leaves, the policy's.
  pub room: u64,
}

impl Ceiling {
  /// The ceiling of a directory holding `held` on a filesystem where this account may write `free` more: the
  /// setting, never more than what leaves the policy's room free there.
  pub fn of(setting: CeilingSetting, held: u64, free: u64) -> Self {
    let room = PRUNE.ceiling_room_gigabytes.saturating_mul(GIGABYTE);
    let fits = held.saturating_add(free).saturating_sub(room);

    Self {
      bytes: setting.bytes.min(fits),
      setting,
      fits,
      room,
    }
  }
}

/// What keeps output from the enforcement whatever its turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HeldBy {
  /// A build holds the build lock of the profile directory it stands in.
  BuildLock,
  /// A running deployment builds in the checkout's directory.
  Deployment,
  /// A program built in the directory is running.
  Program,
}

impl HeldBy {
  /// How the event's payload names it (docs/boundary.md, 「Events」).
  pub fn code(self) -> &'static str {
    match self {
      Self::BuildLock => "build-lock",
      Self::Deployment => "deployment",
      Self::Program => "program",
    }
  }

  /// How the prune's lines name it.
  pub fn words(self) -> &'static str {
    let said = &VERBS.prune;

    match self {
      Self::BuildLock => &said.by_build_lock,
      Self::Deployment => &said.by_deployment,
      Self::Program => &said.by_program,
    }
  }
}

/// A directory whose output the enforcement left because something holds it, with about how much disk that
/// output holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
  pub path: PathBuf,
  pub by: HeldBy,
  pub bytes: u64,
}

/// What keeps output from the enforcement whatever its turn.
#[derive(Clone, Debug, Default)]
pub struct Keepers {
  /// The profile directories whose build lock the prune holds; none where it plans as though it held every
  /// lock it would wait for, as a dry run does.
  pub locked: Option<HashSet<PathBuf>>,
  /// The directories under the build root's `checkouts/` that a running deployment builds in.
  pub deploying: HashSet<PathBuf>,
  /// The executables of the running processes.
  pub running: Vec<PathBuf>,
}

impl Keepers {
  /// What keeps output in the build directory at `root`: the profile directories whose lock `locks` holds, or
  /// every one where none are given; the directories of the checkouts and build trees `deploying` names; and
  /// every program running.
  pub fn read(root: &Path, locks: Option<&[HeldLock]>, deploying: &[PathBuf]) -> Self {
    Self {
      locked: locks
        .map(|locks| locks.iter().filter_map(|lock| lock.path().parent().map(Path::to_path_buf)).collect()),
      deploying: deploying
        .iter()
        .map(|checkout| root.join("checkouts").join(checkout_target_name(checkout)))
        .collect(),
      running: processes().into_iter().filter_map(|process| process.exe).collect(),
    }
  }

  // What keeps a removal: a profile directory it stands in whose lock the prune does not hold, or the running
  // deployment or program its directory answers for.
  fn holder(&self, candidate: &Candidate) -> Option<(PathBuf, HeldBy)> {
    if let Some(locked) = &self.locked
      && let Some(profile) = candidate.profiles.iter().find(|profile| !locked.contains(*profile))
    {
      return Some((profile.clone(), HeldBy::BuildLock));
    }

    let dir = &candidate.dir;

    if self.deploying.contains(dir) {
      return Some((dir.clone(), HeldBy::Deployment));
    }

    self
      .running
      .iter()
      .any(|exe| exe.starts_with(dir))
      .then(|| (dir.clone(), HeldBy::Program))
  }
}

/// One file as the enforcement counts it: its inode, how many links it has and its allocated size.
#[derive(Clone, Copy, Debug)]
struct FileSeen {
  inode: (u64, u64),
  links: u64,
  bytes: u64,
}

/// One removal the enforcement may make.
struct Candidate {
  paths: Vec<PathBuf>,
  /// When anything in it was last read or changed.
  read: i64,
  /// The profile directories it stands in, whose build locks must be the prune's.
  profiles: Vec<PathBuf>,
  /// The directory a running deployment or program keeps it by.
  dir: PathBuf,
  files: Vec<FileSeen>,
}

impl Candidate {
  // The removal of `paths`, read with whatever `gone` holds left out.
  fn of(paths: Vec<PathBuf>, profiles: Vec<PathBuf>, dir: &Path, gone: &Gone) -> Self {
    let mut read = i64::MIN;
    let mut files = Vec::new();

    for path in &paths {
      walk_where(path, &|path| !gone.holds(path), &mut |_, held| {
        read = read.max(used_at(held));

        if held.is_file() {
          files.push(FileSeen {
            inode: (held.dev(), held.ino()),
            links: held.nlink(),
            bytes: held.blocks() * 512,
          });
        }
      });
    }

    Self {
      paths,
      read,
      profiles,
      dir: dir.to_path_buf(),
      files,
    }
  }
}

/// The paths gone already, or that the prune's own rules remove in a run that removes nothing.
struct Gone(HashSet<PathBuf>);

impl Gone {
  fn holds(&self, path: &Path) -> bool {
    path.ancestors().any(|at| self.0.contains(at))
  }
}

/// The links each inode has left once the removals made so far are made, so a removal counts the disk it frees:
/// a file's size once its last link goes.
#[derive(Default)]
struct Links(HashMap<(u64, u64), u64>);

impl Links {
  // Each inode among `files` with how many of their paths link it, its links and its size.
  fn grouped(files: &[FileSeen]) -> HashMap<(u64, u64), (u64, FileSeen)> {
    let mut grouped: HashMap<(u64, u64), (u64, FileSeen)> = HashMap::new();

    for file in files {
      grouped.entry(file.inode).or_insert((0, *file)).0 += 1;
    }

    grouped
  }

  fn left(&self, file: &FileSeen) -> u64 {
    self.0.get(&file.inode).copied().unwrap_or(file.links)
  }

  // The disk removing `files` frees: each file whose last links they are, and none an earlier removal took.
  fn frees(&self, files: &[FileSeen]) -> u64 {
    Self::grouped(files)
      .values()
      .filter(|(count, file)| (1..=*count).contains(&self.left(file)))
      .map(|(_, file)| file.bytes)
      .sum()
  }

  fn take(&mut self, files: &[FileSeen]) {
    for (count, file) in Self::grouped(files).into_values() {
      let left = self.left(&file).saturating_sub(count);

      self.0.insert(file.inode, left);
    }
  }
}

/// One rule of the enforcement: the output it may remove, in no order, and how its line names it.
trait Rule {
  /// How the prune's line names what it removes.
  fn words(&self) -> &'static str;

  /// What it may remove from the build directory at `root`, leaving out whatever `gone` holds.
  fn candidates(&self, root: &Path, gone: &Gone) -> Vec<Candidate>;

  /// The name its line gives one removal, where it names them.
  fn named(&self, _candidate: &Candidate) -> Option<String> {
    None
  }
}

// Every unit in the profile directory that `gone` has not taken, `dir` keeping it.
fn units_of(profile: &Path, dir: &Path, gone: &Gone) -> Vec<Candidate> {
  let mut units: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();

  for found in unit_entries(profile) {
    let path = found.entry.path();

    if !gone.holds(&path) {
      units.entry(found.hash).or_default().push(path);
    }
  }

  units
    .into_values()
    .map(|paths| Candidate::of(paths, vec![profile.to_path_buf()], dir, gone))
    .collect()
}

// The directories under `checkouts/` that `gone` has not taken: the live checkouts' own.
fn live_checkouts(root: &Path, gone: &Gone) -> Vec<PathBuf> {
  entries_of(&root.join("checkouts"))
    .iter()
    .filter(|entry| is_dir(entry) && !gone.holds(&entry.path()))
    .map(|entry| entry.path())
    .collect()
}

/// A unit in a profile directory standing at the build root itself.
struct SharedUnits;

impl Rule for SharedUnits {
  fn words(&self) -> &'static str {
    &VERBS.prune.shared_units
  }

  fn candidates(&self, root: &Path, gone: &Gone) -> Vec<Candidate> {
    profile_dirs(root).iter().flat_map(|profile| units_of(profile, profile, gone)).collect()
  }
}

/// A unit in a profile directory of a live checkout's directory, its dependencies as much as its own crates.
struct CheckoutUnits;

impl Rule for CheckoutUnits {
  fn words(&self) -> &'static str {
    &VERBS.prune.checkout_units
  }

  fn candidates(&self, root: &Path, gone: &Gone) -> Vec<Candidate> {
    live_checkouts(root, gone)
      .iter()
      .flat_map(|checkout| {
        profile_dirs(checkout)
          .into_iter()
          .flat_map(|profile| units_of(&profile, checkout, gone))
      })
      .collect()
  }
}

/// Whatever else a live checkout's profile directories hold but Cargo's lock files, a checkout at a time.
struct CheckoutDirectories;

impl Rule for CheckoutDirectories {
  fn words(&self) -> &'static str {
    &VERBS.prune.checkout_dirs
  }

  fn candidates(&self, root: &Path, gone: &Gone) -> Vec<Candidate> {
    live_checkouts(root, gone)
      .into_iter()
      .filter_map(|checkout| {
        let profiles = profile_dirs(&checkout);
        let paths: Vec<PathBuf> = profiles
          .iter()
          .flat_map(|profile| entries_of(profile))
          .filter(|entry| !is_cargo_lock(&name_of(entry)) && !gone.holds(&entry.path()))
          .map(|entry| entry.path())
          .collect();

        (!paths.is_empty()).then(|| Candidate::of(paths, profiles, &checkout, gone))
      })
      .collect()
  }

  fn named(&self, candidate: &Candidate) -> Option<String> {
    candidate.dir.file_name().map(|name| name.to_string_lossy().into_owned())
  }
}

/// What one rule of the enforcement removed, or would.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enforced {
  pub words: &'static str,
  pub count: usize,
  pub bytes: u64,
  /// The removals it names, where it names them.
  pub names: Vec<String>,
}

/// What the enforcement of the ceiling came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enforcement {
  pub ceiling: Ceiling,
  /// What the directory held before it.
  pub before: u64,
  /// What the directory holds after it, as the disk each removal frees counts it.
  pub after: u64,
  /// Each rule that ran, in its order; none where the directory stood under its ceiling.
  pub rules: Vec<Enforced>,
  /// What holds the rest, where it stays over; none otherwise.
  pub holders: Vec<Holder>,
}

impl Enforcement {
  /// Whether the directory stays over its ceiling.
  pub fn over(&self) -> bool {
    self.after > self.ceiling.bytes
  }
}

// One rule's removals in its order of least recent read, each with the name its line gives it.
struct RulePlan {
  words: &'static str,
  candidates: Vec<(Candidate, Option<String>)>,
}

/// What the enforcement of the ceiling may remove from a build directory, read before anything is removed, so
/// neither the prune's rules nor the enforcement's own removals make what is left look read.
pub struct CeilingPlan {
  ceiling: Ceiling,
  held: u64,
  rules: Vec<RulePlan>,
}

impl CeilingPlan {
  /// What the enforcement may remove from the build directory at `root`, which holds `held` once the prune's
  /// rules have removed every path `gone` names: each rule's removals in its order of least recent read, read
  /// only where `held` is over `ceiling`.
  pub fn of(root: &Path, gone: &[&Path], ceiling: Ceiling, held: u64) -> Self {
    let rules: [&dyn Rule; 3] = [&SharedUnits, &CheckoutUnits, &CheckoutDirectories];
    let gone = Gone(gone.iter().map(|path| path.to_path_buf()).collect());
    let rules = if held > ceiling.bytes {
      rules
        .iter()
        .map(|rule| {
          let mut candidates = rule.candidates(root, &gone);

          candidates
            .sort_by(|one, other| one.read.cmp(&other.read).then_with(|| one.paths.cmp(&other.paths)));

          RulePlan {
            words: rule.words(),
            candidates: candidates
              .into_iter()
              .map(|candidate| {
                let name = rule.named(&candidate);

                (candidate, name)
              })
              .collect(),
          }
        })
        .collect()
    } else {
      Vec::new()
    };

    Self {
      ceiling,
      held,
      rules,
    }
  }

  /// The ceiling enforced: while the directory holds more than its ceiling, each rule's removals made in its
  /// order, the next rule beginning only where the one before leaves it over, passing over a removal `keepers`
  /// keep or that frees nothing. Where `remove` does not hold, nothing is removed and what would be is counted.
  pub fn enforce(self, keepers: &Keepers, remove: bool) -> Enforcement {
    let ceiling = self.ceiling;
    let mut links = Links::default();
    let mut after = self.held;
    let mut enforced = Vec::new();
    let mut holders: BTreeMap<(PathBuf, HeldBy), HashMap<(u64, u64), u64>> = BTreeMap::new();

    for rule in self.rules {
      if after <= ceiling.bytes {
        break;
      }

      let mut done = Enforced {
        words: rule.words,
        count: 0,
        bytes: 0,
        names: Vec::new(),
      };

      for (candidate, name) in rule.candidates {
        if after <= ceiling.bytes {
          break;
        }

        if let Some(held_by) = keepers.holder(&candidate) {
          let held = holders.entry(held_by).or_default();

          held.extend(candidate.files.iter().map(|file| (file.inode, file.bytes)));
          continue;
        }

        let frees = links.frees(&candidate.files);

        if frees == 0 {
          continue;
        }

        if remove {
          for path in &candidate.paths {
            remove_path(path);
          }
        }

        links.take(&candidate.files);
        after = after.saturating_sub(frees);
        done.count += 1;
        done.bytes += frees;
        done.names.extend(name);
      }

      enforced.push(done);
    }

    let holders = if after > ceiling.bytes {
      holders
        .into_iter()
        .map(|((path, by), files)| Holder {
          path,
          by,
          bytes: files.values().sum(),
        })
        .collect()
    } else {
      Vec::new()
    };

    Enforcement {
      ceiling,
      before: self.held,
      after,
      rules: enforced,
      holders,
    }
  }
}
