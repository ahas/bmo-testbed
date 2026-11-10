//! The `bmo-testbed` binary (docs/boundary.md, 「The binary」): each verb's words read, and handed to the
//! crates that do the work; the classification of a shell line; the placement endpoint asked over HTTP; the
//! record its verbs keep; the HTTP service and the harness plugin it serves; and a checkout of the testbed
//! running its own binary.

pub mod backup;
pub mod cargo;
pub mod check;
pub mod classify;
pub mod dependents;
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
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::close_line::close_line_path;
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::languages::LANGUAGES;
use bmo_testbed_core::placement::{Local, PLACED_VARIABLE, Placement};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::turn::{Env, NOT_RUN_EXIT_CODE, process_env};
use bmo_testbed_core::verb::{self, REFUSED_EXIT_CODE, RunWords, VerbGround, data_root};
use bmo_testbed_suite::dependents::CHECKOUTS_VARIABLE;

use crate::classify::{TakesTurn, answer, heavy_command};
use crate::placement::{HttpPlacement, MachinePlacement};
use crate::recorded::{Recorded, command_run};
use crate::words::{
  CargoArgs, CargoWord, ClassifyArgs, CountArgs, DeployArgs, DeployWord, LogArgs, PlacedArgs, RunArgs,
  TestArgs, TestWord, Verb,
};

/// The placement a run started in a directory asks: the endpoint the environment names, the placed machine the
/// settings name, the endpoint the declaration names, or the local one.
pub fn placement_of(env: &Env, dir: &Path) -> Box<dyn Placement> {
  if let Some(endpoint) = env.get(placement::PLACEMENT_VARIABLE).filter(|named| !named.is_empty()) {
    return Box::new(HttpPlacement {
      endpoint: endpoint.clone(),
    });
  }

  if let Some(machine) = MachinePlacement::named(&data_root(), &home_of(env)) {
    return Box::new(machine);
  }

  match HttpPlacement::named(env, dir) {
    Some(http) => Box::new(http),
    None => Box::new(Local),
  }
}

// A placed run's own registry of checkouts, under the data root's `testbed/`, named by this process; none where
// the run was handed no checkout. The line saying why where it could not be written.
fn placed_registry(data_root: &Path, checkouts: &[String]) -> Result<Option<PathBuf>, String> {
  if checkouts.is_empty() {
    return Ok(None);
  }

  let dir = data_root.join("testbed");
  let file = dir.join(format!("placed-checkouts-{}.toml", std::process::id()));
  let mut document = toml_edit::DocumentMut::new();
  let mut list = toml_edit::Array::new();

  for checkout in checkouts {
    list.push(checkout.as_str());
  }

  document["checkouts"] = toml_edit::value(list);
  std::fs::create_dir_all(&dir)
    .and_then(|()| std::fs::write(&file, document.to_string()))
    .map_err(|why| {
      fill_marks(
        &LANES.placed.no_registry,
        &[("file", &file.to_string_lossy()), ("why", &why.to_string())],
      )
    })?;

  Ok(Some(file))
}

// A file taken away as the run ends.
struct TakenAway(PathBuf);

impl Drop for TakenAway {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
  }
}

/// The account's home the environment names.
pub fn home_of(env: &Env) -> PathBuf {
  env.get("HOME").map(PathBuf::from).unwrap_or_default()
}

/// The far side of a placed run (docs/boundary.md, 「`placed`」): the declared setup where asked, then the
/// words run as a command of their own, marked so the run is never placed again; its code.
fn placed(args: PlacedArgs) -> u8 {
  let said = &LANES.placed;
  let cwd = std::env::current_dir().unwrap_or_default();
  // The checkouts this run knows: a registry of its own, written for it and taken away as it ends.
  let registry = placed_registry(&data_root(), &args.checkouts);
  let registry = match &registry {
    Ok(Some(file)) => Some(file.clone()),
    Ok(None) => None,
    Err(why) => {
      eprintln!("{why}");
      return NOT_RUN_EXIT_CODE;
    }
  };
  let _taken_away = registry.clone().map(TakenAway);
  let handed = |command: &mut Command| {
    command.env(PLACED_VARIABLE, "1");

    if let Some(file) = &registry {
      command.env(CHECKOUTS_VARIABLE, file);
    }
  };

  if args.setup
    && let Ok(repo) = Repository::read(&cwd)
  {
    for step in &repo.declaration.test.setup {
      let mut command = Command::new("sh");

      command.args(["-c", step]).current_dir(&repo.root).stdin(Stdio::null());
      handed(&mut command);

      let ran = command.status();
      let code = ran.ok().and_then(|status| status.code()).unwrap_or(1);

      if code != 0 {
        eprintln!("{}", fill_marks(&said.setup_failed, &[("command", step), ("code", &code.to_string())]));

        return NOT_RUN_EXIT_CODE;
      }
    }
  }

  let Some((program, rest)) = args.command.split_first() else {
    return REFUSED_EXIT_CODE;
  };
  let mut command = Command::new(program);

  command.args(rest).current_dir(&cwd).stdin(Stdio::null());
  handed(&mut command);

  match command.status() {
    Ok(status) => status.code().and_then(|code| u8::try_from(code).ok()).unwrap_or(1),
    Err(why) => {
      eprintln!("{why}");
      NOT_RUN_EXIT_CODE
    }
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
    Verb::Placed(args) => placed(args),
    Verb::Status(args) => status(args.json),
    Verb::Classify(args) => classify(args),
    Verb::Suite(args) => suite::suite(args),
    Verb::Dependents(args) => dependents::dependents(&args),
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
