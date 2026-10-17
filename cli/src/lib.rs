//! The `bmo-testbed` binary (docs/boundary.md, 「The binary」): each verb's words read, and handed to the
//! crates that do the work; the classification of a shell line; the placement endpoint asked over HTTP; and a
//! checkout of the testbed running its own binary.

pub mod cargo;
pub mod check;
pub mod classify;
pub mod imports;
pub mod own;
pub mod placement;
pub mod said;
pub mod shell;
pub mod suite;
pub mod words;

use std::ffi::OsString;
use std::path::Path;

use bmo_testbed_core::close_line::close_line_path;
use bmo_testbed_core::languages::LANGUAGES;
use bmo_testbed_core::placement::{Local, Placement};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::turn::{Env, process_env};
use bmo_testbed_core::verb::{self, REFUSED_EXIT_CODE, RunWords, VerbGround, data_root};

use crate::classify::{TakesTurn, answer, heavy_command};
use crate::placement::HttpPlacement;
use crate::words::{ClassifyArgs, RunArgs, Verb};

/// The placement a run started in a directory asks: the endpoint named for it, or the local one.
pub fn placement_of(env: &Env, dir: &Path) -> Box<dyn Placement> {
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
    Verb::Suite(args) => suite::suite(args),
    Verb::Count(args) => suite::count(&args, &std::env::current_dir().unwrap_or_default()).print(),
    Verb::Log(args) => suite::log(&args, &own::called_from()).print(),
    Verb::Template(args) => suite::template(&args),
    Verb::Check(args) => check::check(args),
    Verb::Imports(args) => match suite::repository() {
      Ok(repo) => imports::imports_in(&args.files, args.fix, &repo, &own::called_from()).print(),
      Err(code) => code,
    },
    Verb::Cargo(args) => cargo::cargo(&args.words),
    Verb::Prune(args) => cargo::prune(&args),
  }
}
