//! A heavy command run in the machine's turn and run again where the kernel killed it for want of the
//! machine's memory rather than at the ceiling of a fence it stands in (docs/testbed.md, 「Memory」). The
//! Cargo verb runs every compiling command so, as does a declared check step that asks for it. Each ends
//! on a line saying how long it waited and how long it ran.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::policy::fill_marks;
use crate::policy::lanes::LANES;
use crate::silence::{HUNG_EXIT_CODE, Waited};
use crate::turn::{
  Env, Lane, NOT_RUN_EXIT_CODE, TestbedTurn, TurnAsk, record_turn_times, take_turn, times_words,
};

/// How the wait for memory is paced, and how many runs a command is given.
#[derive(Clone, Copy, Debug)]
pub struct MemoryLimits {
  /// How long a killed command waits before it looks at the memory at all.
  pub settle: Duration,
  /// How much memory has to be available before a killed command runs again.
  pub needed_bytes: u64,
  /// How often it looks, and how often it says it is still waiting.
  pub poll: Duration,
  pub notice: Duration,
  /// Runs in all, the first included.
  pub runs: u32,
}

impl Default for MemoryLimits {
  /// The limits the policy keeps to.
  fn default() -> Self {
    let limits = &LANES.memory_limits;

    Self {
      settle: Duration::from_secs(limits.settle_seconds),
      needed_bytes: limits.needed_gigabytes << 30,
      poll: Duration::from_secs(limits.poll_seconds),
      notice: Duration::from_secs(limits.notice_seconds),
      runs: limits.runs,
    }
  }
}

/// How one run of the command ended: its exit code, or the signal that ended it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunEnd {
  pub code: Option<i32>,
  pub signal: Option<i32>,
}

impl RunEnd {
  pub fn passed(self) -> bool {
    self.code == Some(0)
  }

  /// The code a caller ends on: the run's own, or 1 where a signal ended it.
  pub fn exit_code(self) -> u8 {
    self.code.and_then(|code| u8::try_from(code).ok()).unwrap_or(1)
  }

  fn words(self) -> String {
    let said = &LANES.memory;

    match (self.signal, self.code) {
      (Some(signal), _) => fill_marks(&said.signal, &[("signal", &signal.to_string())]),
      (None, code) => fill_marks(&said.exit, &[("code", &code.unwrap_or(1).to_string())]),
    }
  }
}

impl From<&Waited> for RunEnd {
  /// A watched run: the hung code where it was ended as hung, its status otherwise.
  fn from(waited: &Waited) -> Self {
    match (&waited.hung, waited.status) {
      (Some(_), _) | (None, None) => Self {
        code: waited.code(),
        signal: None,
      },
      (None, Some(status)) => Self::from(status),
    }
  }
}

impl From<std::process::ExitStatus> for RunEnd {
  fn from(status: std::process::ExitStatus) -> Self {
    use std::os::unix::process::ExitStatusExt as _;

    Self {
      code: status.code(),
      signal: status.signal(),
    }
  }
}

/// A control group with a memory ceiling, and how often its own usage has reached that ceiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryFence {
  pub cgroup: String,
  pub ceiling_bytes: u64,
  pub ceiling_reached: u64,
}

/// What the command stands on, each a seam a test fills.
pub trait HeavyHands {
  /// The command, as the lines about it name it.
  fn what(&self) -> String;
  /// The machine's turn, with whatever waiting it says on the way.
  fn take_turn(&mut self) -> TestbedTurn;
  /// One run of the command, inside the turn it is handed.
  fn run(&mut self, turn: &TestbedTurn) -> RunEnd;
  /// The kernel's count of out-of-memory kills since boot, and the memory available now; none where the
  /// machine does not say.
  fn oom_kills(&self) -> Option<u64>;
  fn available_bytes(&self) -> Option<u64>;
  /// The memory fences the command stands in, as they stand now.
  fn fences(&self) -> Vec<MemoryFence>;
  fn sleep(&mut self, how_long: Duration);
  /// Seconds on a clock that only moves forward.
  fn now(&self) -> f64;
  fn say(&mut self, line: &str);
  /// Where the wait for the turn and the run after it are written down, for a caller that reads them.
  fn record(&mut self, waited_seconds: f64, ran_seconds: f64);
}

// The kernel's own listings under /proc are lines of a name and a number, a shape of their own that no
// library reads; a line is found by its name and its first number taken.
pub fn proc_number(file: &Path, name: &str) -> Option<u64> {
  let text = std::fs::read_to_string(file).ok()?;

  text.lines().find_map(|line| {
    let mut words = line.split(|c: char| c.is_whitespace() || c == ':').filter(|word| !word.is_empty());

    (words.next() == Some(name)).then(|| words.next()?.parse().ok())?
  })
}

/// The kernel's count of out-of-memory kills since boot.
pub fn machine_oom_kills() -> Option<u64> {
  proc_number(Path::new("/proc/vmstat"), "oom_kill")
}

/// The memory available now.
pub fn machine_available_bytes() -> Option<u64> {
  proc_number(Path::new("/proc/meminfo"), "MemAvailable").map(|kilobytes| kilobytes * 1024)
}

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

// The unified hierarchy's line of /proc/self/cgroup is its id, an empty list of controllers and the group's
// path, colon to colon; the path is everything after the second colon.
fn own_cgroup() -> Option<String> {
  let text = std::fs::read_to_string("/proc/self/cgroup").ok()?;

  text.lines().find_map(|line| line.strip_prefix("0::")).map(str::to_owned)
}

/// Every group from this process's own up to the root that has a memory ceiling, each with its own count of
/// reaching it: the local `oom` event, which only that group's ceiling raises. Its `oom_kill` is no such
/// sign, since a kill of the whole machine's shortage counts in the victim's group as well. None where the
/// machine keeps no unified hierarchy.
pub fn machine_fences() -> Vec<MemoryFence> {
  let Some(own) = own_cgroup() else { return Vec::new() };

  Path::new(&own)
    .ancestors()
    .filter(|group| !matches!(group.to_str(), Some("/" | "")))
    .filter_map(|group| {
      let dir = Path::new(CGROUP_ROOT).join(group.strip_prefix("/").unwrap_or(group));
      let ceiling_bytes = std::fs::read_to_string(dir.join("memory.max")).ok()?.trim().parse().ok()?;
      let ceiling_reached = proc_number(&dir.join("memory.events.local"), "oom")?;

      Some(MemoryFence {
        cgroup: group.to_string_lossy().into_owned(),
        ceiling_bytes,
        ceiling_reached,
      })
    })
    .collect()
}

/// The fence whose own ceiling was reached between the two readings, or none.
pub fn fence_reached<'a>(before: &[MemoryFence], after: &'a [MemoryFence]) -> Option<&'a MemoryFence> {
  after.iter().find(|fence| {
    before
      .iter()
      .find(|one| one.cgroup == fence.cgroup)
      .is_some_and(|earlier| fence.ceiling_reached > earlier.ceiling_reached)
  })
}

/// Whether a run is taken for the kernel's out-of-memory kill: it failed while the kernel's count of such
/// kills rose. The kill may land on a process the command started rather than the command itself, and the
/// command then fails with a code of its own rather than the signal. A failure of the code that met an
/// unrelated kill only runs once more, and fails again without one. A count that cannot be read takes
/// nothing for a kill, and neither does a run ended as hung, which would hang again.
pub fn killed_for_memory(run: RunEnd, before: Option<u64>, after: Option<u64>) -> bool {
  !run.passed()
    && run.code != Some(i32::from(HUNG_EXIT_CODE))
    && matches!((before, after), (Some(before), Some(after)) if after > before)
}

fn gigabytes(bytes: u64) -> String {
  format!("{:.1} GB", bytes as f64 / f64::from(1u32 << 30))
}

// What a line says, opened by the mark.
fn say(hands: &mut dyn HeavyHands, line: &str) {
  hands.say(&format!("{} {line}", LANES.memory.mark));
}

// After a kill: a settling pause, then a wait until enough memory is available, however long that takes, said
// now and again while it goes on.
fn wait_for_memory(hands: &mut dyn HeavyHands, limits: &MemoryLimits) {
  let said = &LANES.memory;
  let what = hands.what();

  hands.sleep(limits.settle);

  let started = hands.now();
  let mut next_notice = started + limits.notice.as_secs_f64();

  loop {
    let Some(available) = hands.available_bytes() else {
      say(hands, &fill_marks(&said.unread, &[("what", &what)]));

      return;
    };

    if available >= limits.needed_bytes {
      say(
        hands,
        &fill_marks(&said.freed, &[("what", &what), ("available", &gigabytes(available))]),
      );

      return;
    }

    if hands.now() >= next_notice {
      let minutes = ((hands.now() - started) / 60.0).round().to_string();

      say(
        hands,
        &fill_marks(
          &said.short,
          &[("what", &what), ("available", &gigabytes(available)), ("minutes", &minutes)],
        ),
      );
      next_notice += limits.notice.as_secs_f64();
    }

    hands.sleep(limits.poll);
  }
}

/// The command, run in its turn and run again after each out-of-memory kill until the runs are spent. Answers
/// the exit code the caller ends on, `on_pass` run inside the turn once a run has passed; a turn not had runs
/// nothing and answers the testbed's code for it.
pub fn run_in_turn(hands: &mut dyn HeavyHands, limits: &MemoryLimits, on_pass: &mut dyn FnMut()) -> u8 {
  let said = &LANES.memory;
  let what = hands.what();
  let turn = hands.take_turn();

  match &turn {
    TestbedTurn::Queued { .. } => {
      say(hands, &fill_marks(&said.not_run, &[("what", &what), ("turn", &turn.words())]));

      return NOT_RUN_EXIT_CODE;
    }
    TestbedTurn::Own { waited_seconds, .. } if waited_seconds.round() > 0.0 => {
      let seconds = waited_seconds.round().to_string();

      say(hands, &fill_marks(&said.took, &[("what", &what), ("seconds", &seconds)]));
    }
    _ => {}
  }

  let started = hands.now();
  let code = runs(hands, &turn, limits, on_pass);
  let ran = hands.now() - started;
  let waited = turn.waited_seconds();

  say(
    hands,
    &fill_marks(&said.times, &[("what", &what), ("times", &times_words(waited, ran))]),
  );
  hands.record(waited, ran);
  drop(turn);

  code
}

// The runs inside a turn had: the first, and one more after each kill for want of memory.
fn runs(
  hands: &mut dyn HeavyHands,
  turn: &TestbedTurn,
  limits: &MemoryLimits,
  on_pass: &mut dyn FnMut(),
) -> u8 {
  let said = &LANES.memory;
  let what = hands.what();
  let runs = limits.runs.to_string();

  for run in 1.. {
    let before = hands.oom_kills();
    let fences_before = hands.fences();
    let done = hands.run(turn);

    if done.passed() {
      on_pass();

      return 0;
    }

    if !killed_for_memory(done, before, hands.oom_kills()) {
      return done.exit_code();
    }

    let fences_after = hands.fences();

    if let Some(fence) = fence_reached(&fences_before, &fences_after) {
      let line = fill_marks(
        &said.fenced,
        &[
          ("what", &what),
          ("end", &done.words()),
          ("ceiling", &gigabytes(fence.ceiling_bytes)),
          ("fence", &fence.cgroup),
        ],
      );

      say(hands, &line);

      return done.exit_code();
    }

    if run >= limits.runs {
      say(hands, &fill_marks(&said.spent, &[("what", &what), ("runs", &runs)]));

      return 1;
    }

    let line = fill_marks(
      &said.again,
      &[("what", &what), ("end", &done.words()), ("run", &run.to_string()), ("runs", &runs)],
    );

    say(hands, &line);
    wait_for_memory(hands, limits);
  }

  unreachable!("the runs end inside the loop")
}

/// The machine's own hands for a command of `lane`: its turn taken with the waiting said on standard error
/// under the mark, the machine's memory and fences read, and each run started by `start`, handed the turn it
/// runs under, whose variable a command started under it carries and whose share of the width it runs on.
pub struct MachineHands<'a> {
  pub what: String,
  pub lane: Lane,
  pub checkout: String,
  pub env: Env,
  pub start: &'a mut dyn FnMut(&TestbedTurn) -> RunEnd,
  started: Instant,
}

impl<'a> MachineHands<'a> {
  pub fn new(
    what: String,
    lane: Lane,
    checkout: String,
    env: Env,
    start: &'a mut dyn FnMut(&TestbedTurn) -> RunEnd,
  ) -> Self {
    Self {
      what,
      lane,
      checkout,
      env,
      start,
      started: Instant::now(),
    }
  }
}

impl HeavyHands for MachineHands<'_> {
  fn what(&self) -> String {
    self.what.clone()
  }

  fn take_turn(&mut self) -> TestbedTurn {
    let say = |line: &str| eprintln!("{} {line}", LANES.memory.mark);

    take_turn(&TurnAsk {
      command: self.what.clone(),
      lane: self.lane,
      say: &say,
      path: None,
      env: self.env.clone(),
      checkout: self.checkout.clone(),
      poll: None,
      notice_seconds: None,
    })
  }

  fn run(&mut self, turn: &TestbedTurn) -> RunEnd {
    (self.start)(turn)
  }

  fn oom_kills(&self) -> Option<u64> {
    machine_oom_kills()
  }

  fn available_bytes(&self) -> Option<u64> {
    machine_available_bytes()
  }

  fn fences(&self) -> Vec<MemoryFence> {
    machine_fences()
  }

  fn sleep(&mut self, how_long: Duration) {
    std::thread::sleep(how_long);
  }

  fn now(&self) -> f64 {
    self.started.elapsed().as_secs_f64()
  }

  fn say(&mut self, line: &str) {
    eprintln!("{line}");
  }

  fn record(&mut self, waited_seconds: f64, ran_seconds: f64) {
    record_turn_times(&self.env, &self.what, self.lane, waited_seconds, ran_seconds);
  }
}
