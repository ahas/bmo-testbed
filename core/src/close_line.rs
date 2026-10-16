//! The repository's close line (docs/testbed.md, 「The close line」): every close of one repository on this
//! machine stands in it, the first to ask goes first, and the holder holds it from its rebase to the end of
//! its landing. A close waits its turn out while its job lives, saying where it stands as it waits, and
//! leaves the line once its job is gone. And the close's hold on the wide lane through each part of its
//! check.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::policy::close_line::CLOSE_LINE;
use crate::policy::fill_marks;
use crate::queue::{HeldLock, LockWait, Rank, now_ms, process_alive, queue_tickets, take_lock};
use crate::turn::{self, Env, Lane, TestbedTurn, TurnAsk, job_pid, place_name, watch_job};

/// Who stands in the line, or holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseEntry {
  /// The agent place of the job whose close it is, where one runs it.
  pub place: Option<String>,
  pub checkout: String,
  /// When it asked for the line, and when its turn came, in milliseconds.
  pub asked: i64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub since: Option<i64>,
  pub pid: i64,
}

impl CloseEntry {
  /// Where it stands, in a few words: its job's place and its checkout.
  pub fn words(&self) -> String {
    let parts: Vec<&str> = [self.place.as_deref().unwrap_or(""), &self.checkout]
      .into_iter()
      .filter(|part| !part.is_empty())
      .collect();

    if parts.is_empty() {
      fill_marks(&CLOSE_LINE.line.pid, &[("pid", &self.pid.to_string())])
    } else {
      parts.join(&CLOSE_LINE.line.joiner)
    }
  }
}

/// The line as it stands: the close holding it, and the closes waiting behind it in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloseLine {
  pub holder: Option<CloseEntry>,
  pub waiting: Vec<CloseEntry>,
}

fn holder_record_path(path: &Path) -> PathBuf {
  let mut record = path.as_os_str().to_owned();

  record.push(".holder");
  PathBuf::from(record)
}

fn read_holder(path: &Path) -> Option<CloseEntry> {
  let record: CloseEntry =
    serde_json::from_str(&std::fs::read_to_string(holder_record_path(path)).ok()?).ok()?;

  process_alive(record.pid).then_some(record)
}

/// The line read now. A holder record whose process is gone is nobody's.
pub fn read_close_line(path: &Path) -> CloseLine {
  let holder = read_holder(path);
  let waiting = queue_tickets(path)
    .into_iter()
    .map(|ticket| CloseEntry {
      pid: ticket.pid,
      ..serde_json::from_str::<CloseEntry>(&ticket.holder).unwrap_or_default()
    })
    .filter(|entry| holder.as_ref().is_none_or(|holder| holder.pid != entry.pid))
    .collect();

  CloseLine { holder, waiting }
}

/// The line had, let go when dropped: its record taken away, the watch on its job stopped, the lock let go.
pub struct CloseTurn {
  pub waited_seconds: f64,
  path: PathBuf,
  pid: i64,
  _lock: HeldLock,
  watching: Option<Arc<AtomicBool>>,
}

impl Drop for CloseTurn {
  fn drop(&mut self) {
    if let Some(watching) = &self.watching {
      watching.store(false, Ordering::Relaxed);
    }

    if read_holder(&self.path).is_some_and(|holder| holder.pid == self.pid) {
      let _ = std::fs::remove_file(holder_record_path(&self.path));
    }
  }
}

/// A close's ask for the line.
pub struct CloseLineAsk<'a> {
  pub path: PathBuf,
  pub env: Env,
  pub checkout: String,
  pub say: &'a dyn Fn(&str),
  /// How often it says where it stands, and how often it looks; a test sets both short.
  pub notice_seconds: Option<f64>,
  pub poll: Option<Duration>,
  /// Whether the job holding the line is watched, so a job that dies takes its close with it.
  pub watch: bool,
}

/// The close line's turn, waited out: had once every close that asked before this one has let it go, or
/// none where the job this close runs for is gone before then.
pub fn take_close_line(ask: &CloseLineAsk<'_>) -> Option<CloseTurn> {
  let notice = ask.notice_seconds.unwrap_or(CLOSE_LINE.numbers.notice_seconds as f64);
  let job = job_pid(&ask.env);
  let asked = now_ms();
  let me = i64::from(std::process::id());
  let entry = CloseEntry {
    place: place_name(&ask.env),
    checkout: ask.checkout.clone(),
    asked,
    since: None,
    pid: me,
  };
  let mut wait_on = |_: Option<&str>| -> Option<i64> {
    if job.is_some_and(|job| !process_alive(job)) {
      return None;
    }

    let line = read_close_line(&ask.path);
    let ahead = line.waiting.iter().position(|one| one.pid == me).unwrap_or(line.waiting.len());
    let holder = line.holder.as_ref().map_or_else(
      || CLOSE_LINE.line.nobody.clone(),
      |holder| fill_marks(&CLOSE_LINE.line.held_by, &[("who", &holder.words())]),
    );

    (ask.say)(&fill_marks(
      &CLOSE_LINE.line.waiting,
      &[
        ("seconds", &(((now_ms() - asked) as f64) / 1000.0).round().to_string()),
        ("ahead", &ahead.to_string()),
        ("holder", &holder),
      ],
    ));

    Some(now_ms() + (notice * 1000.0) as i64)
  };
  let lock = take_lock(
    &ask.path,
    notice,
    LockWait {
      holder: serde_json::to_string(&entry).unwrap_or_default(),
      wait_on: Some(&mut wait_on),
      keep: None,
      rank: Rank::Ordinary,
      poll: ask.poll,
    },
  )?;
  let record = CloseEntry {
    since: Some(now_ms()),
    ..entry
  };

  let _ = std::fs::write(holder_record_path(&ask.path), serde_json::to_string(&record).unwrap_or_default());

  Some(CloseTurn {
    waited_seconds: (now_ms() - asked) as f64 / 1000.0,
    path: ask.path.clone(),
    pid: me,
    _lock: lock,
    watching: job.filter(|_| ask.watch).map(watch_job),
  })
}

/// The testbed's wide lane held through a close's check, from its static check to the end of its suite, so
/// no other job's heavy command comes between its steps. It is taken standing next, and before each part a
/// deployment waiting at the front goes first.
pub struct WideStretch<'a> {
  env: Env,
  checkout: String,
  say: &'a dyn Fn(&str),
  path: Option<PathBuf>,
  poll: Option<Duration>,
  held: Option<TestbedTurn>,
}

impl<'a> WideStretch<'a> {
  /// The stretch of a close standing next, with the environment its commands are handed.
  pub fn new(env: Env, checkout: String, say: &'a dyn Fn(&str)) -> Self {
    Self {
      env,
      checkout,
      say,
      path: None,
      poll: None,
      held: None,
    }
  }

  /// The same on a lock of a test's own, looked at as often as it says.
  pub fn on(mut self, path: PathBuf, poll: Duration) -> Self {
    self.path = Some(path);
    self.poll = Some(poll);
    self
  }

  fn lock(&self) -> PathBuf {
    self.path.clone().unwrap_or_else(|| turn::lock_path(&self.env, Lane::Wide))
  }

  /// Held, a deployment waiting at the front let in first: false where the job ended meanwhile.
  pub fn ready(&mut self) -> bool {
    if self.held.is_some() && queue_tickets(&self.lock()).iter().any(|one| one.front && !one.kept) {
      (self.say)(&CLOSE_LINE.stretch.front_first);
      self.release();
    }

    if self.held.is_some() {
      return true;
    }

    let turn = turn::take_turn(&TurnAsk {
      command: CLOSE_LINE.stretch.command.clone(),
      lane: Lane::Wide,
      wait_seconds: None,
      say: self.say,
      path: self.path.clone(),
      env: self.env.clone(),
      checkout: self.checkout.clone(),
      poll: self.poll,
      notice_seconds: None,
    });

    if !turn.had() {
      return false;
    }

    (self.say)(&fill_marks(&CLOSE_LINE.stretch.held, &[("words", &turn.words())]));
    self.held = Some(turn);
    true
  }

  /// The variable a command started inside the stretch is handed, so it runs inside it.
  pub fn held_variable(&self) -> Option<(&'static str, String)> {
    self.held.as_ref().and_then(TestbedTurn::held_variable)
  }

  pub fn release(&mut self) {
    self.held = None;
  }
}
