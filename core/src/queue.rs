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
//! time. A ticket's file holds the name its taker stands under.
//!
//! A lane of several seats is several such locks sharing the first one's queue (docs/testbed.md, 「The
//! turn」). A taker that has its seat marks its ticket seated, a line of its own under the name, and a seated
//! ticket stands ahead of nobody taking a seat: only the first unseated ticket goes for one.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::policy::lanes::LANES;

const FRONT_MARK: &str = "0-";
const NEXT_MARK: &str = "0.";
const SEATED_MARK: &str = "seated";

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

/// A ticket as the queue holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueTicket {
  /// The process standing it.
  pub pid: i64,
  /// The name its taker stands under; empty where it gave none.
  pub holder: String,
  pub front: bool,
  pub next: bool,
  /// It has taken its seat of a lane.
  pub seated: bool,
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

fn seated(lines: &[String]) -> bool {
  lines.get(1).is_some_and(|line| line == SEATED_MARK)
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
        front: name.starts_with(FRONT_MARK),
        next: name.starts_with(NEXT_MARK),
        seated: seated(&lines),
      })
    })
    .collect()
}

// The living ticket standing ahead of this one, as the name it was stood under (empty where its taker
// gave none), or none where none stands ahead; a seated ticket is passed over where `pass_seated` says so.
// A ticket whose process is gone is taken away.
fn ticket_ahead(queue: &Path, ticket: &str, pass_seated: bool) -> Option<String> {
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

    if pass_seated && seated(&lines) {
      continue;
    }

    return Some(lines.first().cloned().unwrap_or_default());
  }

  None
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
  let ticket = ticket_name(wait.rank);
  let ticket_path = queue.join(&ticket);
  let poll = wait.poll.unwrap_or(Duration::from_millis(LANES.numbers.poll_milliseconds));
  let mut deadline = now_ms() + (wait_seconds.max(0.0) * 1000.0) as i64;

  let _ = std::fs::write(&ticket_path, &wait.holder);

  // A taker goes for the lock only while no living ticket stands ahead of its own, and only without
  // waiting on it: a taker blocked inside flock would be handed the lock by the kernel over a ticket stood
  // ahead of it meanwhile.
  loop {
    let ahead = ticket_ahead(&queue, &ticket, false);

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
          let _ = std::fs::remove_file(&ticket_path);

          return None;
        }
      }
    }

    let left = Duration::from_millis((deadline - now_ms()).max(0) as u64);

    std::thread::sleep(poll.min(left).max(Duration::from_millis(1)));
  }
}

/// How a taker waits for a lane's seats.
pub struct SeatWait<'a> {
  /// The name it stands under, how it waits, its rank and how often it looks, as for one lock.
  pub lock: LockWait<'a>,
  /// Every seat of the lane rather than one.
  pub every: bool,
  /// Asked before one seat is taken while another may be held: whether the machine can take one more.
  pub spare: &'a dyn Fn() -> bool,
}

/// A lane's seats as their taker holds them, let go when this is dropped: the ticket taken away and each
/// seat's descriptor closed.
pub struct HeldSeats {
  files: Vec<(usize, File)>,
  ticket: PathBuf,
}

impl HeldSeats {
  /// The seats held, by their place among the lane's, in order.
  pub fn seats(&self) -> Vec<usize> {
    let mut seats: Vec<usize> = self.files.iter().map(|(at, _)| *at).collect();

    seats.sort_unstable();
    seats
  }
}

impl Drop for HeldSeats {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.ticket);
  }
}

// One free seat for the first unseated taker: any where the machine can take one more, and otherwise one only
// where no seat of the lane is held at all, which every seat taken at once shows.
fn take_one_seat(seats: &[PathBuf], spare: &dyn Fn() -> bool) -> Option<(usize, File)> {
  if spare() {
    return seats.iter().enumerate().find_map(|(at, seat)| try_flock(seat).map(|file| (at, file)));
  }

  let mut every: Vec<File> = seats.iter().map_while(|seat| try_flock(seat)).collect();

  (every.len() == seats.len()).then(|| (0, every.swap_remove(0)))
}

// The ticket marked seated, a line under its name, so the takers behind it no longer wait on it.
fn mark_seated(ticket: &Path) {
  let file = OpenOptions::new().append(true).open(ticket);
  let _ =
    file.and_then(|mut file| std::io::Write::write_all(&mut file, format!("\n{SEATED_MARK}").as_bytes()));
}

/// A lane's seats, the first of them the lane's queue's lock: one, or every one, or none where they could not
/// be had within the wait. Only the first unseated ticket takes a seat; a taker of every seat keeps each seat
/// it takes until it holds them all, and stays unseated until then, so nobody behind it takes one meanwhile.
pub fn take_seats(seats: &[PathBuf], wait_seconds: f64, mut wait: SeatWait<'_>) -> Option<HeldSeats> {
  for seat in seats {
    let _ = OpenOptions::new().append(true).create(true).open(seat);
  }

  let queue = queue_dir_of(seats.first()?);
  let _ = std::fs::create_dir_all(&queue);
  let ticket = ticket_name(wait.lock.rank);
  let ticket_path = queue.join(&ticket);
  let poll = wait.lock.poll.unwrap_or(Duration::from_millis(LANES.numbers.poll_milliseconds));
  let mut deadline = now_ms() + (wait_seconds.max(0.0) * 1000.0) as i64;
  let mut held: Vec<(usize, File)> = Vec::new();

  let _ = std::fs::write(&ticket_path, &wait.lock.holder);

  // As for one lock, a seat is only ever tried without waiting on it, so the kernel never hands one to a
  // taker over a ticket stood ahead of it meanwhile.
  loop {
    let ahead = ticket_ahead(&queue, &ticket, true);

    if ahead.is_none() {
      if wait.every {
        for (at, seat) in seats.iter().enumerate() {
          if !held.iter().any(|(one, _)| *one == at)
            && let Some(file) = try_flock(seat)
          {
            held.push((at, file));
          }
        }
      } else if let Some(one) = take_one_seat(seats, wait.spare) {
        held.push(one);
      }

      if held.len() == if wait.every { seats.len() } else { 1 } {
        mark_seated(&ticket_path);

        return Some(HeldSeats {
          files: held,
          ticket: ticket_path,
        });
      }
    } else {
      // A ticket stood ahead meanwhile, at the front, waits for the seats this one has gathered so far.
      held.clear();
    }

    if now_ms() >= deadline {
      let named = ahead.as_deref().filter(|name| !name.is_empty());
      let later = wait.lock.wait_on.as_mut().and_then(|wait_on| wait_on(named));

      match later {
        Some(later) if later > now_ms() => deadline = later,
        _ => {
          let _ = std::fs::remove_file(&ticket_path);

          return None;
        }
      }
    }

    let left = Duration::from_millis((deadline - now_ms()).max(0) as u64);

    std::thread::sleep(poll.min(left).max(Duration::from_millis(1)));
  }
}
