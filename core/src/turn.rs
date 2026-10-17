//! The machine's turns for heavy test and build commands (docs/testbed.md, 「The turn」): two lanes, each
//! with one holder at a time on the whole machine, whatever checkout or repository it runs in, the rest in
//! line behind it in the order they asked, a caller at the front ahead of them and a caller standing next
//! between the two. The wide lane is the machine's whole width; the narrow lane runs beside it on a quarter
//! of it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::policy::fill_marks;
use crate::policy::lanes::LANES;
use crate::queue::{HeldLock, LockKeep, LockWait, Rank, now_ms, process_alive, queue_tickets, take_lock};

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

/// A lane's width: the machine's for the wide lane, a quarter of it for the narrow one.
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
  /// The agent place of the job that started it, where one did.
  pub place: Option<String>,
  pub checkout: String,
  pub command: String,
  /// When it asked for the turn, and when the turn came, in milliseconds.
  pub asked: i64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub since: Option<i64>,
  pub pid: i64,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub kept: bool,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub front: bool,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub next: bool,
}

impl TurnEntry {
  /// Where it stands, in a few words: its job's place, its checkout and its command.
  pub fn words(&self) -> String {
    let parts: Vec<&str> = [self.place.as_deref().unwrap_or(""), &self.checkout, &self.command]
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

/// A lane's line as it stands: who holds the turn, and who waits behind it in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TestbedQueue {
  pub holder: Option<TurnEntry>,
  pub waiting: Vec<TurnEntry>,
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

/// One lane's line read now. A holder record whose process is gone is nobody's.
pub fn read_queue(path: &Path) -> TestbedQueue {
  let holder = read_holder_record(path);
  let waiting = queue_tickets(path)
    .into_iter()
    .map(|ticket| {
      // A ticket stood by a taker outside the testbed names itself in words, or not at all.
      let mut entry = serde_json::from_str::<TurnEntry>(&ticket.holder).unwrap_or_else(|_| TurnEntry {
        command: ticket.holder.clone(),
        ..TurnEntry::default()
      });

      entry.pid = ticket.pid;
      entry.kept = ticket.kept;
      entry.front = ticket.front;
      entry.next = ticket.next;
      entry
    })
    .filter(|entry| holder.as_ref().is_none_or(|holder| holder.pid != entry.pid))
    .collect();

  TestbedQueue { holder, waiting }
}

/// Whether this process runs inside a lane's turn a caller of its own holds.
pub fn inside_held_turn(path: &Path, env: &Env) -> bool {
  let held = env.get(HELD_VARIABLE).and_then(|pid| leading_number(pid));

  held.is_some() && read_holder_record(path).map(|record| record.pid) == held
}

/// The agent place a command runs in, by the place's own name.
pub fn place_name(env: &Env) -> Option<String> {
  named(env, "BMO_AGENT_PLACE")
    .map(|place| place.trim_end_matches('/'))
    .and_then(|place| place.rsplit('/').next())
    .filter(|name| !name.is_empty())
    .map(str::to_owned)
}

/// The job a command runs for, by its agent process.
pub fn job_pid(env: &Env) -> Option<i64> {
  env.get("CLAUDE_PID").and_then(|pid| leading_number(pid)).filter(|pid| *pid > 0)
}

/// Whether a caller stands at the front: one the environment stands there, and no job's.
pub fn stands_at_front(env: &Env) -> bool {
  env.get(FRONT_VARIABLE).is_some_and(|value| value == "1") && job_pid(env).is_none()
}

/// Whether a caller stands next: one the environment stands there, a job's among them, and not at the front.
pub fn stands_next(env: &Env) -> bool {
  env.get(NEXT_VARIABLE).is_some_and(|value| value == "1") && !stands_at_front(env)
}

/// Whether a caller at the front holds the lane or stands ahead of a waiter in it — a kept place of one
/// passed over — which is the time a waiter's wait does not count, up to its allowance.
pub fn front_ahead(queue: &TestbedQueue, pid: i64) -> bool {
  let mine = queue.waiting.iter().position(|entry| entry.pid == pid);
  let ahead = &queue.waiting[..mine.unwrap_or(queue.waiting.len())];

  queue.holder.as_ref().is_some_and(|holder| holder.front)
    || ahead.iter().any(|entry| entry.front && !entry.kept)
}

/// A turn asked for.
pub struct TurnAsk<'a> {
  /// The command, as the line names it.
  pub command: String,
  pub lane: Lane,
  /// How long to wait; the caller's own default where none is given.
  pub wait_seconds: Option<f64>,
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

/// A turn asked for: had, had already by a caller, or not had within the wait.
pub enum TestbedTurn {
  Inside,
  Own {
    waited_seconds: f64,
    held: Holding,
  },
  Queued {
    waited_seconds: f64,
    ahead: usize,
    holder: Option<TurnEntry>,
    kept_seconds: Option<u64>,
  },
}

/// A lane's turn held by this process, let go when dropped: its record taken away, the watch on its job
/// stopped, and the lock let go.
pub struct Holding {
  path: PathBuf,
  _lock: HeldLock,
  watching: Option<Arc<AtomicBool>>,
}

impl Drop for Holding {
  fn drop(&mut self) {
    if let Some(watching) = &self.watching {
      watching.store(false, Ordering::Relaxed);
    }

    if read_holder_record(&self.path).is_some_and(|record| record.pid == i64::from(std::process::id())) {
      let _ = std::fs::remove_file(holder_record_path(&self.path));
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
      Self::Inside => 0.0,
      Self::Own { waited_seconds, .. } | Self::Queued { waited_seconds, .. } => *waited_seconds,
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
      Self::Inside => said.inside.clone(),
      Self::Own { .. } => fill_marks(&said.own, &[("seconds", &waited)]),
      Self::Queued {
        ahead,
        holder,
        kept_seconds,
        ..
      } => {
        let holder = holder.as_ref().map_or_else(
          || said.nobody.clone(),
          |holder| fill_marks(&LANES.waiting.held_by, &[("who", &holder.words())]),
        );
        let kept = kept_seconds
          .map_or_else(String::new, |seconds| fill_marks(&said.kept, &[("seconds", &seconds.to_string())]));

        fill_marks(
          &said.queued,
          &[
            ("seconds", &waited),
            ("ahead", &ahead.to_string()),
            ("holder", &holder),
            ("kept", &kept),
          ],
        )
      }
    }
  }
}

// Every process under one, read from the kernel's own children lists, nearest first.
fn descendants_of(pid: i64) -> Vec<i64> {
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

fn signal(pids: &[i64], signal: i32) {
  for pid in pids {
    // SAFETY: a signal to a process of this holder's own tree.
    unsafe { libc::kill(*pid as i32, signal) };
  }
}

/// A process and everything under it stopped: asked first, killed once the grace is out.
pub fn stop_tree(pid: i64, grace: Duration) {
  let all: Vec<i64> = std::iter::once(pid).chain(descendants_of(pid)).collect();

  signal(&all, libc::SIGTERM);
  std::thread::sleep(grace);
  signal(&all.into_iter().filter(|pid| process_alive(*pid)).collect::<Vec<_>>(), libc::SIGKILL);
}

/// The watch on a job holding a turn or a line: once the job's agent process is gone, this process and
/// everything under it are stopped, and what it held goes with them. Answers the switch that stops the
/// watch.
pub fn watch_job(job: i64) -> Arc<AtomicBool> {
  let watching = Arc::new(AtomicBool::new(true));
  let still = Arc::clone(&watching);

  std::thread::spawn(move || {
    while still.load(Ordering::Relaxed) {
      if !process_alive(job) {
        let me = i64::from(std::process::id());
        let under = descendants_of(me);

        signal(&under, libc::SIGTERM);
        std::thread::sleep(Duration::from_secs(LANES.numbers.stop_grace_seconds));
        signal(&under.into_iter().filter(|pid| process_alive(*pid)).collect::<Vec<_>>(), libc::SIGKILL);
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
  let holder = queue.holder.as_ref().map_or_else(
    || LANES.waiting.nobody.clone(),
    |holder| fill_marks(&LANES.waiting.held_by, &[("who", &holder.words())]),
  );

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

/// A lane's turn: at once inside a caller's, in line otherwise, and not had once the wait runs out.
pub fn take_turn(ask: &TurnAsk<'_>) -> TestbedTurn {
  let env = &ask.env;
  let path = ask.path.clone().unwrap_or_else(|| lock_path(env, ask.lane));

  if inside_held_turn(&path, env) {
    return TestbedTurn::Inside;
  }

  let job = job_pid(env);
  let front = stands_at_front(env);
  let next = stands_next(env);
  // A caller standing next waits its turn out; the job it runs for ends its wait by ending.
  let numbers = &LANES.numbers;
  let wait_seconds = ask.wait_seconds.unwrap_or(if next {
    f64::INFINITY
  } else if job.is_none() {
    numbers.unit_wait_seconds as f64
  } else {
    numbers.job_wait_seconds as f64
  });
  let notice = ask.notice_seconds.unwrap_or(numbers.notice_seconds as f64);
  let asked = now_ms();
  let mut until = if wait_seconds.is_finite() { asked + (wait_seconds * 1000.0) as i64 } else { i64::MAX };
  let mut allowance = (numbers.front_allowance_seconds * 1000) as i64;
  let mut looked = asked;
  let me = i64::from(std::process::id());
  let entry = TurnEntry {
    place: place_name(env),
    checkout: ask.checkout.clone(),
    command: ask.command.clone(),
    asked,
    since: None,
    pid: me,
    kept: false,
    front,
    next,
  };
  // A place is kept for a job that may come back, which a caller standing next never leaves to do.
  let keeps = job.is_some() && !next;
  let mut wait_on = |_: Option<&str>| -> Option<i64> {
    let now = now_ms();

    // Nobody is left to run what a gone job waits for.
    if job.is_some_and(|job| !process_alive(job)) {
      return None;
    }

    let queue = read_queue(&path);

    // The time spent behind a caller at the front is not the wait's own, up to the allowance.
    if front_ahead(&queue, me) {
      let behind = allowance.min(now - looked);

      until = until.saturating_add(behind);
      allowance -= behind;
    }

    looked = now;

    if now >= until {
      return None;
    }

    (ask.say)(&waiting_line(ask, &queue, (now - asked) as f64 / 1000.0));

    Some(until.min(now + (notice * 1000.0) as i64))
  };
  let held = take_lock(
    &path,
    notice.min(wait_seconds),
    LockWait {
      holder: serde_json::to_string(&entry).unwrap_or_default(),
      wait_on: Some(&mut wait_on),
      keep: keeps.then(|| LockKeep {
        key: format!("job {}", job.unwrap_or_default()),
        seconds: numbers.kept_place_seconds,
      }),
      rank: if front {
        Rank::Front
      } else if next {
        Rank::Next
      } else {
        Rank::Ordinary
      },
      poll: ask.poll,
    },
  );
  let waited_seconds = (now_ms() - asked) as f64 / 1000.0;

  let Some(lock) = held else {
    let queue = read_queue(&path);
    let ahead = queue.waiting.iter().position(|one| one.kept && one.asked == entry.asked);

    return TestbedTurn::Queued {
      waited_seconds,
      ahead: ahead.unwrap_or(queue.waiting.len()),
      holder: queue.holder,
      kept_seconds: keeps.then_some(numbers.kept_place_seconds),
    };
  };

  let record = TurnEntry {
    since: Some(now_ms()),
    ..entry
  };
  let _ = std::fs::write(holder_record_path(&path), serde_json::to_string(&record).unwrap_or_default());

  TestbedTurn::Own {
    waited_seconds,
    held: Holding {
      path,
      _lock: lock,
      watching: job.map(watch_job),
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
      Self::Had { inside: true, .. } => TestbedTurn::Inside.words(),
      Self::Had { waited_seconds, .. } => {
        fill_marks(&LANES.settled.own, &[("seconds", &waited_seconds.round().to_string())])
      }
      Self::Queued(turn) => turn.words(),
    }
  }
}

/// Several lanes' turns, taken one after another in the order given: had once every one is, and not had
/// where any one is not, those already taken let go again. The wait is the whole time taken, and what the
/// holder starts with its variable runs inside each of them.
pub fn take_turns(lanes: &[Lane], ask: &TurnAsk<'_>) -> TestbedTurns {
  let asked = Instant::now();
  let mut taken = TurnsHeld(Vec::new());

  for lane in lanes {
    let turn = take_turn(&TurnAsk {
      command: ask.command.clone(),
      lane: *lane,
      wait_seconds: ask.wait_seconds,
      say: ask.say,
      path: None,
      env: ask.env.clone(),
      checkout: ask.checkout.clone(),
      poll: ask.poll,
      notice_seconds: ask.notice_seconds,
    });

    if let TestbedTurn::Queued {
      ahead,
      holder,
      kept_seconds,
      ..
    } = turn
    {
      taken.release();

      return TestbedTurns::Queued(TestbedTurn::Queued {
        waited_seconds: asked.elapsed().as_secs_f64(),
        ahead,
        holder,
        kept_seconds,
      });
    }

    taken.0.push(turn);
  }

  TestbedTurns::Had {
    waited_seconds: asked.elapsed().as_secs_f64(),
    inside: taken.0.iter().all(|turn| matches!(turn, TestbedTurn::Inside)),
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
