//! The `bmo-testbed` binary (docs/boundary.md, 「The binary」): each verb's words read, and handed to the
//! crates that do the work; the classification of a shell line; the placement endpoint asked over HTTP; the
//! record its verbs keep; the HTTP service and the harness plugin it serves; and a checkout of the testbed
//! running its own binary.

pub mod backup;
pub mod cargo;
pub mod check;
pub mod classify;
pub mod deploy;
pub mod full;
pub mod imports;
pub mod nouns;
pub mod own;
pub mod placement;
pub mod recorded;
pub mod said;
pub mod serve;
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
use crate::recorded::{Recorded, command_run};
use crate::words::{
  CargoArgs, CargoWord, ClassifyArgs, CountArgs, DeployArgs, DeployWord, LogArgs, RunArgs, TestArgs,
  TestWord, Verb,
};

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
  let root = data_root();
  let witness = Recorded::commands(&root);
  let outcome = verb::run(
    &RunWords {
      command: args.command,
      narrow: args.narrow,
      adapter: args.adapter,
      guard_seconds: args.guard.map(|seconds| seconds as f64),
      wait_seconds: args.wait.map(|seconds| seconds as f64),
      json: args.json,
    },
    &VerbGround {
      data_root: root.clone(),
      env,
      place: place.as_ref(),
      witness: &witness,
    },
  );

  if args.json {
    println!(
      "{}",
      command_run(
        outcome.code,
        outcome.log.as_deref(),
        outcome.waited_seconds,
        outcome.seconds,
        outcome.test_run
      )
    );
  }

  outcome.code
}

fn status(json: bool) -> u8 {
  let cwd = std::env::current_dir().unwrap_or_default();
  let close_line = close_line_path(&data_root(), &cwd);

  if json {
    println!("{}", serve::status_json(&process_env(), close_line.as_deref()));
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

/// The OpenAPI document as `docs/openapi.json` keeps it, its commands the binary's verbs.
pub fn openapi_document() -> String {
  use clap::CommandFactory as _;

  let command = words::Words::command();
  // `nouns` is the protocol's word and `test` the platform's name for verbs already listed.
  let verbs: Vec<&str> = command
    .get_subcommands()
    .map(clap::Command::get_name)
    .filter(|name| !matches!(*name, "nouns" | "test"))
    .collect();

  bmo_testbed_plugin::openapi_json(&verbs)
}

fn run_count(args: CountArgs) -> u8 {
  suite::count(&args, &std::env::current_dir().unwrap_or_default()).print()
}

fn run_log(args: LogArgs) -> u8 {
  suite::log(&args, &own::called_from()).print()
}

fn run_test(args: TestArgs) -> u8 {
  match args.word {
    TestWord::Suite(args) => suite::suite(args),
    TestWord::Count(args) => run_count(args),
    TestWord::Log(args) => run_log(args),
    TestWord::Template(args) => suite::template(&args),
    TestWord::Full(args) => full::full(&args),
  }
}

fn run_cargo(args: CargoArgs) -> u8 {
  match args.word {
    Some(CargoWord::Prune(args)) => cargo::prune(&args),
    None => cargo::cargo(&args.words),
  }
}

fn run_deploy(args: DeployArgs) -> u8 {
  match args.word {
    Some(DeployWord::Commit(flags)) => deploy::deploy(&flags),
    Some(DeployWord::Backups) => deploy::backups(),
    None => deploy::deploy(&args.flags),
  }
}

fn print_nouns() -> u8 {
  for noun in nouns::lines() {
    println!("{noun}");
  }

  0
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
    Verb::Count(args) => run_count(args),
    Verb::Log(args) => run_log(args),
    Verb::Template(args) => suite::template(&args),
    Verb::Check(args) => check::check(args),
    Verb::Imports(args) => match suite::repository() {
      Ok(repo) => imports::imports_in(&args.files, args.fix, &repo, &own::called_from()).print(),
      Err(code) => code,
    },
    Verb::Cargo(args) => run_cargo(args),
    Verb::Prune(args) => cargo::prune(&args),
    Verb::Full(args) => full::full(&args),
    Verb::Deploy(args) => run_deploy(args),
    Verb::Backups => deploy::backups(),
    Verb::Backup(args) => backup::backup(&args.word),
    Verb::Serve(args) => serve::serve(&args, &data_root(), process_env()),
    Verb::Mcp(args) => serve::mcp(&args),
    Verb::Nouns => print_nouns(),
    Verb::Test(args) => run_test(args),
  }
}
