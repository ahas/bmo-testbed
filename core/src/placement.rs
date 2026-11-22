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
  Env, HELD_VARIABLE, Lane, TurnEntry, lane_seats_held, leading_number, lock_path, machine_threads,
  read_queue, scope_of, stands_at_front, stands_next,
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

/// A change check over several checkouts asked about before its turn (docs/boundary.md, 「Placement」): the run as
/// one placed run, its checkouts' top levels in the order given, and the words that run the set elsewhere.
pub struct SetAsk<'a> {
  /// The set as one run: the words it was started with, the wide lane, where it was started.
  pub run: PlaceAsk<'a>,
  pub checkouts: &'a [PathBuf],
  /// The testbed's words, after its binary, that run the set there, handed each checkout's path from the first
  /// checkout's place there, in the order of `checkouts`.
  pub words: &'a dyn Fn(&[String]) -> Vec<String>,
}

/// The code a change check over several checkouts ends with where a placement refuses it.
pub const SET_REFUSED_EXIT_CODE: u8 = 1;

/// Where a run goes.
pub trait Placement {
  /// The exit code the run ended with where it ran elsewhere, or none where it runs here.
  fn place(&self, ask: &PlaceAsk<'_>) -> Option<u8>;

  /// The exit code a change check over several checkouts ended with where it ran elsewhere or was refused, or
  /// none where it runs here. A placement carrying one checkout refuses the set, said on a line, and never
  /// runs it here.
  fn place_set(&self, _: &SetAsk<'_>) -> Option<u8> {
    eprintln!("{}", LANES.placed.set_one_checkout);
    Some(SET_REFUSED_EXIT_CODE)
  }
}

/// The placement that keeps every run where it was started.
pub struct Local;

impl Placement for Local {
  fn place(&self, _: &PlaceAsk<'_>) -> Option<u8> {
    None
  }

  /// A change check over several checkouts runs here too: no placed machine is named to take it.
  fn place_set(&self, _: &SetAsk<'_>) -> Option<u8> {
    None
  }
}

/// The placement of a run that reads what only this machine reaches: a variable its checkout's declaration
/// names, set in its environment, or a file it names, which the checkout holds. Asked, it says which variable or
/// file keeps the run here, and keeps it here.
pub struct KeptHere {
  /// The variable or the file, as the declaration names it.
  pub named: String,
  /// Whether a change check over several checkouts may stay here too: the settings name no placed machine,
  /// or the placed machine's `combined_order` names this machine.
  pub set_here: bool,
}

impl KeptHere {
  /// The placement of a run whose environment sets one of the variables named, the first of them it sets;
  /// none where it sets none.
  pub fn of(env: &Env, variables: &[String]) -> Option<Self> {
    variables
      .iter()
      .find(|name| env.get(*name).is_some_and(|value| !value.is_empty()))
      .map(|variable| Self {
        named: variable.clone(),
        set_here: false,
      })
  }

  /// The placement of a run whose checkout holds one of the files named, each found where `resolved` puts it,
  /// the first of them it holds, a link to a file counting as the file; none where it holds none.
  pub fn holding(files: &[PathBuf], resolved: impl Fn(&Path) -> PathBuf) -> Option<Self> {
    files.iter().find(|file| resolved(file).is_file()).map(|file| Self {
      named: file.to_string_lossy().into_owned(),
      set_here: false,
    })
  }
}

impl Placement for KeptHere {
  fn place(&self, _: &PlaceAsk<'_>) -> Option<u8> {
    eprintln!("{}", fill_marks(&LANES.placed.kept, &[("named", &self.named)]));
    None
  }

  /// The set stays here where no placed machine is named or its `combined_order` names this machine; otherwise
  /// it is refused, naming the variable or the file that would keep it here.
  fn place_set(&self, ask: &SetAsk<'_>) -> Option<u8> {
    if self.set_here {
      return self.place(&ask.run);
    }

    eprintln!("{}", fill_marks(&LANES.placed.set_kept, &[("named", &self.named)]));
    Some(SET_REFUSED_EXIT_CODE)
  }
}

/// Whether a run goes where it stands whatever a placement would answer: one started inside a turn its
/// caller holds, one on a placed machine, and one inside a deployment's or a whole run's unit.
pub fn runs_here(env: &Env) -> bool {
  [HELD_VARIABLE, DEPLOY_UNIT_VARIABLE, PLACED_VARIABLE]
    .iter()
    .any(|name| env.get(*name).is_some_and(|value| !value.is_empty()))
}

/// The variable naming another share of the machine's threads, in percent, at or above which its load makes it
/// loaded, over the lane policy's `loaded_percent`.
pub const LOADED_VARIABLE: &str = "BMO_TESTBED_LOADED_PERCENT";

/// The code `status --free` ends with where the machine takes no run because every seat of the lane is held or
/// waited for.
pub const SEATS_EXIT_CODE: u8 = 1;

/// The code `status --free` ends with where the machine takes no run because it is loaded. Taking a run ends
/// with 0. The placement road reads these codes off the far machine's answer (docs/boundary.md, 「`status`」).
pub const LOADED_EXIT_CODE: u8 = 3;

/// Why a lane takes no run now, and what a machine standing last in a placement order does about it.
#[derive(Debug, PartialEq, Eq)]
pub enum Closed {
  /// Every seat of the lane is held or someone already waits for one. A run that comes to a machine with
  /// this answer last in the order waits in its line all the same: the line is what the order is for.
  Seats,
  /// The machine's load stands at or above the share of its threads, with the line naming its figures. This
  /// takes no run wherever the machine stands, since a run sent to a loaded machine takes many times what it
  /// takes on a calm one.
  Loaded(String),
}

impl Closed {
  /// The line `status --free` says the reason in.
  pub fn said(&self) -> &str {
    match self {
      Self::Seats => &LANES.placed.seats_taken,
      Self::Loaded(line) => line,
    }
  }

  /// The code `status --free` ends with for this reason.
  pub fn code(&self) -> u8 {
    match self {
      Self::Seats => SEATS_EXIT_CODE,
      Self::Loaded(..) => LOADED_EXIT_CODE,
    }
  }
}

/// Why a lane of this machine takes no run now: every one of its own seats held or someone already waiting for
/// one, or the machine loaded (docs/testbed.md, 「A run on another machine」); none where it takes one. The
/// deployment's seat is never one such a run could take.
pub fn lane_closed(env: &Env, lane: Lane) -> Option<Closed> {
  let lock = lock_path(env, lane);

  if lane_seats_held(&lock) >= lane.seats(env) || !read_queue(&lock).waiting.is_empty() {
    return Some(Closed::Seats);
  }

  loaded(machine_load()?, machine_threads(), loaded_percent(env))
}

/// Whether a lane takes a run from a machine standing `last` in a placement order: seats and waiters turn a
/// run away only where another machine follows, while load turns it away wherever the machine stands. Where
/// none of the order's machines takes a run, the run runs where it was started.
pub fn lane_takes(closed: Option<Closed>, last: bool) -> bool {
  match closed {
    None => true,
    Some(Closed::Seats) => last,
    Some(Closed::Loaded(..)) => false,
  }
}

/// Where the machine is loaded: `load`, its load figure, stands at or above `percent` of its `threads`. The
/// share of 0 takes any machine for loaded, and the line it carries names its own figures.
pub fn loaded(load: f64, threads: usize, percent: u64) -> Option<Closed> {
  let float = |count: u64| f64::from(u32::try_from(count).unwrap_or(u32::MAX));
  let bound = float(u64::try_from(threads).unwrap_or(u64::MAX)) * float(percent);

  (load * 100.0 >= bound).then(|| Closed::Loaded(loaded_line(load, threads, percent)))
}

/// The line naming a machine's load figures in the words the policy holds.
fn loaded_line(load: f64, threads: usize, percent: u64) -> String {
  fill_marks(
    &LANES.placed.loaded,
    &[
      ("load", &format!("{load:.2}")),
      ("threads", &threads.to_string()),
      ("percent", &percent.to_string()),
    ],
  )
}

/// The share of the machine's threads, in percent, at or above which its load makes it loaded: the lane
/// policy's, or the whole number of 0 or more `BMO_TESTBED_LOADED_PERCENT` names.
pub fn loaded_percent(env: &Env) -> u64 {
  match env.get(LOADED_VARIABLE).and_then(|given| leading_number(given)) {
    Some(given) if given >= 0 => given.unsigned_abs(),
    _ => LANES.numbers.loaded_percent,
  }
}

// The machine's load figure; none where it does not read, which counts as no load.
fn machine_load() -> Option<f64> {
  load_figure(&std::fs::read_to_string("/proc/loadavg").ok()?)
}

/// The larger of the first two figures `written` names, the last-minute and last-five-minute averages the
/// kernel writes in /proc/loadavg: a minute that has only just eased off does not stand for a calm machine.
/// None where neither reads.
pub fn load_figure(written: &str) -> Option<f64> {
  written
    .split_whitespace()
    .take(2)
    .map_while(|figure| figure.parse::<f64>().ok())
    .reduce(f64::max)
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
