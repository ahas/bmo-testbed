//! The shared Cargo build directory's ceiling and its enforcement (docs/deploy.md, 「The Cargo build
//! directory」). The ceiling is one size the environment sets, never more than the filesystem holding the
//! directory leaves it with the policy's room free. Once the prune's rules have run, the enforcement removes
//! rebuildable output by five rules in order, each taking its own by least recent read, until the directory
//! holds no more than its ceiling: the shared units and the working checkouts' first, the deployments' checkouts'
//! last, never in a profile directory whose build lock the prune does not hold, never a lock file, and never what a
//! running deployment builds in or a running program was built in. Where the machine sets the deployments'
//! checkouts a ceiling of their own, the last two rules also run on until they hold no more than it. What it may
//! remove is read before the prune removes anything, so a dry run orders it as the run does.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::HeldLock;
use bmo_testbed_core::turn::{Env, leading_number};

use crate::crates::is_deployment_checkout;
use crate::prune::{
  checkout_dirs, entries_of, is_cargo_lock, is_dir, name_of, processes, profile_dirs, remove_path,
  unit_entries, used_at, walk_where,
};

/// The variable naming the ceiling, in gigabytes of a thousand million bytes.
pub const CEILING_VARIABLE: &str = "BMO_CARGO_TARGET_CEILING_GB";

/// The variable naming the deployments' checkouts' own ceiling, in gigabytes of a thousand million bytes.
pub const DEPLOYMENT_CEILING_VARIABLE: &str = "BMO_CARGO_TARGET_DEPLOYMENT_CEILING_GB";

const GIGABYTE: u64 = 1_000_000_000;

/// The size the ceiling is set to: the gigabytes `BMO_CARGO_TARGET_CEILING_GB` names, or the policy's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CeilingSetting {
  pub bytes: u64,
  /// Whether the environment named it.
  pub named: bool,
  /// The deployments' checkouts' own ceiling, in bytes; none, the default, leaves them to the directory's.
  pub deployments: Option<u64>,
}

impl CeilingSetting {
  /// The setting `env` names: a positive whole number of gigabytes, anything else meaning the policy's; and the
  /// deployments' own the same way, the policy's none leaving them to the directory's.
  pub fn of(env: &Env) -> Self {
    let gigabytes = |variable: &str| {
      env
        .get(variable)
        .and_then(|given| leading_number(given))
        .filter(|given| *given > 0)
        .map(|given| (given as u64).saturating_mul(GIGABYTE))
    };
    let deployments = gigabytes(DEPLOYMENT_CEILING_VARIABLE).or_else(|| {
      (PRUNE.deployment_ceiling_gigabytes > 0)
        .then(|| PRUNE.deployment_ceiling_gigabytes.saturating_mul(GIGABYTE))
    });

    match gigabytes(CEILING_VARIABLE) {
      Some(bytes) => Self {
        bytes,
        named: true,
        deployments,
      },
      None => Self {
        bytes: PRUNE.ceiling_gigabytes.saturating_mul(GIGABYTE),
        named: false,
        deployments,
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
      deploying: checkout_dirs(root, deploying),
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

// The live checkouts' directories a rule takes from at `now`: the working checkouts', or the deployments'.
fn checkouts_of(root: &Path, gone: &Gone, deployments: bool, now: i64) -> Vec<PathBuf> {
  live_checkouts(root, gone)
    .into_iter()
    .filter(|dir| is_deployment_checkout(dir, now) == deployments)
    .collect()
}

/// A unit in a profile directory of a live checkout's directory, its dependencies as much as its own crates: of
/// the working checkouts, or of the deployments' checkouts, judged at `now`.
struct CheckoutUnits {
  deployments: bool,
  now: i64,
}

impl Rule for CheckoutUnits {
  fn words(&self) -> &'static str {
    if self.deployments {
      &VERBS.prune.deployment_units
    } else {
      &VERBS.prune.checkout_units
    }
  }

  fn candidates(&self, root: &Path, gone: &Gone) -> Vec<Candidate> {
    checkouts_of(root, gone, self.deployments, self.now)
      .iter()
      .flat_map(|checkout| {
        profile_dirs(checkout)
          .into_iter()
          .flat_map(|profile| units_of(&profile, checkout, gone))
      })
      .collect()
  }
}

/// Whatever else a live checkout's profile directories hold but Cargo's lock files, a checkout at a time: of the
/// working checkouts, or of the deployments' checkouts, judged at `now`.
struct CheckoutDirectories {
  deployments: bool,
  now: i64,
}

impl Rule for CheckoutDirectories {
  fn words(&self) -> &'static str {
    if self.deployments { &VERBS.prune.deployment_dirs } else { &VERBS.prune.checkout_dirs }
  }

  fn candidates(&self, root: &Path, gone: &Gone) -> Vec<Candidate> {
    checkouts_of(root, gone, self.deployments, self.now)
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

// One rule's removals in its order of least recent read, each with the name its line gives it, and whether it takes
// from the deployments' checkouts, which their own ceiling may hold as well as the directory's.
struct RulePlan {
  words: &'static str,
  candidates: Vec<(Candidate, Option<String>)>,
  deployments: bool,
}

/// What the enforcement of the ceiling may remove from a build directory, read before anything is removed, so
/// neither the prune's rules nor the enforcement's own removals make what is left look read.
pub struct CeilingPlan {
  ceiling: Ceiling,
  held: u64,
  rules: Vec<RulePlan>,
  /// What the deployments' checkouts hold, where the machine sets them a ceiling of their own.
  deployments_held: u64,
}

impl CeilingPlan {
  /// Whether the directory holds more than its ceiling, or its deployments' checkouts more than the ceiling the
  /// machine sets them, so the enforcement has something to remove.
  pub fn is_over(&self) -> bool {
    self.held > self.ceiling.bytes
      || self.ceiling.setting.deployments.is_some_and(|limit| self.deployments_held > limit)
  }

  /// What the enforcement may remove from the build directory at `root`, which holds `held` once the prune's
  /// rules have removed every path `gone` names, its deployments' checkouts judged at `now`: each rule's removals
  /// in its order of least recent read, the working checkouts' rules read only where `held` is over `ceiling` and
  /// the deployments' checkouts' also where they hold more than the ceiling the machine sets them.
  pub fn of(root: &Path, gone: &[&Path], ceiling: Ceiling, held: u64, now: i64) -> Self {
    let gone = Gone(gone.iter().map(|path| path.to_path_buf()).collect());
    let over = held > ceiling.bytes;
    let deployments_held = ceiling.setting.deployments.map_or(0, |_| {
      checkouts_of(root, &gone, true, now)
        .iter()
        .map(|dir| {
          let seen = Candidate::of(profile_dirs(dir), Vec::new(), dir, &gone);

          seen.files.iter().map(|file| file.bytes).sum::<u64>()
        })
        .sum()
    });
    let deployments_over = ceiling.setting.deployments.is_some_and(|limit| deployments_held > limit);
    let mut rules: Vec<(Box<dyn Rule>, bool)> = Vec::new();

    if over {
      rules.push((Box::new(SharedUnits), false));
      rules.push((
        Box::new(CheckoutUnits {
          deployments: false,
          now,
        }),
        false,
      ));
      rules.push((
        Box::new(CheckoutDirectories {
          deployments: false,
          now,
        }),
        false,
      ));
    }

    // The deployments' checkouts come last, for the directory's ceiling as much as for their own.
    if over || deployments_over {
      rules.push((
        Box::new(CheckoutUnits {
          deployments: true,
          now,
        }),
        true,
      ));
      rules.push((
        Box::new(CheckoutDirectories {
          deployments: true,
          now,
        }),
        true,
      ));
    }

    let rules = rules
      .into_iter()
      .map(|(rule, deployments)| {
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
          deployments,
        }
      })
      .collect();

    Self {
      ceiling,
      held,
      rules,
      deployments_held,
    }
  }

  /// The ceiling enforced: while the directory holds more than its ceiling, each rule's removals made in its
  /// order, the next rule beginning only where the one before leaves it over, passing over a removal `keepers`
  /// keep or that frees nothing; the deployments' checkouts' two rules last, running on while those hold more than
  /// their own ceiling where the machine sets one. Where `remove` does not hold, nothing is removed and what would
  /// be is counted.
  pub fn enforce(self, keepers: &Keepers, remove: bool) -> Enforcement {
    let ceiling = self.ceiling;
    let mut links = Links::default();
    let mut after = self.held;
    let mut deployments_after = self.deployments_held;
    let mut enforced = Vec::new();
    let mut holders: BTreeMap<(PathBuf, HeldBy), HashMap<(u64, u64), u64>> = BTreeMap::new();

    for rule in self.rules {
      // A rule stops once the directory is under its ceiling, one of the deployments' checkouts' once they are
      // under their own as well.
      let under = |after: u64, deployments_after: u64| {
        after <= ceiling.bytes
          && (!rule.deployments
            || ceiling.setting.deployments.is_none_or(|limit| deployments_after <= limit))
      };

      if under(after, deployments_after) {
        continue;
      }

      let mut done = Enforced {
        words: rule.words,
        count: 0,
        bytes: 0,
        names: Vec::new(),
      };

      for (candidate, name) in rule.candidates {
        if under(after, deployments_after) {
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

        if rule.deployments {
          deployments_after = deployments_after.saturating_sub(frees);
        }

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
