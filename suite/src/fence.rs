//! The fence a run of the suite stands in (docs/suite.md, 「A fence of its own」): once its turn has
//! come the run asks the user's service manager to move it into a transient scope of its own with a memory
//! ceiling, so a runaway test is killed there rather than filling the machine; and every process of the run
//! offers itself first to the kernel's OOM killer.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::turn::leading_number;

const GIB: u64 = 1024 * 1024 * 1024;

/// What the scope holds the run to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FenceLimits {
  pub memory_gb: u64,
  pub swap_gb: u64,
  pub tasks: u64,
}

/// The limits: the gigabytes the declaration names unless `BMO_SUITE_MEMORY_GB` names another positive
/// number.
pub fn fence_limits(declared_gb: u64, memory: Option<&str>) -> FenceLimits {
  let named = memory.and_then(leading_number).filter(|given| *given > 0);

  FenceLimits {
    memory_gb: named.map_or(declared_gb, |given| given as u64),
    swap_gb: SUITE.numbers.fence_swap_gb,
    tasks: SUITE.numbers.fence_tasks,
  }
}

/// Cargo's jobs inside the fence: as many as its memory holds at a build job's peak, never more than the
/// run's width nor fewer than one. A caller's own smaller number stands.
pub fn fenced_build_jobs(limits: FenceLimits, width: usize, given: Option<&str>) -> usize {
  let held =
    (limits.memory_gb * 10 / SUITE.numbers.build_job_tenths_gb).clamp(1, width.max(1) as u64) as usize;

  match given.and_then(leading_number).filter(|named| *named > 0) {
    Some(named) => (named as usize).min(held),
    None => held,
  }
}

/// The slice the suite's units run in: a run's fence, the daily whole run and the test server, one of this
/// repository's `units/` (docs/deploy.md, 「The slices」).
pub const SUITE_SLICE: &str = "bmo-suite.slice";

const FENCE_MARK: &str = "bmo-suite-";

/// The scope a run with this process id stands in.
pub fn fence_unit(pid: u32) -> String {
  format!("{FENCE_MARK}{pid}.scope")
}

/// The pattern the user's service manager matches every run's scope by.
pub fn fence_units() -> String {
  format!("{FENCE_MARK}*.scope")
}

/// Whether the manager's listing of the scopes `fence_units` matches names one other than the scope of the
/// run with this process id.
pub fn other_fence_listed(listed: &str, pid: u32) -> bool {
  let own = fence_unit(pid);

  listed
    .lines()
    .filter_map(|line| line.split_whitespace().find(|word| word.starts_with(FENCE_MARK)))
    .any(|unit| unit != own)
}

/// The manager's own call that starts a scope around a process already running, in `slice`.
pub fn fence_call(unit: &str, slice: &str, pid: u32, limits: FenceLimits) -> Vec<String> {
  let properties = [
    ("MemoryMax", "t", (limits.memory_gb * GIB).to_string()),
    ("MemorySwapMax", "t", (limits.swap_gb * GIB).to_string()),
    ("TasksMax", "t", limits.tasks.to_string()),
    ("OOMPolicy", "s", "continue".to_owned()),
    ("Slice", "s", slice.to_owned()),
    ("CollectMode", "s", "inactive-or-failed".to_owned()),
  ];
  let mut call: Vec<String> = [
    "busctl",
    "--user",
    "call",
    "org.freedesktop.systemd1",
    "/org/freedesktop/systemd1",
    "org.freedesktop.systemd1.Manager",
    "StartTransientUnit",
    "ssa(sv)a(sa(sv))",
  ]
  .map(str::to_owned)
  .to_vec();

  call.extend([unit.to_owned(), "fail".to_owned(), (properties.len() + 1).to_string()]);
  call.extend(["PIDs", "au", "1"].map(str::to_owned));
  call.push(pid.to_string());

  for (name, kind, value) in properties {
    call.extend([name.to_owned(), kind.to_owned(), value]);
  }

  call.push("0".to_owned());
  call
}

/// The environment a call to the user's service manager is handed: the runtime directory named where it is
/// missing, and the disabled bus address taken off.
pub fn user_bus() -> Vec<(String, String)> {
  let mut env: Vec<(String, String)> = std::env::vars()
    .filter(|(name, value)| !(name == "DBUS_SESSION_BUS_ADDRESS" && value == "disabled:"))
    .collect();

  if !env.iter().any(|(name, value)| name == "XDG_RUNTIME_DIR" && !value.is_empty()) {
    // SAFETY: getuid reads the process's own user and cannot fail.
    env.push(("XDG_RUNTIME_DIR".to_owned(), format!("/run/user/{}", unsafe { libc::getuid() })));
  }

  env
}

/// Why the manager cannot be reached from such an environment, or none where it can.
pub fn user_bus_refusal(env: &[(String, String)]) -> Option<String> {
  let runtime = env
    .iter()
    .find(|(name, _)| name == "XDG_RUNTIME_DIR")
    .map_or("", |(_, value)| value.as_str());

  if runtime.is_empty() {
    return Some(SUITE.lines.no_runtime.clone());
  }

  let at = Path::new(runtime);

  if at.join("systemd").join("private").exists() || at.join("bus").exists() {
    return None;
  }

  Some(fill_marks(&SUITE.lines.no_socket, &[("runtime", runtime)]))
}

fn cgroup_of(pid: &str) -> String {
  std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default()
}

fn none(why: &str) -> String {
  fill_marks(&SUITE.lines.fence_none, &[("mark", &SUITE.lines.mark), ("why", why)])
}

/// What the fence is built on, handed in so a test can stand in for the manager.
pub trait FenceSeams {
  fn pid(&self) -> u32;
  /// The environment a call to the manager is handed.
  fn env(&self) -> Vec<(String, String)>;
  /// The manager asked: whether it took the call, and what it said.
  fn call(&mut self, command: &[String], env: &[(String, String)]) -> (bool, String);
  /// A process's control group, as `/proc/<pid>/cgroup` names it.
  fn cgroup(&mut self, pid: u32) -> String;
  fn pause(&mut self);
  /// Raises the process's own OOM score to the most; its children inherit it.
  fn offer_first(&mut self);
}

/// The fence as this process stands.
pub struct LiveFence;

impl FenceSeams for LiveFence {
  fn pid(&self) -> u32 {
    std::process::id()
  }

  fn env(&self) -> Vec<(String, String)> {
    user_bus()
  }

  fn call(&mut self, command: &[String], env: &[(String, String)]) -> (bool, String) {
    let asked = Command::new(&command[0])
      .args(&command[1..])
      .env_clear()
      .envs(env.iter().map(|(name, value)| (name, value)))
      .stdin(Stdio::null())
      .output();

    match asked {
      Ok(output) => (
        output.status.success(),
        format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
          .trim()
          .to_owned(),
      ),
      Err(why) => (false, why.to_string()),
    }
  }

  fn cgroup(&mut self, pid: u32) -> String {
    cgroup_of(&pid.to_string())
  }

  fn pause(&mut self) {
    std::thread::sleep(Duration::from_millis(SUITE.numbers.fence_move_poll_ms));
  }

  fn offer_first(&mut self) {
    // A score that cannot be raised leaves the run as the kernel would have judged it anyway.
    let _ = std::fs::write("/proc/self/oom_score_adj", "1000");
  }
}

/// Moves the run into its scope and says, on one line under the mark, where it now stands. The manager
/// answers before the move is carried out, so the run waits, briefly and bounded, until its own control
/// group names the scope; a run that could not be moved says why and runs where it is.
pub fn fence_suite(limits: FenceLimits, seams: &mut dyn FenceSeams) -> String {
  seams.offer_first();

  let env = seams.env();

  if let Some(refused) = user_bus_refusal(&env) {
    return none(&refused);
  }

  let pid = seams.pid();
  let unit = fence_unit(pid);

  if let Err(why) = start_scope(&unit, SUITE_SLICE, pid, limits, &env, seams) {
    return none(&why);
  }

  fill_marks(
    &SUITE.lines.fence_held,
    &[
      ("mark", &SUITE.lines.mark),
      ("unit", &unit),
      ("memory", &limits.memory_gb.to_string()),
      ("swap", &limits.swap_gb.to_string()),
      ("tasks", &limits.tasks.to_string()),
    ],
  )
}

/// Asks the manager for the scope `unit` in `slice` around the process `pid`, and waits, briefly and bounded,
/// until that process's control group names it, since the manager answers before the move is carried out.
/// Answers the scope's group, its path under the cgroup root, or why the process does not stand in it.
pub fn start_scope(
  unit: &str,
  slice: &str,
  pid: u32,
  limits: FenceLimits,
  env: &[(String, String)],
  seams: &mut dyn FenceSeams,
) -> Result<String, String> {
  let (taken, said) = seams.call(&fence_call(unit, slice, pid, limits), env);

  if !taken {
    return Err(fill_marks(&SUITE.lines.fence_refused, &[("said", &said)]));
  }

  let scope = format!("/{unit}");
  let group = |seams: &mut dyn FenceSeams| {
    seams
      .cgroup(pid)
      .lines()
      .map(|line| line.trim_start_matches("0::").to_owned())
      .find(|path| path.ends_with(&scope))
  };

  for _ in 0..SUITE.numbers.fence_move_tries {
    if let Some(found) = group(seams) {
      return Ok(found);
    }

    seams.pause();
  }

  group(seams).ok_or_else(|| fill_marks(&SUITE.lines.fence_not_moved, &[("unit", unit)]))
}

/// What emptying the fence reads and kills, handed in so a test can stand in for the kernel.
pub trait ClearSeams {
  fn pid(&self) -> u32;
  /// The process's own control group, as `/proc/self/cgroup` names it.
  fn cgroup(&mut self) -> String;
  /// The processes in a control group, by its path under the cgroup root.
  fn members(&mut self, group: &str) -> Vec<u32>;
  /// A process's command line, empty once it has gone.
  fn command(&mut self, pid: u32) -> String;
  fn kill(&mut self, pid: u32);
}

/// The scope as the kernel holds it.
pub struct LiveClear;

impl ClearSeams for LiveClear {
  fn pid(&self) -> u32 {
    std::process::id()
  }

  fn cgroup(&mut self) -> String {
    cgroup_of("self")
  }

  fn members(&mut self, group: &str) -> Vec<u32> {
    std::fs::read_to_string(
      Path::new("/sys/fs/cgroup").join(group.trim_start_matches('/')).join("cgroup.procs"),
    )
    .unwrap_or_default()
    .lines()
    .filter_map(|line| line.trim().parse().ok())
    .collect()
  }

  fn command(&mut self, pid: u32) -> String {
    std::fs::read(format!("/proc/{pid}/cmdline"))
      .map(|bytes| {
        bytes
          .split(|byte| *byte == 0)
          .filter(|part| !part.is_empty())
          .map(String::from_utf8_lossy)
          .collect::<Vec<_>>()
          .join(" ")
      })
      .unwrap_or_default()
  }

  fn kill(&mut self, pid: u32) {
    // SAFETY: a kill of a process the run's own scope holds.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
  }
}

/// Kills whatever the files left in the run's own scope, so the scope goes with the run, and names them on
/// one line under the mark. A run that is not in its own scope shares its caller's control group and kills
/// nothing.
pub fn clear_fence(seams: &mut dyn ClearSeams) -> Option<String> {
  let me = seams.pid();
  let unit = fence_unit(me);
  let group = seams
    .cgroup()
    .lines()
    .map(|line| line.trim_start_matches("0::").to_owned())
    .find(|path| path.ends_with(&format!("/{unit}")))?;
  let left: Vec<u32> = seams.members(&group).into_iter().filter(|pid| *pid != me).collect();

  if left.is_empty() {
    return None;
  }

  let named: Vec<String> = left
    .iter()
    .map(|pid| {
      let command = seams.command(*pid);

      format!("{pid} {}", if command.is_empty() { SUITE.lines.gone.clone() } else { command })
    })
    .collect();

  for pid in &left {
    seams.kill(*pid);
  }

  Some(fill_marks(
    &SUITE.lines.fence_killed,
    &[
      ("mark", &SUITE.lines.mark),
      ("count", &left.len().to_string()),
      ("processes", if left.len() == 1 { &SUITE.lines.process } else { &SUITE.lines.processes }),
      ("unit", &unit),
      ("named", &named.join("; ")),
    ],
  ))
}
