//! The testbed's `run` and `status` verbs (docs/boundary.md, 「The verbs」): the words a caller hands them,
//! read the same way on every machine the binary runs on.
//!
//! ```text
//! [--narrow] [--adapter <name>] [--guard <seconds>] [--wait <seconds>] -- <command…>
//! status [--json]
//! ```
//!
//! A run waits its turn behind every other heavy command of its lane on this machine, runs the command at
//! the lane's width, keeps its whole output in a log of its own and ends on one result line, with what
//! failed under it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::close_line::{CloseEntry, read_close_line};
use crate::languages::{LANGUAGES, language_for};
use crate::policy::fill_marks;
use crate::policy::lanes::LANES;
use crate::queue::now_ms;
use crate::run::{RunAsk, run_in_testbed};
use crate::turn::{
  Lane, TestbedQueue, TurnAsk, TurnEntry, lane_width, leading_number, lock_path, machine_threads,
  process_env, read_queue,
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

/// What the words ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum Asked {
  Usage,
  Status { json: bool },
  Run(RunWords),
}

/// Words the verb refuses: what it says, and whether the usage's hint follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
  pub said: String,
  pub hint: bool,
}

// How many values each of the verb's own flags takes.
const FLAGS: [(&str, usize); 7] = [
  ("--narrow", 0),
  ("--adapter", 1),
  ("--guard", 1),
  ("--wait", 1),
  ("--json", 0),
  ("--help", 0),
  ("-h", 0),
];

fn arity(word: &str) -> Option<usize> {
  FLAGS.iter().find(|(flag, _)| *flag == word).map(|(_, arity)| *arity)
}

// Where the command begins: at `--` where one stands, and otherwise at the first word that is neither one
// of the verb's flags, a flag's value, nor `status`.
fn command_start(args: &[String]) -> usize {
  let mut at = 0;

  while at < args.len() {
    let word = args[at].as_str();

    if word == "--" {
      return at;
    }

    if let Some(arity) = arity(word) {
      at += arity + 1;
      continue;
    }

    if word != "status" {
      return at;
    }

    at += 1;
  }

  args.len()
}

fn refused(said: String, hint: bool) -> Refused {
  Refused { said, hint }
}

// A positive number of seconds: its whole number at the head.
fn seconds(flag: &str, value: Option<&String>) -> Result<Option<f64>, Refused> {
  let Some(value) = value else { return Ok(None) };

  match leading_number(value) {
    Some(read) if read > 0 => Ok(Some(read as f64)),
    _ => Err(refused(fill_marks(&LANES.verb.seconds, &[("flag", flag)]), false)),
  }
}

/// The verb's words read: its own before the command, the command after them.
pub fn read_words(args: &[String]) -> Result<Asked, Refused> {
  let said = &LANES.verb;
  let start = command_start(args);
  let own = &args[..start];
  let command = &args[(if args.get(start).is_some_and(|word| word == "--") { start + 1 } else { start })..];

  if own.first().is_some_and(|word| word == "--help" || word == "-h") {
    return Ok(Asked::Usage);
  }

  let mut given: BTreeMap<&str, Option<&String>> = BTreeMap::new();
  let mut loose: Vec<&String> = Vec::new();
  let mut at = 0;

  // The verb's own words are its flags, each with its value, and `status`: the command begins at any other.
  while at < own.len() {
    let word = &own[at];

    match arity(word) {
      None => loose.push(word),
      Some(0) => {
        given.insert(word, None);
      }
      Some(_) => {
        let value = own
          .get(at + 1)
          .ok_or_else(|| refused(fill_marks(&said.takes_value, &[("flag", word)]), true))?;

        given.insert(word, Some(value));
        at += 1;
      }
    }

    at += 1;
  }

  if loose.first().is_some_and(|word| *word == "status") {
    return Ok(Asked::Status {
      json: given.contains_key("--json"),
    });
  }

  if let Some(first) = command.first().filter(|first| first.starts_with('-')) {
    return Err(refused(fill_marks(&said.flag_as_command, &[("word", first)]), true));
  }

  if command.is_empty() || !loose.is_empty() {
    return Err(refused(said.no_command.clone(), true));
  }

  Ok(Asked::Run(RunWords {
    command: command.to_vec(),
    narrow: given.contains_key("--narrow"),
    adapter: given.get("--adapter").copied().flatten().cloned(),
    guard_seconds: seconds("--guard", given.get("--guard").copied().flatten())?,
    wait_seconds: seconds("--wait", given.get("--wait").copied().flatten())?,
  }))
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

// Every lane's line, by lane.
#[derive(Serialize)]
struct Lanes {
  wide: TestbedQueue,
  narrow: TestbedQueue,
}

/// Every lane's line as JSON, the wide lane's first.
pub fn status_json() -> String {
  let env = process_env();
  let lanes = Lanes {
    wide: read_queue(&lock_path(&env, Lane::Wide)),
    narrow: read_queue(&lock_path(&env, Lane::Narrow)),
  };

  serde_json::to_string_pretty(&lanes).unwrap_or_default()
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

/// Where the verb stands: the name it was run by, the data root its logs go under, the close line its
/// status prints, and where a run goes before it takes its turn here — another machine's answer, its exit
/// code, or none for this one.
pub struct VerbGround<'a> {
  pub verb: &'a str,
  pub data_root: PathBuf,
  pub close_line: Option<PathBuf>,
  pub place: &'a mut dyn FnMut(&[String]) -> Option<u8>,
}

/// The verb run over its words; the code it ends with.
pub fn main(args: &[String], ground: &mut VerbGround<'_>) -> u8 {
  let said = &LANES.verb;
  let hint = || fill_marks(&said.hint, &[("verb", ground.verb)]);
  let words = match read_words(args) {
    Ok(Asked::Usage) => {
      let numbers = &LANES.numbers;

      println!(
        "{}",
        fill_marks(
          &said.usage,
          &[
            ("verb", ground.verb),
            ("guard", &numbers.guard_seconds.to_string()),
            ("job_wait", &numbers.job_wait_seconds.to_string()),
            ("unit_wait", &numbers.unit_wait_seconds.to_string()),
            ("kept", &numbers.kept_place_seconds.to_string()),
          ],
        )
      );

      return 0;
    }
    Ok(Asked::Status { json }) => {
      if json {
        println!("{}", status_json());
      } else {
        for line in status_lines(ground.close_line.as_deref()) {
          println!("{line}");
        }
      }

      return 0;
    }
    Ok(Asked::Run(words)) => words,
    Err(refused) => {
      eprintln!("{}", refused.said);

      if refused.hint {
        eprintln!("{}", hint());
      }

      return REFUSED_EXIT_CODE;
    }
  };
  let language = match language_for(&LANGUAGES, &words.command, words.adapter.as_deref()) {
    Ok(language) => language,
    Err(names) => {
      let name = words.adapter.as_deref().unwrap_or_default();

      eprintln!("{}", fill_marks(&said.no_language, &[("name", name), ("names", &names.join(", "))]));

      return REFUSED_EXIT_CODE;
    }
  };

  if let Some(code) = (ground.place)(&words.command) {
    return code;
  }

  let cwd = std::env::current_dir().unwrap_or_default();
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
      lane: if words.narrow { Lane::Narrow } else { Lane::Wide },
      wait_seconds: words.wait_seconds,
      say: &say,
      path: None,
      env: process_env(),
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
