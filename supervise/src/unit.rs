//! A unit file read into what the supervisor runs (docs/supervise.md, 「What of a unit it reads」): a service's
//! commands, environment, restart policy and stop, or a timer's elapses, with the keys the supervisor reads and
//! does not apply named. The file is read by `systemd-unit-edit`, as systemd reads it: sections, repeated keys,
//! continued lines and specifiers; a command line and an `Environment=` value are split by `shlex`, the way the
//! shell and systemd split words.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_core::policy::supervise::SUPERVISE;
use oncalendar::CalendarEvent;
use systemd_unit_edit::{Section, SpecifierContext, SystemdUnit, parse_timespan};

/// The keys the supervisor reads and leaves without effect, since they act through control groups or the
/// scheduler a user process does not hold.
pub const NOT_APPLIED: [&str; 7] = [
  "Slice",
  "Nice",
  "IOSchedulingClass",
  "OOMPolicy",
  "MemoryMax",
  "MemorySwapMax",
  "MemoryHigh",
];

/// The values the specifiers of a user unit stand for.
#[derive(Debug, Clone)]
pub struct Specifiers {
  /// `%h`, the home directory.
  pub home: PathBuf,
  /// `%u`, the user's name.
  pub user: String,
  /// `%U`, the user's id.
  pub uid: u32,
  /// `%t`, the runtime directory.
  pub runtime: PathBuf,
}

impl Specifiers {
  /// The specifiers of the account this process runs as.
  pub fn of_this_process() -> Self {
    // SAFETY: getuid only reads the calling process's real user id.
    let uid = unsafe { libc::getuid() };
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
      .map(PathBuf::from)
      .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")));

    Self {
      home,
      user: bmo_testbed_core::account::account_name(),
      uid,
      runtime,
    }
  }

  fn context(&self, unit: &str) -> SpecifierContext {
    let mut context = SpecifierContext::with_unit_name(unit);

    context.set("h", &self.home.to_string_lossy());
    context.set("u", &self.user);
    context.set("U", &self.uid.to_string());
    context.set("t", &self.runtime.to_string_lossy());
    context
  }
}

/// One unit as the supervisor runs it.
#[derive(Debug, Clone)]
pub struct Unit {
  /// The unit's name, `<name>.service` or `<name>.timer`.
  pub name: String,
  pub description: String,
  /// The units this one starts after, where both start together.
  pub after: Vec<String>,
  /// Every `ConditionPathExists=`: a path, and whether it must be absent.
  pub conditions: Vec<(PathBuf, bool)>,
  /// The deploy mark the unit carries (docs/deploy.md, 「The unit links」), where it carries one.
  pub mark: Option<String>,
  /// The keys the file names that the supervisor does not apply.
  pub not_applied: Vec<String>,
  pub kind: Kind,
}

#[derive(Debug, Clone)]
pub enum Kind {
  Service(Service),
  Timer(Timer),
}

/// How a service counts as started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceType {
  /// Started once its main process runs: `simple`, `exec`, and the types the supervisor reads as them.
  Simple,
  /// Started once its main process has ended well.
  Oneshot,
}

/// When a service whose main process ended is started again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
  No,
  Always,
  OnSuccess,
  OnFailure,
  OnAbnormal,
  OnAbort,
}

/// How a main process ended, as `Restart=` weighs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
  /// Exit code 0, or one `SuccessExitStatus=` names, or the clean signals.
  Clean,
  /// Any other exit code.
  Code,
  /// A signal that is not clean.
  Signal,
  /// A signal that dumps core.
  Core,
}

impl Restart {
  /// Whether a main process that ended so is started again.
  pub fn restarts(self, ending: Ending) -> bool {
    match self {
      Restart::No => false,
      Restart::Always => true,
      Restart::OnSuccess => ending == Ending::Clean,
      Restart::OnFailure => ending != Ending::Clean,
      Restart::OnAbnormal => matches!(ending, Ending::Signal | Ending::Core),
      Restart::OnAbort => matches!(ending, Ending::Signal | Ending::Core),
    }
  }
}

/// Which processes a stop signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillMode {
  /// Every process of the unit's process group, the stand-in for its control group.
  Group,
  /// The main process alone.
  Process,
  /// The signal to the main process, the kill at the bound to the whole group.
  Mixed,
}

/// A command line of a unit: its words, and whether its failure is overlooked (the `-` prefix).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
  pub words: Vec<String>,
  pub overlook_failure: bool,
}

#[derive(Debug, Clone)]
pub struct Service {
  pub kind: ServiceType,
  pub start: Option<Line>,
  pub start_pre: Vec<Line>,
  /// The working directory, and whether a missing one is overlooked.
  pub working_directory: Option<(PathBuf, bool)>,
  pub environment: Vec<(String, String)>,
  /// The environment files, each with whether a missing one is overlooked, read as the unit starts.
  pub environment_files: Vec<(PathBuf, bool)>,
  pub restart: Restart,
  pub restart_after: Duration,
  pub start_limit_interval: Duration,
  pub start_limit_burst: u32,
  /// How long a stop waits before the kill; none waits however long the process takes.
  pub stop_bound: Option<Duration>,
  pub kill_mode: KillMode,
  pub kill_signal: i32,
  pub success_codes: Vec<i32>,
  pub success_signals: Vec<i32>,
  pub remain_after_exit: bool,
}

#[derive(Debug, Clone)]
pub struct Timer {
  pub calendars: Vec<CalendarEvent>,
  pub on_boot: Option<Duration>,
  pub on_unit_active: Option<Duration>,
  pub persistent: bool,
  pub randomized_delay: Duration,
  /// The service the timer starts.
  pub unit: String,
}

impl Service {
  /// How a main process that ended with `status` ended, as `Restart=` weighs it.
  pub fn ending(&self, code: Option<i32>, signal: Option<i32>, core: bool) -> Ending {
    match (code, signal) {
      (Some(code), _) if code == 0 || self.success_codes.contains(&code) => Ending::Clean,
      (Some(_), _) => Ending::Code,
      (None, Some(signal)) if clean_signal(signal) || self.success_signals.contains(&signal) => {
        Ending::Clean
      }
      (None, Some(_)) if core => Ending::Core,
      _ => Ending::Signal,
    }
  }

  /// A transient service of `systemd-run`: one command, its directory and its environment, never restarted.
  pub fn transient(words: Vec<String>, cwd: Option<PathBuf>, environment: Vec<(String, String)>) -> Self {
    let numbers = &SUPERVISE.numbers;

    Service {
      kind: ServiceType::Simple,
      start: Some(Line {
        words,
        overlook_failure: false,
      }),
      start_pre: Vec::new(),
      working_directory: cwd.map(|dir| (dir, false)),
      environment,
      environment_files: Vec::new(),
      restart: Restart::No,
      restart_after: Duration::from_secs_f64(numbers.default_restart_seconds),
      start_limit_interval: Duration::from_secs(numbers.default_start_limit_seconds),
      start_limit_burst: numbers.default_start_limit_burst,
      stop_bound: Some(Duration::from_secs(numbers.default_stop_seconds)),
      kill_mode: KillMode::Group,
      kill_signal: libc::SIGTERM,
      success_codes: Vec::new(),
      success_signals: Vec::new(),
      remain_after_exit: false,
    }
  }
}

/// The signals systemd takes as a clean end of a service: the hang-up, the interrupt, the stop and the pipe.
pub fn clean_signal(signal: i32) -> bool {
  [libc::SIGHUP, libc::SIGINT, libc::SIGTERM, libc::SIGPIPE].contains(&signal)
}

/// Reads the unit `name` from its file's text; the reason where it does not read as a unit the supervisor runs.
pub fn read(name: &str, text: &str, specifiers: &Specifiers) -> Result<Unit, String> {
  let file: SystemdUnit = text.parse().map_err(|error: systemd_unit_edit::Error| error.to_string())?;
  let context = specifiers.context(name);
  let expand = |value: String| context.expand(&value);
  let section = |title: &str| file.get_section(title);
  let all = |section: &Option<Section>, key: &str| -> Vec<String> {
    section
      .as_ref()
      .map(|section| section.get_all(key).into_iter().map(expand).collect())
      .unwrap_or_default()
  };
  let one = |section: &Option<Section>, key: &str| all(section, key).into_iter().last();
  let unit = section("Unit");
  let install = section("Install");
  let after = all(&unit, "After")
    .iter()
    .flat_map(|value| value.split_whitespace().map(str::to_owned))
    .collect();
  let conditions = all(&unit, "ConditionPathExists")
    .into_iter()
    .filter(|path| !path.is_empty())
    .map(|path| match path.strip_prefix('!') {
      Some(absent) => (PathBuf::from(absent), true),
      None => (PathBuf::from(path), false),
    })
    .collect();
  let mut not_applied = Vec::new();
  let kind = if name.ends_with(".timer") {
    Kind::Timer(timer(name, &section("Timer"), &one)?)
  } else if name.ends_with(".service") {
    let body = section("Service");

    for key in NOT_APPLIED {
      if one(&body, key).is_some() {
        not_applied.push(key.to_owned());
      }
    }

    let mut service = service(&body, &all, &one)?;

    // The limit on starts stands in the `[Unit]` section, where systemd keeps it.
    if let Some(interval) = one(&unit, "StartLimitIntervalSec") {
      service.start_limit_interval =
        parse_timespan(&interval).map_err(|error| format!("StartLimitIntervalSec: {error}"))?;
    }

    if let Some(burst) = one(&unit, "StartLimitBurst") {
      service.start_limit_burst =
        burst.parse().map_err(|_| format!("StartLimitBurst={burst} is no count"))?;
    }

    Kind::Service(service)
  } else {
    return Err(format!("{name} is neither a service nor a timer"));
  };

  Ok(Unit {
    name: name.to_owned(),
    description: one(&unit, "Description").unwrap_or_default(),
    after,
    conditions,
    mark: one(&install, "X-BMO-Deploy"),
    not_applied,
    kind,
  })
}

type All<'a> = dyn Fn(&Option<Section>, &str) -> Vec<String> + 'a;
type One<'a> = dyn Fn(&Option<Section>, &str) -> Option<String> + 'a;

fn service(body: &Option<Section>, all: &All<'_>, one: &One<'_>) -> Result<Service, String> {
  let numbers = &SUPERVISE.numbers;
  let lines = |key: &str| -> Result<Vec<Line>, String> {
    all(body, key)
      .iter()
      .filter(|value| !value.is_empty())
      .map(|value| line(value))
      .collect()
  };
  let kind = match one(body, "Type").as_deref() {
    Some("oneshot") => ServiceType::Oneshot,
    _ => ServiceType::Simple,
  };
  let restart = match one(body, "Restart").as_deref() {
    None | Some("no") => Restart::No,
    Some("always") => Restart::Always,
    Some("on-success") => Restart::OnSuccess,
    Some("on-failure") => Restart::OnFailure,
    Some("on-abnormal") | Some("on-watchdog") => Restart::OnAbnormal,
    Some("on-abort") => Restart::OnAbort,
    Some(other) => return Err(format!("Restart={other} is no restart policy")),
  };
  let span = |key: &str, default: Duration| -> Result<Duration, String> {
    one(body, key)
      .map_or(Ok(default), |value| parse_timespan(&value).map_err(|error| format!("{key}: {error}")))
  };
  let stop_bound = match one(body, "TimeoutStopSec").as_deref() {
    Some("infinity") | Some("0") => None,
    Some(value) => Some(parse_timespan(value).map_err(|error| format!("TimeoutStopSec: {error}"))?),
    None => Some(Duration::from_secs(numbers.default_stop_seconds)),
  };
  let kill_mode = match one(body, "KillMode").as_deref() {
    Some("process") => KillMode::Process,
    Some("mixed") => KillMode::Mixed,
    _ => KillMode::Group,
  };
  let kill_signal = one(body, "KillSignal").map_or(Ok(libc::SIGTERM), |name| signal_number(&name))?;
  let mut success_codes = Vec::new();
  let mut success_signals = Vec::new();

  for word in all(body, "SuccessExitStatus").iter().flat_map(|value| value.split_whitespace()) {
    match word.parse::<i32>() {
      Ok(code) => success_codes.push(code),
      Err(_) => success_signals.push(signal_number(word)?),
    }
  }

  let mut environment = Vec::new();

  for value in all(body, "Environment") {
    for word in
      shlex::split(&value).ok_or_else(|| format!("Environment={value} does not split into words"))?
    {
      if let Some((key, value)) = word.split_once('=') {
        environment.push((key.to_owned(), value.to_owned()));
      }
    }
  }

  let optional = |value: String| match value.strip_prefix('-') {
    Some(rest) => (PathBuf::from(rest), true),
    None => (PathBuf::from(value), false),
  };
  let start = lines("ExecStart")?.into_iter().last();

  Ok(Service {
    kind,
    start,
    start_pre: lines("ExecStartPre")?,
    working_directory: one(body, "WorkingDirectory").filter(|dir| dir != "~").map(optional),
    environment,
    environment_files: all(body, "EnvironmentFile")
      .into_iter()
      .filter(|file| !file.is_empty())
      .map(optional)
      .collect(),
    restart,
    restart_after: span("RestartSec", Duration::from_secs_f64(numbers.default_restart_seconds))?,
    start_limit_interval: Duration::from_secs(numbers.default_start_limit_seconds),
    start_limit_burst: numbers.default_start_limit_burst,
    stop_bound,
    kill_mode,
    kill_signal,
    success_codes,
    success_signals,
    remain_after_exit: one(body, "RemainAfterExit").is_some_and(|value| truthy(&value)),
  })
}

fn timer(name: &str, body: &Option<Section>, one: &One<'_>) -> Result<Timer, String> {
  let span = |key: &str| -> Result<Option<Duration>, String> {
    one(body, key)
      .map(|value| parse_timespan(&value).map_err(|error| format!("{key}: {error}")))
      .transpose()
  };
  let calendars = body
    .as_ref()
    .map(|body| body.get_all("OnCalendar"))
    .unwrap_or_default()
    .into_iter()
    .filter(|value| !value.is_empty())
    .map(|value| value.parse::<CalendarEvent>().map_err(|error| format!("OnCalendar={value}: {error}")))
    .collect::<Result<Vec<_>, _>>()?;
  let own = format!("{}.service", name.trim_end_matches(".timer"));

  Ok(Timer {
    calendars,
    on_boot: span("OnBootSec")?.or(span("OnStartupSec")?),
    on_unit_active: span("OnUnitActiveSec")?,
    persistent: one(body, "Persistent").is_some_and(|value| truthy(&value)),
    randomized_delay: span("RandomizedDelaySec")?.unwrap_or_default(),
    unit: one(body, "Unit").unwrap_or(own),
  })
}

/// A command line of a unit read into its words: the prefixes systemd reads taken off it, `-` kept as whether a
/// failure is overlooked and the others without effect for a user process.
pub fn line(value: &str) -> Result<Line, String> {
  let mut rest = value.trim_start();
  let mut overlook_failure = false;

  while let Some(prefix) = rest.chars().next().filter(|first| "-@+!:|".contains(*first)) {
    overlook_failure |= prefix == '-';
    rest = &rest[prefix.len_utf8()..];
  }

  let words = shlex::split(rest).ok_or_else(|| format!("{value} does not split into words"))?;

  if words.is_empty() {
    return Err(SUPERVISE.said.no_command.clone());
  }

  Ok(Line {
    words,
    overlook_failure,
  })
}

fn truthy(value: &str) -> bool {
  matches!(value.to_ascii_lowercase().as_str(), "yes" | "true" | "on" | "1")
}

/// A signal named as systemd names one, `SIGTERM` or `TERM`, or by its number.
pub fn signal_number(name: &str) -> Result<i32, String> {
  let bare = name.trim().trim_start_matches("SIG");

  if let Ok(number) = bare.parse::<i32>() {
    return Ok(number);
  }

  Ok(match bare {
    "HUP" => libc::SIGHUP,
    "INT" => libc::SIGINT,
    "QUIT" => libc::SIGQUIT,
    "KILL" => libc::SIGKILL,
    "USR1" => libc::SIGUSR1,
    "USR2" => libc::SIGUSR2,
    "PIPE" => libc::SIGPIPE,
    "TERM" => libc::SIGTERM,
    "CONT" => libc::SIGCONT,
    "STOP" => libc::SIGSTOP,
    "ABRT" => libc::SIGABRT,
    _ => return Err(format!("{name} is no signal")),
  })
}

/// Whether every condition of the unit holds.
pub fn conditions_hold(unit: &Unit) -> bool {
  unit.conditions.iter().all(|(path, absent)| Path::new(path).exists() != *absent)
}
