//! The machine's turns for heavy test and build commands (docs/testbed.md, 「The turn」): two lanes, each
//! with its seats on the whole machine, whatever checkout or repository a holder runs in, the rest in line
//! behind them in the order they asked, a caller at the front ahead of them and a caller standing next
//! between the two. The wide lane is the machine's whole width shared by a few seats, a seat beside a held
//! one taken only while the machine has memory to spare; the narrow lane runs beside it, one seat on a
//! quarter of it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::memory::machine_available_bytes;
use crate::policy::fill_marks;
use crate::policy::lanes::LANES;
use crate::queue::{HeldSeats, LockWait, Rank, SeatWait, now_ms, process_alive, queue_tickets, take_seats};

/// The exit code of a command that did not run because its turn did not come.
pub const NOT_RUN_EXIT_CODE: u8 = 75;
/// The variable a holder hands its children: its own process number, so a command started under it runs
/// inside its turn.
pub const HELD_VARIABLE: &str = "BMO_TESTBED_HELD";
/// The variable standing a caller at the front of every lane: a deployment's.
pub const FRONT_VARIABLE: &str = "BMO_TESTBED_FRONT";
/// The variable standing a caller next, behind the front: a close's once its line has come.
pub const NEXT_VARIABLE: &str = "BMO_TESTBED_NEXT";
/// The variable naming a file every turn taken under it writes its wait and its run into.
pub const TURN_TIMES_VARIABLE: &str = "BMO_TESTBED_TIMES";
/// The variable naming the wide lane's seats over the policy's.
pub const SEATS_VARIABLE: &str = "BMO_TESTBED_SEATS";
/// The variable naming the gigabytes a seat beside a held one needs available, over the policy's.
pub const SEAT_FREE_VARIABLE: &str = "BMO_TESTBED_SEAT_FREE_GB";

/// The variables a turn reads, as the process holds them or a test hands them.
pub type Env = HashMap<String, String>;

/// The process's own variables.
pub fn process_env() -> Env {
  std::env::vars().collect()
}

/// The lanes, named in a declaration by their own lowercase names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
  /// The machine's whole width.
  Wide,
  /// A quarter of it, beside the wide lane.
  Narrow,
}

impl Lane {
  /// Every lane, in the order a caller holding them all takes them.
  pub const ALL: [Lane; 2] = [Lane::Wide, Lane::Narrow];

  /// The lane's turn, in words.
  pub fn words(self) -> &'static str {
    match self {
      Self::Wide => &LANES.lanes.wide,
      Self::Narrow => &LANES.lanes.narrow,
    }
  }

  /// How many seats the lane has: the policy's for the wide lane, `BMO_TESTBED_SEATS` naming another, and
  /// one for the narrow lane.
  pub fn seats(self, env: &Env) -> usize {
    match self {
      Self::Wide => match env.get(SEATS_VARIABLE).and_then(|given| leading_number(given)) {
        Some(given) if given > 0 => given as usize,
        _ => LANES.numbers.seats.max(1),
      },
      Self::Narrow => 1,
    }
  }
}

fn named<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
  env.get(name).map(String::as_str).filter(|value| !value.is_empty())
}

/// A lane's lock: the wide lane's in the machine's temporary directory, `BMO_TESTBED_LOCK` naming another,
/// and the narrow lane's beside it.
pub fn lock_path(env: &Env, lane: Lane) -> PathBuf {
  let wide = named(env, "BMO_TESTBED_LOCK").map_or_else(
    || {
      named(env, "TMPDIR")
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
        .join("bmo-testbed.lock")
    },
    PathBuf::from,
  );

  match lane {
    Lane::Wide => wide,
    Lane::Narrow => {
      let text = wide.to_string_lossy();

      PathBuf::from(format!("{}-narrow.lock", text.strip_suffix(".lock").unwrap_or(&text)))
    }
  }
}

/// A lane's seats from its first one's lock: that lock, then each further seat's beside it, `-seat2.lock`
/// and on in place of `.lock`.
pub fn seat_paths(path: &Path, seats: usize) -> Vec<PathBuf> {
  let text = path.to_string_lossy();
  let stem = text.strip_suffix(".lock").unwrap_or(&text);

  std::iter::once(path.to_path_buf())
    .chain((2..=seats).map(|seat| PathBuf::from(format!("{stem}-seat{seat}.lock"))))
    .collect()
}

// The seats of a lane as they stand on the disk, whatever number its takers were given: the first one's lock
// and each further one whose lock stands, in order.
fn standing_seats(path: &Path) -> Vec<PathBuf> {
  let mut seats = vec![path.to_path_buf()];

  while let Some(next) = seat_paths(path, seats.len() + 1).pop().filter(|next| next.exists()) {
    seats.push(next);
  }

  seats
}

/// The bytes of available memory a seat beside a held one needs: the policy's gigabytes, or the ones
/// `BMO_TESTBED_SEAT_FREE_GB` names.
pub fn seat_free_bytes(env: &Env) -> u64 {
  let gigabytes = match env.get(SEAT_FREE_VARIABLE).and_then(|given| leading_number(given)) {
    Some(given) if given >= 0 => given as u64,
    _ => LANES.numbers.seat_free_gigabytes,
  };

  gigabytes.saturating_mul(1 << 30)
}

/// A whole number at the head of a word, as JavaScript's `parseInt` reads one: `12x` is 12, `x` none.
pub fn leading_number(word: &str) -> Option<i64> {
  let word = word.trim_start();
  let (sign, digits) = match word.strip_prefix('-') {
    Some(rest) => (-1, rest),
    None => (1, word.strip_prefix('+').unwrap_or(word)),
  };
  let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());

  digits[..end].parse::<i64>().ok().map(|value| sign * value)
}

/// The machine's width for the one command holding the wide lane: every thread, `BMO_TEST_PARALLEL` naming
/// another; anything that is not a positive whole number means the default.
pub fn machine_width(env: &Env, threads: usize) -> usize {
  match env.get("BMO_TEST_PARALLEL").and_then(|given| leading_number(given)) {
    Some(given) if given > 0 => given as usize,
    _ => threads.max(1),
  }
}

/// The threads this machine has.
pub fn machine_threads() -> usize {
  std::thread::available_parallelism().map_or(1, usize::from)
}

/// A lane's whole width, which a holder of every seat runs on: the machine's for the wide lane, a quarter of
/// it for the narrow one.
pub fn lane_width(lane: Lane, env: &Env, threads: usize) -> usize {
  let width = machine_width(env, threads);

  match lane {
    Lane::Wide => width,
    Lane::Narrow => (width / LANES.numbers.narrow_divisor).max(1),
  }
}

/// Who stands in the line, or holds the turn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnEntry {
  /// The caller's opaque scope, which the testbed never reads; tickets an earlier binary stood carry it
  /// as `place`.
  #[serde(alias = "place")]
  pub scope: Option<String>,
  pub checkout: String,
  pub command: String,
  /// When it asked for the turn, and when the turn came, in milliseconds.
  pub asked: i64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub since: Option<i64>,
  pub pid: i64,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub front: bool,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub next: bool,
}

impl TurnEntry {
  /// Where it stands, in a few words: its caller's scope, its checkout and its command.
  pub fn words(&self) -> String {
    let parts: Vec<&str> = [self.scope.as_deref().unwrap_or(""), &self.checkout, &self.command]
      .into_iter()
      .filter(|part| !part.is_empty())
      .collect();

    if parts.is_empty() {
      fill_marks(&LANES.entry.pid, &[("pid", &self.pid.to_string())])
    } else {
      parts.join(&LANES.entry.joiner)
    }
  }
}

/// How many of a lane's seats a turn runs in, and how many the lane has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seats {
  pub held: usize,
  pub of: usize,
}

impl Seats {
  /// The share of the lane's whole width a command in these seats runs on: the width divided by the seats
  /// for each one held, never less than one, and the whole width for every seat.
  pub fn width(self, lane_width: usize) -> usize {
    if self.held >= self.of {
      return lane_width;
    }

    (lane_width * self.held / self.of.max(1)).max(1)
  }
}

/// A lane's line as it stands: who holds each seat held, in the seats' order, and who waits behind them in
/// order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TestbedQueue {
  pub holders: Vec<TurnEntry>,
  pub waiting: Vec<TurnEntry>,
}

impl TestbedQueue {
  /// The first seat's holder held, or none where no seat is.
  pub fn holder(&self) -> Option<&TurnEntry> {
    self.holders.first()
  }
}

// Who holds a lane's seats, as a waiter says it: each holder as an entry names it, once however many seats it
// holds, or nobody.
fn holders_words(holders: &[TurnEntry], nobody: &str) -> String {
  if holders.is_empty() {
    return nobody.to_owned();
  }

  let mut who: Vec<String> = Vec::new();

  for (at, holder) in holders.iter().enumerate() {
    if holders[..at].iter().all(|earlier| earlier.pid != holder.pid) {
      who.push(holder.words());
    }
  }

  fill_marks(&LANES.waiting.held_by, &[("who", &who.join(&LANES.waiting.between))])
}

fn holder_record_path(path: &Path) -> PathBuf {
  let mut record = path.as_os_str().to_owned();

  record.push(".holder");
  PathBuf::from(record)
}

fn read_holder_record(path: &Path) -> Option<TurnEntry> {
  let record: TurnEntry =
    serde_json::from_str(&std::fs::read_to_string(holder_record_path(path)).ok()?).ok()?;

  process_alive(record.pid).then_some(record)
}

/// One lane's line read now, from its first seat's lock: every seat that stands. A holder record whose
/// process is gone is nobody's.
pub fn read_queue(path: &Path) -> TestbedQueue {
  let holders: Vec<TurnEntry> =
    standing_seats(path).iter().filter_map(|seat| read_holder_record(seat)).collect();
  let waiting = queue_tickets(path)
    .into_iter()
    .map(|ticket| {
      // A ticket stood by a taker outside the testbed names itself in words, or not at all.
      let mut entry = serde_json::from_str::<TurnEntry>(&ticket.holder).unwrap_or_else(|_| TurnEntry {
        command: ticket.holder.clone(),
        ..TurnEntry::default()
      });

      entry.pid = ticket.pid;
      entry.front = ticket.front;
      entry.next = ticket.next;
      entry
    })
    .filter(|entry| holders.iter().all(|holder| holder.pid != entry.pid))
    .collect();

  TestbedQueue { holders, waiting }
}

// How many of these seats the caller named in `BMO_TESTBED_HELD` holds.
fn seats_held_by_caller(seats: &[PathBuf], env: &Env) -> usize {
  let Some(held) = env.get(HELD_VARIABLE).and_then(|pid| leading_number(pid)) else {
    return 0;
  };

  seats
    .iter()
    .filter(|seat| read_holder_record(seat).is_some_and(|record| record.pid == held))
    .count()
}

/// Whether this process runs inside a lane's turn a caller of its own holds: in any seat of the lane whose
/// first seat's lock this is.
pub fn inside_held_turn(path: &Path, env: &Env) -> bool {
  seats_held_by_caller(&standing_seats(path), env) > 0
}

/// The variable naming the caller's opaque scope.
pub const SCOPE_VARIABLE: &str = "BMO_TESTBED_SCOPE";

/// The caller's scope, as the environment names it.
pub fn scope_of(env: &Env) -> Option<String> {
  named(env, SCOPE_VARIABLE).map(str::to_owned)
}

/// The harness a command runs for, by its harness process.
pub fn harness_pid(env: &Env) -> Option<i64> {
  env.get("CLAUDE_PID").and_then(|pid| leading_number(pid)).filter(|pid| *pid > 0)
}

/// Whether a caller stands at the front: one the environment stands there, and no harness's.
pub fn stands_at_front(env: &Env) -> bool {
  env.get(FRONT_VARIABLE).is_some_and(|value| value == "1") && harness_pid(env).is_none()
}

/// Whether a caller stands next: one the environment stands there, a harness's among them, and not at the front.
pub fn stands_next(env: &Env) -> bool {
  env.get(NEXT_VARIABLE).is_some_and(|value| value == "1") && !stands_at_front(env)
}

/// A turn asked for.
pub struct TurnAsk<'a> {
  /// The command, as the line names it.
  pub command: String,
  pub lane: Lane,
  pub say: &'a dyn Fn(&str),
  /// The lock, for a test; the lane's own otherwise.
  pub path: Option<PathBuf>,
  pub env: Env,
  /// The directory the command runs in, which the line names.
  pub checkout: String,
  /// How often the line is looked at while waiting, and how often the waiter says where it stands.
  pub poll: Option<Duration>,
  pub notice_seconds: Option<f64>,
}

/// A turn asked for: had, had already by a caller in its seats, or not had within the wait.
pub enum TestbedTurn {
  Inside {
    seats: Seats,
  },
  Own {
    waited_seconds: f64,
    held: Holding,
  },
  Queued {
    waited_seconds: f64,
    ahead: usize,
    holders: Vec<TurnEntry>,
  },
}

/// A lane's seats held by this process, let go when dropped: their records taken away, the watch on its
/// harness stopped, and the seats let go.
pub struct Holding {
  paths: Vec<PathBuf>,
  seats: Seats,
  _seats: HeldSeats,
  watching: Option<Arc<AtomicBool>>,
}

impl Drop for Holding {
  fn drop(&mut self) {
    if let Some(watching) = &self.watching {
      watching.store(false, Ordering::Relaxed);
    }

    for path in &self.paths {
      if read_holder_record(path).is_some_and(|record| record.pid == i64::from(std::process::id())) {
        let _ = std::fs::remove_file(holder_record_path(path));
      }
    }
  }
}

impl TestbedTurn {
  /// Whether the caller may run its command: inside a turn or holding its own.
  pub fn had(&self) -> bool {
    !matches!(self, Self::Queued { .. })
  }

  pub fn waited_seconds(&self) -> f64 {
    match self {
      Self::Inside { .. } => 0.0,
      Self::Own { waited_seconds, .. } | Self::Queued { waited_seconds, .. } => *waited_seconds,
    }
  }

  /// The share of the lane's whole width a command run under this turn is handed: its seats' share, inside a
  /// caller's turn its caller's, and the whole width for a turn not had.
  pub fn width(&self, lane_width: usize) -> usize {
    match self {
      Self::Inside { seats } => seats.width(lane_width),
      Self::Own { held, .. } => held.seats.width(lane_width),
      Self::Queued { .. } => lane_width,
    }
  }

  /// The variable a command started under this turn is handed, so it runs inside it.
  pub fn held_variable(&self) -> Option<(&'static str, String)> {
    matches!(self, Self::Own { .. }).then(|| (HELD_VARIABLE, std::process::id().to_string()))
  }

  /// What a caller says once its turn is settled.
  pub fn words(&self) -> String {
    let said = &LANES.settled;
    let waited = self.waited_seconds().round().to_string();

    match self {
      Self::Inside { .. } => said.inside.clone(),
      Self::Own { .. } => fill_marks(&said.own, &[("seconds", &waited)]),
      Self::Queued { ahead, holders, .. } => {
        let holder = holders_words(holders, &said.nobody);

        fill_marks(
          &said.queued,
          &[("seconds", &waited), ("ahead", &ahead.to_string()), ("holder", &holder)],
        )
      }
    }
  }
}

// Every process under one, read from the kernel's own children lists, nearest first.
pub(crate) fn descendants_of(pid: i64) -> Vec<i64> {
  let mut found: Vec<i64> = Vec::new();
  let mut next = vec![pid];

  while let Some(parent) = next.pop() {
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{parent}/task")) else { continue };

    for task in tasks.flatten() {
      let Ok(children) = std::fs::read_to_string(task.path().join("children")) else {
        continue;
      };

      for child in children.split_whitespace().filter_map(|word| word.parse::<i64>().ok()) {
        if !found.contains(&child) {
          found.push(child);
          next.push(child);
        }
      }
    }
  }

  found
}

pub(crate) fn signal(pids: &[i64], signal: i32) {
  for pid in pids {
    // SAFETY: a signal to a process of this holder's own tree.
    unsafe { libc::kill(*pid as i32, signal) };
  }
}

/// The watch on a harness holding a turn or a line: once the harness's process is gone, this process and
/// everything under it are killed at once, since nobody is left to read what they would answer, and what it
/// held goes with them. Answers the switch that stops the watch.
pub fn watch_harness(job: i64) -> Arc<AtomicBool> {
  let watching = Arc::new(AtomicBool::new(true));
  let still = Arc::clone(&watching);

  std::thread::spawn(move || {
    while still.load(Ordering::Relaxed) {
      if !process_alive(job) {
        let me = i64::from(std::process::id());
        signal(&descendants_of(me), libc::SIGKILL);
        std::process::exit(i32::from(NOT_RUN_EXIT_CODE));
      }

      std::thread::sleep(Duration::from_secs(LANES.numbers.watch_seconds));
    }
  });

  watching
}

fn waiting_line(ask: &TurnAsk<'_>, queue: &TestbedQueue, waited: f64) -> String {
  let me = i64::from(std::process::id());
  let ahead = queue.waiting.iter().position(|entry| entry.pid == me).unwrap_or(queue.waiting.len());
  let holder = holders_words(&queue.holders, &LANES.waiting.nobody);

  fill_marks(
    &LANES.waiting.line,
    &[
      ("command", &ask.command),
      ("lane", ask.lane.words()),
      ("seconds", &waited.round().to_string()),
      ("ahead", &ahead.to_string()),
      ("holder", &holder),
    ],
  )
}

/// A lane's turn: at once inside a caller's, in line otherwise for as long as the line takes, and not had only
/// where the harness the caller runs for is gone while it waits. A caller at the front takes every seat of
/// the lane, any other one seat.
pub fn take_turn(ask: &TurnAsk<'_>) -> TestbedTurn {
  take_seated_turn(ask, stands_at_front(&ask.env))
}

// A lane's turn in one seat, or in every seat of the lane where `every` says so.
fn take_seated_turn(ask: &TurnAsk<'_>, every: bool) -> TestbedTurn {
  let env = &ask.env;
  let path = ask.path.clone().unwrap_or_else(|| lock_path(env, ask.lane));
  let seats = seat_paths(&path, ask.lane.seats(env));
  let inside = seats_held_by_caller(&seats, env);

  if inside > 0 {
    return TestbedTurn::Inside {
      seats: Seats {
        held: inside,
        of: seats.len(),
      },
    };
  }

  let job = harness_pid(env);
  let front = stands_at_front(env);
  let next = stands_next(env);
  let notice = ask.notice_seconds.unwrap_or(LANES.numbers.notice_seconds as f64);
  let asked = now_ms();
  let me = i64::from(std::process::id());
  let entry = TurnEntry {
    scope: scope_of(env),
    checkout: ask.checkout.clone(),
    command: ask.command.clone(),
    asked,
    since: None,
    pid: me,
    front,
    next,
  };
  // The wait goes on until the turn comes; only a gone harness ends it, since nobody is left to run what it
  // waits for. The notice says where the waiter stands now and again.
  let mut wait_on = |_: Option<&str>| -> Option<i64> {
    let now = now_ms();

    if job.is_some_and(|job| !process_alive(job)) {
      return None;
    }

    (ask.say)(&waiting_line(ask, &read_queue(&path), (now - asked) as f64 / 1000.0));

    Some(now + (notice * 1000.0) as i64)
  };
  // A seat beside a held one only while the machine has the memory a further command needs; memory that
  // cannot be read keeps nobody out.
  let needed = seat_free_bytes(env);
  let spare = || machine_available_bytes().is_none_or(|available| available >= needed);
  let held = take_seats(
    &seats,
    notice,
    SeatWait {
      lock: LockWait {
        holder: serde_json::to_string(&entry).unwrap_or_default(),
        wait_on: Some(&mut wait_on),
        rank: if front {
          Rank::Front
        } else if next {
          Rank::Next
        } else {
          Rank::Ordinary
        },
        poll: ask.poll,
      },
      every,
      spare: &spare,
    },
  );
  let waited_seconds = (now_ms() - asked) as f64 / 1000.0;

  let Some(taken) = held else {
    let queue = read_queue(&path);

    return TestbedTurn::Queued {
      waited_seconds,
      ahead: queue.waiting.len(),
      holders: queue.holders,
    };
  };

  let record = TurnEntry {
    since: Some(now_ms()),
    ..entry
  };
  let record = serde_json::to_string(&record).unwrap_or_default();
  let paths: Vec<PathBuf> = taken.seats().into_iter().map(|at| seats[at].clone()).collect();

  for seat in &paths {
    let _ = std::fs::write(holder_record_path(seat), &record);
  }

  TestbedTurn::Own {
    waited_seconds,
    held: Holding {
      seats: Seats {
        held: paths.len(),
        of: seats.len(),
      },
      paths,
      _seats: taken,
      watching: job.map(watch_harness),
    },
  }
}

/// Several lanes' turns asked for together: had, each lane's turn held until this is dropped, or one lane
/// not had and none held.
pub enum TestbedTurns {
  Had {
    /// The whole time taken over every lane.
    waited_seconds: f64,
    /// Every lane was already its caller's.
    inside: bool,
    held: TurnsHeld,
  },
  Queued(TestbedTurn),
}

/// The lanes' turns held, let go in the reverse of the order they were taken.
pub struct TurnsHeld(Vec<TestbedTurn>);

impl TurnsHeld {
  fn release(&mut self) {
    while self.0.pop().is_some() {}
  }
}

impl Drop for TurnsHeld {
  fn drop(&mut self) {
    self.release();
  }
}

impl TestbedTurns {
  pub fn had(&self) -> bool {
    matches!(self, Self::Had { .. })
  }

  pub fn waited_seconds(&self) -> f64 {
    match self {
      Self::Had { waited_seconds, .. } => *waited_seconds,
      Self::Queued(turn) => turn.waited_seconds(),
    }
  }

  /// The variable a command started under these turns is handed, so it runs inside every one of them.
  pub fn held_variable(&self) -> Option<(&'static str, String)> {
    match self {
      Self::Had { inside: false, .. } => Some((HELD_VARIABLE, std::process::id().to_string())),
      _ => None,
    }
  }

  /// What a caller says once its turns are settled.
  pub fn words(&self) -> String {
    match self {
      Self::Had { inside: true, .. } => LANES.settled.inside.clone(),
      Self::Had { waited_seconds, .. } => {
        fill_marks(&LANES.settled.own, &[("seconds", &waited_seconds.round().to_string())])
      }
      Self::Queued(turn) => turn.words(),
    }
  }
}

/// Several lanes' turns, taken one after another in the order given, every seat of each: had once every one
/// is, and not had where any one is not, those already taken let go again. The wait is the whole time taken,
/// and what the holder starts with its variable runs inside each of them.
pub fn take_turns(lanes: &[Lane], ask: &TurnAsk<'_>) -> TestbedTurns {
  let asked = Instant::now();
  let mut taken = TurnsHeld(Vec::new());

  for lane in lanes {
    let turn = take_seated_turn(
      &TurnAsk {
        command: ask.command.clone(),
        lane: *lane,
        say: ask.say,
        path: None,
        env: ask.env.clone(),
        checkout: ask.checkout.clone(),
        poll: ask.poll,
        notice_seconds: ask.notice_seconds,
      },
      true,
    );

    if let TestbedTurn::Queued { ahead, holders, .. } = turn {
      taken.release();

      return TestbedTurns::Queued(TestbedTurn::Queued {
        waited_seconds: asked.elapsed().as_secs_f64(),
        ahead,
        holders,
      });
    }

    taken.0.push(turn);
  }

  TestbedTurns::Had {
    waited_seconds: asked.elapsed().as_secs_f64(),
    inside: taken.0.iter().all(|turn| matches!(turn, TestbedTurn::Inside { .. })),
    held: taken,
  }
}

/// A command's wait and run written into the file the environment names; nothing where it names none.
pub fn record_turn_times(env: &Env, command: &str, lane: Lane, waited_seconds: f64, ran_seconds: f64) {
  let Some(file) = named(env, TURN_TIMES_VARIABLE) else { return };
  let line = serde_json::json!({
    "command": command,
    "lane": match lane { Lane::Wide => "wide", Lane::Narrow => "narrow" },
    "waitedSeconds": waited_seconds,
    "ranSeconds": ran_seconds,
    "at": now_ms(),
  });

  // What the file says is for whoever reads the run; a command never fails over it.
  let _ = std::fs::OpenOptions::new()
    .append(true)
    .create(true)
    .open(file)
    .and_then(|mut out| std::io::Write::write_all(&mut out, format!("{line}\n").as_bytes()));
}

/// One heavy command's wait for its turn and its run, as a turn taken under `BMO_TESTBED_TIMES` writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnTimes {
  pub command: String,
  pub lane: Lane,
  pub waited_seconds: f64,
  pub ran_seconds: f64,
  /// When it ended, in milliseconds.
  pub at: i64,
}

/// Every wait and run the file holds, in the order written; a line that does not read is passed over.
pub fn read_turn_times(file: &Path) -> Vec<TurnTimes> {
  std::fs::read_to_string(file)
    .unwrap_or_default()
    .lines()
    .filter_map(|line| {
      let one: serde_json::Value = serde_json::from_str(line).ok()?;

      Some(TurnTimes {
        command: one["command"].as_str()?.to_owned(),
        lane: if one["lane"] == "narrow" { Lane::Narrow } else { Lane::Wide },
        waited_seconds: one["waitedSeconds"].as_f64()?,
        ran_seconds: one["ranSeconds"].as_f64()?,
        at: one["at"].as_f64()? as i64,
      })
    })
    .collect()
}

/// A wait and a run in words, as every step of a check says them.
pub fn times_words(waited_seconds: f64, ran_seconds: f64) -> String {
  fill_marks(
    &LANES.times.line,
    &[("waited", &waited_seconds.round().to_string()), ("ran", &format!("{ran_seconds:.1}"))],
  )
}
