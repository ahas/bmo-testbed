//! The fence a check step runs in (docs/checks.md, 「The static check」): a scope the manager starts around a
//! keeper in the checks' slice, with the policy's ceilings, the keeper ended with the fence, and a command
//! that cannot move into it still run, saying why. A stand-in manager answers each test, and one test asks
//! the machine's own manager where it can be reached.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::policy::static_check::STATIC_CHECK;
use bmo_testbed_suite::fence::{FenceSeams, LiveFence, user_bus, user_bus_refusal};
use bmo_testbed_suite::step_fence::{CHECK_SLICE, StepFence, step_limits, step_unit};

// A manager that answers as told, which moves the keeper into a scope that stands nowhere on the machine.
struct Manager {
  runtime: tempfile::TempDir,
  takes: bool,
  calls: Vec<Vec<String>>,
}

fn manager(takes: bool) -> Manager {
  let runtime = tempfile::tempdir().unwrap();

  std::fs::create_dir_all(runtime.path().join("systemd")).unwrap();
  std::fs::write(runtime.path().join("systemd").join("private"), "").unwrap();

  Manager {
    runtime,
    takes,
    calls: Vec::new(),
  }
}

impl Manager {
  // The keeper the last call asked a scope around.
  fn keeper(&self) -> PathBuf {
    let call = self.calls.last().unwrap();
    let at = call.iter().position(|word| word == "PIDs").unwrap();

    Path::new("/proc").join(&call[at + 3])
  }
}

impl FenceSeams for Manager {
  fn pid(&self) -> u32 {
    4242
  }

  fn env(&self) -> Vec<(String, String)> {
    vec![("XDG_RUNTIME_DIR".to_owned(), self.runtime.path().to_string_lossy().into_owned())]
  }

  fn call(&mut self, command: &[String], _: &[(String, String)]) -> (bool, String) {
    self.calls.push(command.to_vec());
    (self.takes, "Unit already exists".to_owned())
  }

  fn cgroup(&mut self, _: u32) -> String {
    format!("0::/bmo-testbed-test-nowhere.slice/{}\n", self.calls.last().unwrap()[8])
  }

  fn pause(&mut self) {}

  fn offer_first(&mut self) {}
}

// "a scope of its own in the checks' slice, with the policy's ceilings, around a keeper ended with it"
#[test]
fn a_step_s_fence_is_a_scope_in_the_checks_slice_around_a_keeper_ended_with_it() {
  let mut asked = manager(true);
  let fence = StepFence::open(step_limits(), &mut asked).unwrap();
  let call = asked.calls[0].join(" ");
  let ceilings = &STATIC_CHECK.fence;

  assert!(
    fence.unit.starts_with("bmo-check-4242-") && fence.unit.ends_with(".scope"),
    "{}",
    fence.unit
  );
  assert_eq!(step_unit(4242, 7), "bmo-check-4242-7.scope");
  assert_eq!(fence.group, format!("/bmo-testbed-test-nowhere.slice/{}", fence.unit));
  assert!(
    call.contains(&format!(
      "MemoryMax t {} MemorySwapMax t {} TasksMax t {} OOMPolicy s continue Slice s {CHECK_SLICE}",
      ceilings.memory_gb << 30,
      ceilings.swap_gb << 30,
      ceilings.tasks
    )),
    "{call}"
  );
  assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join("../units").join(CHECK_SLICE).is_file());

  let keeper = asked.keeper();

  assert!(keeper.exists());
  drop(fence);
  assert!(!keeper.exists());
}

// "the manager refusing it"
#[test]
fn a_fence_the_manager_refuses_says_why_and_leaves_no_keeper() {
  let mut asked = manager(false);
  let Err(why) = StepFence::open(step_limits(), &mut asked) else {
    panic!("the manager refused the scope")
  };

  assert!(why.contains("Unit already exists"), "{why}");
  assert!(!asked.keeper().exists());
}

// "the command unable to move into it ... runs where its caller stands"
#[test]
fn a_command_that_cannot_move_into_its_fence_still_runs_and_says_why() {
  let fence = StepFence::open(step_limits(), &mut manager(true)).unwrap();
  let mut command = Command::new("sh");

  command.args(["-c", "exit 3"]).stdin(Stdio::null());

  let (child, missed) = fence.spawn(&mut command);
  let status = child.unwrap().wait().unwrap();
  let why = missed.unwrap();

  assert_eq!(status.code(), Some(3));
  assert!(why.contains(&fence.unit) && why.contains("No such file or directory"), "{why}");
  assert!(why.contains("the service manager did not move it"), "{why}");
}

// "The command moves itself into the scope before it executes, so it ... is born inside"
#[test]
fn a_command_started_into_a_fence_of_the_machine_s_manager_is_born_inside_it() {
  if let Some(why) = user_bus_refusal(&user_bus()) {
    // A machine with no user service manager has no scope to give; the refusal is the answer there.
    assert!(StepFence::open(step_limits(), &mut LiveFence).is_err(), "{why}");
    return;
  }

  let fence = StepFence::open(step_limits(), &mut LiveFence).unwrap();
  let mut command = Command::new("cat");

  command.arg("/proc/self/cgroup").stdin(Stdio::null()).stdout(Stdio::piped());

  let (child, missed) = fence.spawn(&mut command);
  let output = child.unwrap().wait_with_output().unwrap();

  assert_eq!(missed, None);
  assert!(String::from_utf8_lossy(&output.stdout).trim().ends_with(&format!("/{}", fence.unit)));
  assert!(fence.group.ends_with(&format!("/{CHECK_SLICE}/{}", fence.unit)), "{}", fence.group);
}

// The machine's own manager, standing for a caller in a login session's scope, which may not move itself.
struct FromSession;

impl FenceSeams for FromSession {
  fn pid(&self) -> u32 {
    LiveFence.pid()
  }

  fn env(&self) -> Vec<(String, String)> {
    LiveFence.env()
  }

  fn call(&mut self, command: &[String], env: &[(String, String)]) -> (bool, String) {
    LiveFence.call(command, env)
  }

  fn cgroup(&mut self, pid: u32) -> String {
    LiveFence.cgroup(pid)
  }

  fn pause(&mut self) {
    LiveFence.pause();
  }

  fn offer_first(&mut self) {}

  fn moves_itself(&self) -> bool {
    false
  }
}

// "where it may not, as from a login session's scope, the manager moves it there while it waits to execute, so
// it is born inside all the same"
#[test]
fn a_command_that_may_not_move_itself_is_moved_by_the_manager_before_it_executes() {
  if user_bus_refusal(&user_bus()).is_some() {
    return;
  }

  let fence = StepFence::open(step_limits(), &mut FromSession).unwrap();
  let mut command = Command::new("cat");

  command.arg("/proc/self/cgroup").stdin(Stdio::null()).stdout(Stdio::piped());

  let (child, missed) = fence.spawn(&mut command);
  let output = child.unwrap().wait_with_output().unwrap();

  assert_eq!(missed, None);
  assert!(String::from_utf8_lossy(&output.stdout).trim().ends_with(&format!("/{}", fence.unit)));
}
