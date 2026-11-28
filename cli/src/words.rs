//! The binary's words (docs/boundary.md, 「The binary」), read by clap into each verb's flags and arguments.
//! The usage each `--help` prints is the policy's, the binary's numbers filled in; words that do not read
//! end with clap's own line naming the one thing wrong and pointing at `--help`, and exit code 2.

use std::ffi::OsString;
use std::path::PathBuf;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::install::INSTALL;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::service::SERVICE;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::policy::supervise::SUPERVISE;
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
  CrateTests(CrateTestsArgs),
  Status(StatusArgs),
  CloseLine(CloseLineArgs),
  Classify(ClassifyArgs),
  Suite(SuiteArgs),
  Dependents(DependentsArgs),
  Passed(PassedArgs),
  Toolchain(ToolchainArgs),
  Count(CountArgs),
  Log(LogArgs),
  Template(TemplateArgs),
  Check(CheckArgs),
  Imports(ImportsArgs),
  Cargo(CargoArgs),
  Prune(PruneArgs),
  Full(FullArgs),
  Nightly(NightlyArgs),
  Deploy(DeployArgs),
  Backups,
  Backup(BackupArgs),
  Serve(ServeArgs),
  Mcp(McpArgs),
  Notice(NoticeArgs),
  Install(InstallArgs),
  Upgrade(UpgradeArgs),
  /// A repository's release built once and packed as an artifact (docs/artifacts.md).
  Pack(PackArgs),
  Supervise(SuperviseArgs),
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

/// `nightly`: the round over the machine's repositories, or with `--dry-run` what it would run.
#[derive(Debug, Args)]
pub struct NightlyArgs {
  #[arg(long)]
  pub when_due: bool,
  #[arg(long)]
  pub dry_run: bool,
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
  /// Mark the deployment first in the deployment line, with this English reason.
  #[arg(long, value_parser = named)]
  pub first: Option<String>,
  /// Who marks it first; the account the verb runs as where none is named.
  #[arg(long, value_parser = named, requires = "first")]
  pub by: Option<String>,
  /// An artifact to put live in place of a commit of the checkout (docs/artifacts.md).
  #[arg(long, conflicts_with_all = ["commit", "no_check", "before_start", "force_live", "first"])]
  pub artifact: Option<std::path::PathBuf>,
}

/// `deploy first`: a waiting deployment raised to first in the deployment line, by its unit.
#[derive(Debug, Args)]
pub struct FirstArgs {
  #[arg(value_parser = named)]
  pub unit: String,
  /// Why it goes first, in English.
  #[arg(long, value_parser = named)]
  pub why: String,
  /// Who raises it; the account the verb runs as where none is named.
  #[arg(long, value_parser = named)]
  pub by: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum DeployWord {
  /// The deployment itself, as `deploy` runs it.
  Commit(DeployFlags),
  /// The backups a deployment took, sent away and rotated.
  Backups,
  /// A waiting deployment raised to first in the deployment line.
  First(FirstArgs),
  /// The machine's deployments as the record keeps them, newest first.
  History(HistoryArgs),
}

/// `deploy history`: the machine's deployments, one repository's where one is named.
#[derive(Debug, Args)]
pub struct HistoryArgs {
  #[arg(long, value_parser = named)]
  pub repository: Option<String>,
  #[arg(long)]
  pub limit: Option<usize>,
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

/// `crate-tests`: the suite's own two halves of a crate's tests (docs/boundary.md, 「crate-tests」).
#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct CrateTestsArgs {
  #[command(subcommand)]
  pub word: CrateTestsWord,
}

#[derive(Debug, Subcommand)]
pub enum CrateTestsWord {
  /// Cargo's runner while the suite builds: the test binary written down, nothing run.
  Record {
    record: PathBuf,
    executable: String,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
  },
  /// Every test binary the record holds for the package, run to its end.
  Run {
    record: PathBuf,
    #[arg(value_parser = named)]
    package: String,
  },
}

#[derive(Debug, Args)]
pub struct PlacedArgs {
  /// The declared setup steps first.
  #[arg(long)]
  pub setup: bool,
  /// A checkout the run knows on this machine: the bench and the trees copied beside it.
  #[arg(long = "checkout", value_name = "PATH")]
  pub checkouts: Vec<String>,
  /// The folder of the bench the command runs in, from the bench's top.
  #[arg(long, value_name = "PATH")]
  pub dir: Option<String>,
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
  /// Print nothing and exit 0 where a seat of the wide lane is free now with nobody waiting for one, else 1.
  #[arg(long, conflicts_with = "json")]
  pub free: bool,
  /// With `--free`, ask the narrow lane instead.
  #[arg(long, requires = "free")]
  pub narrow: bool,
}

/// `close-line`: the close lines of the named checkouts' repositories, held until standard input ends.
#[derive(Debug, Args)]
pub struct CloseLineArgs {
  /// A checkout whose repository's close line is held, given once for each.
  #[arg(long = "checkout", value_name = "PATH", required = true, value_parser = named)]
  pub checkouts: Vec<String>,
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

/// `toolchain`: the toolchains the checkout is built and tested with.
#[derive(Debug, Args)]
pub struct ToolchainArgs {
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
  /// A checkout of a change check over several checkouts, its since after an `@` where given; named again
  /// for each.
  #[arg(long = "checkout", value_name = "PATH[@REV]", conflicts_with_all = ["since", "files"])]
  pub checkouts: Vec<String>,
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
  PersonalData,
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
  #[arg(long, conflicts_with = "all")]
  pub when_due: bool,
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

/// `install`: a preset installed, a repository added, the install's status, or an input kept (docs/install.md).
#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("mode").required(true).args(["preset", "add", "status", "set"])))]
pub struct InstallArgs {
  #[arg(long, value_parser = named)]
  pub preset: Option<String>,
  #[arg(long, value_parser = named, requires = "preset")]
  pub with: Vec<String>,
  #[arg(long, value_parser = named)]
  pub add: Option<String>,
  #[arg(long)]
  pub status: bool,
  #[arg(long, value_parser = named)]
  pub set: Option<String>,
  #[arg(long)]
  pub index: Option<PathBuf>,
  #[arg(long)]
  pub data_root: Option<PathBuf>,
  #[arg(long)]
  pub prefix: Option<PathBuf>,
  #[arg(long, value_parser = named)]
  pub postgres_host: Option<String>,
  #[arg(long, value_parser = named)]
  pub postgres_port: Option<String>,
  #[arg(long, value_parser = named)]
  pub valkey: Option<String>,
  #[arg(long, conflicts_with_all = ["status", "set"])]
  pub dry_run: bool,
  /// The artifact channel every repository is taken from in place of a clone (docs/artifacts.md).
  #[arg(long, value_parser = named)]
  pub from: Option<String>,
}

/// `upgrade`: the install moved to the newest release tag, or with `--nightly` to the head of every main branch
/// (docs/install.md, 「Upgrading: `upgrade`」).
#[derive(Debug, Args)]
pub struct UpgradeArgs {
  #[arg(long)]
  pub nightly: bool,
  #[arg(long)]
  pub index: Option<PathBuf>,
  #[arg(long)]
  pub data_root: Option<PathBuf>,
  #[arg(long)]
  pub prefix: Option<PathBuf>,
  #[arg(long)]
  pub dry_run: bool,
}

/// `pack`: the repository the verb runs in packed at a commit as an artifact (docs/artifacts.md, 「The words」).
#[derive(Debug, Args)]
pub struct PackArgs {
  #[arg(long)]
  pub commit: Option<String>,
  #[arg(long)]
  pub out: Option<PathBuf>,
  #[arg(long)]
  pub key: Option<PathBuf>,
}

/// `supervise`: the set's services run without systemd, and what is asked of them (docs/supervise.md).
#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub struct SuperviseArgs {
  #[command(subcommand)]
  pub word: SuperviseWord,
}

#[derive(Debug, Subcommand)]
pub enum SuperviseWord {
  /// The supervisor run in the foreground until it is stopped.
  Serve {
    #[arg(long)]
    data_root: Option<PathBuf>,
    #[arg(long)]
    anyway: bool,
  },
  /// Each unit's state, main process, restarts and health.
  Status {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    data_root: Option<PathBuf>,
  },
  /// The lines that put a command under the supervisor.
  Env {
    #[arg(long)]
    data_root: Option<PathBuf>,
  },
  /// Every unit stopped and the supervisor ended.
  StopAll {
    #[arg(long)]
    data_root: Option<PathBuf>,
  },
}

/// `notice`: the event a hook is handed, by its name.
#[derive(Debug, Args)]
pub struct NoticeArgs {
  #[arg(value_parser = named)]
  pub event: String,
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
      ("due_hours", &PRUNE.due_hours.to_string()),
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
    "personal-data",
    "shared-versions",
    "language",
    "dashes",
  ];
  let command = Words::command()
    .override_help(usage(&said.binary))
    .mut_subcommand("run", |verb| verb.override_help(usage(&said.run)))
    .mut_subcommand("placed", |verb| verb.override_help(usage(&said.placed)))
    .mut_subcommand("crate-tests", |verb| {
      ["record", "run"]
        .iter()
        .fold(verb.override_help(usage(&said.crate_tests)), |verb, word| {
          verb.mut_subcommand(word, |word| word.override_help(usage(&said.crate_tests)))
        })
    })
    .mut_subcommand("status", |verb| verb.override_help(usage(&said.status)))
    .mut_subcommand("close-line", |verb| verb.override_help(usage(&said.close_line)))
    .mut_subcommand("classify", |verb| verb.override_help(usage(&said.classify)))
    .mut_subcommand("suite", |verb| verb.override_help(usage(&said.suite)))
    .mut_subcommand("dependents", |verb| verb.override_help(usage(&said.dependents)))
    .mut_subcommand("passed", |verb| verb.override_help(usage(&said.passed)))
    .mut_subcommand("toolchain", |verb| verb.override_help(usage(&said.toolchain)))
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
    .mut_subcommand("nightly", |verb| verb.override_help(usage(&said.nightly)))
    .mut_subcommand("deploy", |verb| {
      verb
        .override_help(usage(&said.deploy))
        .mut_subcommand("commit", |word| word.override_help(usage(&said.deploy)))
        .mut_subcommand("backups", |word| word.override_help(usage(&said.backups)))
        .mut_subcommand("first", |word| word.override_help(usage(&said.deploy_first)))
        .mut_subcommand("history", |word| word.override_help(usage(&said.deploy_history)))
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
    .mut_subcommand("notice", |verb| verb.override_help(usage(&said.notice)))
    .mut_subcommand("install", |verb| verb.override_help(usage(&INSTALL.usage)))
    .mut_subcommand("upgrade", |verb| verb.override_help(usage(&INSTALL.upgrade_usage)))
    .mut_subcommand("supervise", |verb| {
      ["serve", "status", "env", "stop-all"]
        .iter()
        .fold(verb.override_help(usage(&SUPERVISE.usage)), |verb, word| {
          verb.mut_subcommand(word, |word| word.override_help(usage(&SUPERVISE.usage)))
        })
    })
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
