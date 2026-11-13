//! The testbed's `run` verb and the `status` verb's lines (docs/boundary.md, 「The verbs」) over the words
//! the binary read for them. A run is placed first where its placement sends it elsewhere; here it waits its turn behind
//! every other heavy command of its lane on this machine, runs the command at the lane's width, keeps its
//! whole output in a log of its own and ends on one result line, with what failed under it.

use std::path::{Path, PathBuf};

use crate::close_line::{CloseEntry, read_close_line};
use crate::languages::{LANGUAGES, language_for};
use crate::placement::{PlaceAsk, Placement, runs_here};
use crate::policy::fill_marks;
use crate::policy::lanes::LANES;
use crate::queue::now_ms;
use crate::run::{RunAsk, run_in_testbed};
use crate::silence::Silence;
use crate::turn::{
  Env, Lane, TurnAsk, TurnEntry, lane_width, lock_path, machine_threads, process_env, read_queue,
};
use crate::witness::RunWitness;

/// The exit code of words the verb does not read.
pub const REFUSED_EXIT_CODE: u8 = 2;

/// A run's own words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunWords {
  pub command: Vec<String>,
  pub narrow: bool,
  pub adapter: Option<String>,
  /// The lines go to standard error, so the binary's standard output carries the outcome alone.
  pub json: bool,
}

impl RunWords {
  pub fn lane(&self) -> Lane {
    if self.narrow { Lane::Narrow } else { Lane::Wide }
  }
}

// How long ago a moment was, as the status says it; nothing for a moment not known.
fn age(at: i64, now: i64) -> String {
  if at > 0 {
    fill_marks(&LANES.status.age, &[("seconds", &(((now - at) as f64) / 1000.0).round().to_string())])
  } else {
    String::new()
  }
}

fn entry_line(entry: &TurnEntry, now: i64) -> String {
  let said = &LANES.status;

  [
    entry.words(),
    age(entry.since.unwrap_or(entry.asked), now),
    if entry.front { said.front.clone() } else { String::new() },
    if entry.next { said.next.clone() } else { String::new() },
  ]
  .concat()
}

fn close_entry_line(entry: &CloseEntry, at: i64, now: i64) -> String {
  format!("{}{}", entry.words(), age(at, now))
}

/// Each lane's line, and the close line where the caller names one: who holds it, a line for each seat held,
/// since when, and who waits behind it in order.
pub fn status_lines(close_line: Option<&Path>) -> Vec<String> {
  let said = &LANES.status;
  let env = process_env();
  let now = now_ms();
  let holding = |who: String| fill_marks(&said.holding, &[("who", &who)]);
  let waiting =
    |at: usize, who: String| fill_marks(&said.waiting, &[("at", &(at + 1).to_string()), ("who", &who)]);
  let mut lines = Vec::new();

  for lane in Lane::ALL {
    let queue = read_queue(&lock_path(&env, lane));

    lines.push(fill_marks(
      &said.lane,
      &[
        ("lane", lane.words()),
        ("width", &lane_width(lane, &env, machine_threads()).to_string()),
      ],
    ));
    if queue.holders.is_empty() {
      lines.push(holding(said.nobody.clone()));
    }

    lines.extend(queue.holders.iter().map(|holder| holding(entry_line(holder, now))));
    lines.extend(queue.waiting.iter().enumerate().map(|(at, entry)| waiting(at, entry_line(entry, now))));
  }

  if let Some(path) = close_line {
    let line = read_close_line(path);

    lines.push(said.close_line.clone());
    lines.push(holding(line.holder.as_ref().map_or_else(
      || said.nobody.clone(),
      |holder| close_entry_line(holder, holder.since.unwrap_or(holder.asked), now),
    )));
    lines.extend(
      line
        .waiting
        .iter()
        .enumerate()
        .map(|(at, entry)| waiting(at, close_entry_line(entry, entry.asked, now))),
    );
  }

  lines
}

/// The data root a run's log goes under: the one `BMO_DATA_DIR` names, or this machine's.
pub fn data_root() -> PathBuf {
  data_root_named(std::env::var_os("BMO_DATA_DIR").as_deref(), std::env::var_os("HOME").as_deref())
}

/// The data root the environment `env` names: its `BMO_DATA_DIR`, or the one under its `HOME`.
pub fn data_root_of(env: &Env) -> PathBuf {
  let named = |key: &str| env.get(key).map(std::ffi::OsStr::new);

  data_root_named(named("BMO_DATA_DIR"), named("HOME"))
}

fn data_root_named(named: Option<&std::ffi::OsStr>, home: Option<&std::ffi::OsStr>) -> PathBuf {
  match named {
    Some(named) if !named.is_empty() => PathBuf::from(named),
    _ => PathBuf::from(home.unwrap_or("/".as_ref())).join(".local/share/bmo/data"),
  }
}

/// Where a run stands: the data root its log goes under, the environment it was started in, where it
/// goes before it takes its turn here, and whoever keeps the record of it.
pub struct VerbGround<'a> {
  pub data_root: PathBuf,
  pub env: Env,
  pub place: &'a dyn Placement,
  pub witness: &'a dyn RunWitness,
}

/// What a run came to: the code it ends with, and where it ran here, its log, its wait, its time and the
/// test run its witness kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunOutcome {
  pub code: u8,
  pub log: Option<PathBuf>,
  pub waited_seconds: f64,
  pub seconds: f64,
  pub test_run: Option<i64>,
}

impl RunOutcome {
  fn ended(code: u8) -> Self {
    Self {
      code,
      ..Self::default()
    }
  }
}

/// A run of the command its words name, and what it came to.
pub fn run(words: &RunWords, ground: &VerbGround<'_>) -> RunOutcome {
  let said = &LANES.verb;
  let language = match language_for(&LANGUAGES, &words.command, words.adapter.as_deref()) {
    Ok(language) => language,
    Err(names) => {
      let name = words.adapter.as_deref().unwrap_or_default();

      eprintln!("{}", fill_marks(&said.no_language, &[("name", name), ("names", &names.join(", "))]));

      return RunOutcome::ended(REFUSED_EXIT_CODE);
    }
  };
  let env = ground.env.clone();
  let cwd = std::env::current_dir().unwrap_or_default();

  if !runs_here(&env) {
    let ask = PlaceAsk {
      command: &words.command,
      adapter: words.adapter.as_deref(),
      lane: words.lane(),
      cwd: cwd.clone(),
      env: &env,
      to_free_seat: false,
    };

    if let Some(code) = ground.place.place(&ask) {
      return RunOutcome::ended(code);
    }
  }

  let say = |line: &str| eprintln!("{line}");
  let ask = RunAsk {
    command: words.command.clone(),
    language,
    languages: &LANGUAGES,
    log_dir: ground.data_root.join("testbed"),
    cwd: cwd.clone(),
    turn: TurnAsk {
      command: words.command.join(" "),
      lane: words.lane(),
      say: &say,
      path: None,
      env,
      checkout: cwd.to_string_lossy().into_owned(),
      poll: None,
      notice_seconds: None,
    },
    witness: ground.witness,
    silence: Silence::default(),
  };
  let result = match run_in_testbed(&ask) {
    Ok(result) => result,
    Err(why) => {
      eprintln!("{why}");

      return RunOutcome::ended(1);
    }
  };
  let print = |line: String| {
    if words.json {
      eprintln!("{line}");
    } else {
      println!("{line}");
    }
  };
  let run = &LANES.run;
  let most = LANES.numbers.failure_lines;

  print(result.line());

  if !result.failures.is_empty() {
    print(if result.failures.len() >= most {
      fill_marks(&run.failures_first, &[("count", &most.to_string())])
    } else {
      run.failures.clone()
    });

    for line in &result.failures {
      print(fill_marks(&run.failure, &[("line", line)]));
    }
  }

  RunOutcome {
    code: result.exit_code(),
    log: result.log.clone(),
    waited_seconds: result.waited_seconds,
    seconds: result.seconds,
    test_run: result.test_run,
  }
}
