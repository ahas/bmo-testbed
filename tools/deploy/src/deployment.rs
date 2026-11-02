//! One call of `testbed/deploy` (docs/deploy.md, 「The hosted deploy tool」): the arguments read, the
//! checkout found, the deploy verb run outside the harness the session stands in, its unit waited out and
//! the deployment record read for what came of it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use bmo_testbed_core::answer::{Cut, cap_items};
use bmo_testbed_core::policy::deploy_tool::DEPLOY_TOOL;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::turn::{Env, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_deploy::deployment::DeployPlaces;
use bmo_testbed_deploy::tree::{DeployRecord, read_deploy_record};
use bmo_testbed_deploy::unit::{DEPLOY_UNIT_MARKER, DEPLOY_UNIT_PREFIX};
use bmo_testbed_suite::dependents::registry_path;
use serde::Deserialize;
use serde_json::Value;

use crate::call::Answer;
use crate::checkout::find_checkout;

/// The testbed's binary, found on the path where the release's own does not stand.
pub const TESTBED: &str = "bmo-testbed";

/// The folder the tool host runs a placed tool from, under the tool's folder in the release.
pub const PLACED: &str = ".placed";

/// The variable naming the data root the registry stands under.
pub const DATA_ROOT_VARIABLE: &str = "BMO_DATA_DIR";

/// The variable naming a harness's process, which the verb refuses and which is taken out of its
/// environment.
pub const HARNESS_VARIABLE: &str = "CLAUDE_PID";

// What the service manager answers of a unit that has not yet ended.
const STILL_ACTIVE: [&str; 4] = ["active", "activating", "deactivating", "reloading"];

/// The arguments the entry's input schema states.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arguments {
  pub repository: String,
  pub commit: String,
  pub why: String,
  #[serde(default)]
  pub dry_run: Option<bool>,
}

/// The arguments read off a call, or why they do not read.
pub fn read_arguments(arguments: &Value) -> Result<Arguments, String> {
  let said = &DEPLOY_TOOL.lines;
  let read: Arguments = serde_json::from_value(arguments.clone())
    .map_err(|why| fill_marks(&said.arguments, &[("why", &why.to_string())]))?;

  for (argument, value) in [("repository", &read.repository), ("commit", &read.commit), ("why", &read.why)]
  {
    if value.trim().is_empty() {
      return Err(fill_marks(&said.empty, &[("argument", argument)]));
    }
  }

  let numbers = &DEPLOY_TOOL.numbers;
  let commit = read.commit.trim();

  if !(numbers.commit_shortest..=numbers.commit_longest).contains(&commit.len())
    || !commit.chars().all(|digit| digit.is_ascii_hexdigit())
  {
    return Err(fill_marks(
      &said.commit,
      &[
        ("shortest", &numbers.commit_shortest.to_string()),
        ("longest", &numbers.commit_longest.to_string()),
        ("commit", commit),
      ],
    ));
  }

  Ok(read)
}

/// Where a call runs: its environment, the testbed binary the verb is run with, and how the unit is waited
/// on.
#[derive(Clone, Debug)]
pub struct Ground {
  /// The call's environment, which the tool host made from the session's record.
  pub env: Env,
  pub testbed: PathBuf,
  pub poll: Duration,
}

impl Ground {
  /// The ground of this process: its own environment, the release's binary where the executable runs from a
  /// release's placed folder, and the policy's waits.
  pub fn of_process() -> Self {
    let release = std::env::current_exe().ok().and_then(|exe| release_testbed(&exe));

    Self {
      env: process_env(),
      testbed: release.unwrap_or_else(|| PathBuf::from(TESTBED)),
      poll: Duration::from_secs(DEPLOY_TOOL.numbers.poll_seconds),
    }
  }

  fn data_root(&self) -> PathBuf {
    self
      .env
      .get(DATA_ROOT_VARIABLE)
      .filter(|root| !root.is_empty())
      .map_or_else(data_root, PathBuf::from)
  }

  // The verb's environment: the call's, less what would make the verb refuse it or run where it stands.
  fn verb_env(&self) -> impl Iterator<Item = (&String, &String)> {
    self
      .env
      .iter()
      .filter(|(name, _)| *name != HARNESS_VARIABLE && *name != DEPLOY_UNIT_MARKER)
  }

  fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);

    command.env_clear().envs(self.verb_env()).stdin(Stdio::null());
    command
  }
}

/// The testbed binary of the release an executable at `exe` was placed from, `<release>/bin/bmo-testbed`
/// beside `<release>/tools/<tool>/.placed/`, where the executable stands in such a folder and the binary
/// there.
pub fn release_testbed(exe: &Path) -> Option<PathBuf> {
  let placed = exe.parent()?;

  if placed.file_name()? != PLACED {
    return None;
  }

  let binary = placed.parent()?.parent()?.parent()?.join("bin").join(TESTBED);

  binary.is_file().then_some(binary)
}

/// The unit the verb said it runs in: the first word of its output naming a deployment's unit.
pub fn unit_of(said: &str) -> Option<String> {
  said
    .split_whitespace()
    .map(|word| {
      word.trim_matches(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
    })
    .find(|word| word.strip_prefix(DEPLOY_UNIT_PREFIX).is_some_and(|stamp| !stamp.is_empty()))
    .map(str::to_owned)
}

// What the verb wrote, its lines cut to whole ones under the answer's ceiling.
fn capped(text: &str) -> String {
  let said = &DEPLOY_TOOL.lines;
  let lines: Vec<&str> = text.trim_end().lines().collect();
  let capped = cap_items(&lines, |lines| lines.join("\n"), Cut::new(&said.said_unit, &said.said_how));
  let mut out = capped.kept.join("\n");

  if let Some(note) = capped.note {
    out.push('\n');
    out.push_str(&note);
  }

  out
}

// What a finished verb said: standard error where it wrote there, standard output otherwise.
fn what_it_said(out: &str, err: &str) -> String {
  capped(if err.trim().is_empty() { out } else { err })
}

// Whether the unit has not yet ended, as the service manager answers; why it could not be asked.
fn still_active(ground: &Ground, unit: &str) -> Result<bool, String> {
  let asked = ground
    .command("systemctl")
    .args(["--user", "is-active", unit])
    .stderr(Stdio::null())
    .output()
    .map_err(|why| why.to_string())?;
  let state = String::from_utf8_lossy(&asked.stdout);

  Ok(STILL_ACTIVE.contains(&state.trim()))
}

// The last lines of the unit's journal, or why they could not be read.
fn journal_tail(ground: &Ground, unit: &str) -> String {
  let lines = DEPLOY_TOOL.numbers.journal_lines.to_string();
  let read = ground
    .command("journalctl")
    .args(["--user", "-u", unit, "--no-pager", "-o", "cat", "-n", &lines])
    .output();

  match read {
    Ok(read) if read.status.success() => capped(&String::from_utf8_lossy(&read.stdout)),
    Ok(read) => fill_marks(
      &DEPLOY_TOOL.lines.journal_unread,
      &[("why", String::from_utf8_lossy(&read.stderr).trim())],
    ),
    Err(why) => fill_marks(&DEPLOY_TOOL.lines.journal_unread, &[("why", &why.to_string())]),
  }
}

/// What came of a deployment the unit ran, judged by the record it left: live where it names the commit
/// asked for and is not the record read before the call.
pub fn outcome(
  commit: &str,
  unit: &str,
  before: Option<&DeployRecord>,
  after: Option<&DeployRecord>,
  journal: &dyn Fn() -> String,
) -> Answer {
  let said = &DEPLOY_TOOL.lines;
  let asked = commit.trim().to_ascii_lowercase();

  if let Some(record) = after.filter(|record| record.commit.to_ascii_lowercase().starts_with(&asked)) {
    if before != Some(record) {
      let previous = before.map_or_else(
        || said.no_previous.clone(),
        |previous| {
          fill_marks(&said.previous, &[("commit", &previous.commit), ("subject", &previous.subject)])
        },
      );

      return Answer::text(
        [
          fill_marks(&said.live, &[("commit", &record.commit), ("subject", &record.subject)]),
          previous,
          fill_marks(&said.message, &[("message", &record.message)]),
          fill_marks(&said.unit, &[("unit", unit)]),
        ]
        .join("\n"),
      );
    }

    return Answer::error(format!("{}\n{}", fill_marks(&said.not_written, &[("unit", unit)]), journal()));
  }

  let recorded = after.map_or_else(|| said.nothing_recorded.clone(), |record| record.commit.clone());

  Answer::error(format!(
    "{}\n{}",
    fill_marks(&said.not_live, &[("unit", unit), ("recorded", &recorded), ("commit", commit.trim())]),
    journal()
  ))
}

/// One call: the commit put live from the checkout the registry names for the repository, and what came of
/// it.
pub fn deploy(arguments: &Arguments, ground: &Ground) -> Answer {
  let said = &DEPLOY_TOOL.lines;
  let registry = registry_path(&ground.data_root(), &ground.env);
  let repo = match find_checkout(&arguments.repository, &registry, &ground.env) {
    Ok(repo) => repo,
    Err(why) => return Answer::error(why),
  };
  let places = match DeployPlaces::of(&repo, &ground.env) {
    Ok(places) => places,
    Err(why) => {
      return Answer::error(fill_marks(
        &said.no_deploy,
        &[("repository", &repo.name()), ("root", &repo.root.to_string_lossy()), ("why", &why)],
      ));
    }
  };
  let commit = arguments.commit.trim();
  let dry_run = arguments.dry_run.unwrap_or(false);
  let before = read_deploy_record(&places.record);
  let mut verb = ground.command(&ground.testbed);

  verb
    .args([
      "deploy".to_owned(),
      format!("--commit={commit}"),
      format!("--message={}", arguments.why.trim()),
    ])
    .args(dry_run.then_some("--dry-run"))
    .current_dir(&repo.root);

  let ran = match verb.output() {
    Ok(ran) => ran,
    Err(why) => {
      return Answer::error(fill_marks(
        &said.not_started,
        &[("binary", &ground.testbed.to_string_lossy()), ("why", &why.to_string())],
      ));
    }
  };
  let out = String::from_utf8_lossy(&ran.stdout).into_owned();
  let err = String::from_utf8_lossy(&ran.stderr).into_owned();
  let code = ran.status.code();

  if code == Some(i32::from(REFUSED_EXIT_CODE)) {
    return Answer::error(fill_marks(&said.refused, &[("said", &what_it_said(&out, &err))]));
  }

  if code != Some(0) {
    let code = code.map_or_else(|| said.no_code.clone(), |code| code.to_string());

    return Answer::error(fill_marks(
      &said.failed,
      &[("code", &code), ("said", &what_it_said(&out, &err))],
    ));
  }

  if dry_run {
    return Answer::text(capped(&out));
  }

  let Some(unit) = unit_of(&out) else {
    return Answer::error(fill_marks(&said.no_unit, &[("said", &what_it_said(&out, &err))]));
  };
  // The deployment is waited for until its unit ends, however long that takes.
  loop {
    match still_active(ground, &unit) {
      Ok(false) => break,
      Ok(true) => std::thread::sleep(ground.poll),
      Err(why) => return Answer::error(fill_marks(&said.unasked, &[("unit", &unit), ("why", &why)])),
    }
  }

  let after = read_deploy_record(&places.record);

  outcome(commit, &unit, before.as_ref(), after.as_ref(), &|| journal_tail(ground, &unit))
}
