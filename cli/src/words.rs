//! The binary's words (docs/boundary.md, 「The binary」), read by clap into each verb's flags and arguments.
//! The usage each `--help` prints is the policy's, the binary's numbers filled in; words that do not read
//! end with clap's own line naming the one thing wrong and pointing at `--help`, and exit code 2.

use std::ffi::OsString;
use std::path::PathBuf;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_suite::template::is_database_name;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};

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
  Count(CountArgs),
  Log(LogArgs),
  Template(TemplateArgs),
  Check(CheckArgs),
  Imports(ImportsArgs),
  Cargo(CargoArgs),
  Prune(PruneArgs),
  Full(FullArgs),
  Deploy(DeployArgs),
  Backups,
}

// A word that names something: not empty, and not a flag.
fn named(word: &str) -> Result<String, String> {
  if word.trim().is_empty() || word.starts_with('-') {
    Err(VERBS.words.not_named.clone())
  } else {
    Ok(word.to_owned())
  }
}

// A time of day, as a clock shows it.
fn time_of_day(word: &str) -> Result<chrono::NaiveTime, String> {
  chrono::NaiveTime::parse_from_str(word, "%H:%M").map_err(|_| VERBS.words.not_a_time.clone())
}

#[derive(Debug, Args)]
pub struct FullArgs {
  #[arg(long, value_parser = time_of_day)]
  pub due: Option<chrono::NaiveTime>,
}

#[derive(Debug, Args)]
pub struct DeployArgs {
  #[arg(long, value_parser = named)]
  pub commit: Option<String>,
  #[arg(long)]
  pub dry_run: bool,
  #[arg(long)]
  pub no_check: bool,
  #[arg(long, value_parser = named)]
  pub before_start: Option<String>,
}

// A positive whole number of seconds.
fn seconds() -> clap::builder::RangedU64ValueParser<u64> {
  clap::value_parser!(u64).range(1..)
}

#[derive(Debug, Args)]
pub struct RunArgs {
  #[arg(long)]
  pub narrow: bool,
  #[arg(long)]
  pub adapter: Option<String>,
  #[arg(long, value_parser = seconds())]
  pub guard: Option<u64>,
  #[arg(long, value_parser = seconds())]
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

/// The suite's own flags, then every other word as given, for the units and the runner.
#[derive(Clone, Debug, Args)]
pub struct SuiteArgs {
  #[arg(long)]
  pub since: Option<String>,
  #[arg(long)]
  pub again: bool,
  #[arg(long, value_parser = seconds())]
  pub guard: Option<u64>,
  #[arg(long, value_parser = seconds())]
  pub file_guard: Option<u64>,
  #[arg(long, value_parser = seconds())]
  pub turn_wait: Option<u64>,
  #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
  pub words: Vec<String>,
}

#[derive(Debug, Args)]
pub struct CountArgs {
  #[arg(long)]
  pub since: Option<String>,
}

#[derive(Debug, Args)]
pub struct LogArgs {
  pub log: PathBuf,
  #[arg(long)]
  pub json: bool,
}

/// What the template verb is asked to do.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum TemplateWord {
  Make,
  Drop,
  Sweep,
}

// A template's name the verb writes into its statements.
fn database_name(name: &str) -> Result<String, String> {
  if is_database_name(name) {
    Ok(name.to_owned())
  } else {
    Err(fill_marks(&SUITE.template.bad_name, &[("name", name)]))
  }
}

#[derive(Debug, Args)]
pub struct TemplateArgs {
  #[arg(value_enum)]
  pub verb: TemplateWord,
  #[arg(value_parser = database_name)]
  pub name: String,
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct CheckArgs {
  #[command(subcommand)]
  pub part: Option<CheckPart>,
  #[arg(long, conflicts_with = "files")]
  pub since: Option<String>,
  #[arg(long)]
  pub log: Option<PathBuf>,
  pub files: Vec<String>,
}

/// One of the quick checks, run alone.
#[derive(Debug, Subcommand)]
pub enum CheckPart {
  Quick,
  Wording,
  Layout,
  Dependencies,
  Korean {
    #[arg(long)]
    kinds: bool,
    #[arg(long, conflicts_with_all = ["kinds", "prefix"])]
    settle: bool,
    prefix: Option<String>,
  },
  Dashes {
    #[arg(required = true)]
    files: Vec<String>,
    #[arg(long)]
    write: bool,
  },
}

#[derive(Debug, Args)]
pub struct ImportsArgs {
  pub files: Vec<String>,
  #[arg(long)]
  pub fix: bool,
}

/// Cargo's own words, every one of them handed on.
#[derive(Debug, Args)]
pub struct CargoArgs {
  #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
  pub words: Vec<String>,
}

#[derive(Debug, Args)]
pub struct PruneArgs {
  #[arg(long)]
  pub dry_run: bool,
  #[arg(long, value_name = "days", conflicts_with = "all")]
  pub older_than: Option<f64>,
  #[arg(long, value_name = "hours", conflicts_with = "all")]
  pub incremental_older_than: Option<f64>,
  #[arg(long)]
  pub all: bool,
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
      ("days", &PRUNE.older_than_days.to_string()),
      ("hours", &PRUNE.incremental_hours.to_string()),
    ],
  )
}

/// The words read, or clap's error, which says the help or the fault and the code to end on.
pub fn read(args: impl IntoIterator<Item = OsString>) -> Result<Words, clap::Error> {
  let said = &VERBS.usage;
  let check = usage(&said.check);
  let parts = ["quick", "wording", "layout", "dependencies", "korean", "dashes"];
  let command = Words::command()
    .override_help(usage(&said.binary))
    .mut_subcommand("run", |verb| verb.override_help(usage(&said.run)))
    .mut_subcommand("status", |verb| verb.override_help(usage(&said.status)))
    .mut_subcommand("classify", |verb| verb.override_help(usage(&said.classify)))
    .mut_subcommand("suite", |verb| verb.override_help(usage(&said.suite)))
    .mut_subcommand("count", |verb| verb.override_help(usage(&said.count)))
    .mut_subcommand("log", |verb| verb.override_help(usage(&said.log)))
    .mut_subcommand("template", |verb| verb.override_help(usage(&said.template)))
    .mut_subcommand("check", |verb| {
      parts.iter().fold(verb.override_help(check.clone()), |verb, part| {
        verb.mut_subcommand(part, |part| part.override_help(check.clone()))
      })
    })
    .mut_subcommand("imports", |verb| verb.override_help(usage(&said.imports)))
    .mut_subcommand("cargo", |verb| verb.override_help(usage(&said.cargo)))
    .mut_subcommand("prune", |verb| verb.override_help(usage(&said.prune)))
    .mut_subcommand("full", |verb| verb.override_help(usage(&said.full)))
    .mut_subcommand("deploy", |verb| verb.override_help(usage(&said.deploy)))
    .mut_subcommand("backups", |verb| verb.override_help(usage(&said.backups)));
  let matches = command.try_get_matches_from(args)?;

  Words::from_arg_matches(&matches)
}
