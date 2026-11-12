//! A command holding a lane's turn that writes nothing for the silence limit is ended as hung, and the turn
//! passes on (docs/testbed.md, 「A holder whose command falls silent lets go」). The holder watches where the
//! command's output goes and the file it hands the command for a watch under it to touch; once neither has
//! moved for the limit, the command's process group and every process under it are sent a TERM, and whatever
//! still stands after the grace a KILL.

use std::path::PathBuf;
use std::process::{Child, ExitStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime};

use crate::policy::fill_marks;
use crate::policy::lanes::LANES;
use crate::queue::process_alive;
use crate::turn::{descendants_of, signal};

/// The exit code of a command ended as hung, the one `timeout` ends a command on.
pub const HUNG_EXIT_CODE: u8 = 124;
/// The variable a holder watching its command hands it: a file a watch under it touches while its own command
/// writes, which the holder counts as its command writing.
pub const HEARD_VARIABLE: &str = "BMO_TESTBED_HEARD";

/// How long a command may write nothing, how long it is given to end once sent a TERM, and how often its
/// output is looked at.
#[derive(Clone, Copy, Debug)]
pub struct Silence {
  pub limit: Duration,
  pub grace: Duration,
  pub look: Duration,
}

impl Default for Silence {
  /// The numbers the policy keeps to.
  fn default() -> Self {
    let numbers = &LANES.numbers;

    Self {
      limit: Duration::from_secs(numbers.silence_seconds),
      grace: Duration::from_secs(numbers.silence_grace_seconds),
      look: Duration::from_secs(numbers.silence_look_seconds),
    }
  }
}

/// Where a command's output goes, as the watch reads how much of it there is.
#[derive(Clone, Debug)]
pub enum Output {
  /// Regular files, each read through its path: a link under `/proc/self/fd` reads as the file it stands
  /// for, even one taken away since. None of them means nothing is watched.
  Files(Vec<PathBuf>),
  /// Pipes, counted as their readers take from them.
  Read(Arc<AtomicU64>),
}

impl Output {
  /// The paths as files to watch where every one of them is a regular file, and nothing to watch otherwise:
  /// output that also goes where the watch cannot read would be taken for silence.
  pub fn files_only(paths: impl IntoIterator<Item = PathBuf>) -> Self {
    let paths: Vec<PathBuf> = paths.into_iter().collect();
    let all_files = paths.iter().all(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file()));

    Self::Files(if all_files { paths } else { Vec::new() })
  }

  /// This process's own standard output and error, where both are files.
  pub fn own() -> Self {
    Self::files_only(["/proc/self/fd/1", "/proc/self/fd/2"].map(PathBuf::from))
  }

  /// Whether there is anything to watch.
  pub fn watched(&self) -> bool {
    match self {
      Self::Files(paths) => !paths.is_empty(),
      Self::Read(_) => true,
    }
  }

  fn size(&self) -> u64 {
    match self {
      Self::Files(paths) => paths
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum(),
      Self::Read(count) => count.load(Ordering::Relaxed),
    }
  }

  /// Where the output went, as the line saying a command hung names it.
  pub fn words(&self) -> String {
    match self {
      Self::Files(paths) => {
        let mut named: Vec<String> = Vec::new();

        for path in paths {
          let name = std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.clone())
            .to_string_lossy()
            .into_owned();

          if !named.contains(&name) {
            named.push(name);
          }
        }

        named.join(", ")
      }
      Self::Read(_) => LANES.silence.piped.clone(),
    }
  }
}

/// How a watched command ended.
#[derive(Debug)]
pub struct Waited {
  /// Its status, none where it could not be waited for.
  pub status: Option<ExitStatus>,
  /// The line saying it was ended as hung, where it was.
  pub hung: Option<String>,
}

impl Waited {
  /// The code it ended with: the hung code where it was ended as hung, none where a signal ended it.
  pub fn code(&self) -> Option<i32> {
    if self.hung.is_some() {
      return Some(i32::from(HUNG_EXIT_CODE));
    }

    self.status.and_then(|status| status.code())
  }
}

/// The watch over one command holding a turn. Made before the command starts, so the command is handed the
/// file a watch under it touches, and then waited on; the file goes with it.
pub struct Watch {
  output: Output,
  silence: Silence,
  heard: Option<PathBuf>,
}

// A file of this watch's own in the machine's temporary directory, or none where none could be made.
fn heard_file() -> Option<PathBuf> {
  static MADE: AtomicU64 = AtomicU64::new(0);

  let made = MADE.fetch_add(1, Ordering::Relaxed);
  let path = std::env::temp_dir().join(format!("bmo-testbed-heard-{}-{made}", std::process::id()));

  std::fs::File::create(&path).ok().map(|_| path)
}

// The moment a file was last touched, none where it cannot be read.
fn touched(path: Option<&PathBuf>) -> Option<SystemTime> {
  std::fs::metadata(path?).and_then(|meta| meta.modified()).ok()
}

// The file the holder above this process handed it touched, so that holder counts this command's writing as
// its own command's.
fn speak_for_holder() {
  let Some(path) = std::env::var_os(HEARD_VARIABLE) else { return };
  let file = std::fs::File::options().append(true).open(path);

  // A holder gone, or its file with it, has nobody left to tell.
  let _ = file.and_then(|file| file.set_modified(SystemTime::now()));
}

fn group_standing(leader: i32) -> bool {
  // SAFETY: signal 0 asks the kernel only whether the group has a process left.
  unsafe { libc::kill(-leader, 0) == 0 }
}

fn signal_group(leader: i32, sent: i32) {
  // SAFETY: a signal to the process group the hung command heads.
  unsafe { libc::kill(-leader, sent) };
}

impl Watch {
  pub fn new(output: Output, silence: Silence) -> Self {
    let heard = if output.watched() { heard_file() } else { None };

    Self {
      output,
      silence,
      heard,
    }
  }

  /// The variable the command is handed, where it is watched.
  pub fn variable(&self) -> Option<(&'static str, PathBuf)> {
    self.heard.clone().map(|path| (HEARD_VARIABLE, path))
  }

  /// The command, `what` as the line names it, waited for until it ends, or ended as hung once nothing of it
  /// has moved for the limit. A command with nothing to watch is waited for until it ends.
  pub fn wait(&self, child: &mut Child, what: &str) -> Waited {
    if !self.output.watched() {
      return Waited {
        status: child.wait().ok(),
        hung: None,
      };
    }

    let leader = child.id();
    let (ended, ending) = mpsc::channel::<()>();

    std::thread::scope(|scope| {
      let watcher = scope.spawn(move || self.watch(leader, &ending));
      let status = child.wait().ok();

      drop(ended);

      let hung = watcher.join().unwrap_or(false);

      Waited {
        status,
        hung: hung.then(|| self.hung_line(what)),
      }
    })
  }

  // How much of the command there is to read: its output and the touch of a watch under it.
  fn heard(&self) -> (u64, Option<SystemTime>) {
    (self.output.size(), touched(self.heard.as_ref()))
  }

  // Looks at the command every so often until it ends, and ends it whole once nothing of it has moved for the
  // limit; whether it was ended so. While it moves, the holder above is told so in turn.
  fn watch(&self, leader: u32, ending: &Receiver<()>) -> bool {
    let mut last = self.heard();
    let mut moved = Instant::now();

    loop {
      let left = self.silence.limit.saturating_sub(moved.elapsed());

      if !matches!(ending.recv_timeout(self.silence.look.min(left)), Err(RecvTimeoutError::Timeout)) {
        return false;
      }

      let now = self.heard();

      if now != last {
        last = now;
        moved = Instant::now();
        speak_for_holder();
      } else if moved.elapsed() >= self.silence.limit {
        self.end_whole(leader);

        return true;
      }
    }
  }

  // The command's process group where it heads one, and every process under it, sent a TERM; whatever still
  // stands once the grace has passed, a KILL.
  fn end_whole(&self, leader: u32) {
    let Ok(leader) = i32::try_from(leader) else { return };
    // SAFETY: asks the kernel only which process group the command stands in.
    let heads_group = unsafe { libc::getpgid(leader) } == leader;
    let mut tree = descendants_of(i64::from(leader));

    tree.push(i64::from(leader));

    if heads_group {
      signal_group(leader, libc::SIGTERM);
    }

    signal(&tree, libc::SIGTERM);

    let until = Instant::now() + self.silence.grace;
    let standing =
      |tree: &[i64]| (heads_group && group_standing(leader)) || tree.iter().any(|pid| process_alive(*pid));

    while standing(&tree) {
      let left = until.saturating_duration_since(Instant::now());

      if left.is_zero() {
        break;
      }

      std::thread::sleep(self.silence.look.min(left));
    }

    for pid in descendants_of(i64::from(leader)) {
      if !tree.contains(&pid) {
        tree.push(pid);
      }
    }

    if heads_group {
      signal_group(leader, libc::SIGKILL);
    }

    signal(&tree, libc::SIGKILL);
  }

  fn hung_line(&self, what: &str) -> String {
    fill_marks(
      &LANES.silence.hung,
      &[
        ("command", what),
        ("output", &self.output.words()),
        ("seconds", &self.silence.limit.as_secs_f64().to_string()),
      ],
    )
  }
}

impl Drop for Watch {
  fn drop(&mut self) {
    if let Some(heard) = &self.heard {
      let _ = std::fs::remove_file(heard);
    }
  }
}
