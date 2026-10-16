//! The change check, in one call (docs/checks.md, 「The change check」): the quick checks, the declared static
//! steps and `cargo check` the change owes, then the tests it touches and the check of each project, tool and
//! package it reaches, every step run to its end and written to one log, and what the tests came to read back
//! by the `log` verb. What the change touches is `touched`'s answer alone.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::turn::{Lane, NOT_RUN_EXIT_CODE, TurnAsk, process_env, take_turn};
use sha2::{Digest, Sha256};

use crate::cargo::cargo_verb_line;
use crate::own_binary::testbed_line;
use crate::selection::changed_paths;
use crate::static_check::{CheckStep, TimedRun, declared_steps, run_timed, spawn_step};
use crate::test_server::test_postgres;
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

// How many lines of a failing static step's output the call prints; the rest is in the log.
const SHOWN_LINES: usize = 40;

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
  }
}

// A static step, named by its id and where it runs, which is how a person reads it.
fn of(step: CheckStep) -> Step {
  let at = if step.dir == Path::new(".") {
    String::new()
  } else {
    format!(" in {}", step.dir.display())
  };

  Step {
    name: format!("{}{at}", step.id),
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

// A project's own check in its folder, through the declared script; where it names none, a step failing with
// the reason.
fn project_step(project: &OwnProject, script: &str) -> Step {
  let command = if project.check {
    ["bun", "run", script].map(str::to_owned).to_vec()
  } else {
    refusal(fill_marks(&SUITE.file.no_check, &[("script", script)]))
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

    (format!("since {since}"), changed)
  } else {
    (format!("{} file(s) given", ask.files.len()), ask.files.clone())
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

  if !touched.tests.is_empty() {
    let mut command: Vec<String> = ["bun", "test", "--parallel"].map(str::to_owned).to_vec();

    command.extend(touched.tests.iter().cloned());
    steps.push(Step {
      name: "bun test".to_owned(),
      step: plain(".", command, database.clone()),
      tests: true,
    });
  }

  if !touched.tested_crates.is_empty() {
    steps.push(cargo("test", &touched.tested_crates, database));
  }

  for project in &touched.projects {
    steps.push(project_step(project, &repo.declaration.projects.check_script));
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

/// What a call came to: the lines it prints and the code it ends with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
  pub lines: Vec<String>,
  pub code: u8,
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
  let mut lines = vec![format!(
    "check: {} path(s) changed ({}); {} test file(s), {} crate(s) tested, {} checked, {} project check(s), {} \
     tool check(s)",
    plan.changed.len(),
    plan.from,
    plan.touched.tests.len(),
    plan.touched.tested_crates.len(),
    plan.touched.checked_crates.len(),
    plan.touched.projects.len(),
    plan.touched.tools.len(),
  )];
  let mut failed = false;
  let mut written = String::new();

  for step in &plan.steps {
    let done = run(root, step);

    written.push_str(&format!(
      "==> {} ({}): exit {}\n{}\n",
      step.name,
      step.step.command.join(" "),
      done.code,
      done.output
    ));
    let _ = std::fs::write(log, &written);

    let outcome = if done.code == 0 { "ok" } else { "FAILED" };

    lines.push(format!(
      "{outcome} {} · exit {} · ran {:.0}s, waited {:.0}s",
      step.name,
      done.code,
      done.ran_seconds.max(0.0),
      done.waited_seconds.max(0.0)
    ));

    if done.code != 0 {
      failed = true;

      // The tests' failures are the log verb's to name; a static step's own lines are what tell its fault.
      if !step.tests {
        lines.extend(done.output.lines().take(SHOWN_LINES).map(|line| format!("  {line}")));
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

  lines.push(format!("log: {}", log.display()));

  Checked {
    lines,
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
    wait_seconds: None,
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
      output: format!("the machine's turn did not come: {}", turn.words()),
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
