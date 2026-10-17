//! The binary's words (docs/boundary.md, 「The binary」), read by clap into each verb's flags and arguments.
//! The usage each `--help` prints is the policy's, the binary's numbers filled in; words that do not read
//! end with clap's own line naming the one thing wrong and pointing at `--help`, and exit code 2.

use std::ffi::OsString;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::verbs::VERBS;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};

/// The binary's name, as its usage names it.
pub const BINARY: &str = "bmo-testbed";

#[derive(Debug, Parser)]
#[command(name = BINARY, disable_version_flag = true, subcommand_required = true, arg_required_else_help = true)]
pub struct Words {
  #[command(subcommand)]
  pub verb: Verb,
}

#[derive(Debug, Subcommand)]
pub enum Verb {
  Run(RunArgs),
  Status(StatusArgs),
  Classify(ClassifyArgs),
  Suite(SuiteArgs),
}

#[derive(Debug, Args)]
pub struct RunArgs {
  #[arg(long)]
  pub narrow: bool,
  #[arg(long)]
  pub adapter: Option<String>,
  #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
  pub guard: Option<u64>,
  #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
  pub wait: Option<u64>,
  /// The command, after `--`.
  #[arg(last = true, required = true, num_args = 1..)]
  pub command: Vec<String>,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
  #[arg(long)]
  pub json: bool,
}

#[derive(Debug, Args)]
pub struct ClassifyArgs {
  #[arg(long)]
  pub json: bool,
  #[arg(long = "takes-turn")]
  pub takes_turn: Vec<String>,
  /// The shell line, whole as one word.
  #[arg(allow_hyphen_values = true)]
  pub line: String,
}

/// The suite's words as given: the suite reads its own flags among them, and hands the rest to the runner.
#[derive(Debug, Args)]
pub struct SuiteArgs {
  #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
  pub words: Vec<String>,
}

// A usage of the policy with the binary's name and numbers filled in.
fn usage(text: &str) -> String {
  let numbers = &LANES.numbers;

  fill_marks(
    text.trim_start_matches('\n'),
    &[
      ("verb", BINARY),
      ("guard", &numbers.guard_seconds.to_string()),
      ("job_wait", &numbers.job_wait_seconds.to_string()),
      ("unit_wait", &numbers.unit_wait_seconds.to_string()),
      ("kept", &numbers.kept_place_seconds.to_string()),
    ],
  )
}

/// The words read, or clap's error, which says the help or the fault and the code to end on.
pub fn read(args: impl IntoIterator<Item = OsString>) -> Result<Words, clap::Error> {
  let said = &VERBS.usage;
  let command = Words::command()
    .override_help(usage(&said.binary))
    .mut_subcommand("run", |verb| verb.override_help(usage(&said.run)))
    .mut_subcommand("status", |verb| verb.override_help(usage(&said.status)))
    .mut_subcommand("classify", |verb| verb.override_help(usage(&said.classify)))
    .mut_subcommand("suite", |verb| verb.override_help(usage(&said.suite)));
  let matches = command.try_get_matches_from(args)?;

  Words::from_arg_matches(&matches)
}
