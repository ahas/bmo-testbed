//! The change check, in one call (docs/checks.md, 「The change check」): the static check's own steps for the
//! change, the Cargo workspace's formatting and lints among them where it marks a crate, then the tests it
//! touches and the check of each project, tool and package it reaches, every step run to its end and written
//! to one log, and what the tests came to read back by the `log` verb. What the change touches is `touched`'s
//! answer alone.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use bmo_testbed_core::declaration::{Repository, Test};
use bmo_testbed_core::languages::{LANGUAGES, failures_of_log, tally_of_log};
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::policy::{fill_marks, fill_shown};
use bmo_testbed_core::queue::now_ms;
use bmo_testbed_core::silence::Silence;
use bmo_testbed_core::turn::{Env, Lane, NOT_RUN_EXIT_CODE, TurnAsk, process_env, take_turn};
use bmo_testbed_core::witness::{RunBegun, RunEnded, RunVerdict, RunWitness};
use sha2::{Digest, Sha256};

use crate::cargo::cargo_verb_line;
use crate::own_binary::testbed_line;
use crate::run::{browser_file, file_groups, holds_browser, limit_words, script_runner};
use crate::selection::changed_paths;
use crate::static_check::{CheckStep, TimedRun, run_fenced, spawn_step, static_steps};
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
    watched: false,
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

// The crates owing their tests whole, in one step.
fn whole_tests(crates: &[String], env: Vec<(String, String)>) -> Step {
  let mut command = cargo_verb_line(&["test", "--locked"]);

  for one in crates {
    command.extend(["-p".to_owned(), one.clone()]);
  }

  Step {
    name: "cargo test".to_owned(),
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
  let mut steps: Vec<Step> = static_steps(repo, Some(&changed)).into_iter().map(of).collect();
  let database: Vec<(String, String)> = if repo.declaration.test.database.is_some() {
    vec![("BMO_TEST_POSTGRES".to_owned(), test_postgres())]
  } else {
    vec![]
  };

  // The touched test files, one step to each declared language that runs some of them and folder it runs
  // them from.
  let test = &repo.declaration.test;

  for group in file_groups(&repo.root, test, &touched.tests) {
    // A step holding a file the declaration names as a browser file is handed no limit of the runner's own:
    // a runner that ends one test of such a file takes down the browser the whole command shares
    // (docs/suite.md, 「How long a unit and a test may run」).
    let browser = holds_browser(&group.dir, &group.files, &|file| browser_file(test, file));
    let limit = limit_words(group.language, &[], test.per_test_seconds(), browser);

    if let Some(command) = group.language.test_command(&group.files, &limit) {
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
    steps.push(whole_tests(&whole, database.clone()));
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

/// What a call came to: the lines it prints, the code it ends with, how long its steps waited for their turns,
/// added up, and the names of the steps that failed.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
  pub lines: Vec<String>,
  pub code: u8,
  pub waited_seconds: f64,
  pub failed: Vec<String>,
}

// The lines of a call as they are made: kept, and each handed to `say` at once.
struct Lines<'a> {
  kept: Vec<String>,
  say: &'a mut dyn FnMut(&str),
}

impl Lines<'_> {
  fn push(&mut self, line: String) {
    (self.say)(&line);
    self.kept.push(line);
  }

  fn extend(&mut self, lines: impl IntoIterator<Item = String>) {
    for line in lines {
      self.push(line);
    }
  }
}

/// Every step of the plan run to its end through `run`, each one's output appended to `log`, a step the Cargo
/// workspace check covers not run and said as passed in one line; then, where the tests ran, the `log` verb's
/// reading of that log through the same `run`. A failing step stops none after it.
pub fn run_check(
  root: &Path,
  plan: &CheckPlan,
  log: &Path,
  run: &dyn Fn(&Path, &Step) -> TimedRun,
) -> Checked {
  run_check_saying(root, plan, log, run, &mut |_| {})
}

/// The check `run_check` runs, each line handed to `say` as it is made: the heading first, each step's line as
/// the step ends.
pub fn run_check_saying(
  root: &Path,
  plan: &CheckPlan,
  log: &Path,
  run: &dyn Fn(&Path, &Step) -> TimedRun,
  say: &mut dyn FnMut(&str),
) -> Checked {
  let words = &SUITE.check;
  let mut lines = Lines {
    kept: Vec::new(),
    say,
  };

  lines.push(fill_shown(
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
  ));
  let mut failed = Vec::new();
  let mut written = String::new();
  let mut waited_seconds = 0.0;

  for step in &plan.steps {
    if let Some(by) = step.step.covered_by {
      let left_out = fill_shown(&words.left_out, &[("name", &step.name), ("by", &by.name())]);

      written.push_str(&format!("{left_out}\n"));
      let _ = std::fs::write(log, &written);
      lines.push(left_out);
      continue;
    }

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
      failed.push(step.name.clone());

      // The test files' failures are the log verb's to name; every other step's are read out of its own output.
      if !step.tests {
        lines.extend(what_failed(&done.output));
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

  // The next run of the checkout writes the log afresh, so a failing one is kept apart until another fails.
  if !failed.is_empty() {
    let kept = kept_log(log);

    if std::fs::copy(log, &kept).is_ok() {
      lines.push(fill_shown(&words.kept, &[("path", &kept.display())]));
    }
  }

  lines.push(fill_shown(&words.log, &[("path", &log.display())]));

  Checked {
    lines: lines.kept,
    waited_seconds,
    code: u8::from(!failed.is_empty()),
    failed,
  }
}

/// One checkout of a change check over several checkouts: its repository, its plan, the log its own check
/// writes, and the words its test run is kept under.
pub struct SetMember {
  pub repo: Repository,
  pub plan: CheckPlan,
  pub log: PathBuf,
  pub command: String,
}

/// What a change check over several checkouts came to: the code it ends with, the steps' waits added up, and
/// the lines naming each failed repository and its failed steps.
#[derive(Clone, Debug, PartialEq)]
pub struct SetChecked {
  pub code: u8,
  pub waited_seconds: f64,
  pub lines: Vec<String>,
}

/// Every checkout's check run in turn through `run`, inside the turn its caller holds (docs/checks.md, 「The
/// change check over several checkouts」): each checkout's lines handed to `say` as they are made, opened with
/// its repository's name; each checkout kept as one test run through the witness `witness_of` gives it, its counts
/// those its own log adds up to; each checkout's log appended to the set's `log` as it ends; then a line for each
/// failed repository naming its failed steps, the set's log kept apart where any failed, and the set's log.
pub fn run_set(
  members: &[SetMember],
  log: &Path,
  env: &Env,
  run: &dyn Fn(&Path, &Step) -> TimedRun,
  witness_of: &dyn Fn(&Repository) -> Box<dyn RunWitness>,
  say: &mut dyn FnMut(&str),
) -> SetChecked {
  let words = &SUITE.check;
  let mut written = String::new();
  let mut lines = Vec::new();
  let mut waited_seconds = 0.0;
  let mut any_failed = false;

  let _ = std::fs::write(log, &written);

  for member in members {
    let repository = member.repo.name();
    let witness = witness_of(&member.repo);
    let begun = witness.began(&RunBegun {
      command: &member.command,
      checkout: &member.repo.root,
      log,
      started_at: now_ms(),
      env,
      whole: false,
    });
    let checked = run_check_saying(&member.repo.root, &member.plan, &member.log, run, &mut |line| {
      say(&fill_shown(&words.set_line, &[("repository", &repository), ("line", &line)]));
    });
    let own = std::fs::read_to_string(&member.log).unwrap_or_default();

    waited_seconds += checked.waited_seconds;
    written.push_str(&format!(
      "{}\n{own}",
      fill_shown(
        &words.set_logged,
        &[("repository", &repository), ("checkout", &member.repo.root.display())]
      )
    ));
    let _ = std::fs::write(log, &written);

    if let Some(id) = begun {
      let tally = tally_of_log(&LANGUAGES, &own).unwrap_or_default();

      witness.ended(
        id,
        &RunEnded {
          verdict: if checked.code == 0 { RunVerdict::Passed } else { RunVerdict::Failed },
          passed: tally.pass,
          failed: tally.fail,
          skipped: tally.skip,
          ended_at: now_ms(),
        },
      );
    }

    if !checked.failed.is_empty() {
      any_failed = true;
      lines.push(fill_shown(
        &words.set_failed,
        &[("repository", &repository), ("steps", &checked.failed.join(&words.set_steps_joiner))],
      ));
    }
  }

  if any_failed {
    let kept = kept_log(log);

    if std::fs::copy(log, &kept).is_ok() {
      lines.push(fill_shown(&words.kept, &[("path", &kept.display())]));
    }
  }

  lines.push(fill_shown(&words.log, &[("path", &log.display())]));

  for line in &lines {
    say(line);
  }

  SetChecked {
    code: u8::from(any_failed),
    waited_seconds,
    lines,
  }
}

// What a failing step's output says failed, as the lines under its step line: its totals where a language
// reads them, then the first lines of what the languages name, each failed test with its panic and each
// compiler error; or, where none names anything, its last lines. The lines past the policy's number are counted
// as left to the log.
fn what_failed(output: &str) -> Vec<String> {
  let words = &SUITE.check;
  let most = SUITE.numbers.check_shown_lines;
  let named: Vec<String> = failures_of_log(&LANGUAGES, output)
    .iter()
    .flat_map(|one| one.split('\n').map(str::to_owned).collect::<Vec<_>>())
    .collect();
  let mut lines: Vec<String> = tally_of_log(&LANGUAGES, output)
    .map(|tally| {
      fill_shown(&words.totals, &[("pass", &tally.pass), ("fail", &tally.fail), ("skip", &tally.skip)])
    })
    .into_iter()
    .collect();
  let indented = |line: &String| format!("  {line}");

  if named.is_empty() {
    let all: Vec<String> = output.trim_end().lines().map(str::to_owned).collect();
    let earlier = all.len().saturating_sub(most);

    if earlier > 0 {
      lines.push(fill_shown(&words.earlier, &[("count", &earlier)]));
    }

    lines.extend(all[earlier..].iter().map(indented));
  } else {
    lines.extend(named.iter().take(most).map(indented));

    if named.len() > most {
      lines.push(fill_shown(&words.more, &[("count", &(named.len() - most))]));
    }
  }

  lines
}

/// Where a failing check's log is kept apart from the next run's: beside it, named as it is with the policy's
/// mark before its extension.
pub fn kept_log(log: &Path) -> PathBuf {
  let mark = &SUITE.check.kept_mark;
  let stem = log.file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
  let name = match log.extension() {
    Some(extension) => format!("{stem}.{mark}.{}", extension.to_string_lossy()),
    None => format!("{stem}.{mark}"),
  };

  log.with_file_name(name)
}

/// A step run through `run` while `say` is handed a line saying it still runs every `every` (docs/checks.md,
/// 「The change check」), from a thread of its own that ends with the step.
pub fn run_standing(
  step: &Step,
  every: Duration,
  say: &(dyn Fn(&str) + Sync),
  run: impl FnOnce() -> TimedRun,
) -> TimedRun {
  let started = Instant::now();
  let (ran, ended) = mpsc::channel::<()>();

  std::thread::scope(|scope| {
    scope.spawn(move || {
      while let Err(RecvTimeoutError::Timeout) = ended.recv_timeout(every) {
        let seconds = started.elapsed().as_secs();

        say(&fill_shown(&SUITE.check.standing, &[("name", &step.name), ("seconds", &seconds)]));
      }
    });

    let done = run();

    drop(ran);
    done
  })
}

/// One step run in the checkout at `root`: the test files in the machine's wide lane, waiting for its turn as
/// every test run does and not run where it never came, every other step as the static check runs it. Every
/// step is watched for silence, since the check says each one still runs for as long as it runs and so
/// leaves no silence for a watcher above it to read.
pub fn run_step(root: &Path, step: &Step) -> TimedRun {
  let mut env = process_env();

  // A step handed the turn its caller holds takes it as its own.
  env.extend(step.step.env.iter().cloned());

  if !step.tests {
    return spawn_step(
      root,
      &CheckStep {
        watched: true,
        ..step.step.clone()
      },
    );
  }

  let turn = take_turn(&TurnAsk {
    command: step.step.command.join(" "),
    lane: Lane::Wide,
    say: &|line| eprintln!("{line}"),
    path: None,
    env,
    checkout: root.to_string_lossy().into_owned(),
    poll: None,
    notice_seconds: None,
  });

  if !turn.had() {
    return TimedRun {
      code: i32::from(NOT_RUN_EXIT_CODE),
      signal: None,
      output: fill_shown(&SUITE.check.no_turn, &[("words", &turn.words())]),
      waited_seconds: turn.waited_seconds(),
      ran_seconds: 0.0,
    };
  }

  let mut handed = step.step.env.clone();

  handed.extend(turn.held_variable().map(|(name, value)| (name.to_owned(), value)));

  let mut done = run_fenced(
    &step.name,
    &step.step.command,
    &root.join(&step.step.dir),
    &handed,
    Some(Silence::at(root)),
  );

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

/// Where a change check over several checkouts keeps its one log unless it is named: under the data root's
/// `testbed/`, named by the set's checkouts in their order, so another set never shares it.
pub fn default_set_log(data_root: &Path, roots: &[PathBuf]) -> PathBuf {
  let joined = roots
    .iter()
    .map(|root| root.to_string_lossy().into_owned())
    .collect::<Vec<_>>()
    .join("\n");
  let hash = format!("{:x}", Sha256::digest(joined.as_bytes()));

  data_root
    .join("testbed")
    .join(fill_shown(&SUITE.check.set_log, &[("hash", &&hash[..8])]))
}
