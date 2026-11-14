//! One unit in a process of its own (docs/suite.md, 「Every unit runs to its end」): what a unit left holding
//! its output is found, and nothing the run started itself is taken for it; a unit that falls silent is ended.

use std::io::{BufRead, BufReader};
use std::os::fd::AsRawFd;
use std::process::Command;
use std::time::{Duration, Instant};

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::silence::{HUNG_EXIT_CODE, Silence};
use bmo_testbed_suite::unit::{UnitCommand, holders_of, run_unit};

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

// "nothing of the suite's cuts a unit or a test for its length": a unit that keeps writing past the limit runs
// to its end.
#[test]
fn a_unit_that_keeps_writing_past_the_limit_runs_to_its_end() {
  let ran = run_unit(&short_unit("for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo $i >&2; sleep 0.1; done"));

  assert_eq!(ran.code, 0, "{}", ran.output);
  assert!(ran.seconds > 0.4);
}
