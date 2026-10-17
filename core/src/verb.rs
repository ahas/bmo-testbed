//! The testbed's `run` and `status` verbs (docs/boundary.md, 「The verbs」) over the words the binary read
//! for them. A run is placed first where its placement sends it elsewhere; here it waits its turn behind
//! every other heavy command of its lane on this machine, runs the command at the lane's width, keeps its
//! whole output in a log of its own and ends on one result line, with what failed under it.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::close_line::{CloseEntry, CloseLine, read_close_line};
use crate::languages::{LANGUAGES, language_for};
use crate::placement::{PlaceAsk, Placement, runs_here};
use crate::policy::fill_marks;
use crate::policy::lanes::LANES;
use crate::queue::now_ms;
use crate::run::{RunAsk, run_in_testbed};
use crate::turn::{
  Env, Lane, TestbedQueue, TurnAsk, TurnEntry, lane_width, lock_path, machine_threads, process_env,
  read_queue,
};

/// The exit code of words the verb does not read.
pub const REFUSED_EXIT_CODE: u8 = 2;

/// A run's own words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunWords {
  pub command: Vec<String>,
  pub narrow: bool,
  pub adapter: Option<String>,
  pub guard_seconds: Option<f64>,
  pub wait_seconds: Option<f64>,
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
    if entry.kept { said.kept.clone() } else { String::new() },
  ]
  .concat()
}

fn close_entry_line(entry: &CloseEntry, at: i64, now: i64) -> String {
  format!("{}{}", entry.words(), age(at, now))
}

/// Each lane's line, and the close line where the caller names one: who holds it, since when, and who
/// waits behind it in order.
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
    lines.push(holding(
      queue
        .holder
        .as_ref()
        .map_or_else(|| said.nobody.clone(), |holder| entry_line(holder, now)),
    ));
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

// Every lane's line by lane, and the repository's close line.
#[derive(Serialize)]
struct Status {
  wide: TestbedQueue,
  narrow: TestbedQueue,
  close_line: Option<CloseLine>,
}

/// Every lane's line as JSON, the wide lane's first, then the close line where the caller names one.
pub fn status_json(close_line: Option<&Path>) -> String {
  let env = process_env();
  let status = Status {
    wide: read_queue(&lock_path(&env, Lane::Wide)),
    narrow: read_queue(&lock_path(&env, Lane::Narrow)),
    close_line: close_line.map(read_close_line),
  };

  serde_json::to_string_pretty(&status).unwrap_or_default()
}

/// The data root a run's log goes under: the one `BMO_DATA_DIR` names, or this machine's.
pub fn data_root() -> PathBuf {
  match std::env::var_os("BMO_DATA_DIR") {
    Some(named) if !named.is_empty() => PathBuf::from(named),
    _ => std::env::var_os("HOME")
      .map_or_else(|| PathBuf::from("/"), PathBuf::from)
      .join(".local/share/bmo/data"),
  }
}

/// Where a run stands: the data root its log goes under, the environment it was started in, and where it
/// goes before it takes its turn here.
pub struct VerbGround<'a> {
  pub data_root: PathBuf,
  pub env: Env,
  pub place: &'a dyn Placement,
}

/// A run of the command its words name; the code it ends with.
pub fn run(words: &RunWords, ground: &VerbGround<'_>) -> u8 {
  let said = &LANES.verb;
  let language = match language_for(&LANGUAGES, &words.command, words.adapter.as_deref()) {
    Ok(language) => language,
    Err(names) => {
      let name = words.adapter.as_deref().unwrap_or_default();

      eprintln!("{}", fill_marks(&said.no_language, &[("name", name), ("names", &names.join(", "))]));

      return REFUSED_EXIT_CODE;
    }
  };
  let env = ground.env.clone();
  let cwd = std::env::current_dir().unwrap_or_default();

  if !runs_here(&env) {
    let ask = PlaceAsk {
      command: &words.command,
      lane: words.lane(),
      wait_seconds: PlaceAsk::wait_of(words.wait_seconds, &env),
      guard_seconds: PlaceAsk::guard_of(words.guard_seconds),
      cwd: cwd.clone(),
      env: &env,
    };

    if let Some(code) = ground.place.place(&ask) {
      return code;
    }
  }

  let say = |line: &str| eprintln!("{line}");
  let ask = RunAsk {
    command: words.command.clone(),
    language,
    languages: &LANGUAGES,
    log_dir: ground.data_root.join("testbed"),
    guard_seconds: words.guard_seconds,
    cwd: cwd.clone(),
    turn: TurnAsk {
      command: words.command.join(" "),
      lane: words.lane(),
      wait_seconds: words.wait_seconds,
      say: &say,
      path: None,
      env,
      checkout: cwd.to_string_lossy().into_owned(),
      poll: None,
      notice_seconds: None,
    },
  };
  let result = match run_in_testbed(&ask) {
    Ok(result) => result,
    Err(why) => {
      eprintln!("{why}");

      return 1;
    }
  };
  let run = &LANES.run;
  let most = LANES.numbers.failure_lines;

  println!("{}", result.line());

  if !result.failures.is_empty() {
    println!(
      "{}",
      if result.failures.len() >= most {
        fill_marks(&run.failures_first, &[("count", &most.to_string())])
      } else {
        run.failures.clone()
      }
    );

    for line in &result.failures {
      println!("{}", fill_marks(&run.failure, &[("line", line)]));
    }
  }

  result.exit_code()
}
