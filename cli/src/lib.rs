//! The `bmo-testbed` binary (docs/boundary.md, 「The binary」): each verb's words read, and handed to the
//! crates that do the work; the classification of a shell line; and the placement endpoint asked over HTTP.

pub mod classify;
pub mod placement;
pub mod shell;
pub mod words;

use std::ffi::OsString;
use std::path::Path;

use bmo_testbed_core::close_line::close_line_path;
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::languages::LANGUAGES;
use bmo_testbed_core::placement::{Local, PlaceAsk, Placement, runs_here};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::turn::{Env, Lane, process_env};
use bmo_testbed_core::verb::{self, REFUSED_EXIT_CODE, RunWords, VerbGround, data_root};
use bmo_testbed_suite::run::{Refused, read_ask};

use crate::classify::{TakesTurn, answer, heavy_command};
use crate::placement::HttpPlacement;
use crate::words::{BINARY, ClassifyArgs, RunArgs, SuiteArgs, Verb};

// The placement a run started in a directory asks: the endpoint named for it, or the local one.
fn placement_of(env: &Env, dir: &Path) -> Box<dyn Placement> {
  match HttpPlacement::named(env, dir) {
    Some(http) => Box::new(http),
    None => Box::new(Local),
  }
}

fn run(args: RunArgs) -> u8 {
  let env = process_env();
  let cwd = std::env::current_dir().unwrap_or_default();
  let place = placement_of(&env, &cwd);

  verb::run(
    &RunWords {
      command: args.command,
      narrow: args.narrow,
      adapter: args.adapter,
      guard_seconds: args.guard.map(|seconds| seconds as f64),
      wait_seconds: args.wait.map(|seconds| seconds as f64),
    },
    &VerbGround {
      data_root: data_root(),
      env,
      place: place.as_ref(),
    },
  )
}

fn status(json: bool) -> u8 {
  let cwd = std::env::current_dir().unwrap_or_default();
  let close_line = close_line_path(&data_root(), &cwd);

  if json {
    println!("{}", verb::status_json(close_line.as_deref()));
  } else {
    for line in verb::status_lines(close_line.as_deref()) {
      println!("{line}");
    }
  }

  0
}

fn classify(args: ClassifyArgs) -> u8 {
  let mut takes_turn = Vec::new();

  for given in &args.takes_turn {
    match TakesTurn::read(given) {
      Some(one) => takes_turn.push(one),
      None => {
        eprintln!("{}", fill_marks(&VERBS.classify.unread, &[("command", given)]));
        return REFUSED_EXIT_CODE;
      }
    }
  }

  let heavy = heavy_command(&args.line, &LANGUAGES, &takes_turn);
  let (said, code) = answer(heavy.as_ref(), args.json);

  println!("{said}");
  code
}

fn suite(args: SuiteArgs) -> u8 {
  let cwd = std::env::current_dir().unwrap_or_default();
  let repo = match Repository::read(&cwd) {
    Ok(repo) => repo,
    Err(refusal) => {
      eprintln!("{refusal}");
      return REFUSED_EXIT_CODE;
    }
  };
  let ask = match read_ask(&args.words, &repo.declaration.test) {
    Ok(ask) => ask,
    Err(Refused(said)) => {
      eprintln!("{said}");
      return REFUSED_EXIT_CODE;
    }
  };
  let env = process_env();

  // A placed suite runs on the other machine as this same verb of the testbed there.
  if !runs_here(&env) {
    let command: Vec<String> =
      [BINARY.to_owned(), "suite".to_owned()].into_iter().chain(args.words).collect();
    let placed = placement_of(&env, &repo.root).place(&PlaceAsk {
      command: &command,
      lane: Lane::Wide,
      wait_seconds: PlaceAsk::wait_of(ask.turn_wait, &env),
      guard_seconds: ask.whole_guard,
      cwd,
      env: &env,
    });

    if let Some(code) = placed {
      return code;
    }
  }

  bmo_testbed_suite::run::run(&repo, &ask)
}

/// The binary over its words; the code it ends with.
pub fn main(args: impl IntoIterator<Item = OsString>) -> u8 {
  let words = match words::read(args) {
    Ok(words) => words,
    Err(error) => {
      let _ = error.print();
      return u8::try_from(error.exit_code()).unwrap_or(REFUSED_EXIT_CODE);
    }
  };

  match words.verb {
    Verb::Run(args) => run(args),
    Verb::Status(args) => status(args.json),
    Verb::Classify(args) => classify(args),
    Verb::Suite(args) => suite(args),
  }
}
