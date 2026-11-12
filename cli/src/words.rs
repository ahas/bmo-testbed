//! The binary's words (docs/boundary.md, 「The binary」), read by clap into each verb's flags and arguments.
//! The usage each `--help` prints is the policy's, the binary's numbers filled in; words that do not read
//! end with clap's own line naming the one thing wrong and pointing at `--help`, and exit code 2.

use std::ffi::OsString;
use std::path::PathBuf;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::service::SERVICE;
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
  Placed(PlacedArgs),
  Status(StatusArgs),
  Classify(ClassifyArgs),
  Suite(SuiteArgs),
  Dependents(DependentsArgs),
  Passed(PassedArgs),
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
  Backup(BackupArgs),
  Serve(ServeArgs),
  Mcp(McpArgs),
  /// The nouns `bmo` dispatches to this binary, one a line.
  Nouns,
  /// The platform's names for the suite verbs.
  Test(TestArgs),
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

/// `deploy`, or the platform's `deploy commit` and `deploy backups`.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true, disable_help_subcommand = true)]
pub struct DeployArgs {
  #[command(subcommand)]
  pub word: Option<DeployWord>,
  #[command(flatten)]
  pub flags: DeployFlags,
}

#[derive(Debug, Args)]
pub struct DeployFlags {
  #[arg(long, value_parser = named)]
  pub commit: Option<String>,
  /// Why this deploy is made, written into the record and the event; the commit's subject where none.
  #[arg(long, value_parser = named)]
  pub message: Option<String>,
  #[arg(long)]
  pub dry_run: bool,
  #[arg(long)]
  pub no_check: bool,
  #[arg(long, value_parser = named)]
  pub before_start: Option<String>,
  #[arg(long)]
  pub force_live: bool,
}

#[derive(Debug, Subcommand)]
pub enum DeployWord {
  /// The deployment itself, as `deploy` runs it.
  Commit(DeployFlags),
  /// The backups a deployment took, sent away and rotated.
  Backups,
}

/// `backup` and the word naming what it does (docs/backup.md).
#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct BackupArgs {
  #[command(subcommand)]
  pub word: BackupWord,
}

#[derive(Debug, Subcommand)]
pub enum BackupWord {
  /// The databases dumped and the data folders packed, sent and rotated.
  Snapshot,
  /// The media and the large files packed and sent.
  Media,
  /// The newest snapshot restored and counted.
  Drill,
  /// The rotation alone.
  Prune(BackupPruneArgs),
}

#[derive(Debug, Args)]
pub struct BackupPruneArgs {
  #[arg(long)]
  pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct PlacedArgs {
  /// The declared setup steps first.
  #[arg(long)]
  pub setup: bool,
  /// A checkout the run knows on this machine: the bench and the trees copied beside it.
  #[arg(long = "checkout", value_name = "PATH")]
  pub checkouts: Vec<String>,
  /// The command, after `--`.
  #[arg(last = true, required = true, num_args = 1..)]
  pub command: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RunArgs {
  #[arg(long)]
  pub narrow: bool,
  #[arg(long)]
  pub adapter: Option<String>,
  /// Read and given no effect: a run waits for its turn and runs until it ends.
  #[arg(long, hide = true, value_name = "SECONDS")]
  pub guard: Option<u64>,
  /// Read and given no effect.
  #[arg(long, hide = true, value_name = "SECONDS")]
  pub wait: Option<u64>,
  #[arg(long)]
  pub json: bool,
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
  /// Read and given no effect: every unit is started and runs until it ends, after the turn has come.
  #[arg(long, hide = true, value_name = "SECONDS")]
  pub guard: Option<u64>,
  /// Read and given no effect.
  #[arg(long, hide = true, value_name = "SECONDS")]
  pub file_guard: Option<u64>,
  /// Read and given no effect.
  #[arg(long, hide = true, value_name = "SECONDS")]
  pub turn_wait: Option<u64>,
  #[arg(long)]
  pub json: bool,
  #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
  pub words: Vec<String>,
}

/// `dependents`: the checkouts that name this repository among their upstreams, and each one's suite.
#[derive(Debug, Args)]
pub struct DependentsArgs {
  #[arg(long)]
  pub run: bool,
}

/// `passed`: the passed run of the whole suite on a commit that this machine's record holds.
#[derive(Debug, Args)]
pub struct PassedArgs {
  #[arg(long)]
  pub commit: Option<String>,
  #[arg(long)]
  pub json: bool,
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
  #[arg(long)]
  pub json: bool,
  pub files: Vec<String>,
}

/// One of the quick checks, run alone.
#[derive(Debug, Subcommand)]
pub enum CheckPart {
  Quick,
  /// The static check a deployment's gate passes, alone: no test runs.
  Static {
    #[arg(long)]
    since: Option<String>,
  },
  Wording,
  Layout,
  Dependencies,
  CargoProfile,
  /// The manifests and locks held to the shared versions; `--report` holds every checkout of the
  /// machine's registry to them instead.
  SharedVersions {
    #[arg(long)]
    report: bool,
  },
  /// The lines that are not English; `korean` names it as well.
  #[command(alias = "korean")]
  Language {
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

/// Cargo's own words, every one of them handed on, but `prune`, which is the testbed's own removal.
#[derive(Debug, Args)]
#[command(subcommand_negates_reqs = true, disable_help_subcommand = true)]
pub struct CargoArgs {
  #[command(subcommand)]
  pub word: Option<CargoWord>,
  #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
  pub words: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum CargoWord {
  /// What no build reads any more, removed from the shared Cargo build directory.
  Prune(PruneArgs),
}

/// The platform's names for the suite verbs: `test` followed by one of them.
#[derive(Debug, Args)]
pub struct TestArgs {
  #[command(subcommand)]
  pub word: TestWord,
}

#[derive(Debug, Subcommand)]
pub enum TestWord {
  Suite(SuiteArgs),
  Count(CountArgs),
  Log(LogArgs),
  Template(TemplateArgs),
  Full(FullArgs),
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

#[derive(Debug, Args)]
pub struct ServeArgs {
  #[arg(long, value_name = "address")]
  pub listen: Option<String>,
  #[arg(long, env = "BMO_TESTBED_TOKEN", hide_env_values = true, default_value = "")]
  pub token: String,
}

#[derive(Debug, Args)]
pub struct McpArgs {
  #[arg(long, env = "BMO_TESTBED_SERVICE", value_name = "url")]
  pub service: Option<String>,
  #[arg(long, env = "BMO_TESTBED_TOKEN", hide_env_values = true, default_value = "")]
  pub token: String,
}

// A usage of the policy with the binary's name and numbers filled in.
fn usage(text: &str) -> String {
  fill_marks(
    text.trim_start_matches('\n'),
    &[
      ("verb", BINARY),
      ("days", &PRUNE.older_than_days.to_string()),
      ("hours", &PRUNE.incremental_hours.to_string()),
      ("build_days", &PRUNE.deploy_build_days.to_string()),
      ("listen", &SERVICE.listen),
    ],
  )
}

/// The words read, or clap's error, which says the help or the fault and the code to end on.
pub fn read(args: impl IntoIterator<Item = OsString>) -> Result<Words, clap::Error> {
  let said = &VERBS.usage;
  let check = usage(&said.check);
  let parts = [
    "quick",
    "wording",
    "layout",
    "dependencies",
    "cargo-profile",
    "shared-versions",
    "language",
    "dashes",
  ];
  let command = Words::command()
    .override_help(usage(&said.binary))
    .mut_subcommand("run", |verb| verb.override_help(usage(&said.run)))
    .mut_subcommand("placed", |verb| verb.override_help(usage(&said.placed)))
    .mut_subcommand("status", |verb| verb.override_help(usage(&said.status)))
    .mut_subcommand("classify", |verb| verb.override_help(usage(&said.classify)))
    .mut_subcommand("suite", |verb| verb.override_help(usage(&said.suite)))
    .mut_subcommand("dependents", |verb| verb.override_help(usage(&said.dependents)))
    .mut_subcommand("passed", |verb| verb.override_help(usage(&said.passed)))
    .mut_subcommand("count", |verb| verb.override_help(usage(&said.count)))
    .mut_subcommand("log", |verb| verb.override_help(usage(&said.log)))
    .mut_subcommand("template", |verb| verb.override_help(usage(&said.template)))
    .mut_subcommand("check", |verb| {
      parts.iter().fold(verb.override_help(check.clone()), |verb, part| {
        verb.mut_subcommand(part, |part| part.override_help(check.clone()))
      })
    })
    .mut_subcommand("imports", |verb| verb.override_help(usage(&said.imports)))
    .mut_subcommand("cargo", |verb| {
      verb
        .override_help(usage(&said.cargo))
        .mut_subcommand("prune", |word| word.override_help(usage(&said.prune)))
    })
    .mut_subcommand("prune", |verb| verb.override_help(usage(&said.prune)))
    .mut_subcommand("full", |verb| verb.override_help(usage(&said.full)))
    .mut_subcommand("deploy", |verb| {
      verb
        .override_help(usage(&said.deploy))
        .mut_subcommand("commit", |word| word.override_help(usage(&said.deploy)))
        .mut_subcommand("backups", |word| word.override_help(usage(&said.backups)))
    })
    .mut_subcommand("backups", |verb| verb.override_help(usage(&said.backups)))
    .mut_subcommand("backup", |verb| {
      ["snapshot", "media", "drill", "prune"]
        .iter()
        .fold(verb.override_help(usage(&said.backup)), |verb, word| {
          verb.mut_subcommand(word, |word| word.override_help(usage(&said.backup)))
        })
    })
    .mut_subcommand("serve", |verb| verb.override_help(usage(&said.serve)))
    .mut_subcommand("mcp", |verb| verb.override_help(usage(&said.mcp)))
    .mut_subcommand("test", |verb| {
      verb
        .mut_subcommand("suite", |word| word.override_help(usage(&said.suite)))
        .mut_subcommand("count", |word| word.override_help(usage(&said.count)))
        .mut_subcommand("log", |word| word.override_help(usage(&said.log)))
        .mut_subcommand("template", |word| word.override_help(usage(&said.template)))
        .mut_subcommand("full", |word| word.override_help(usage(&said.full)))
    });
  let matches = command.try_get_matches_from(args)?;

  Words::from_arg_matches(&matches)
}
