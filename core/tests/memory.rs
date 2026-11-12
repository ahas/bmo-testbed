//! A heavy command run again where the kernel killed it for want of memory (docs/testbed.md, 「Memory」):
//! each test runs the command on a machine of its own, the runs it ends with, the kill counts and the
//! memory each reading gives handed in, and its clock moved only by the sleeps asked of it.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::time::Duration;

use bmo_testbed_core::memory::{
  HeavyHands, MemoryFence, MemoryLimits, RunEnd, killed_for_memory, proc_number, run_in_turn,
};
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::silence::HUNG_EXIT_CODE;
use bmo_testbed_core::turn::{NOT_RUN_EXIT_CODE, Seats, TestbedTurn};

// A machine in a test: the runs it ends with, the kill count each reading gives, and what it said.
struct Bench {
  turn: Option<TestbedTurn>,
  ends: VecDeque<RunEnd>,
  kills: RefCell<VecDeque<Option<u64>>>,
  available: RefCell<VecDeque<Option<u64>>>,
  fences: RefCell<VecDeque<Vec<MemoryFence>>>,
  clock: Cell<f64>,
  ran: usize,
  said: Vec<String>,
  recorded: Option<(f64, f64)>,
}

impl Bench {
  fn new(ends: &[RunEnd], kills: &[Option<u64>]) -> Self {
    Self {
      turn: Some(TestbedTurn::Inside {
        seats: Seats { held: 1, of: 1 },
      }),
      ends: ends.iter().copied().collect(),
      kills: RefCell::new(kills.iter().copied().collect()),
      available: RefCell::new(VecDeque::new()),
      fences: RefCell::new(VecDeque::new()),
      clock: Cell::new(0.0),
      ran: 0,
      said: Vec::new(),
      recorded: None,
    }
  }
}

impl HeavyHands for Bench {
  fn what(&self) -> String {
    "cargo test".to_owned()
  }

  fn take_turn(&mut self) -> TestbedTurn {
    self.turn.take().expect("one turn")
  }

  fn run(&mut self, _: &TestbedTurn) -> RunEnd {
    self.ran += 1;
    self.clock.set(self.clock.get() + 2.0);
    self.ends.pop_front().expect("a run's end")
  }

  fn oom_kills(&self) -> Option<u64> {
    self.kills.borrow_mut().pop_front().flatten()
  }

  fn available_bytes(&self) -> Option<u64> {
    self.available.borrow_mut().pop_front().unwrap_or(Some(8 << 30))
  }

  fn fences(&self) -> Vec<MemoryFence> {
    self.fences.borrow_mut().pop_front().unwrap_or_default()
  }

  fn sleep(&mut self, how_long: Duration) {
    self.clock.set(self.clock.get() + how_long.as_secs_f64());
  }

  fn now(&self) -> f64 {
    self.clock.get()
  }

  fn say(&mut self, line: &str) {
    self.said.push(line.to_owned());
  }

  fn record(&mut self, waited: f64, ran: f64) {
    self.recorded = Some((waited, ran));
  }
}

const PASSED: RunEnd = RunEnd {
  code: Some(0),
  signal: None,
};
const FAILED: RunEnd = RunEnd {
  code: Some(101),
  signal: None,
};
const KILLED: RunEnd = RunEnd {
  code: None,
  signal: Some(9),
};

fn fence(reached: u64) -> MemoryFence {
  MemoryFence {
    cgroup: "/user.slice/job".to_owned(),
    ceiling_bytes: 16 << 30,
    ceiling_reached: reached,
  }
}

// A line as the memory turn says it, its mark ahead of it.
fn marked(line: &str) -> String {
  format!("{} {line}", LANES.memory.mark)
}

#[test]
fn a_passed_run_runs_once_and_ends_on_its_times() {
  let mut bench = Bench::new(&[PASSED], &[Some(3), Some(3)]);
  let mut passed = 0;
  let code = run_in_turn(&mut bench, &MemoryLimits::default(), &mut || passed += 1);

  assert_eq!((code, bench.ran, passed), (0, 1, 1));
  assert_eq!(bench.recorded, Some((0.0, 2.0)));
  assert!(bench.said.last().unwrap().starts_with(&marked("cargo test waited 0s")));
}

#[test]
fn a_failure_with_no_kill_is_the_codes_and_runs_nothing_after_it() {
  let mut bench = Bench::new(&[FAILED], &[Some(3), Some(3)]);
  let mut passed = 0;

  assert_eq!(run_in_turn(&mut bench, &MemoryLimits::default(), &mut || passed += 1), 101);
  assert_eq!((bench.ran, passed), (1, 0));
}

#[test]
fn a_kill_for_want_of_memory_runs_again_once_memory_frees() {
  let mut bench = Bench::new(&[KILLED, PASSED], &[Some(3), Some(4), Some(4), Some(4)]);

  bench.available = RefCell::new([Some(1 << 30), Some(7 << 30)].into());

  assert_eq!(run_in_turn(&mut bench, &MemoryLimits::default(), &mut || ()), 0);
  assert_eq!(bench.ran, 2);
  assert!(
    bench
      .said
      .iter()
      .any(|line| line.contains("failed (signal 9)") && line.contains("run 1 of 4"))
  );
  assert!(
    bench
      .said
      .iter()
      .any(|line| line.contains("7.0 GB available; running cargo test again"))
  );
}

#[test]
fn a_kill_at_a_fences_ceiling_is_not_run_again() {
  let mut bench = Bench::new(&[KILLED], &[Some(3), Some(4)]);

  bench.fences = RefCell::new([vec![fence(1)], vec![fence(2)]].into());

  assert_eq!(run_in_turn(&mut bench, &MemoryLimits::default(), &mut || ()), 1);
  assert_eq!(bench.ran, 1);
  assert!(
    bench
      .said
      .iter()
      .any(|line| line.contains("16.0 GB memory ceiling") && line.contains("/user.slice/job"))
  );
}

#[test]
fn the_runs_spent_on_kills_fail_saying_the_code_was_never_judged() {
  let limits = MemoryLimits {
    runs: 2,
    ..MemoryLimits::default()
  };
  let mut bench = Bench::new(&[KILLED, KILLED], &[Some(1), Some(2), Some(2), Some(3)]);

  assert_eq!(run_in_turn(&mut bench, &limits, &mut || ()), 1);
  assert_eq!(bench.ran, 2);
  assert!(bench.said.iter().any(|line| line.contains("on all 2 runs")));
}

#[test]
fn a_killed_run_waits_for_memory_however_long_it_takes_saying_so_now_and_again() {
  let limits = MemoryLimits {
    notice: Duration::from_secs(10),
    ..MemoryLimits::default()
  };
  let mut bench = Bench::new(&[KILLED, PASSED], &[Some(1), Some(2), Some(2), Some(2)]);

  bench.available = RefCell::new(std::iter::repeat_n(Some(1 << 30), 40).collect());

  assert_eq!(run_in_turn(&mut bench, &limits, &mut || ()), 0);
  assert_eq!(bench.ran, 2);
  assert!(
    bench
      .said
      .iter()
      .filter(|line| line.contains("waits for more before it runs again"))
      .count()
      > 1
  );
}

#[test]
fn a_turn_not_had_runs_nothing_and_ends_on_the_testbeds_code() {
  let mut bench = Bench::new(&[], &[]);

  bench.turn = Some(TestbedTurn::Queued {
    waited_seconds: 180.0,
    ahead: 2,
    holders: Vec::new(),
  });

  assert_eq!(run_in_turn(&mut bench, &MemoryLimits::default(), &mut || ()), NOT_RUN_EXIT_CODE);
  assert_eq!(bench.ran, 0);
  assert!(bench.said[0].starts_with(&marked("cargo test not run: the machine's turn was not had")));
}

#[test]
fn an_unread_kill_count_takes_nothing_for_a_kill() {
  assert!(!killed_for_memory(KILLED, None, Some(4)));
  assert!(!killed_for_memory(PASSED, Some(3), Some(4)));
  assert!(killed_for_memory(FAILED, Some(3), Some(4)));
}

// "a command ended as hung is not run again, since it would hang again"
#[test]
fn a_run_ended_as_hung_is_never_taken_for_a_kill() {
  let hung = RunEnd {
    code: Some(i32::from(HUNG_EXIT_CODE)),
    signal: None,
  };

  assert!(!killed_for_memory(hung, Some(3), Some(4)));
}

#[test]
fn reads_a_kernel_listing_by_its_name() {
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("meminfo");

  std::fs::write(&file, "MemTotal:       32000 kB\nMemAvailable:    2048 kB\n").unwrap();

  assert_eq!(proc_number(&file, "MemAvailable"), Some(2048));
  assert_eq!(proc_number(&file, "SwapTotal"), None);
}
