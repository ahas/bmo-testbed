//! Where a heavy run goes before it asks for its turn (docs/boundary.md, 「Placement」): here, or on another
//! machine, whose output and exit code come back. The common type asks one thing of a placement; the local
//! placement, always here, is the default every caller with no other stands on. A run placed elsewhere stands
//! on this machine while it lasts, in the lane it asked for (docs/boundary.md, 「A run placed elsewhere stands
//! here too」), one file for each run beside the lane's lock.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::policy::fill_marks;
use crate::policy::lanes::LANES;
use crate::queue::{now_ms, process_alive};
use crate::turn::{
  Env, HELD_VARIABLE, Lane, TurnEntry, lane_seats_held, lock_path, read_queue, scope_of, stands_at_front,
  stands_next,
};

/// The variable a deployment's or a whole run's unit is named by inside it.
pub const DEPLOY_UNIT_VARIABLE: &str = "BMO_DEPLOY_UNIT";
/// The variable the `placed` verb marks the run it starts with: a run on a placed machine, never placed again.
pub const PLACED_VARIABLE: &str = "BMO_TESTBED_PLACED";

/// A run asked about before its turn: its words, its lane and where its caller stands.
pub struct PlaceAsk<'a> {
  pub command: &'a [String],
  /// The language the caller named to read the command, where it named one.
  pub adapter: Option<&'a str>,
  pub lane: Lane,
  /// The directory the run was started in.
  pub cwd: PathBuf,
  pub env: &'a Env,
}

/// Where a run goes.
pub trait Placement {
  /// The exit code the run ended with where it ran elsewhere, or none where it runs here.
  fn place(&self, ask: &PlaceAsk<'_>) -> Option<u8>;
}

/// The placement that keeps every run where it was started.
pub struct Local;

impl Placement for Local {
  fn place(&self, _: &PlaceAsk<'_>) -> Option<u8> {
    None
  }
}

/// The placement of a run that reads what only this machine reaches: a variable its checkout's declaration
/// names, set in its environment. Asked, it says which variable keeps the run here, and keeps it here.
pub struct KeptHere {
  pub variable: String,
}

impl KeptHere {
  /// The placement of a run whose environment sets one of the variables named, the first of them it sets;
  /// none where it sets none.
  pub fn of(env: &Env, variables: &[String]) -> Option<Self> {
    variables
      .iter()
      .find(|name| env.get(*name).is_some_and(|value| !value.is_empty()))
      .map(|variable| Self {
        variable: variable.clone(),
      })
  }
}

impl Placement for KeptHere {
  fn place(&self, _: &PlaceAsk<'_>) -> Option<u8> {
    eprintln!("{}", fill_marks(&LANES.placed.kept, &[("variable", &self.variable)]));
    None
  }
}

/// Whether a run goes where it stands whatever a placement would answer: one started inside a turn its
/// caller holds, one on a placed machine, and one inside a deployment's or a whole run's unit.
pub fn runs_here(env: &Env) -> bool {
  [HELD_VARIABLE, DEPLOY_UNIT_VARIABLE, PLACED_VARIABLE]
    .iter()
    .any(|name| env.get(*name).is_some_and(|value| !value.is_empty()))
}

/// Whether a lane of this machine has no seat for a run now: every one of its own seats held, or someone
/// already waiting for one. A run walking a placement order passes this machine over then; the deployment's
/// seat is never one such a run could take.
pub fn lane_full(env: &Env, lane: Lane) -> bool {
  let lock = lock_path(env, lane);

  lane_seats_held(&lock) >= lane.seats(env) || !read_queue(&lock).waiting.is_empty()
}

static PLACED_RUNS: AtomicU64 = AtomicU64::new(0);

/// A run this machine placed on another, as its lane here keeps it while it lasts: the entry a run here would
/// stand under, its `since` the moment its first turn there came, and the machine it runs on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedEntry {
  #[serde(flatten)]
  pub entry: TurnEntry,
  /// The label of the machine it runs on.
  pub machine: String,
}

// The directory a lane's placed runs stand in, beside its first seat's lock.
fn placed_dir_of(lock: &Path) -> PathBuf {
  let mut dir = lock.as_os_str().to_owned();

  dir.push(".placed");
  PathBuf::from(dir)
}

// Every entry the directory holds, by its file, in the order the files sort: the order the runs were placed.
fn placed_files(dir: &Path) -> Vec<(PathBuf, PlacedEntry)> {
  let Ok(files) = std::fs::read_dir(dir) else { return Vec::new() };
  let mut files: Vec<PathBuf> = files.filter_map(|file| file.ok().map(|file| file.path())).collect();

  files.sort();
  files
    .into_iter()
    .filter_map(|file| {
      let entry = serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;

      Some((file, entry))
    })
    .collect()
}

/// The runs this machine placed elsewhere in a lane, from the lane's first seat's lock, in the order they were
/// placed. An entry whose process is gone is nobody's and is left out.
pub fn read_placed(lock: &Path) -> Vec<PlacedEntry> {
  placed_files(&placed_dir_of(lock))
    .into_iter()
    .map(|(_, placed)| placed)
    .filter(|placed| process_alive(placed.entry.pid))
    .collect()
}

/// A placed run standing in its lane here while it lasts, taken away when dropped. Writing it never changes
/// the run: a file that cannot be written leaves the run unnamed here, and it goes all the same.
pub struct PlacedRun {
  file: PathBuf,
  placed: PlacedEntry,
  came: AtomicBool,
}

impl PlacedRun {
  /// The run a placement is about to send to a machine, standing from now in the lane it asked for, under the
  /// checkout it runs from. Entries whose process is gone are taken away as it stands.
  pub fn stand(ask: &PlaceAsk<'_>, checkout: &Path, machine: &str) -> Self {
    let dir = placed_dir_of(&lock_path(ask.env, ask.lane));
    let pid = std::process::id();
    let asked = now_ms();
    let counter = PLACED_RUNS.fetch_add(1, Ordering::Relaxed) + 1;
    let placed = PlacedEntry {
      entry: TurnEntry {
        scope: scope_of(ask.env),
        checkout: checkout.to_string_lossy().into_owned(),
        command: ask.command.join(" "),
        asked,
        since: None,
        pid: i64::from(pid),
        front: stands_at_front(ask.env),
        next: stands_next(ask.env),
      },
      machine: machine.to_owned(),
    };

    for (file, gone) in placed_files(&dir) {
      if !process_alive(gone.entry.pid) {
        let _ = std::fs::remove_file(file);
      }
    }

    let run = Self {
      file: dir.join(format!("{asked:015}-{counter:06}-{pid}")),
      placed,
      came: AtomicBool::new(false),
    };

    let _ = std::fs::create_dir_all(&dir);
    run.write(&run.placed);
    run
  }

  fn write(&self, placed: &PlacedEntry) {
    let _ = std::fs::write(&self.file, serde_json::to_string(placed).unwrap_or_default());
  }

  /// Its first turn there came: the entry says since when, once, whatever later turns there say.
  pub fn turn_came(&self) {
    if self.came.swap(true, Ordering::Relaxed) {
      return;
    }

    let placed = PlacedEntry {
      entry: TurnEntry {
        since: Some(now_ms()),
        ..self.placed.entry.clone()
      },
      machine: self.placed.machine.clone(),
    };

    self.write(&placed);
  }
}

impl Drop for PlacedRun {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.file);
  }
}
