//! Watching the `bot` service come up after a restart, and saying what came of it where it did not
//! (docs/deploy.md, 「What the deploy run does, in order」, step 10, and 「Rollback」). A unit that gave up
//! starting is the one moment the failing service can still be looked at — the rollback stops it next — so
//! what was waited for, what systemd and the kernel hold of it and the tail of its journal are read here, into
//! lines the deploy output keeps.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use tokio::time::Instant;

use crate::run::{Ran, command, shell_words, words};
use crate::unit::first_chars;

/// What one wait for the declared health came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthWait {
  /// The health command, as the declaration writes it.
  pub command: String,
  pub answered: bool,
  pub waited_ms: u64,
  pub asks: u32,
  /// What the last ask heard: its exit and the start of what it printed, or why nothing came.
  pub last: String,
  /// The unit gave up starting before the health answered, which ended the wait at once.
  pub gave_up: bool,
}

/// Whether the unit has given up starting.
pub type GaveUp<'a> = Box<dyn Fn() -> bool + 'a>;

/// What a wait asks and how often.
pub struct HealthWatch<'a> {
  /// The declared health command, run by the shell in `cwd`; it answers 0 once the service is healthy.
  pub command: String,
  pub cwd: PathBuf,
  /// Whether the unit has given up starting; asked between asks, at most every `check_every_ms`.
  pub gave_up: Option<GaveUp<'a>>,
  pub poll_ms: u64,
  pub check_every_ms: u64,
}

impl<'a> HealthWatch<'a> {
  /// A wait for a health command, asking and checking the unit as the policy says, until it answers or the
  /// unit gives up starting.
  pub fn new(command: &str, cwd: &Path) -> Self {
    let numbers = &DEPLOY.numbers;

    Self {
      command: command.to_owned(),
      cwd: cwd.to_path_buf(),
      gave_up: None,
      poll_ms: numbers.health_poll_ms,
      check_every_ms: numbers.unit_check_ms,
    }
  }
}

// One ask of the health command: whether it answered 0, and what it said of itself.
async fn ask_health(watch: &HealthWatch<'_>) -> (bool, String) {
  let mut ask = tokio::process::Command::from(command(&shell_words(&watch.command), Some(&watch.cwd)));

  ask.kill_on_drop(true);

  // One ask is waited for until the command ends.
  match ask.output().await {
    Err(why) => (false, fill_shown(&DEPLOY.boot.not_run, &[("why", &why)])),
    Ok(done) => {
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

/// The health asked until it answers 0 or the unit gives up starting, however long that takes.
pub async fn wait_for_health(watch: &HealthWatch<'_>) -> HealthWait {
  let started = Instant::now();
  let mut checked = started;
  let mut asks = 0;
  let ended = |answered: bool, gave_up: bool, asks: u32, last: String| HealthWait {
    command: watch.command.clone(),
    answered,
    waited_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    asks,
    last,
    gave_up,
  };

  loop {
    asks += 1;

    let (answered, last) = ask_health(watch).await;

    if answered {
      return ended(true, false, asks, last);
    }

    if let Some(gave_up) = &watch.gave_up
      && checked.elapsed() >= Duration::from_millis(watch.check_every_ms)
    {
      checked = Instant::now();

      if gave_up() {
        return ended(false, true, asks, last);
      }
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
          "gave_up",
          &if wait.gave_up {
            fill_shown(&DEPLOY.boot.gave_up, &[("unit", &scene.unit)])
          } else {
            String::new()
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
