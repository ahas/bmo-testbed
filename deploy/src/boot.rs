//! Watching the `bot` service come up after a restart, and saying what came of it where it did not
//! (docs/deploy.md, 「What the deploy run does, in order」, step 11, and 「Rollback」). A unit that gave up
//! starting, or whose health had not answered when the wait's bound passed, is the one moment the failing
//! service can still be looked at — the rollback stops it next — so what was waited for, what systemd and the
//! kernel hold of it and the tail of its journal are read here, into lines the deploy output keeps.

use std::future::Future;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use tokio::time::Instant;

use crate::run::{Ran, command, shell_words, words};
use crate::unit::first_chars;

/// How a wait for the declared health ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthEnd {
  /// The health answered 0.
  Answered,
  /// The unit gave up starting before the health answered.
  GaveUp,
  /// The wait's bound passed before the health answered.
  RanOut,
}

/// What one wait for the declared health came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthWait {
  /// The health command, as the declaration writes it.
  pub command: String,
  pub end: HealthEnd,
  pub waited_ms: u64,
  /// The bound the wait was held to.
  pub bound_ms: u64,
  pub asks: u32,
  /// What the last ask heard: its exit and the start of what it printed, or why nothing came.
  pub last: String,
}

impl HealthWait {
  pub fn answered(&self) -> bool {
    self.end == HealthEnd::Answered
  }
}

/// Whether the unit has given up starting.
pub type GaveUp<'a> = Box<dyn Fn() -> bool + 'a>;

/// One ask of the health: whether it answered 0, and what it said of itself. A wait drops an ask still
/// running when its bound passes, and the ask ends whatever it started as it is dropped.
pub type HealthAsk<'a> = Box<dyn Fn() -> Pin<Box<dyn Future<Output = (bool, String)> + 'a>> + 'a>;

/// What a wait asks, how often and for how long.
pub struct HealthWatch<'a> {
  /// The declared health command, as the deploy output names it.
  pub command: String,
  /// One ask of it; the command run by the shell in the deploy tree, where the watch is made for one.
  pub ask: HealthAsk<'a>,
  /// Whether the unit has given up starting; asked between asks, at most every `check_every_ms`.
  pub gave_up: Option<GaveUp<'a>>,
  /// How long the wait may last, counted from its start.
  pub bound: Duration,
  pub poll_ms: u64,
  pub check_every_ms: u64,
}

impl<'a> HealthWatch<'a> {
  /// A wait for a health command run in `cwd`, asking and checking the unit as the policy says, until it
  /// answers, the unit gives up starting or `bound_seconds` pass.
  pub fn new(command: &str, cwd: &Path, bound_seconds: u64) -> Self {
    let numbers = &DEPLOY.numbers;
    let line = command.to_owned();
    let cwd = cwd.to_path_buf();

    Self {
      command: command.to_owned(),
      ask: Box::new(move || Box::pin(ask_health(line.clone(), cwd.clone()))),
      gave_up: None,
      bound: Duration::from_secs(bound_seconds),
      poll_ms: numbers.health_poll_ms,
      check_every_ms: numbers.unit_check_ms,
    }
  }
}

// Ends a process group where it is dropped while armed: an ask dropped before its command ended takes
// down every process the command started, not the shell alone.
struct GroupEnd(Option<i32>);

impl Drop for GroupEnd {
  fn drop(&mut self) {
    if let Some(group) = self.0 {
      // SAFETY: killpg takes a group id and a signal and touches no memory; a group already gone answers
      // an error that is of no account here.
      unsafe {
        libc::killpg(group, libc::SIGKILL);
      }
    }
  }
}

/// One ask of the health command, run by the shell in `cwd` in a process group of its own: whether it
/// answered 0, and what it said of itself. The ask is waited for until the command ends; dropped before
/// that, it ends the command's whole group.
pub async fn ask_health(line: String, cwd: PathBuf) -> (bool, String) {
  let mut ask = command(&shell_words(&line), Some(&cwd));

  ask
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .process_group(0);

  let child = match ask.spawn() {
    Ok(child) => child,
    Err(why) => return (false, fill_shown(&DEPLOY.boot.not_run, &[("why", &why)])),
  };
  let mut group = GroupEnd(i32::try_from(child.id()).ok());
  // The command is waited for on a thread of its own, which the runtime counts as work under way: a clock
  // paused under a test moves on only once the command has ended, never past one still running. Where the
  // ask is dropped first, the group's end ends the command and the thread with it.
  let done = tokio::task::spawn_blocking(move || child.wait_with_output()).await;

  // The command ended and nothing holds its output any more, so nothing of its group is left to end.
  group.0 = None;

  match done {
    Err(why) => (false, fill_shown(&DEPLOY.boot.not_run, &[("why", &why)])),
    Ok(Err(why)) => (false, fill_shown(&DEPLOY.boot.not_run, &[("why", &why)])),
    Ok(Ok(done)) => {
      let code = done.status.code().map_or_else(|| DEPLOY.boot.a_signal.clone(), |code| code.to_string());
      let said =
        format!("{}{}", String::from_utf8_lossy(&done.stdout), String::from_utf8_lossy(&done.stderr));
      let said = said.trim().lines().next().unwrap_or_default();
      let last = if said.is_empty() {
        fill_shown(&DEPLOY.boot.exit, &[("code", &code)])
      } else {
        fill_shown(
          &DEPLOY.boot.exit_said,
          &[("code", &code), ("said", &first_chars(said, DEPLOY.numbers.quoted_chars))],
        )
      };

      (done.status.success(), last)
    }
  }
}

/// The health asked until it answers 0, the unit gives up starting or the watch's bound passes, whichever
/// comes first. An ask still running when the bound passes is dropped there.
pub async fn wait_for_health(watch: &HealthWatch<'_>) -> HealthWait {
  let started = Instant::now();
  let deadline = started + watch.bound;
  let mut checked = started;
  let mut asks = 0;
  let ended = |end: HealthEnd, asks: u32, last: String| HealthWait {
    command: watch.command.clone(),
    end,
    waited_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    bound_ms: u64::try_from(watch.bound.as_millis()).unwrap_or(u64::MAX),
    asks,
    last,
  };

  loop {
    asks += 1;

    let Ok((answered, last)) = tokio::time::timeout_at(deadline, (watch.ask)()).await else {
      return ended(HealthEnd::RanOut, asks, DEPLOY.boot.cut.clone());
    };

    if answered {
      return ended(HealthEnd::Answered, asks, last);
    }

    if let Some(gave_up) = &watch.gave_up
      && checked.elapsed() >= Duration::from_millis(watch.check_every_ms)
    {
      checked = Instant::now();

      if gave_up() {
        return ended(HealthEnd::GaveUp, asks, last);
      }
    }

    if Instant::now() >= deadline {
      return ended(HealthEnd::RanOut, asks, last);
    }

    tokio::time::sleep(Duration::from_millis(watch.poll_ms)).await;
  }
}

/// The unit as systemd holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitStatus {
  pub active: String,
  pub sub: String,
  pub pid: String,
  pub restarts: String,
  pub exit_status: String,
}

/// Asks systemd for the unit through `run`; none where it would not say.
pub fn unit_status(run: &dyn Fn(&[String]) -> Ran, unit: &str) -> Option<UnitStatus> {
  let shown = run(&words([
    "systemctl",
    "--user",
    "show",
    "-p",
    "ActiveState,SubState,MainPID,NRestarts,ExecMainStatus",
    unit,
  ]));

  if !shown.ok {
    return None;
  }

  // `systemctl show` prints one `Name=value` to a line, its own listing rather than a file format any
  // reader is kept for; each property asked is the one line that starts with its name.
  let property = |name: &str| {
    shown
      .out
      .lines()
      .find_map(|line| line.strip_prefix(&format!("{name}=")))
      .map_or_else(|| "?".to_owned(), |value| value.trim().to_owned())
  };

  Some(UnitStatus {
    active: property("ActiveState"),
    sub: property("SubState"),
    pid: property("MainPID"),
    restarts: property("NRestarts"),
    exit_status: property("ExecMainStatus"),
  })
}

fn number(text: &str) -> i64 {
  text.trim().parse().unwrap_or(0)
}

/// A unit that has given up starting, `now` against the reading `first` taken as the wait began: one that
/// has stopped trying, or one whose process ended and that systemd restarted since, which says the process
/// the deployment started is gone. A reading systemd would not give says nothing.
pub fn unit_gave_up(first: Option<&UnitStatus>, now: Option<&UnitStatus>) -> bool {
  now.is_some_and(|now| {
    now.active == "failed"
      || now.active == "inactive"
      || first.is_some_and(|first| number(&now.restarts) > number(&first.restarts))
  })
}

/// What one reading of the service, once it came up, came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StayUp {
  Stayed,
  Fell {
    why: String,
    status: Option<UnitStatus>,
  },
}

fn is_running(status: &UnitStatus) -> bool {
  status.active == "active" && status.sub == "running" && status.pid != "0"
}

/// The unit read once it came up: running is up. A health that answered says the service came up; where
/// none is declared, this reading is what says it.
pub fn stands_up(status: &dyn Fn() -> Option<UnitStatus>) -> StayUp {
  let Some(now) = status() else {
    return StayUp::Fell {
      why: DEPLOY.boot.unasked.clone(),
      status: None,
    };
  };

  if !is_running(&now) {
    return StayUp::Fell {
      why: fill_shown(&DEPLOY.boot.unit_is, &[("active", &now.active), ("sub", &now.sub)]),
      status: Some(now),
    };
  }

  StayUp::Stayed
}

// What the kernel says the process is doing: running, or asleep on a disk it is waiting for.
fn process_state(pid: &str) -> Option<String> {
  if pid.is_empty() || pid == "0" || pid == "?" {
    return None;
  }

  let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;

  status
    .lines()
    .find_map(|line| line.strip_prefix("State:"))
    .map(|state| state.trim().to_owned())
}

// The load and the share of the last minute every task spent stalled on I/O and on memory, which is what
// makes a start run long on a strained machine.
fn machine_strain() -> String {
  let stalled = |resource: &str| {
    std::fs::read_to_string(format!("/proc/pressure/{resource}"))
      .ok()
      .and_then(|text| {
        text.lines().find(|line| line.starts_with("some ")).and_then(|line| {
          line
            .split(' ')
            .find_map(|field| field.strip_prefix("avg60="))
            .map(|value| format!("{value}%"))
        })
      })
      .unwrap_or_else(|| DEPLOY.boot.unknown.clone())
  };
  let load = std::fs::read_to_string("/proc/loadavg")
    .map(|text| text.split(' ').take(3).collect::<Vec<_>>().join(" "))
    .unwrap_or_else(|_| DEPLOY.boot.unknown.clone());

  fill_shown(
    &DEPLOY.boot.strain,
    &[("load", &load), ("io", &stalled("io")), ("memory", &stalled("memory"))],
  )
}

/// Everything read about a service whose health did not answer.
#[derive(Debug, Clone)]
pub struct BootScene {
  pub wait: HealthWait,
  pub unit: String,
  pub status: Option<UnitStatus>,
  pub process: Option<String>,
  pub strain: String,
  pub journal: Vec<String>,
}

fn journal_since(run: &dyn Fn(&[String]) -> Ran, unit: &str, restarted_at_ms: i64) -> Vec<String> {
  let journal = run(&[
    "journalctl".to_owned(),
    "--user".to_owned(),
    "-u".to_owned(),
    unit.to_owned(),
    "--since".to_owned(),
    format!("@{}", restarted_at_ms.div_euclid(1000)),
    "-n".to_owned(),
    DEPLOY.numbers.journal_lines.to_string(),
    "--no-pager".to_owned(),
    "-o".to_owned(),
    "short-iso".to_owned(),
  ]);

  if journal.ok {
    journal.out.lines().filter(|line| !line.trim().is_empty()).map(str::to_owned).collect()
  } else {
    vec![fill_shown(
      &DEPLOY.boot.journal_unread,
      &[("said", &first_chars(journal.out.trim(), DEPLOY.numbers.command_chars))],
    )]
  }
}

/// Reads the scene at once, while the service whose health did not answer is still there to be read.
pub fn boot_scene(
  run: &dyn Fn(&[String]) -> Ran,
  unit: &str,
  restarted_at_ms: i64,
  wait: HealthWait,
) -> BootScene {
  let status = unit_status(run, unit);

  BootScene {
    wait,
    unit: unit.to_owned(),
    process: status.as_ref().and_then(|status| process_state(&status.pid)),
    status,
    strain: machine_strain(),
    journal: journal_since(run, unit, restarted_at_ms),
  }
}

fn seconds(ms: u64) -> u64 {
  (ms + 500) / 1000
}

fn unit_line(unit: &str, status: Option<&UnitStatus>, process: Option<&str>) -> String {
  match status {
    None => fill_shown(&DEPLOY.boot.unit_unasked, &[("unit", &unit)]),
    Some(status) => fill_shown(
      &DEPLOY.boot.unit,
      &[
        ("unit", &unit),
        ("active", &status.active),
        ("sub", &status.sub),
        ("pid", &status.pid),
        (
          "process",
          &process
            .map(|process| fill_shown(&DEPLOY.boot.process, &[("process", &process)]))
            .unwrap_or_default(),
        ),
        ("restarts", &status.restarts),
        ("status", &status.exit_status),
      ],
    ),
  }
}

fn journal_report(unit: &str, journal: &[String]) -> Vec<String> {
  std::iter::once(fill_shown(
    &DEPLOY.boot.journal,
    &[("unit", &unit), ("lines", &DEPLOY.numbers.journal_lines)],
  ))
  .chain(journal.iter().map(|line| format!("  {line}")))
  .collect()
}

/// The lines the deploy output keeps of it.
pub fn boot_report(scene: &BootScene) -> Vec<String> {
  let wait = &scene.wait;
  let mut lines = vec![
    fill_shown(
      &DEPLOY.boot.waited,
      &[
        ("waited", &seconds(wait.waited_ms)),
        ("command", &wait.command),
        ("asks", &wait.asks),
        ("last", &wait.last),
        (
          "end",
          &match wait.end {
            HealthEnd::Answered => String::new(),
            HealthEnd::GaveUp => fill_shown(&DEPLOY.boot.gave_up, &[("unit", &scene.unit)]),
            HealthEnd::RanOut => fill_shown(&DEPLOY.boot.ran_out, &[("bound", &seconds(wait.bound_ms))]),
          },
        ),
      ],
    ),
    unit_line(&scene.unit, scene.status.as_ref(), scene.process.as_deref()),
    fill_shown(&DEPLOY.boot.machine, &[("strain", &scene.strain)]),
  ];

  lines.extend(journal_report(&scene.unit, &scene.journal));

  lines
}

/// The lines the deploy output keeps of a service that answered and did not stay up, read at once.
pub fn stay_report(
  run: &dyn Fn(&[String]) -> Ran,
  unit: &str,
  restarted_at_ms: i64,
  why: &str,
  fell: Option<UnitStatus>,
) -> Vec<String> {
  let status = unit_status(run, unit).or(fell);
  let process = status.as_ref().and_then(|status| process_state(&status.pid));
  let mut lines = vec![
    fill_shown(&DEPLOY.boot.fell, &[("unit", &unit), ("why", &why)]),
    unit_line(unit, status.as_ref(), process.as_deref()),
  ];

  lines.extend(journal_report(unit, &journal_since(run, unit, restarted_at_ms)));

  lines
}
