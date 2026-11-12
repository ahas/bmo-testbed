//! A repository's declaration (docs/declaration.md): the one `testbed.toml` at its checkout's root, read
//! against the schema into what every verb run in that checkout asks of it. A key the schema does not
//! name, a major of another testbed's and keys that break what they ask of each other are refused as the
//! file is read; with no file, or outside any checkout, every default stands.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::languages::{LANGUAGES, Language};
use crate::policy::declaration::DECLARATION;
use crate::policy::fill_marks;
use crate::turn::{Env, Lane};

/// The major version of the testbed this build is, the one a declaration names.
pub fn own_major() -> u64 {
  env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or_default()
}

/// A declaration the testbed refuses, in the words its verb answers with before ending with
/// [`crate::verb::REFUSED_EXIT_CODE`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter.write_str(&self.0)
  }
}

impl std::error::Error for Refusal {}

// A table whose keys all have defaults, as it reads when the declaration leaves it out.
fn empty<T: DeserializeOwned>() -> T {
  toml::from_str("")
    .unwrap_or_else(|error| panic!("an empty table of the declaration does not read: {error}"))
}

fn yes() -> bool {
  true
}

/// The whole declaration.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
  /// The major of the testbed it is written for.
  pub testbed: u64,
  /// The repository's name, when it is not the root directory's.
  pub name: Option<String>,
  /// The repositories this one builds on, by the names their declarations give them.
  #[serde(default)]
  pub upstreams: Vec<String>,
  #[serde(default)]
  pub test: Test,
  #[serde(default)]
  pub projects: Projects,
  #[serde(default)]
  pub tools: Tools,
  #[serde(default)]
  pub check: Check,
  #[serde(default)]
  pub build: Build,
  #[serde(default)]
  pub deploy: Deploy,
  #[serde(default)]
  pub runner: Runner,
  #[serde(default)]
  pub events: Events,
}

impl Default for Declaration {
  /// What a repository with no declaration has: every default, at this testbed's own major.
  fn default() -> Self {
    Self {
      testbed: own_major(),
      name: None,
      upstreams: Vec::new(),
      test: Test::default(),
      projects: Projects::default(),
      tools: Tools::default(),
      check: Check::default(),
      build: Build::default(),
      deploy: Deploy::default(),
      runner: Runner::default(),
      events: Events::default(),
    }
  }
}

/// `[test]`: how the suite runs.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Test {
  /// The languages' names, each one a member of the language type.
  #[serde(default)]
  pub languages: Vec<String>,
  /// The test files of the file languages; none leaves each runner to its own naming.
  pub files: Option<Vec<String>>,
  #[serde(default)]
  pub browser: Vec<String>,
  /// Read and given no effect: nothing the testbed runs has a span it must end within.
  #[serde(default)]
  pub guard_seconds: Option<u64>,
  /// Read and given no effect: nothing the testbed runs has a span it must end within.
  #[serde(default)]
  pub file_guard_seconds: Option<u64>,
  /// Read and given no effect: nothing the testbed runs has a span it must end within.
  #[serde(default)]
  pub test_timeout_seconds: Option<u64>,
  #[serde(default = "Test::memory_gb")]
  pub memory_gb: u64,
  #[serde(default)]
  pub setup: Vec<String>,
  /// The test database, where the repository's tests use PostgreSQL.
  pub database: Option<Database>,
}

impl Test {
  fn memory_gb() -> u64 {
    DECLARATION.defaults.test.memory_gb
  }

  /// The members the declared languages name, in the declaration's order.
  pub fn languages(&self) -> Vec<&'static dyn Language> {
    self
      .languages
      .iter()
      .filter_map(|name| LANGUAGES.into_iter().find(|language| language.name() == name))
      .collect()
  }
}

impl Default for Test {
  fn default() -> Self {
    empty()
  }
}

/// `[test.database]`: the test database.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
  #[serde(default)]
  pub migrations: Vec<String>,
  pub migrate: Option<String>,
  #[serde(default = "Database::max_files")]
  pub max_files: u64,
}

impl Database {
  fn max_files() -> u64 {
    DECLARATION.defaults.database.max_files
  }
}

/// `[projects]`: projects of their own.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projects {
  #[serde(default = "yes")]
  pub discover: bool,
  #[serde(default)]
  pub paths: Vec<PathBuf>,
  #[serde(default = "Projects::check_script")]
  pub check_script: String,
}

impl Projects {
  fn check_script() -> String {
    DECLARATION.defaults.projects.check_script.clone()
  }
}

impl Default for Projects {
  fn default() -> Self {
    empty()
  }
}

/// `[tools]`: tools checked by their manifest.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
  pub dir: Option<PathBuf>,
  #[serde(default = "Tools::manifest")]
  pub manifest: String,
  #[serde(default = "Tools::check_key")]
  pub check_key: String,
}

impl Tools {
  fn manifest() -> String {
    DECLARATION.defaults.tools.manifest.clone()
  }

  fn check_key() -> String {
    DECLARATION.defaults.tools.check_key.clone()
  }
}

impl Default for Tools {
  fn default() -> Self {
    empty()
  }
}

/// A quick check a declaration can name; `korean` names the language check as well.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QuickCheck {
  Wording,
  Dashes,
  #[serde(alias = "korean")]
  Language,
  Layout,
  Dependencies,
  CargoProfile,
  SharedVersions,
}

impl QuickCheck {
  /// The `[check]` key naming the input the check cannot run without, and what the declaration names
  /// there; none for a check that reads only what every repository has.
  pub fn input(self, check: &Check) -> Option<(&'static str, Option<&Path>)> {
    match self {
      Self::Wording => Some(("wording_document", check.wording_document.as_deref())),
      Self::Language => Some(("language_ledger", check.language_ledger.as_deref())),
      Self::Dashes | Self::Layout | Self::Dependencies | Self::CargoProfile | Self::SharedVersions => None,
    }
  }

  /// The check's name, as a declaration writes it.
  pub fn name(self) -> &'static str {
    match self {
      Self::Wording => "wording",
      Self::Dashes => "dashes",
      Self::Language => "language",
      Self::Layout => "layout",
      Self::Dependencies => "dependencies",
      Self::CargoProfile => "cargo-profile",
      Self::SharedVersions => "shared-versions",
    }
  }
}

/// `[check]`: the static check and the quick checks.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
  #[serde(default)]
  pub quick: Vec<QuickCheck>,
  pub wording_document: Option<PathBuf>,
  /// The files `wording` holds to the document; none is every file.
  pub wording_files: Option<Vec<String>>,
  #[serde(default)]
  pub people_text: Vec<String>,
  /// The ledger of lines holding letters that are not English, which `korean_ledger` names as well; a
  /// declaration naming both is refused.
  #[serde(alias = "korean_ledger")]
  pub language_ledger: Option<PathBuf>,
  #[serde(default)]
  pub base_libraries: Vec<String>,
  /// The same upstream repositories as the top level, for a declaration that keeps its dependency words
  /// together; a declaration naming both adds them.
  #[serde(default)]
  pub upstreams: Vec<String>,
  /// What the `shared-versions` check reads: where the shared versions stand and this repository's
  /// exceptions to them.
  pub shared_versions: Option<SharedVersions>,
  #[serde(default = "Check::main")]
  pub main: String,
  /// `[[check.step]]`, in order.
  #[serde(rename = "step", default)]
  pub steps: Vec<CheckStep>,
}

impl Check {
  fn main() -> String {
    DECLARATION.defaults.check.main.clone()
  }
}

impl Default for Check {
  fn default() -> Self {
    empty()
  }
}

/// `[check.shared_versions]`: the repository holding the shared versions, and the crates this one takes
/// otherwise.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedVersions {
  /// The repository whose root holds the shared versions, by its declared name.
  pub repository: String,
  /// `[check.shared_versions.exceptions.<crate>]`, by the crate's name.
  #[serde(default)]
  pub exceptions: BTreeMap<String, SharedException>,
}

/// One crate a repository takes otherwise than the shared versions, and why.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedException {
  pub reason: String,
  /// The line the manifests write for the crate instead of the shared one, its key included.
  pub line: Option<String>,
  /// The versions of the crate the lock holds instead of the snapshot's.
  #[serde(default)]
  pub locked: Vec<String>,
}

/// One static step of `[[check.step]]`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckStep {
  pub id: String,
  pub command: String,
  /// Where it runs; none is the root.
  pub dir: Option<PathBuf>,
  /// The testbed's lane it takes its turn in; none runs where it stands.
  pub lane: Option<Lane>,
  /// The paths that reach it; none is every change.
  pub when: Option<Vec<String>>,
  #[serde(default)]
  pub retry_out_of_memory: bool,
}

/// `[build]`: building a release.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
  /// The binaries a release carries; none is every binary target of the Cargo workspace.
  pub binaries: Option<Vec<String>>,
  #[serde(default = "Build::cargo_profile")]
  pub cargo_profile: String,
  /// `[[build.step]]`, in order.
  #[serde(rename = "step", default)]
  pub steps: Vec<BuildStep>,
}

impl Build {
  fn cargo_profile() -> String {
    DECLARATION.defaults.build.cargo_profile.clone()
  }
}

impl Default for Build {
  fn default() -> Self {
    empty()
  }
}

/// One build step of `[[build.step]]`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildStep {
  pub id: String,
  pub command: String,
  pub dir: Option<PathBuf>,
  pub lane: Option<Lane>,
}

/// `[deploy]`: putting a commit live.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deploy {
  /// Where the deploy tree stands; none deploys nothing.
  pub root: Option<PathBuf>,
  #[serde(default = "Deploy::link")]
  pub link: String,
  pub data_root: Option<PathBuf>,
  #[serde(default)]
  pub env_links: Vec<EnvLink>,
  /// Where the unit files stand; none is every tracked service and timer.
  pub units: Option<Vec<String>>,
  pub health: Option<String>,
  /// Read and given no effect: nothing the testbed runs has a span it must end within.
  #[serde(default)]
  pub health_wait_seconds: Option<u64>,
  /// Read and given no effect: nothing the testbed runs has a span it must end within.
  #[serde(default)]
  pub stay_up_seconds: Option<u64>,
  /// Answers 0 where no work stands under way that a restart would cut; asked just before the record and the flip.
  pub quiet: Option<String>,
  /// Read and given no effect: nothing the testbed runs has a span it must end within.
  #[serde(default)]
  pub quiet_wait_seconds: Option<u64>,
  #[serde(default = "yes")]
  pub gate: bool,
  /// `[[deploy.step]]`, in order.
  #[serde(rename = "step", default)]
  pub steps: Vec<DeployStep>,
  /// `[deploy.console]`: the console screen the repository brings, whose face the deployment probes; none
  /// brings none.
  pub console: Option<DeployConsole>,
  pub backup: Option<Backup>,
}

impl Deploy {
  fn link() -> String {
    DECLARATION.defaults.deploy.link.clone()
  }
}

impl Default for Deploy {
  fn default() -> Self {
    empty()
  }
}

/// A link inside a release to a file under the data root.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvLink {
  pub link: PathBuf,
  pub target: PathBuf,
}

/// When a declared deploy step runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
  /// After the build, before the record and the flip; a failure stops the deployment.
  BeforeFlip,
  /// Once the `bot` service stays up and the console probe passed; a failure fails the deployment, which rolls
  /// back.
  AfterHealth,
}

/// One declared step of `[[deploy.step]]`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployStep {
  pub id: String,
  pub command: String,
  pub phase: Phase,
  /// Read and given no effect: nothing the testbed runs has a span it must end within.
  #[serde(default)]
  pub timeout_seconds: Option<u64>,
}

/// `[deploy.console]`: the console screen a repository's service brings (docs/deploy.md, 「The console
/// probe」).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployConsole {
  /// Run from the release once the service is up: answers on its standard output how a console reaches the
  /// service, its base address, the console's credential and the service's own token, as one JSON object.
  pub reach: String,
}

/// `[deploy.backup]`: the backup a deployment takes.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backup {
  /// Where the backups stand; none is `backups/` under the deploy root.
  pub dir: Option<PathBuf>,
  /// The backup file's name before its time; none is the repository's name.
  pub name: Option<String>,
  pub snapshot: Option<String>,
  pub verify: Option<String>,
  #[serde(default)]
  pub exclude: Vec<PathBuf>,
  pub remote: Option<String>,
  pub profile: Option<String>,
  #[serde(default = "Backup::keep")]
  pub keep: u64,
}

impl Backup {
  fn keep() -> u64 {
    DECLARATION.defaults.backup.keep
  }
}

/// `[runner]`: placement.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
  pub placement: Option<String>,
}

/// `[events]`: where events go.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Events {
  pub hook: Option<String>,
  /// Read and given no effect: nothing the testbed runs has a span it must end within.
  #[serde(default)]
  pub timeout_seconds: Option<u64>,
}

impl Default for Events {
  fn default() -> Self {
    empty()
  }
}

/// The top level alone, read before the rest so a declaration of another major is refused as that
/// rather than for a key its own schema names.
#[derive(Deserialize)]
struct Major {
  testbed: u64,
}

// A step of a table whose steps each carry an id of their own.
trait Step {
  fn id(&self) -> &str;
}

impl Step for CheckStep {
  fn id(&self) -> &str {
    &self.id
  }
}

impl Step for BuildStep {
  fn id(&self) -> &str {
    &self.id
  }
}

impl Step for DeployStep {
  fn id(&self) -> &str {
    &self.id
  }
}

fn shared_id<S: Step>(steps: &[S]) -> Option<&str> {
  let mut seen = BTreeSet::new();
  steps.iter().map(Step::id).find(|id| !seen.insert(*id))
}

impl Declaration {
  /// The declaration a file's text holds, `file` naming it in a refusal.
  pub fn parse(file: &str, text: &str) -> Result<Self, Refusal> {
    let major: Major = deserialize(file, text)?;

    if major.testbed != own_major() {
      return Err(Refusal(fill_marks(
        &DECLARATION.refusal.other_major,
        &[
          ("file", file),
          ("declared", &major.testbed.to_string()),
          ("own", &own_major().to_string()),
        ],
      )));
    }

    let declaration: Self = deserialize(file, text)?;

    match declaration.broken_key() {
      Some((key, why)) => Err(Refusal(fill_marks(
        &DECLARATION.refusal.breaks,
        &[("file", file), ("key", key), ("why", &why)],
      ))),
      None => Ok(declaration),
    }
  }

  // The first key that breaks what the keys ask of each other, and what it asks for.
  fn broken_key(&self) -> Option<(&'static str, String)> {
    let why = &DECLARATION.refusal.why;

    if let Some(name) = self
      .test
      .languages
      .iter()
      .find(|name| !LANGUAGES.iter().any(|language| language.name() == name.as_str()))
    {
      let names = LANGUAGES.map(|language| language.name()).join(", ");
      return Some((
        "test.languages",
        fill_marks(&why.unknown_language, &[("name", name), ("names", &names)]),
      ));
    }

    if let Some(database) = &self.test.database
      && database.migrations.is_empty() != database.migrate.is_none()
    {
      return Some(("test.database", why.half_database.clone()));
    }

    for quick in &self.check.quick {
      if let Some((input, None)) = quick.input(&self.check) {
        return Some((
          "check.quick",
          fill_marks(&why.needs_input, &[("name", quick.name()), ("input", input)]),
        ));
      }
    }

    if self.check.quick.contains(&QuickCheck::SharedVersions) && self.check.shared_versions.is_none() {
      return Some((
        "check.quick",
        fill_marks(
          &why.needs_input,
          &[("name", QuickCheck::SharedVersions.name()), ("input", "shared_versions.repository")],
        ),
      ));
    }

    let exceptions = self.check.shared_versions.iter().flat_map(|shared| &shared.exceptions);

    for (name, exception) in exceptions {
      if exception.reason.trim().is_empty() {
        return Some(("check.shared_versions.exceptions", fill_marks(&why.no_reason, &[("name", name)])));
      }

      if exception.line.is_none() && exception.locked.is_empty() {
        return Some((
          "check.shared_versions.exceptions",
          fill_marks(&why.empty_exception, &[("name", name)]),
        ));
      }
    }

    if self.deploy.root.is_some() && self.deploy.data_root.is_none() {
      return Some(("deploy.root", why.needs_data_root.clone()));
    }

    let shared = [
      ("check.step", shared_id(&self.check.steps)),
      ("build.step", shared_id(&self.build.steps)),
      ("deploy.step", shared_id(&self.deploy.steps)),
    ];

    shared
      .into_iter()
      .find_map(|(key, id)| id.map(|id| (key, fill_marks(&why.duplicate_step, &[("id", id)]))))
  }
}

// The text read as `T`, a key that does not read refused naming where it stands.
fn deserialize<T: DeserializeOwned>(file: &str, text: &str) -> Result<T, Refusal> {
  serde_path_to_error::deserialize(toml::Deserializer::new(text)).map_err(|error| {
    let table = error.path().to_string();
    let table = if table == "." { DECLARATION.refusal.top_level.clone() } else { table };

    Refusal(fill_marks(
      &DECLARATION.refusal.unreadable,
      &[("file", file), ("table", &table), ("error", error.inner().message())],
    ))
  })
}

/// A checkout's root and the declaration standing there.
#[derive(Clone, Debug)]
pub struct Repository {
  /// The checkout's top level, as git names it, or the directory a verb was run in outside any checkout.
  pub root: PathBuf,
  pub declaration: Declaration,
}

impl Repository {
  /// The repository a verb run in `dir` stands in: the checkout's root and its declaration, every
  /// default where there is no file or no checkout.
  pub fn read(dir: &Path) -> Result<Self, Refusal> {
    let root = root_of(dir).unwrap_or_else(|| dir.to_path_buf());
    let path = root.join(&DECLARATION.file);
    let declaration = match std::fs::read_to_string(&path) {
      Ok(text) => Declaration::parse(&path.to_string_lossy(), &text)?,
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => Declaration::default(),
      Err(error) => {
        return Err(Refusal(fill_marks(
          &DECLARATION.refusal.unreadable,
          &[
            ("file", &path.to_string_lossy()),
            ("table", &DECLARATION.refusal.top_level),
            ("error", &error.to_string()),
          ],
        )));
      }
    };

    Ok(Self { root, declaration })
  }

  /// The repository's name: the one declared, or the root directory's.
  pub fn name(&self) -> String {
    self.declaration.name.clone().unwrap_or_else(|| {
      self
        .root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
    })
  }

  /// A declared path where it stands: under `HOME` when it begins with `~/`, as written when it begins
  /// with `/`, and under the root otherwise.
  pub fn path(&self, declared: &Path, env: &Env) -> PathBuf {
    match declared.strip_prefix("~") {
      Ok(under_home) => PathBuf::from(env.get("HOME").map_or("/", String::as_str)).join(under_home),
      Err(_) => self.root.join(declared),
    }
  }
}

/// The top level of the checkout `dir` stands in, as git names it; none outside any checkout.
pub fn root_of(dir: &Path) -> Option<PathBuf> {
  let output = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(["rev-parse", "--show-toplevel"])
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output()
    .ok()?;
  let top = String::from_utf8(output.stdout).ok()?;
  let top = top.trim_end_matches('\n');

  (output.status.success() && !top.is_empty()).then(|| PathBuf::from(top))
}
