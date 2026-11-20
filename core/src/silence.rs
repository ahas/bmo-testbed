//! A command holding a lane's turn that neither writes nor works for the silence limit is ended as hung, and the
//! turn passes on (docs/testbed.md, 「A holder whose command falls silent lets go」). The holder watches where the
//! command's output goes, the file it hands the command for a watch under it to touch, and the processor time the
//! command's processes use; once none of them has moved for the limit, the command's process group and every
//! process under it are sent a TERM, and whatever still stands after the grace a KILL. A command whose output this
//! process passes on as its own, as a placed run's is, is counted byte by byte as it goes through.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime};

use crate::declaration::Repository;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl Silence {
  /// The silence of the repository a command runs for in `dir`, as its declaration names it; the policy's
  /// where the declaration does not read, since the verb reading it refuses the run on its own.
  pub fn at(dir: &Path) -> Self {
    Repository::read(dir).map_or_else(|_| Self::default(), |repo| repo.declaration.test.silence())
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
  says_working: Option<Duration>,
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

/// The file the holder above this process handed it touched, so that holder counts this process as its
/// command writing.
pub fn speak_for_holder() {
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
      says_working: None,
    }
  }

  /// The same watch, saying on this process's own standard error, at most once every `every`, that its command
  /// still works while none of its output has come for `every`: the `placed` verb's, whose standard error is the
  /// connection back to the machine that placed the run (docs/testbed.md, 「A holder whose command falls silent
  /// lets go」).
  pub fn saying_work(mut self, every: Duration) -> Self {
    self.says_working = Some(every);
    self
  }

  /// The variable the command is handed, where it is watched.
  pub fn variable(&self) -> Option<(&'static str, PathBuf)> {
    self.heard.clone().map(|path| (HEARD_VARIABLE, path))
  }

  /// The command, `what` as the line names it, waited for until it ends, or ended as hung once it has neither
  /// written nor worked for the limit. A command with nothing to watch is waited for until it ends.
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
      let watcher = scope.spawn(move || self.watch(leader, &ending, what));
      let status = child.wait().ok();

      drop(ended);

      let hung = watcher.join().unwrap_or(None);

      Waited {
        status,
        hung: hung.map(|waiting| self.hung_line(what, &waiting)),
      }
    })
  }

  // How much of the command there is to read: its output and the touch of a watch under it.
  fn heard(&self) -> (u64, Option<SystemTime>) {
    (self.output.size(), touched(self.heard.as_ref()))
  }

  // Looks at the command every so often until it ends, and ends it whole once it has neither written nor
  // worked for the limit; what still stood under it then where it was ended so. While it writes or works, the
  // holder above is told so in turn.
  fn watch(&self, leader: u32, ending: &Receiver<()>, what: &str) -> Option<Vec<String>> {
    let mut last = self.heard();
    let mut work = Work::of(leader);
    let mut moved = Instant::now();
    let mut wrote = Instant::now();
    let mut said = Instant::now();

    loop {
      let left = self.silence.limit.saturating_sub(moved.elapsed());

      if !matches!(ending.recv_timeout(self.silence.look.min(left)), Err(RecvTimeoutError::Timeout)) {
        return None;
      }

      let now = self.heard();
      let worked = work.went_on(leader);

      if now.0 != last.0 {
        wrote = Instant::now();
      }

      if now != last || worked {
        last = now;
        moved = Instant::now();
        speak_for_holder();
        self.say_working(what, wrote, &mut said);
      } else if moved.elapsed() >= self.silence.limit {
        let waiting = waiting_under(leader);

        self.end_whole(leader);

        return Some(waiting);
      }
    }
  }

  // That the command still works, said where this watch says so and none of its output came for the while it
  // names, nor was it said within that while.
  fn say_working(&self, what: &str, wrote: Instant, said: &mut Instant) {
    let Some(every) = self.says_working else { return };

    if wrote.elapsed() < every || said.elapsed() < every {
      return;
    }

    eprintln!(
      "{}",
      fill_marks(
        &LANES.silence.working,
        &[("command", what), ("seconds", &wrote.elapsed().as_secs().to_string())]
      )
    );
    *said = Instant::now();
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

  fn hung_line(&self, what: &str, waiting: &[String]) -> String {
    let standing = if waiting.is_empty() {
      String::new()
    } else {
      fill_marks(&LANES.silence.standing, &[("processes", &waiting.join("; "))])
    };

    fill_marks(
      &LANES.silence.hung,
      &[
        ("command", what),
        ("output", &self.output.words()),
        ("seconds", &self.silence.limit.as_secs_f64().to_string()),
        ("standing", &standing),
      ],
    )
  }
}

/// The processor time a command's processes have used, each process's own as the kernel last counted it, and
/// when it was read: what tells a command that computes without a word from one that waits.
struct Work {
  ticks: HashMap<i64, u64>,
  at: Instant,
}

impl Work {
  fn of(leader: u32) -> Self {
    Self {
      ticks: tree_ticks(leader),
      at: Instant::now(),
    }
  }

  // Whether the command's processes used at least the policy's share of one processor since the last reading,
  // a process new since then counting all it used; the reading kept for the next.
  fn went_on(&mut self, leader: u32) -> bool {
    let now = tree_ticks(leader);
    let used: u64 = now
      .iter()
      .map(|(pid, ticks)| ticks.saturating_sub(self.ticks.get(pid).copied().unwrap_or(0)))
      .sum();
    let share = LANES.numbers.working_share_percent as f64 / 100.0;
    let owed = self.at.elapsed().as_secs_f64() * ticks_per_second() * share;

    self.ticks = now;
    self.at = Instant::now();

    used as f64 >= owed && used > 0
  }
}

// The kernel's clock ticks in a second, the unit it counts a process's processor time in.
fn ticks_per_second() -> f64 {
  // SAFETY: asks the kernel for a constant of the system.
  let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };

  if ticks > 0 { ticks as f64 } else { 100.0 }
}

// The processor time each process of a command's tree has used, the command it started among them, by its pid.
fn tree_ticks(leader: u32) -> HashMap<i64, u64> {
  let leader = i64::from(leader);
  let mut tree = descendants_of(leader);

  tree.push(leader);
  tree
    .into_iter()
    .filter_map(|pid| process_ticks(pid).map(|ticks| (pid, ticks)))
    .collect()
}

// A process's processor time in user and system mode, in clock ticks: the fourteenth and fifteenth fields of the
// kernel's record of it, laid out as proc(5) says; none where it has gone. The record is the kernel's own line, which
// no crate the workspace holds reads. Its second field, the process's name in parentheses, is the one that may hold
// spaces or parentheses itself, so the fields are counted from the last closing parenthesis, which ends it.
fn process_ticks(pid: i64) -> Option<u64> {
  let record = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
  let after_name = &record[record.rfind(')')? + 1..];
  let fields: Vec<&str> = after_name.split_whitespace().collect();
  let user: u64 = fields.get(11)?.parse().ok()?;
  let system: u64 = fields.get(12)?.parse().ok()?;

  Some(user + system)
}

// What still stood under a command found silent: each process of its tree that had started nothing further, by
// its command line, the policy's number of them at most. The process at the end of a chain is the one that
// waits; the command the watch started is only where the chain begins.
fn waiting_under(leader: u32) -> Vec<String> {
  let numbers = &LANES.numbers;
  let leader = i64::from(leader);
  let mut tree = descendants_of(leader);

  tree.push(leader);
  tree
    .into_iter()
    .filter(|pid| descendants_of(*pid).is_empty())
    .filter_map(command_line)
    .take(numbers.hung_named_processes)
    .map(|line| line.chars().take(numbers.hung_process_chars).collect())
    .collect()
}

// A process's command line as the kernel keeps it, its words joined by spaces; none where it has gone or holds
// none. The kernel keeps the words one after another, each ended by a NUL byte.
fn command_line(pid: i64) -> Option<String> {
  let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
  let words: Vec<String> = bytes
    .split(|byte| *byte == 0)
    .filter(|word| !word.is_empty())
    .map(|word| String::from_utf8_lossy(word).into_owned())
    .collect();

  (!words.is_empty()).then(|| words.join(" "))
}

/// A command's output read through this process: every byte read counted into `count`, and the holder above
/// this process told at most once every `look` while bytes come, so the holder counts the command writing.
pub struct Heard<R> {
  from: R,
  count: Arc<AtomicU64>,
  look: Duration,
  told: Option<Instant>,
}

impl<R: Read> Heard<R> {
  pub fn new(from: R, count: Arc<AtomicU64>, look: Duration) -> Self {
    Self {
      from,
      count,
      look,
      told: None,
    }
  }
}

impl<R: Read> Read for Heard<R> {
  fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
    let read = self.from.read(into)?;

    if read > 0 {
      self.count.fetch_add(read as u64, Ordering::Relaxed);

      if self.told.is_none_or(|at| at.elapsed() >= self.look) {
        speak_for_holder();
        self.told = Some(Instant::now());
      }
    }

    Ok(read)
  }
}

/// A stream read to its end and written on to `to` as it comes. Writing on stops once `to` refuses, and the
/// stream is still read to its end, so its writer never stalls on a full pipe.
pub fn pass_on(mut from: impl Read, mut to: impl Write) {
  let mut chunk = [0u8; 16 * 1024];
  let mut writing = true;

  while let Ok(read) = from.read(&mut chunk) {
    if read == 0 {
      break;
    }

    if writing {
      writing = to.write_all(&chunk[..read]).and_then(|()| to.flush()).is_ok();
    }
  }
}

/// `work` done while `say` is handed how many seconds it has run, from a thread of its own, at the end of
/// every `every` in which `heard` did not grow: a caller whose run stands on another machine, and burns next
/// to nothing here while it waits, still says it runs, and says nothing more while the run's output comes
/// (docs/testbed.md, 「A run on another machine」).
pub fn said_while_quiet<T>(
  every: Duration,
  heard: &AtomicU64,
  say: &(dyn Fn(u64) + Sync),
  work: impl FnOnce() -> T,
) -> T {
  let started = Instant::now();
  let (ran, ended) = mpsc::channel::<()>();

  std::thread::scope(|scope| {
    scope.spawn(move || {
      let mut last = heard.load(Ordering::Relaxed);

      while let Err(RecvTimeoutError::Timeout) = ended.recv_timeout(every) {
        let now = heard.load(Ordering::Relaxed);

        if now == last {
          say(started.elapsed().as_secs());
        }

        last = now;
      }
    });

    let done = work();

    drop(ran);
    done
  })
}

/// A command run with its standard output and error passed on to this process's own as they come, every byte
/// counted as the command writing and the holder above this process told while bytes come, and ended as hung
/// once it has neither written nor worked for `silence`'s limit; `what` names it on the hung line and on the line
/// said on standard error, at most once every `working_every`, while it works with none of its output coming.
pub fn run_passed_on(
  command: &mut Command,
  silence: Silence,
  what: &str,
  working_every: Duration,
) -> std::io::Result<Waited> {
  let count = Arc::new(AtomicU64::new(0));
  let watch = Watch::new(Output::Read(Arc::clone(&count)), silence).saying_work(working_every);
  let mut child = command.envs(watch.variable()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
  let (done, finished) = mpsc::channel::<()>();
  let mut streams = 0;

  if let Some(out) = child.stdout.take() {
    let (heard, done) = (Heard::new(out, Arc::clone(&count), silence.look), done.clone());

    streams += 1;
    std::thread::spawn(move || {
      pass_on(heard, std::io::stdout());
      let _ = done.send(());
    });
  }

  if let Some(err) = child.stderr.take() {
    let heard = Heard::new(err, count, silence.look);

    streams += 1;
    std::thread::spawn(move || {
      pass_on(heard, std::io::stderr());
      let _ = done.send(());
    });
  }

  let waited = watch.wait(&mut child, what);
  // A process the command left standing may hold its output open past its end; what it still writes within
  // the grace is passed on, and nothing waits for it longer.
  let until = Instant::now() + silence.grace;

  for _ in 0..streams {
    if finished.recv_timeout(until.saturating_duration_since(Instant::now())).is_err() {
      break;
    }
  }

  Ok(waited)
}

impl Drop for Watch {
  fn drop(&mut self) {
    if let Some(heard) = &self.heard {
      let _ = std::fs::remove_file(heard);
    }
  }
}
