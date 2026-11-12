//! One unit in a process of its own (docs/suite.md, 「Every unit runs to its end」): what a unit left holding
//! its output is found, and nothing the run started itself is taken for it.

use std::io::{BufRead, BufReader};
use std::os::fd::AsRawFd;
use std::process::Command;

use bmo_testbed_suite::unit::holders_of;

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
