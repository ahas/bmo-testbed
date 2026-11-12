//! The change check, in one call (docs/checks.md, 「The change check」): the quick checks, the declared static
//! steps and `cargo check` the change owes, then the tests it touches and the check of each project, tool and
//! package it reaches, every step run to its end and written to one log, and what the tests came to read back
//! by the `log` verb. What the change touches is `touched`'s answer alone.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::{Repository, Test};
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::policy::{fill_marks, fill_shown};
use bmo_testbed_core::turn::{Lane, NOT_RUN_EXIT_CODE, TurnAsk, process_env, take_turn};
use sha2::{Digest, Sha256};

use crate::cargo::cargo_verb_line;
use crate::own_binary::testbed_line;
use crate::run::{file_groups, script_runner};
use crate::selection::changed_paths;
use crate::static_check::{CheckStep, TimedRun, declared_steps, run_timed, spawn_step};
use crate::test_server::test_postgres;
use crate::test_targets::TargetTests;
use crate::tool_checks::CheckedTool;
use crate::touched::{Touched, touched};
use crate::workspace::OwnProject;

/// What one call is asked for: a change named by its files, or read from a commit to the working tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckAsk {
  pub since: Option<String>,
  pub files: Vec<String>,
}

/// One step of the check: what it runs and where, as a person reads it by `name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
  pub name: String,
  pub step: CheckStep,
  /// Whether it runs the test files, which wait for the machine's turn as any test run does and whose log
  /// the `log` verb reads back.
  pub tests: bool,
}

/// What a call will run, and the change it was read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckPlan {
  /// Where the change was read from: the files given, or the commit it was read since.
  pub from: String,
  pub changed: Vec<String>,
  pub touched: Touched,
  pub steps: Vec<Step>,
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
  let done = Command::new("git").args(args).current_dir(root).stdin(Stdio::null()).output().ok()?;

  done.status.success().then(|| String::from_utf8_lossy(&done.stdout).trim().to_owned())
}

/// The commit a change is read since where none is named: the one the branch was made from, its merge base
/// with the declared main branch, or HEAD itself where it stands on main or has none, so only what is not
/// committed is read.
pub fn default_since(repo: &Repository) -> String {
  git(&repo.root, &["merge-base", "HEAD", &repo.declaration.check.main])
    .filter(|base| !base.is_empty())
    .unwrap_or_else(|| "HEAD".to_owned())
}

// A step run where it stands, at the root or in `dir`.
fn plain(dir: &str, command: Vec<String>, env: Vec<(String, String)>) -> CheckStep {
  CheckStep {
    id: String::new(),
    dir: PathBuf::from(dir),
    command,
    env,
    lane: None,
    retry_out_of_memory: false,
    covered_by: None,
  }
}

// Where a step runs, as its name says it: nothing at the root.
fn where_it_runs(dir: &Path) -> String {
  if dir == Path::new(".") {
    String::new()
  } else {
    fill_shown(&SUITE.check.where_it_runs, &[("dir", &dir.display())])
  }
}

// A static step, named by its id and where it runs, which is how a person reads it.
fn of(step: CheckStep) -> Step {
  Step {
    name: format!("{}{}", step.id, where_it_runs(&step.dir)),
    step,
    tests: false,
  }
}

fn cargo(verb: &str, crates: &[String], env: Vec<(String, String)>) -> Step {
  let mut command = cargo_verb_line(&[verb, "--locked"]);

  if verb == "check" {
    command.push("--all-targets".to_owned());
  }

  for one in crates {
    command.extend(["-p".to_owned(), one.clone()]);
  }

  Step {
    name: format!("cargo {verb}"),
    step: plain(".", command, env),
    tests: false,
  }
}

// One target's tests of one crate, whole or by its modules' filters.
fn target_step(one: &TargetTests, env: Vec<(String, String)>) -> Step {
  let mut command = cargo_verb_line(&["test", "--locked"]);

  command.extend(["-p".to_owned(), one.krate.clone(), "--test".to_owned(), one.target.clone()]);

  if !one.filters.is_empty() {
    command.push("--".to_owned());
    command.extend(one.filters.iter().cloned());
  }

  Step {
    name: format!("cargo test -p {} --test {}", one.krate, one.target),
    step: plain(".", command, env),
    tests: false,
  }
}

// A command that fails with the reason it could not run, as the suite's unit for it fails.
fn refusal(why: String) -> Vec<String> {
  ["sh", "-c", "printf '%s\\n' \"$1\" >&2; exit 1", "sh"]
    .map(str::to_owned)
    .into_iter()
    .chain([why])
    .collect()
}

// A tool's own check in its folder; where its manifest gives it none to run, a step failing with the reason.
fn tool_step(tool: &CheckedTool) -> Step {
  let (command, env) = tool.command().unwrap_or_else(|why| (refusal(why), vec![]));

  Step {
    name: format!("{}/ check", tool.dir),
    step: plain(&tool.dir, command, env),
    tests: false,
  }
}

// A project's own check in its folder, through the declared script run by the declared language that runs
// scripts; where it names none, or no language runs it, a step failing with the reason.
fn project_step(project: &OwnProject, script: &str, test: &Test) -> Step {
  let command = match script_runner(test).and_then(|runner| runner.script_command(script)) {
    _ if !project.check => refusal(fill_marks(&SUITE.file.no_check, &[("script", script)])),
    Some(command) => command,
    None => refusal(fill_marks(&SUITE.file.no_runner, &[("script", script)])),
  };

  Step {
    name: format!("{}/ {script}", project.dir),
    step: plain(&project.dir, command, vec![]),
    tests: false,
  }
}

/// The steps a change owes, in the order they run: what it changes is the files `ask` names, or every path
/// changed since its commit.
pub fn plan(repo: &Repository, ask: &CheckAsk) -> Result<CheckPlan, String> {
  let (from, changed) = if ask.files.is_empty() {
    let since = ask.since.clone().unwrap_or_else(|| default_since(repo));
    let changed = changed_paths(&repo.root, &since)?;

    (fill_shown(&SUITE.check.since, &[("since", &since)]), changed)
  } else {
    (fill_shown(&SUITE.check.given, &[("count", &ask.files.len())]), ask.files.clone())
  };
  let touched = touched(repo, &changed);
  let mut steps: Vec<Step> = declared_steps(repo, Some(&changed)).into_iter().map(of).collect();
  let database: Vec<(String, String)> = if repo.declaration.test.database.is_some() {
    vec![("BMO_TEST_POSTGRES".to_owned(), test_postgres())]
  } else {
    vec![]
  };

  if !touched.checked_crates.is_empty() {
    steps.push(cargo("check", &touched.checked_crates, vec![]));
  }

  // The touched test files, one step to each declared language that runs some of them and folder it runs
  // them from.
  let test = &repo.declaration.test;

  for group in file_groups(&repo.root, test, &touched.tests) {
    if let Some(command) = group.language.test_command(&group.files, &[]) {
      let dir = if group.dir.as_os_str().is_empty() { Path::new(".") } else { group.dir.as_path() };
      let step = plain(&dir.to_string_lossy(), command, database.clone());

      steps.push(Step {
        name: format!(
          "{}{}",
          fill_marks(&SUITE.file.tests_step, &[("language", group.language.name())]),
          where_it_runs(&step.dir)
        ),
        step,
        tests: true,
      });
    }
  }

  // The crates owing their tests whole run in one step; a crate whose only changed paths are its test files runs
  // those tests alone, a step for each target they reach.
  let whole: Vec<String> = touched
    .tested_crates
    .iter()
    .filter(|name| !touched.targets.iter().any(|one| one.krate == **name))
    .cloned()
    .collect();

  if !whole.is_empty() {
    steps.push(cargo("test", &whole, database.clone()));
  }

  for one in &touched.targets {
    steps.push(target_step(one, database.clone()));
  }

  for project in &touched.projects {
    steps.push(project_step(project, &repo.declaration.projects.check_script, test));
  }

  for tool in &touched.tools {
    steps.push(tool_step(tool));
  }

  Ok(CheckPlan {
    from,
    changed,
    touched,
    steps,
  })
}

/// What a call came to: the lines it prints, the code it ends with, and how long its steps waited for their
/// turns, added up.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
  pub lines: Vec<String>,
  pub code: u8,
  pub waited_seconds: f64,
}

/// Every step of the plan run to its end through `run`, each one's output appended to `log`; then, where the
/// tests ran, the `log` verb's reading of that log through the same `run`. A failing step stops none after
/// it.
pub fn run_check(
  root: &Path,
  plan: &CheckPlan,
  log: &Path,
  run: &dyn Fn(&Path, &Step) -> TimedRun,
) -> Checked {
  let words = &SUITE.check;
  let mut lines = vec![fill_shown(
    &words.heading,
    &[
      ("changed", &plan.changed.len()),
      ("from", &plan.from),
      ("tests", &plan.touched.tests.len()),
      ("tested", &plan.touched.tested_crates.len()),
      ("checked", &plan.touched.checked_crates.len()),
      ("projects", &plan.touched.projects.len()),
      ("tools", &plan.touched.tools.len()),
    ],
  )];
  let mut failed = false;
  let mut written = String::new();
  let mut waited_seconds = 0.0;

  for step in &plan.steps {
    let done = run(root, step);

    waited_seconds += done.waited_seconds.max(0.0);
    let logged = fill_shown(
      &words.logged,
      &[("name", &step.name), ("command", &step.step.command.join(" ")), ("code", &done.code)],
    );

    written.push_str(&format!("{logged}\n{}\n", done.output));
    let _ = std::fs::write(log, &written);

    let outcome = if done.code == 0 { &words.ok } else { &words.failed };

    lines.push(fill_shown(
      &words.step,
      &[
        ("outcome", outcome),
        ("name", &step.name),
        ("code", &done.code),
        ("ran", &format!("{:.0}", done.ran_seconds.max(0.0))),
        ("waited", &format!("{:.0}", done.waited_seconds.max(0.0))),
      ],
    ));

    if done.code != 0 {
      failed = true;

      // The tests' failures are the log verb's to name; a static step's own lines are what tell its fault.
      if !step.tests {
        lines.extend(
          done
            .output
            .lines()
            .take(SUITE.numbers.check_shown_lines)
            .map(|line| format!("  {line}")),
        );
      }
    }
  }

  if plan.steps.iter().any(|step| step.tests) {
    let reading = Step {
      name: "log".to_owned(),
      step: plain(".", testbed_line(&["log", &log.to_string_lossy()]), vec![]),
      tests: false,
    };
    let read = run(root, &reading);

    lines.extend(read.output.lines().map(str::to_owned));
  }

  lines.push(fill_shown(&words.log, &[("path", &log.display())]));

  Checked {
    lines,
    waited_seconds,
    code: u8::from(failed),
  }
}

/// One step run in the checkout at `root`: the test files in the machine's wide lane, waiting for its turn as
/// every test run does and not run where it never came, every other step as the static check runs it.
pub fn run_step(root: &Path, step: &Step) -> TimedRun {
  if !step.tests {
    return spawn_step(root, &step.step);
  }

  let turn = take_turn(&TurnAsk {
    command: step.step.command.join(" "),
    lane: Lane::Wide,
    say: &|line| eprintln!("{line}"),
    path: None,
    env: process_env(),
    checkout: root.to_string_lossy().into_owned(),
    poll: None,
    notice_seconds: None,
  });

  if !turn.had() {
    return TimedRun {
      code: i32::from(NOT_RUN_EXIT_CODE),
      output: fill_shown(&SUITE.check.no_turn, &[("words", &turn.words())]),
      waited_seconds: turn.waited_seconds(),
      ran_seconds: 0.0,
    };
  }

  let mut handed = step.step.env.clone();

  handed.extend(turn.held_variable().map(|(name, value)| (name.to_owned(), value)));

  let mut done = run_timed(&step.step.command, &root.join(&step.step.dir), &handed);

  done.waited_seconds += turn.waited_seconds();
  done
}

/// Where a call's log goes unless it is named: one file per checkout under the data root's `testbed/`, named
/// by the checkout's folder and its path, so two checkouts of one folder name never share a log.
pub fn default_log(data_root: &Path, root: &Path) -> PathBuf {
  let folder = root
    .file_name()
    .map_or_else(|| "root".to_owned(), |name| name.to_string_lossy().into_owned());
  let hash = format!("{:x}", Sha256::digest(root.to_string_lossy().as_bytes()));

  data_root.join("testbed").join(format!("check-{folder}-{}.log", &hash[..8]))
}
