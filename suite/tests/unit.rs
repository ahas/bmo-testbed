//! One unit in a process of its own (docs/suite.md, 「How long a unit and a test may run」): what a unit left
//! holding its output is found, and nothing the run started itself is taken for it; a unit that falls silent is
//! ended, and so is one whose test runs past the per-test limit.

use std::io::{BufRead, BufReader};
use std::os::fd::AsRawFd;
use std::process::Command;
use std::time::{Duration, Instant};

use bmo_testbed_core::languages::Cargo;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::silence::{HUNG_EXIT_CODE, Silence};
use bmo_testbed_suite::unit::{PerTest, UnitCommand, holders_of, run_unit};

// "a process the run started itself is never taken for such a holder": of a process this one started and
// one a unit left behind, both holding the pipe, only the one left behind is named.
#[test]
fn names_what_a_unit_left_holding_its_output_and_never_a_process_the_run_started() {
  let (reader, writer) = std::io::pipe().expect("a pipe opens");
  let pipe =
    std::fs::read_link(format!("/proc/self/fd/{}", reader.as_raw_fd())).expect("the pipe is named");
  let mut own = Command::new("sleep")
    .arg("30")
    .stdout(writer.try_clone().expect("the pipe's end is shared"))
    .spawn()
    .expect("a process of the run's own starts");
  let mut unit = Command::new("sh")
    .args(["-c", "sleep 30 & echo $!"])
    .stdout(writer)
    .spawn()
    .expect("a unit starts");
  let mut said = String::new();

  BufReader::new(&reader).read_line(&mut said).expect("the unit names what it left");
  let _ = unit.wait();
  let left: i32 = said.trim().parse().expect("a process id");
  let holders = holders_of(&pipe);

  // SAFETY: a signal to the process this test's own unit left behind.
  unsafe { libc::kill(left, libc::SIGKILL) };
  let _ = own.kill();
  let _ = own.wait();

  assert_eq!(holders, [left]);
}

// A unit of a shell line, its silence a fraction of a second, so a test of it ends in seconds.
fn short_unit(line: &str) -> UnitCommand {
  let mut unit = UnitCommand::of(
    vec!["sh".to_owned(), "-c".to_owned(), line.to_owned()],
    std::env::temp_dir(),
    std::env::vars().collect(),
    None,
  );

  unit.silence = Silence {
    limit: Duration::from_millis(400),
    grace: Duration::from_millis(300),
    look: Duration::from_millis(50),
  };
  unit
}

// "a unit that writes nothing for the silence limit is ended as hung": it fails with the hung code, the line
// naming it in its output after what it said.
#[test]
fn a_unit_that_writes_nothing_for_the_limit_is_ended_as_hung() {
  let began = Instant::now();
  let ran = run_unit(&short_unit("echo once; sleep 30"));
  let hung = fill_marks(
    &LANES.silence.hung,
    &[
      ("command", "sh -c echo once; sleep 30"),
      ("output", &LANES.silence.piped),
      ("seconds", "0.4"),
      ("standing", &fill_marks(&LANES.silence.standing, &[("processes", "sleep 30")])),
    ],
  );

  assert!(began.elapsed() < Duration::from_secs(10), "the hung unit was not ended");
  assert_eq!((ran.code, ran.signal), (i32::from(HUNG_EXIT_CODE), None));
  assert_eq!(ran.output, format!("once\n{hung}\n"));
}

// "a unit is never cut for its length as a whole": a unit that keeps writing past the limit runs to its end.
#[test]
fn a_unit_that_keeps_writing_past_the_limit_runs_to_its_end() {
  let ran = run_unit(&short_unit("for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo $i >&2; sleep 0.1; done"));

  assert_eq!(ran.code, 0, "{}", ran.output);
  assert!(ran.seconds > 0.4);
}

// A unit of a shell line read as a crate's tests are, its per-test limit a minute, which the harness's own line
// for a test running over a minute meets at once.
fn crate_unit(line: &str) -> UnitCommand {
  let mut unit = short_unit(line);

  unit.silence.limit = Duration::from_secs(30);
  unit.per_test = Some(PerTest {
    language: &Cargo,
    limit: Duration::from_secs(60),
  });
  unit
}

// "once a test so named has run for the limit without its result, it ends the unit's process group": the unit
// fails with the hung code, its output closing on the line naming the test and the limit, long before its
// silence limit.
#[test]
fn a_crate_s_test_named_as_running_past_the_limit_ends_its_unit() {
  let began = Instant::now();
  let ran = run_unit(&crate_unit("echo 'test a::waits has been running for over 60 seconds'; sleep 30"));
  let overran = fill_marks(&SUITE.file.test_overran, &[("test", "a::waits"), ("seconds", "60")]);

  assert!(
    began.elapsed() < Duration::from_secs(10),
    "the unit was not ended at the per-test limit"
  );
  assert_eq!((ran.code, ran.signal), (i32::from(HUNG_EXIT_CODE), None));
  assert_eq!(ran.output, format!("test a::waits has been running for over 60 seconds\n{overran}\n"));
}

// "without its result": a test named as running long whose result follows ends nothing, and the unit runs on.
// Both lines come in one write and the limit stands a little past the minute the first names, so the result is
// heard before the limit is met however the machine is loaded; the pause after it outlasts that margin, so a
// result left unheard would still end the unit.
#[test]
fn a_test_named_as_running_long_whose_result_follows_ends_nothing() {
  let mut unit = crate_unit(
    "printf 'test a::slow has been running for over 60 seconds\\ntest a::slow ... ok\\n'; sleep 0.4; echo done",
  );

  if let Some(per_test) = unit.per_test.as_mut() {
    per_test.limit = Duration::from_millis(60_200);
  }

  let ran = run_unit(&unit);

  assert_eq!(ran.code, 0, "{}", ran.output);
  assert!(ran.output.ends_with("done\n"), "{}", ran.output);
}
