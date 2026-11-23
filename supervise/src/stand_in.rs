//! The stand-ins (docs/supervise.md, 「The stand-ins」): the binary called as `systemctl`, `systemd-run` or
//! `journalctl` from the supervisor's folder answers the set's calls as the user manager would, each call's words
//! read by clap, turned into a request of the control socket, and its answer printed in systemd's own forms.

use std::ffi::OsString;
use std::io::{BufRead, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::supervise::SUPERVISE;
use bmo_testbed_core::verb::data_root;
use clap::Parser;

use crate::control::{Answer, Request, ask_blocking};
use crate::places::Places;
use crate::state::UnitState;
use crate::unit::signal_number;

/// The code systemctl ends with where a unit is not active, `is-active` and `status` among its verbs.
pub const INACTIVE_CODE: i32 = 3;

/// Whether the binary was called by the name of a stand-in.
pub fn stand_in_name(program: &OsString) -> Option<String> {
  let name = Path::new(program).file_name()?.to_str()?;

  SUPERVISE.places.names.iter().any(|known| known == name).then(|| name.to_owned())
}

/// The control socket a stand-in asks: the one `BMO_SUPERVISOR` names, else the data root's.
pub fn socket_of_environment() -> PathBuf {
  std::env::var_os(&SUPERVISE.places.variable)
    .filter(|named| !named.is_empty())
    .map(PathBuf::from)
    .unwrap_or_else(|| Places::of_this_process(&data_root()).socket())
}

/// Runs the stand-in `name` over its words; the code it ends with.
pub fn main(name: &str, args: Vec<OsString>) -> i32 {
  let socket = socket_of_environment();
  let printed = match name {
    "systemctl" => Systemctl::try_parse_from(args).map(|words| systemctl(&words, &socket)),
    "systemd-run" => SystemdRun::try_parse_from(args).map(|words| systemd_run(words, &socket)),
    _ => Journalctl::try_parse_from(args).map(|words| journalctl(&words, &socket)),
  };

  match printed {
    Ok(code) => code,
    Err(error) => {
      let _ = error.print();
      error.exit_code()
    }
  }
}

/// `systemctl`'s words, the ones the set's calls use.
#[derive(Debug, Parser)]
#[command(name = "systemctl", disable_version_flag = true)]
pub struct Systemctl {
  #[arg(long)]
  pub user: bool,
  #[arg(long, short = 'q')]
  pub quiet: bool,
  #[arg(long)]
  pub now: bool,
  #[arg(long = "property", short = 'p', value_delimiter = ',')]
  pub property: Vec<String>,
  #[arg(long)]
  pub value: bool,
  #[arg(long)]
  pub no_pager: bool,
  #[arg(long)]
  pub no_block: bool,
  #[arg(long, short = 'a')]
  pub all: bool,
  #[arg(long, short = 'f')]
  pub force: bool,
  #[arg(long = "signal", short = 's')]
  pub signal: Option<String>,
  #[arg(long)]
  pub plain: bool,
  #[arg(long)]
  pub no_legend: bool,
  pub verb: String,
  pub units: Vec<String>,
}

/// A unit named without its type, as systemctl reads it: a service.
pub fn unit_name(named: &str) -> String {
  if named.contains('/')
    || [".service", ".timer", ".slice", ".target", ".socket", ".scope"]
      .iter()
      .any(|kind| named.ends_with(kind))
  {
    named.to_owned()
  } else {
    format!("{named}.service")
  }
}

/// The request a `systemctl` call is, or the state reading it is answered from; none for a verb the supervisor
/// does not answer.
pub fn systemctl_request(words: &Systemctl) -> Option<Request> {
  let units: Vec<String> = words.units.iter().map(|unit| unit_name(unit)).collect();
  let only_services = || {
    units
      .iter()
      .filter(|unit| !unit.ends_with(".slice") && !unit.ends_with(".target"))
      .cloned()
      .collect()
  };

  Some(match words.verb.as_str() {
    "start" => Request::Start {
      units: only_services(),
    },
    "stop" => Request::Stop {
      units: only_services(),
    },
    "restart" | "reload-or-restart" => Request::Restart {
      units: only_services(),
    },
    "try-restart" | "try-reload-or-restart" => Request::TryRestart {
      units: only_services(),
    },
    "reset-failed" => Request::ResetFailed { units },
    "daemon-reload" | "daemon-reexec" => Request::DaemonReload,
    "enable" => Request::Enable {
      units: words
        .units
        .iter()
        .map(|unit| if unit.contains('/') { unit.clone() } else { unit_name(unit) })
        .collect(),
      now: words.now,
    },
    "disable" => Request::Disable {
      units,
      now: words.now,
    },
    "kill" => Request::Kill {
      units,
      signal: words.signal.as_deref().map_or(Ok(libc::SIGTERM), signal_number).ok()?,
    },
    "show-environment" => Request::ShowEnvironment,
    "is-active" | "is-enabled" | "is-failed" | "show" | "status" | "list-units" => Request::Show { units },
    _ => return None,
  })
}

fn systemctl(words: &Systemctl, socket: &Path) -> i32 {
  let Some(request) = systemctl_request(words) else {
    eprintln!("{}", fill_marks(&SUPERVISE.said.no_verb, &[("verb", &words.verb)]));
    return 1;
  };
  let answer = match ask_blocking(socket, &request) {
    Ok(answer) => answer,
    Err(why) => {
      eprintln!("{why}");
      return 1;
    }
  };
  let (code, out) = systemctl_said(words, &answer);

  if !out.is_empty() {
    println!("{out}");
  }

  if !answer.err.is_empty() && !words.quiet {
    eprintln!("{}", answer.err);
  }

  code
}

/// What a `systemctl` call prints and ends with, given the supervisor's answer.
pub fn systemctl_said(words: &Systemctl, answer: &Answer) -> (i32, String) {
  let states = &answer.states;
  let lines = |each: &dyn Fn(&UnitState) -> String| states.iter().map(each).collect::<Vec<_>>().join("\n");
  let quiet = |out: String| if words.quiet { String::new() } else { out };

  match words.verb.as_str() {
    "is-active" => {
      let code = if states.iter().any(UnitState::is_active) { 0 } else { INACTIVE_CODE };

      (code, quiet(lines(&|state| state.active_state.clone())))
    }
    "is-failed" => {
      let code = if states.iter().any(|state| state.active_state == "failed") { 0 } else { 1 };

      (code, quiet(lines(&|state| state.active_state.clone())))
    }
    "is-enabled" => {
      let code = if states.iter().any(|state| state.unit_file_state == "enabled") { 0 } else { 1 };
      let shown = lines(&|state| {
        if state.load_state == "not-found" {
          "not-found".to_owned()
        } else {
          state.unit_file_state.clone()
        }
      });

      (code, quiet(shown))
    }
    "show" => (0, show(states, &words.property, words.value)),
    "status" => {
      let code = if states.iter().all(|state| state.load_state == "not-found") {
        4
      } else if states.iter().all(UnitState::is_active) {
        0
      } else {
        INACTIVE_CODE
      };

      (code, quiet(lines(&status_block)))
    }
    "list-units" => (
      0,
      quiet(lines(&|state| {
        format!(
          "{} {} {} {} {}",
          state.id, state.load_state, state.active_state, state.sub_state, state.description
        )
      })),
    ),
    _ => (answer.code, answer.out.clone()),
  }
}

/// The properties `systemctl show` prints: `Name=value` a line, a block a unit, only the named properties where
/// any are named, and the values alone with `--value`.
pub fn show(states: &[UnitState], properties: &[String], value_only: bool) -> String {
  states
    .iter()
    .map(|state| {
      state
        .properties()
        .into_iter()
        .filter(|(name, _)| properties.is_empty() || properties.iter().any(|asked| asked == name))
        .map(|(name, value)| if value_only { value } else { format!("{name}={value}") })
        .collect::<Vec<_>>()
        .join("\n")
    })
    .collect::<Vec<_>>()
    .join("\n\n")
}

fn status_block(state: &UnitState) -> String {
  let pid = if state.main_pid == 0 {
    String::new()
  } else {
    format!("\n   Main PID: {}", state.main_pid)
  };

  format!(
    "● {} - {}\n     Loaded: {} ({})\n     Active: {} ({}){pid}",
    state.id,
    state.description,
    state.load_state,
    state.unit_file_state,
    state.active_state,
    state.sub_state
  )
}

/// `systemd-run`'s words, the ones the set's calls use.
#[derive(Debug, Parser)]
#[command(name = "systemd-run", disable_version_flag = true)]
pub struct SystemdRun {
  #[arg(long)]
  pub user: bool,
  #[arg(long, short = 'u')]
  pub unit: Option<String>,
  #[arg(long)]
  pub slice: Option<String>,
  #[arg(long = "setenv", short = 'E')]
  pub setenv: Vec<String>,
  #[arg(long)]
  pub working_directory: Option<PathBuf>,
  #[arg(long, short = 'd')]
  pub same_dir: bool,
  #[arg(long, short = 'G')]
  pub collect: bool,
  #[arg(long, short = 'q')]
  pub quiet: bool,
  #[arg(long)]
  pub wait: bool,
  #[arg(long, short = 'P')]
  pub pipe: bool,
  #[arg(long)]
  pub scope: bool,
  #[arg(long = "property", short = 'p')]
  pub property: Vec<String>,
  #[arg(long)]
  pub description: Option<String>,
  #[arg(long)]
  pub no_block: bool,
  #[arg(long)]
  pub service_type: Option<String>,
  #[arg(long)]
  pub nice: Option<String>,
  #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
  pub command: Vec<String>,
}

/// The variables a `systemd-run` call hands its command: each `--setenv`, a name with no value taking the
/// caller's own.
pub fn setenv(words: &SystemdRun) -> Vec<(String, String)> {
  words
    .setenv
    .iter()
    .filter_map(|given| match given.split_once('=') {
      Some((key, value)) => Some((key.to_owned(), value.to_owned())),
      None => std::env::var(given).ok().map(|value| (given.clone(), value)),
    })
    .collect()
}

fn systemd_run(words: SystemdRun, socket: &Path) -> i32 {
  let env = setenv(&words);
  let cwd = words
    .working_directory
    .clone()
    .or_else(|| words.same_dir.then(|| std::env::current_dir().ok()).flatten());

  // A scope, or a run whose output is piped back, is the caller's own child: the command runs here and its
  // code is the stand-in's.
  if words.scope || words.pipe {
    let mut command = std::process::Command::new(&words.command[0]);

    command.args(&words.command[1..]).envs(env);

    if let Some(dir) = cwd {
      command.current_dir(dir);
    }

    return match command.status() {
      Ok(status) => status.code().unwrap_or(1),
      Err(why) => {
        eprintln!("{}: {why}", words.command[0]);
        1
      }
    };
  }

  let request = Request::Run {
    unit: words.unit.clone(),
    words: words.command.clone(),
    cwd,
    env,
    collect: words.collect,
    wait: words.wait,
  };

  match ask_blocking(socket, &request) {
    Ok(answer) => {
      if !answer.out.is_empty() && !words.quiet {
        eprintln!("{}", answer.out);
      }

      if !answer.err.is_empty() {
        eprintln!("{}", answer.err);
      }

      answer.code
    }
    Err(why) => {
      eprintln!("{why}");
      1
    }
  }
}

/// `journalctl`'s words, the ones the set's calls use.
#[derive(Debug, Parser)]
#[command(name = "journalctl", disable_version_flag = true)]
pub struct Journalctl {
  #[arg(long)]
  pub user: bool,
  #[arg(long = "unit", short = 'u')]
  pub units: Vec<String>,
  #[arg(long, short = 'n', num_args = 0..=1, default_missing_value = "10")]
  pub lines: Option<String>,
  #[arg(long, short = 'f')]
  pub follow: bool,
  #[arg(long)]
  pub no_pager: bool,
  #[arg(long, short = 'o')]
  pub output: Option<String>,
  #[arg(long, short = 'q')]
  pub quiet: bool,
  #[arg(long, short = 'e')]
  pub pager_end: bool,
  #[arg(long, short = 'b', num_args = 0..=1, default_missing_value = "0")]
  pub boot: Option<String>,
  #[arg(long, short = 'x')]
  pub catalog: bool,
  #[arg(long, short = 'S')]
  pub since: Option<String>,
}

/// The last lines of the named units' logs, merged in the order they were written, at most `count` of them where
/// a count is named; each shown as `<time> <unit>: <line>`, or as the line alone with `cat`.
pub fn journal(logs: &Path, units: &[String], count: Option<usize>, bare: bool) -> Vec<String> {
  let mut lines: Vec<(String, String, String)> = Vec::new();

  for unit in units {
    for file in [logs.join(format!("{unit}.log.1")), logs.join(format!("{unit}.log"))] {
      let Ok(text) = std::fs::read_to_string(&file) else { continue };

      for line in text.lines() {
        let (stamp, said) = line.split_once(' ').unwrap_or((line, ""));

        lines.push((stamp.to_owned(), unit.clone(), said.to_owned()));
      }
    }
  }

  lines.sort_by(|left, right| left.0.cmp(&right.0));

  let skip = count.map_or(0, |count| lines.len().saturating_sub(count));

  lines
    .into_iter()
    .skip(skip)
    .map(|(stamp, unit, said)| if bare { said } else { format!("{stamp} {unit}: {said}") })
    .collect()
}

fn journalctl(words: &Journalctl, socket: &Path) -> i32 {
  let logs = socket.parent().map(|folder| folder.join(&SUPERVISE.places.logs)).unwrap_or_default();
  let units: Vec<String> = words.units.iter().map(|unit| unit_name(unit)).collect();
  let count = match words.lines.as_deref() {
    Some("all") => None,
    Some(number) => number.parse().ok().or(Some(SUPERVISE.numbers.journal_lines)),
    None if words.follow => Some(SUPERVISE.numbers.journal_lines),
    None => None,
  };
  let bare = words.output.as_deref() == Some("cat");
  let mut out = std::io::stdout().lock();

  for line in journal(&logs, &units, count, bare) {
    let _ = writeln!(out, "{line}");
  }

  if !words.follow {
    return 0;
  }

  // Following: each unit's log read on from where it stood, every pace.
  let mut at: Vec<(PathBuf, String, u64)> = units
    .iter()
    .map(|unit| {
      let file = logs.join(format!("{unit}.log"));
      let size = std::fs::metadata(&file).map(|meta| meta.len()).unwrap_or(0);

      (file, unit.clone(), size)
    })
    .collect();

  loop {
    for (file, unit, offset) in &mut at {
      let Ok(mut opened) = std::fs::File::open(&*file) else { continue };
      let size = opened.metadata().map(|meta| meta.len()).unwrap_or(0);

      // A log moved aside starts again from its beginning.
      if size < *offset {
        *offset = 0;
      }

      if opened.seek(std::io::SeekFrom::Start(*offset)).is_err() {
        continue;
      }

      for line in std::io::BufReader::new(opened).lines().map_while(Result::ok) {
        *offset += line.len() as u64 + 1;

        let (stamp, said) = line.split_once(' ').unwrap_or((&line, ""));
        let _ = if bare { writeln!(out, "{said}") } else { writeln!(out, "{stamp} {unit}: {said}") };
      }
    }

    let _ = out.flush();
    std::thread::sleep(Duration::from_millis(SUPERVISE.numbers.follow_poll_ms));
  }
}
