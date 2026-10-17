//! Watching the `bot` service come up after a restart, and saying what came of it where it did not
//! (docs/deploy.md, 「What the deploy run does, in order」, step 10, and 「Rollback」). A wait that runs out is
//! the one moment the failing service can still be looked at — the rollback stops it next — so what was
//! waited for, what systemd and the kernel hold of it and the tail of its journal are read here, into lines
//! the deploy output keeps.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_core::policy::deploy::DEPLOY;
use tokio::time::Instant;

use crate::run::{Ran, command, shell_words, words};
use crate::unit::first_chars;

/// What one wait for the declared health came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthWait {
  /// The health command, as the declaration writes it.
  pub command: String,
  pub limit_ms: u64,
  pub answered: bool,
  pub waited_ms: u64,
  pub asks: u32,
  /// What the last ask heard: its exit and the start of what it printed, or why nothing came.
  pub last: String,
  /// The unit gave up starting before the limit, which ended the wait at once.
  pub gave_up: bool,
}

/// Whether the unit has given up starting.
pub type GaveUp<'a> = Box<dyn Fn() -> bool + 'a>;

/// What a wait asks and how often.
pub struct HealthWatch<'a> {
  /// The declared health command, run by the shell in `cwd`; it answers 0 once the service is healthy.
  pub command: String,
  pub cwd: PathBuf,
  pub limit_ms: u64,
  /// The longest one ask may take before it counts as no answer.
  pub ask_limit_ms: u64,
  /// Whether the unit has given up starting; asked between asks, at most every `check_every_ms`.
  pub gave_up: Option<GaveUp<'a>>,
  pub poll_ms: u64,
  pub check_every_ms: u64,
}

impl<'a> HealthWatch<'a> {
  /// A wait of `limit_ms` for a health command, asking and checking the unit as the policy says.
  pub fn new(command: &str, cwd: &Path, limit_ms: u64) -> Self {
    let numbers = &DEPLOY.numbers;

    Self {
      command: command.to_owned(),
      cwd: cwd.to_path_buf(),
      limit_ms,
      ask_limit_ms: numbers.health_ask_ms,
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

  match tokio::time::timeout(Duration::from_millis(watch.ask_limit_ms), ask.output()).await {
    Err(_) => (false, format!("no answer within {}s", seconds(watch.ask_limit_ms))),
    Ok(Err(why)) => (false, format!("could not be run: {why}")),
    Ok(Ok(done)) => {
      let code = done.status.code().map_or_else(|| "a signal".to_owned(), |code| code.to_string());
      let said =
        format!("{}{}", String::from_utf8_lossy(&done.stdout), String::from_utf8_lossy(&done.stderr));
      let said = said.trim().lines().next().unwrap_or_default();
      let last = if said.is_empty() {
        format!("exit {code}")
      } else {
        format!("exit {code}: {}", first_chars(said, DEPLOY.numbers.quoted_chars))
      };

      (done.status.success(), last)
    }
  }
}

/// The health asked until it answers 0, the limit runs out, or the unit gives up starting.
pub async fn wait_for_health(watch: &HealthWatch<'_>) -> HealthWait {
  let started = Instant::now();
  let until = started + Duration::from_millis(watch.limit_ms);
  let mut checked = started;
  let mut asks = 0;
  let ended = |answered: bool, gave_up: bool, asks: u32, last: String| HealthWait {
    command: watch.command.clone(),
    limit_ms: watch.limit_ms,
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

    if Instant::now() >= until {
      return ended(false, false, asks, last);
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

/// A unit that has stopped trying: one that restarts itself is still on its way.
pub fn unit_gave_up(status: Option<&UnitStatus>) -> bool {
  status.is_some_and(|status| status.active == "failed" || status.active == "inactive")
}

/// What one watch of the service staying up came to.
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

fn number(text: &str) -> i64 {
  text.trim().parse().unwrap_or(0)
}

/// Watches the unit for the hold: running throughout, the same main process, and no automatic restart
/// counted past the first reading. A health that answered says only that the service came up, and a start
/// that throws a moment later has already answered once; a unit restarting itself every few seconds is
/// still a dead service. A reading systemd would not give is passed over, so a busy bus does not fail a
/// service that runs, but a hold with no reading at all is not one the service was seen to stay up for.
pub async fn stays_up(hold_ms: u64, poll_ms: u64, status: &dyn Fn() -> Option<UnitStatus>) -> StayUp {
  let Some(first) = status() else {
    return StayUp::Fell {
      why: "systemd could not be asked for the unit".to_owned(),
      status: None,
    };
  };

  if !is_running(&first) {
    return StayUp::Fell {
      why: format!("the unit is {}/{}", first.active, first.sub),
      status: Some(first),
    };
  }

  let until = Instant::now() + Duration::from_millis(hold_ms);
  let mut read = false;

  while Instant::now() < until {
    let left = until.saturating_duration_since(Instant::now());

    tokio::time::sleep(Duration::from_millis(poll_ms).min(left)).await;

    let Some(now) = status() else {
      continue;
    };

    read = true;

    if !is_running(&now) {
      return StayUp::Fell {
        why: format!("the unit went {}/{}", now.active, now.sub),
        status: Some(now),
      };
    }

    if now.pid != first.pid {
      return StayUp::Fell {
        why: format!("its main process changed from {} to {}", first.pid, now.pid),
        status: Some(now),
      };
    }

    if number(&now.restarts) > number(&first.restarts) {
      return StayUp::Fell {
        why: format!("systemd restarted it {} time(s)", number(&now.restarts) - number(&first.restarts)),
        status: Some(now),
      };
    }
  }

  if !read && hold_ms > 0 {
    return StayUp::Fell {
      why: "systemd could not be asked for the unit during the hold".to_owned(),
      status: None,
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
      .unwrap_or_else(|| "unknown".to_owned())
  };
  let load = std::fs::read_to_string("/proc/loadavg")
    .map(|text| text.split(' ').take(3).collect::<Vec<_>>().join(" "))
    .unwrap_or_else(|_| "unknown".to_owned());

  format!(
    "load {load}, stalled on I/O {} and on memory {} of the last minute",
    stalled("io"),
    stalled("memory")
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
    vec![format!(
      "journalctl could not be read: {}",
      first_chars(journal.out.trim(), DEPLOY.numbers.command_chars)
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
    None => format!("{unit}: systemd could not be asked"),
    Some(status) => format!(
      "{unit}: {}/{}, main process {}{}, {} automatic restarts, last exit status {}",
      status.active,
      status.sub,
      status.pid,
      process.map(|process| format!(" ({process})")).unwrap_or_default(),
      status.restarts,
      status.exit_status
    ),
  }
}

fn journal_report(unit: &str, journal: &[String]) -> Vec<String> {
  std::iter::once(format!(
    "{unit}'s journal since the restart, its last {} lines at most:",
    DEPLOY.numbers.journal_lines
  ))
  .chain(journal.iter().map(|line| format!("  {line}")))
  .collect()
}

/// The lines the deploy output keeps of it.
pub fn boot_report(scene: &BootScene) -> Vec<String> {
  let wait = &scene.wait;
  let mut lines = vec![
    format!(
      "waited {}s of {}s for the health `{}`: {} asks, the last heard {}{}",
      seconds(wait.waited_ms),
      seconds(wait.limit_ms),
      wait.command,
      wait.asks,
      wait.last,
      if wait.gave_up { format!("; {} gave up starting", scene.unit) } else { String::new() }
    ),
    unit_line(&scene.unit, scene.status.as_ref(), scene.process.as_deref()),
    format!("machine: {}", scene.strain),
  ];

  lines.extend(journal_report(&scene.unit, &scene.journal));

  lines
}

/// The lines the deploy output keeps of a service that answered and did not stay up, read at once.
pub fn stay_report(
  run: &dyn Fn(&[String]) -> Ran,
  unit: &str,
  restarted_at_ms: i64,
  hold_ms: u64,
  why: &str,
  fell: Option<UnitStatus>,
) -> Vec<String> {
  let status = unit_status(run, unit).or(fell);
  let process = status.as_ref().and_then(|status| process_state(&status.pid));
  let mut lines = vec![
    format!("{unit} answered and did not stay up for {}s: {why}", seconds(hold_ms)),
    unit_line(unit, status.as_ref(), process.as_deref()),
  ];

  lines.extend(journal_report(unit, &journal_since(run, unit, restarted_at_ms)));

  lines
}
