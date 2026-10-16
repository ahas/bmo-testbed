//! A lock on a file held for as long as its taker wants it, handed out in the order its takers asked
//! (docs/testbed.md, 「The turn」). The lock is `flock(2)` on the file itself, so it excludes every
//! `flock <file> <command>` run and every other taker of the same file, in whatever language it is
//! written, and it goes the moment the holding process closes it or dies.
//!
//! `flock` alone hands a released lock to whichever waiter the kernel wakes, so every taker first stands
//! a ticket in a queue directory beside the lock and goes for the lock only once no living ticket stands
//! ahead of its own; the ticket stays while the lock is held and goes on release. A ticket's name sorts
//! by when it was stood and ends with the process standing it; a front ticket's name opens with a mark
//! sorting ahead of every time, and a next ticket's with one sorting behind the front's and ahead of every
//! time. A ticket's file holds the name its taker stands under on its first line and, for a taker that
//! keeps its place, the key it comes back under and, once kept, the word that says so.
//!
//! A taker that may come back keeps its place when its wait runs out: its ticket passes to a stand-in
//! process living the seconds asked, and a taker asking again under the same key within them takes the
//! ticket back where it stood. A kept place holds nobody up while its taker is away.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::policy::lanes::LANES;

const FRONT_MARK: &str = "0-";
const NEXT_MARK: &str = "0.";
const KEPT_MARK: &str = "kept";

static TICKETS: AtomicU64 = AtomicU64::new(0);

/// Milliseconds since the epoch, as the queue's names and records write time.
pub fn now_ms() -> i64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64)
}

/// Whether a process of this machine is still running; another user's answers that it is.
pub fn process_alive(pid: i64) -> bool {
  let Ok(pid) = i32::try_from(pid) else { return false };

  if pid <= 0 {
    return false;
  }

  // SAFETY: signal 0 asks the kernel only whether the process exists.
  let answered = unsafe { libc::kill(pid, 0) };

  answered == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn queue_dir_of(path: &Path) -> PathBuf {
  let mut dir = path.as_os_str().to_owned();

  dir.push(".queue");
  PathBuf::from(dir)
}

/// Where a ticket stands among the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Rank {
  /// Behind every ticket stood before it.
  #[default]
  Ordinary,
  /// Ahead of every ordinary ticket, in the order front tickets were stood.
  Front,
  /// Behind every front ticket and ahead of every ordinary one.
  Next,
}

fn ticket_name(rank: Rank) -> String {
  let counter = TICKETS.fetch_add(1, Ordering::Relaxed) + 1;
  let mark = match rank {
    Rank::Front => FRONT_MARK,
    Rank::Next => NEXT_MARK,
    Rank::Ordinary => "",
  };

  format!("{mark}{:015}-{counter:06}-{}", now_ms(), std::process::id())
}

fn ticket_pid(name: &str) -> i64 {
  name.rsplit('-').next().and_then(|pid| pid.parse().ok()).unwrap_or(0)
}

// The same place in the line, carried by another process.
fn ticket_for(name: &str, pid: u32) -> String {
  format!("{}{pid}", &name[..name.rfind('-').map_or(0, |at| at + 1)])
}

/// A ticket as the queue holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueTicket {
  /// The process standing it: the taker, or the stand-in keeping its place.
  pub pid: i64,
  /// The name its taker stands under; empty where it gave none.
  pub holder: String,
  pub kept: bool,
  pub front: bool,
  pub next: bool,
}

fn sorted_names(queue: &Path) -> Option<Vec<String>> {
  let mut names: Vec<String> = std::fs::read_dir(queue)
    .ok()?
    .filter_map(|entry| entry.ok().map(|entry| entry.file_name().to_string_lossy().into_owned()))
    .collect();

  names.sort();
  Some(names)
}

fn ticket_lines(queue: &Path, name: &str) -> Option<Vec<String>> {
  std::fs::read_to_string(queue.join(name))
    .ok()
    .map(|text| text.split('\n').map(str::to_owned).collect())
}

/// The living tickets of a lock's queue, in the order they stand: the holder's among them while the lock
/// is held. Nothing is taken away; a ticket whose process is gone is left out.
pub fn queue_tickets(path: &Path) -> Vec<QueueTicket> {
  let queue = queue_dir_of(path);

  sorted_names(&queue)
    .unwrap_or_default()
    .into_iter()
    .filter_map(|name| {
      let pid = ticket_pid(&name);

      if !process_alive(pid) {
        return None;
      }

      let lines = ticket_lines(&queue, &name)?;

      Some(QueueTicket {
        pid,
        holder: lines.first().cloned().unwrap_or_default(),
        kept: lines.get(2).is_some_and(|word| word == KEPT_MARK),
        front: name.starts_with(FRONT_MARK),
        next: name.starts_with(NEXT_MARK),
      })
    })
    .collect()
}

// The living ticket standing ahead of this one, as the name it was stood under (empty where its taker
// gave none), or none where none stands ahead. A kept place is passed over; a ticket whose process is gone
// is taken away.
fn ticket_ahead(queue: &Path, ticket: &str) -> Option<String> {
  for name in sorted_names(queue)? {
    if name.as_str() >= ticket {
      return None;
    }

    if !process_alive(ticket_pid(&name)) {
      let _ = std::fs::remove_file(queue.join(&name));
      continue;
    }

    // Let go between the listing and the read: the next look finds the queue as it now is.
    let Some(lines) = ticket_lines(queue, &name) else { return Some(String::new()) };

    if lines.get(2).is_none_or(|word| word != KEPT_MARK) {
      return Some(lines.first().cloned().unwrap_or_default());
    }
  }

  None
}

/// A place kept for a taker that comes back.
#[derive(Clone, Debug)]
pub struct LockKeep {
  /// What a taker coming back asks under; the same key takes back a kept ticket.
  pub key: String,
  /// How long a place is kept once the wait has run out.
  pub seconds: u64,
}

fn ticket_text(holder: &str, keep: Option<&LockKeep>, kept: bool) -> String {
  match keep {
    None => holder.to_owned(),
    Some(keep) if kept => format!("{holder}\n{}\n{KEPT_MARK}", keep.key),
    Some(keep) => format!("{holder}\n{}", keep.key),
  }
}

// The kept ticket standing under the key, taken back for this process: renamed to carry it, in the same
// place, and its stand-in let go. None where none is kept.
fn take_back(queue: &Path, key: &str) -> Option<String> {
  for name in sorted_names(queue)? {
    let Some(lines) = ticket_lines(queue, &name) else { continue };

    if lines.get(1).is_none_or(|given| given != key)
      || lines.get(2).is_none_or(|word| word != KEPT_MARK)
      || !process_alive(ticket_pid(&name))
    {
      continue;
    }

    let mine = ticket_for(&name, std::process::id());

    // Gone between the listing and the rename: its stand-in ended, and the place with it.
    if std::fs::rename(queue.join(&name), queue.join(&mine)).is_err() {
      continue;
    }

    // SAFETY: a signal to the stand-in's own process, which ends it.
    unsafe { libc::kill(ticket_pid(&name) as i32, libc::SIGTERM) };

    return Some(mine);
  }

  None
}

// The ticket handed to a stand-in that keeps the place for the seconds asked; whether it was kept.
fn keep_place(queue: &Path, ticket: &str, holder: &str, keep: &LockKeep) -> bool {
  let Ok(mut stand_in) = Command::new("sleep")
    .arg(keep.seconds.to_string())
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .process_group(0)
    .spawn()
  else {
    return false;
  };
  let kept = ticket_for(ticket, stand_in.id());
  let handed = std::fs::write(queue.join(ticket), ticket_text(holder, Some(keep), true))
    .and_then(|()| std::fs::rename(queue.join(ticket), queue.join(&kept)));

  if handed.is_err() {
    let _ = stand_in.kill();
    let _ = stand_in.wait();
    return false;
  }

  true
}

/// Asked once a wait has run out, with the name of the taker it waits behind: a later moment to wait until,
/// in milliseconds since the epoch, or none to give up.
pub type WaitOn<'a> = &'a mut dyn FnMut(Option<&str>) -> Option<i64>;

/// How a taker waits for the lock.
#[derive(Default)]
pub struct LockWait<'a> {
  /// The name this taker stands in the queue under, which a taker behind it is told while it waits.
  pub holder: String,
  /// Asked once the wait has run out, with the name of the taker it waits behind — none where the lock is
  /// held from outside the queue: a later moment to wait until, in milliseconds since the epoch, or none to
  /// give up. A wait that goes on keeps its place in the queue.
  pub wait_on: Option<WaitOn<'a>>,
  pub keep: Option<LockKeep>,
  pub rank: Rank,
  /// How often a queued taker looks; a test sets it short.
  pub poll: Option<Duration>,
}

/// The lock as its taker holds it, let go when this is dropped: the descriptor closed and the ticket taken
/// away.
pub struct HeldLock {
  _file: File,
  ticket: PathBuf,
}

impl Drop for HeldLock {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.ticket);
  }
}

fn try_flock(path: &Path) -> Option<File> {
  let file = OpenOptions::new().append(true).create(true).open(path).ok()?;

  // SAFETY: the descriptor is the open file's own, and flock reads nothing else.
  let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;

  taken.then_some(file)
}

/// The lock, or none where it could not be had within the wait.
pub fn take_lock(path: &Path, wait_seconds: f64, mut wait: LockWait<'_>) -> Option<HeldLock> {
  let _ = OpenOptions::new().append(true).create(true).open(path);

  let queue = queue_dir_of(path);
  let _ = std::fs::create_dir_all(&queue);
  let ticket = wait
    .keep
    .as_ref()
    .and_then(|keep| take_back(&queue, &keep.key))
    .unwrap_or_else(|| ticket_name(wait.rank));
  let ticket_path = queue.join(&ticket);
  let poll = wait.poll.unwrap_or(Duration::from_millis(LANES.numbers.poll_milliseconds));
  let mut deadline = now_ms() + (wait_seconds.max(0.0) * 1000.0) as i64;

  let _ = std::fs::write(&ticket_path, ticket_text(&wait.holder, wait.keep.as_ref(), false));

  // A taker goes for the lock only while no living ticket stands ahead of its own, and only without
  // waiting on it: a taker blocked inside flock would be handed the lock by the kernel over a ticket stood
  // ahead of it meanwhile.
  loop {
    let ahead = ticket_ahead(&queue, &ticket);

    if ahead.is_none()
      && let Some(file) = try_flock(path)
    {
      return Some(HeldLock {
        _file: file,
        ticket: ticket_path,
      });
    }

    if now_ms() >= deadline {
      let named = ahead.as_deref().filter(|name| !name.is_empty());
      let later = wait.wait_on.as_mut().and_then(|wait_on| wait_on(named));

      match later {
        Some(later) if later > now_ms() => deadline = later,
        _ => {
          let kept = wait.keep.as_ref().is_some_and(|keep| keep_place(&queue, &ticket, &wait.holder, keep));

          if !kept {
            let _ = std::fs::remove_file(&ticket_path);
          }

          return None;
        }
      }
    }

    let left = Duration::from_millis((deadline - now_ms()).max(0) as u64);

    std::thread::sleep(poll.min(left).max(Duration::from_millis(1)));
  }
}
