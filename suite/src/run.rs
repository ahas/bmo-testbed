//! A run of the suite (docs/suite.md): every unit — a test file, a crate's tests, a project of its own,
//! a tool checked by its manifest — in a process of its own, a sweep's width at a time, each under a guard of
//! its own, inside the machine's turn and a fence of its own, each unit's result written out as it ends and the
//! run's last line always its verdict.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bmo_testbed_core::declaration::{Repository, Test};
use bmo_testbed_core::languages::{Cargo, Language, run_folder};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::queue::now_ms;
use bmo_testbed_core::run::log_name;
use bmo_testbed_core::silence::Silence;
use bmo_testbed_core::turn::{self, Lane, NOT_RUN_EXIT_CODE, TestbedTurn, TurnAsk};
use bmo_testbed_core::witness::{RunBegun, RunEnded, RunVerdict, RunWitness};

use crate::cargo::{HeldLock, cargo_verb_line};
use crate::crate_tests::{RunRecord, binaries_once_more, read_record, runner_config, write_down};
use crate::crates::{
  CrateDeclaration, cargo_environment, crate_build_args, library_tests_build_args, set_by_cargo,
  tests_build_args, workspace_crates,
};
use crate::fence::{LiveClear, LiveFence, clear_fence, fence_limits, fence_suite, fenced_build_jobs};
use crate::log::{FailedTest, failed_tests_of, tally_of};
use crate::own_binary::{running_testbed, testbed_line};
use crate::selection::{
  Layout, Selection, changed_paths, glob_matches, select_packages, selected_tests, selection_lines,
};
use crate::sweep::{SweepPlan, largest_first, sweep};
use crate::test_server::{template_name, test_postgres, test_server_settled};
use crate::tool_checks::{CheckedTool, package_check};
use crate::unit::{PerTest, UnitCommand, UnitRun, run_unit};
use crate::workspace::{OwnProject, git_lines, own_projects, project_browser_files, repository_paths};

fn stderr(line: &str) {
  eprintln!("{line}");
}

/// What the caller asked of the run.
#[derive(Clone, Debug, Default)]
pub struct SuiteAsk {
  pub since: Option<String>,
  /// Words naming paths, which pick the units.
  pub picked: Vec<String>,
  /// Every other word, which goes to each test file's run.
  pub extra: Vec<String>,
  /// Where the run keeps its own log and its test run; a run handed none keeps neither.
  pub record: Option<SuiteRecord>,
  /// Whether a unit that ran and failed runs once more as soon as it ends (docs/suite.md, 「What failed runs
  /// once more」).
  pub again: bool,
}

/// Where a run keeps its record: the folder its own log goes under, the words it was asked with as one line,
/// and the witness its test run is kept through.
#[derive(Clone)]
pub struct SuiteRecord {
  pub log_dir: PathBuf,
  pub command: String,
  pub witness: Arc<dyn RunWitness + Send + Sync>,
}

impl std::fmt::Debug for SuiteRecord {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("SuiteRecord")
      .field("log_dir", &self.log_dir)
      .field("command", &self.command)
      .finish()
  }
}

/// What a run came to: the code it ends with, its own log, how long it waited for its turn and ran, the
/// test run its witness kept, and each unit it ran once more.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SuiteEnded {
  pub code: u8,
  pub log: Option<PathBuf>,
  pub waited_seconds: f64,
  pub seconds: f64,
  pub test_run: Option<i64>,
  pub once_more: Vec<UnitOnceMore>,
}

/// A unit that failed and ran once more: the tests its first run named as failing, and whether its second
/// run passed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitOnceMore {
  pub unit: String,
  pub failed_tests: Vec<FailedTest>,
  pub passed: bool,
}

/// The run's words as the binary read them: its own flags, and every other word as given.
#[derive(Clone, Debug, Default)]
pub struct SuiteWords {
  pub since: Option<String>,
  pub words: Vec<String>,
}

/// The declared languages that run test files, in the order the declaration names them.
pub fn file_languages(test: &Test) -> Vec<&'static dyn Language> {
  test
    .languages()
    .into_iter()
    .filter(|language| language.test_command(&[], &[]).is_some())
    .collect()
}

/// The language that runs a test file: the first declared one whose own naming the file carries, or else
/// the first declared one that runs test files.
pub fn runner_of(test: &Test, file: &str) -> Option<&'static dyn Language> {
  let languages = file_languages(test);

  languages
    .iter()
    .copied()
    .find(|language| language.test_naming().iter().any(|naming| glob_matches(naming, file)))
    .or_else(|| languages.first().copied())
}

/// Test files one command runs: the language running them, the folder it runs them from as a path from the
/// repository's root, and each file as a path from that folder.
pub struct FileGroup {
  pub language: &'static dyn Language,
  pub dir: PathBuf,
  pub files: Vec<String>,
}

/// The words that stand a runner's own per-test limit over these files, laid after the caller's `words`
/// (docs/suite.md, 「How long a unit and a test may run」): none where the run drives a browser among them,
/// since a runner that ends one test of such a file takes down the browser every later test of it shares;
/// the language's limit words otherwise. A command handed no limit word keeps none of its own.
pub fn limit_words(language: &dyn Language, words: &[String], seconds: u64, browser: bool) -> Vec<String> {
  if browser { Vec::new() } else { language.per_test_limit(words, seconds) }
}

/// Whether one of these test files, each a path from `within` a folder of the repository, is one the run
/// drives a browser in, as `drives` answers for a path from the repository's root.
pub fn holds_browser(within: &Path, files: &[String], drives: &dyn Fn(&str) -> bool) -> bool {
  files.iter().any(|file| drives(&within.join(file).to_string_lossy()))
}

/// Test files, each a path from the repository's root, gathered into one group for each language that runs
/// them and folder it runs them from (docs/suite.md, 「Which files run」): the languages in the order they are
/// declared, the folders in their own order. A file no declared language runs is in none.
pub fn file_groups(root: &Path, test: &Test, files: &[String]) -> Vec<FileGroup> {
  let mut groups: Vec<FileGroup> = Vec::new();

  for language in file_languages(test) {
    let mut by_folder: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();

    for file in files {
      if runner_of(test, file).is_some_and(|runner| runner.name() == language.name()) {
        let dir = run_folder(language, root, file);
        let from_dir = Path::new(file).strip_prefix(&dir).unwrap_or(Path::new(file));

        by_folder.entry(dir).or_default().push(from_dir.to_string_lossy().into_owned());
      }
    }

    groups.extend(by_folder.into_iter().map(|(dir, files)| FileGroup {
      language,
      dir,
      files,
    }));
  }

  groups
}

/// The language that runs a project's script: the first declared one that runs package scripts.
pub fn script_runner(test: &Test) -> Option<&'static dyn Language> {
  test.languages().into_iter().find(|language| language.script_command("").is_some())
}

// Whether a word names a path: one holding a separator, or a file named as a declared language names its
// test files, by its extension.
fn names_path(word: &str, test: &Test) -> bool {
  let extensions: Vec<&str> = file_languages(test)
    .iter()
    .flat_map(|language| language.test_naming().iter().filter_map(|naming| naming.rsplit_once('.')))
    .map(|(_, extension)| extension)
    .collect();

  !word.starts_with('-')
    && (word.contains('/')
      || word.rsplit_once('.').is_some_and(|(_, extension)| extensions.contains(&extension)))
}

/// The run asked for, the caller's words over what the repository declares for its tests: a word naming a
/// path picking the units, and every other word for the runner.
pub fn ask_of(given: &SuiteWords, test: &Test) -> SuiteAsk {
  // A width for the runner's own pool means nothing to a run of one file, so it is not handed through.
  let for_runner: Vec<&String> =
    given.words.iter().filter(|word| !word.starts_with("--parallel")).collect();

  SuiteAsk {
    since: given.since.clone(),
    picked: for_runner
      .iter()
      .filter(|word| names_path(word, test))
      .map(|word| (*word).clone())
      .collect(),
    extra: for_runner
      .iter()
      .filter(|word| !names_path(word, test))
      .map(|word| (*word).clone())
      .collect(),
    record: None,
    again: false,
  }
}

/// What the log holds of one unit, written whole as it ends: its heading, what it said, and how its process
/// ended where it was ended by a signal or failed having said nothing.
pub fn unit_account(file: &str, ran: &UnitRun) -> String {
  account(mark(ran.code), file, ran)
}

/// What the log holds of a unit's first run where the unit runs once more: its account under the redo mark,
/// which the readers of a log pass over (docs/suite.md, 「What failed runs once more」).
pub fn redo_account(file: &str, ran: &UnitRun) -> String {
  account(&SUITE.file.redo, file, ran)
}

fn account(mark: &str, file: &str, ran: &UnitRun) -> String {
  let said = &SUITE.file;
  let heading = fill_marks(
    &said.heading,
    &[("mark", mark), ("seconds", &format!("{:.1}", ran.seconds)), ("file", file)],
  );
  let ending = match ran.signal {
    Some(signal) => Some(fill_marks(&said.signalled, &[("signal", &signal.to_string())])),
    None if ran.code != 0 && ran.output.trim().is_empty() => {
      Some(fill_marks(&said.silent, &[("code", &ran.code.to_string())]))
    }
    None => None,
  };
  let mut account = format!("\n{heading}\n{}", ran.output);

  if let Some(ending) = ending {
    if !account.ends_with('\n') {
      account.push('\n');
    }
    account.push_str(&ending);
    account.push('\n');
  }

  account
}

/// The one line a run always ends on: how it came out, how long it took and what happened.
pub fn verdict_line(ok: bool, seconds: f64, note: &str) -> String {
  let said = &SUITE.ending;

  fill_marks(
    &said.verdict,
    &[
      ("mark", &SUITE.lines.mark),
      ("outcome", if ok { &said.ok } else { &said.failed }),
      ("seconds", &seconds.round().to_string()),
      ("note", note),
    ],
  )
}

/// What a sweep's units came to, counted, for the verdict it ends on: its files, those that passed and
/// failed, and the tests their own output says they skipped.
#[derive(Clone, Copy, Debug, Default)]
pub struct SweepCount {
  pub files: usize,
  pub passed: usize,
  pub failed: usize,
  pub skipped: u64,
}

/// How a sweep ends: whether it passed, its verdict's clause and its exit code. Tests skipped are named on a
/// pass and a failure alike, so a skip never reads as a bare pass (docs/suite.md, 「A skipped test is said」).
pub fn sweep_ending(count: SweepCount) -> (bool, String, u8) {
  let said = &SUITE.ending;
  let SweepCount {
    files,
    passed,
    failed,
    skipped,
  } = count;
  let ok = passed == files;
  let mut parts = vec![if ok {
    fill_marks(&said.all_passed, &[("files", &files.to_string())])
  } else {
    fill_marks(&said.passed, &[("passed", &passed.to_string()), ("files", &files.to_string())])
  }];

  if failed > 0 {
    parts.push(fill_marks(&said.failed_count, &[("count", &failed.to_string())]));
  }

  if skipped > 0 {
    parts.push(fill_marks(&said.skipped_count, &[("count", &skipped.to_string())]));
  }

  (ok, parts.join(&said.joiner), if ok { 0 } else { 1 })
}

/// Every test file of this repository, the ones git has never seen included, matching the declared test
/// file globs, or where none are declared the declared languages' own naming, outside every project of its
/// own: a test written and not yet committed is the likeliest one to hold the suite. A repository declaring
/// no language that runs test files has none.
pub fn test_files(repo: &Repository) -> Vec<String> {
  let test = &repo.declaration.test;
  let languages = file_languages(test);
  let declared = test.files.as_deref();
  let mut asked = vec!["ls-files", "--cached", "--others", "--exclude-standard"];

  if languages.is_empty() {
    return Vec::new();
  }

  if declared.is_none() {
    asked.extend(languages.iter().flat_map(|language| language.test_naming().iter().copied()));
  }

  let mut found: Vec<String> = git_lines(&repo.root, &asked)
    .unwrap_or_default()
    .into_iter()
    .filter(|file| declared.is_none_or(|globs| globs.iter().any(|glob| glob_matches(glob, file))))
    .collect();

  found.sort();
  found.dedup();
  repository_paths(repo, found)
}

/// Whether a test file, as a path from the repository's root, is one the declaration names as a browser file.
pub(crate) fn browser_file(test: &Test, file: &str) -> bool {
  test.browser.iter().any(|glob| glob_matches(glob, file))
}

// A unit's mark, as the readers of the log find it.
fn mark(code: i32) -> &'static str {
  let said = &SUITE.file;

  if code == 0 { &said.ok } else { &said.failed }
}

// The words of a whole run, or of a run narrowed by words naming units exactly, by directory and a slash.
fn picked_units<T: Clone>(units: &[T], dir: impl Fn(&T) -> &str, picked: &[String]) -> Vec<T> {
  units
    .iter()
    .filter(|one| picked.is_empty() || picked.contains(&format!("{}/", dir(one))))
    .cloned()
    .collect()
}

/// The suite run in the repository's checkout, its lines on standard error; what it came to.
pub fn run(repo: &Repository, ask: &SuiteAsk) -> SuiteEnded {
  run_saying(repo, ask, &stderr)
}

// The run's own log, made under the record's folder; none where the run keeps no record or it cannot be made.
fn own_log(record: Option<&SuiteRecord>) -> Option<(PathBuf, Mutex<File>)> {
  let dir = &record?.log_dir;
  let path = dir.join(log_name(now_ms(), "suite"));

  std::fs::create_dir_all(dir).ok()?;
  File::create(&path).ok().map(|file| (path, Mutex::new(file)))
}

/// The suite run in the repository's checkout, every line it writes about itself and each unit's output
/// handed to `say`, and to its own log where it keeps a record; what it came to.
pub fn run_saying(repo: &Repository, ask: &SuiteAsk, heard: &(dyn Fn(&str) + Sync)) -> SuiteEnded {
  let log = own_log(ask.record.as_ref());
  let teed = |line: &str| {
    heard(line);

    if let Some(Ok(mut file)) = log.as_ref().map(|(_, file)| file.lock()) {
      let _ = writeln!(file, "{line}");
    }
  };
  let say: &(dyn Fn(&str) + Sync) = &teed;
  let ended = |code: u8| SuiteEnded {
    code,
    log: log.as_ref().map(|(path, _)| path.clone()),
    ..SuiteEnded::default()
  };
  let root = repo.root.as_path();
  // The packages a selected run keeps, worked out before the turn is taken so a range that is not a commit
  // is refused at once.
  let selected: Option<(Layout, Selection)> = match &ask.since {
    None => None,
    Some(since) => {
      let changed = match changed_paths(root, since) {
        Ok(changed) => changed,
        Err(why) => {
          say(&fill_marks(&SUITE.selection.not_a_range, &[("since", since), ("why", &why)]));
          return ended(2);
        }
      };
      let layout = Layout::of_checkout(repo).unwrap_or_else(|why| {
        say(&fill_marks(&SUITE.selection.no_crates, &[("why", &why)]));
        Layout::of_parts(repo, Vec::new())
      });

      if layout.packages.is_empty() {
        say(&SUITE.selection.no_packages);
        None
      } else {
        let chosen = select_packages(root, &layout, &changed);

        say(&fill_marks(
          &SUITE.selection.changed,
          &[("count", &changed.len().to_string()), ("since", since)],
        ));

        for line in selection_lines(&layout, &chosen) {
          say(&line);
        }

        Some((layout, chosen))
      }
    }
  };

  // The machine's turn, before anything is timed: a run inside its caller's turn starts at once.
  let env = turn::process_env();
  let turn_command = if selected.is_none() { &SUITE.run.whole } else { &SUITE.run.selected };
  let turn = turn::take_turn(&TurnAsk {
    command: turn_command.clone(),
    lane: Lane::Wide,
    say,
    path: None,
    env: env.clone(),
    checkout: root.to_string_lossy().into_owned(),
    poll: None,
    notice_seconds: None,
  });

  say(&fill_marks(&SUITE.lines.turn, &[("mark", &SUITE.lines.mark), ("words", &turn.words())]));

  // Without the turn nothing starts, and the verdict says the run was not had rather than failed.
  if !turn.had() {
    say(&format!("\n{}", verdict_line(false, 0.0, &SUITE.ending.not_run)));
    return SuiteEnded {
      waited_seconds: turn.waited_seconds(),
      ..ended(NOT_RUN_EXIT_CODE)
    };
  }

  let started = Instant::now();
  let test_run = match (ask.record.as_ref(), log.as_ref()) {
    (Some(record), Some((path, _))) => record.witness.began(&RunBegun {
      command: &record.command,
      checkout: root,
      log: path,
      started_at: now_ms(),
      env: &env,
      whole: ask.since.is_none() && ask.picked.is_empty() && ask.extra.is_empty(),
    }),
    _ => None,
  };
  let (code, once_more) = run_held(repo, ask, selected, &turn, turn_command, &env, say);

  if let (Some(record), Some(id), Some((path, _))) = (ask.record.as_ref(), test_run, log.as_ref()) {
    let tally = tally_of(&std::fs::read_to_string(path).unwrap_or_default());

    record.witness.ended(
      id,
      &RunEnded {
        verdict: match code {
          0 => RunVerdict::Passed,
          NOT_RUN_EXIT_CODE => RunVerdict::NotRun,
          _ => RunVerdict::Failed,
        },
        passed: tally.pass,
        failed: tally.fail,
        skipped: tally.skip,
        ended_at: now_ms(),
      },
    );
  }

  SuiteEnded {
    waited_seconds: turn.waited_seconds(),
    seconds: started.elapsed().as_secs_f64(),
    test_run,
    once_more,
    ..ended(code)
  }
}

fn run_held(
  repo: &Repository,
  ask: &SuiteAsk,
  selected: Option<(Layout, Selection)>,
  turn: &TestbedTurn,
  turn_command: &str,
  env: &HashMap<String, String>,
  say: &(dyn Fn(&str) + Sync),
) -> (u8, Vec<UnitOnceMore>) {
  let root = repo.root.as_path();
  let test = &repo.declaration.test;
  let limits = fence_limits(test.memory_gb, env.get("BMO_SUITE_MEMORY_GB").map(String::as_str));

  say(&fence_suite(limits, &mut LiveFence));

  let threads = turn::machine_threads();
  let pool = turn.width(turn::machine_width(env, threads));
  // Every Cargo command of the run builds at as many jobs as the fence holds, so the linkers of one crate's
  // many test binaries never pass its ceiling together.
  let jobs = fenced_build_jobs(limits, pool, env.get("CARGO_BUILD_JOBS").map(String::as_str));
  let mut handed: Vec<(String, String)> = vec![("CARGO_BUILD_JOBS".to_owned(), jobs.to_string())];

  say(&fill_marks(
    &SUITE.lines.cargo_jobs,
    &[("mark", &SUITE.lines.mark), ("jobs", &jobs.to_string())],
  ));

  if let Some(held) = turn.held_variable() {
    handed.push((held.0.to_owned(), held.1));
  }

  // A repository declaring a test database has the suite's own server standing before the first unit starts,
  // and every unit is handed its address; where it declares migrations, every unit is handed the template its
  // clone comes from too.
  let database = test.database.as_ref();
  let template = template_name(repo);

  if database.is_some() {
    if let Err(why) = test_server_settled() {
      say(&fill_marks(&SUITE.lines.server_down, &[("why", &why)]));
    }

    handed.push(("BMO_TEST_POSTGRES".to_owned(), test_postgres()));
  }

  let template_line = |verb: &str, template: &str| testbed_line(&["template", verb, template]);

  if let Some(template) = &template {
    let sweeping = template_line("sweep", template);

    handed.push(("BMO_TEST_TEMPLATE".to_owned(), template.clone()));

    // A pool whose migrations no checkout makes any longer goes while this run runs.
    let _ = Command::new(&sweeping[0])
      .args(&sweeping[1..])
      .current_dir(root)
      .envs(handed.iter().map(|(name, value)| (name, value)))
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn();

    say(&fill_marks(&SUITE.lines.on_postgres, &[("template", template)]));
  }

  // The crates, the projects of their own and the tools checked by their manifest this run checks, each one
  // unit named by its directory and a slash: every one on a whole run, the ones the change marked on a selected
  // run, and on a run narrowed to paths only those a word names. The crates the run reaches before any word
  // narrows it are the ones its tests are built over, so a run narrowed to one crate stands on the same build.
  let whole_layout = || match workspace_crates(root) {
    Ok(crates) => Layout::of_parts(repo, crates),
    Err(why) => {
      say(&fill_marks(&SUITE.selection.no_crates, &[("why", &why)]));
      Layout::of_parts(repo, Vec::new())
    }
  };
  let whole;
  let (layout, marked): (&Layout, Option<&Selection>) = match &selected {
    Some((layout, chosen)) => (layout, Some(chosen)),
    None => {
      whole = whole_layout();
      (&whole, None)
    }
  };
  let reached_crates: Vec<CrateDeclaration> = layout
    .crates
    .iter()
    .filter(|one| marked.is_none_or(|chosen| chosen.crates.contains_key(&one.name)))
    .cloned()
    .collect();
  let crates: Vec<CrateDeclaration> = picked_units(&reached_crates, |one| one.dir.as_str(), &ask.picked);
  // Every project of its own the checkout holds, which keeps a nested project's files out of the one holding
  // it; and the ones the run checks.
  let every_project: Vec<OwnProject> = match &selected {
    Some((layout, _)) => layout.projects.clone(),
    None => own_projects(repo),
  };
  let projects: Vec<OwnProject> = match &selected {
    Some((layout, chosen)) => picked_units(
      &layout
        .projects
        .iter()
        .filter(|one| chosen.projects.contains_key(&one.dir))
        .cloned()
        .collect::<Vec<_>>(),
      |one| one.dir.as_str(),
      &ask.picked,
    ),
    None => picked_units(&every_project, |one| one.dir.as_str(), &ask.picked),
  };
  // A package declaring a check of its own is checked as a tool is, wherever the package is marked.
  let tools: Vec<CheckedTool> = match &selected {
    Some((layout, chosen)) => picked_units(
      &layout
        .tools
        .iter()
        .filter(|one| chosen.tools.contains_key(&one.dir))
        .cloned()
        .chain(
          layout
            .packages
            .iter()
            .filter(|one| chosen.marked.contains_key(&one.name))
            .filter_map(package_check),
        )
        .collect::<Vec<_>>(),
      |one| one.dir.as_str(),
      &ask.picked,
    ),
    None => picked_units(
      &layout
        .tools
        .iter()
        .cloned()
        .chain(layout.packages.iter().filter_map(package_check))
        .collect::<Vec<_>>(),
      |one| one.dir.as_str(),
      &ask.picked,
    ),
  };
  let unit_of = |dir: &str| format!("{dir}/");
  let crate_units: HashMap<String, CrateDeclaration> =
    crates.iter().map(|one| (unit_of(&one.dir), one.clone())).collect();
  let project_units: HashMap<String, OwnProject> =
    projects.iter().map(|one| (unit_of(&one.dir), one.clone())).collect();
  let tool_units: HashMap<String, CheckedTool> =
    tools.iter().map(|one| (unit_of(&one.dir), one.clone())).collect();
  // The words naming test files by a part of their path: every picked word but one naming a unit.
  let file_words: Vec<&String> = ask
    .picked
    .iter()
    .filter(|word| {
      !crate_units.contains_key(*word)
        && !project_units.contains_key(*word)
        && !tool_units.contains_key(*word)
    })
    .collect();

  // The crates' tests are built over every crate the run reaches, the runner Cargo hands each test binary
  // writing it down into the run's own record, which each crate's unit runs from (docs/suite.md, 「Heavy units
  // run in a lane of one」).
  let reached: Vec<String> = reached_crates.iter().map(|one| one.name.clone()).collect();
  let record = RunRecord::fresh();
  let tests_build = tests_build_args(&reached, &runner_config(&running_testbed(), record.path()));

  // Where the run tests a crate, the workspace's lock with its path packages brought to their manifests from
  // before the first build to the run's end: the crates' tests run outside the Cargo verb, which aligns the lock
  // around its own command alone, and may run Cargo themselves (docs/deploy.md, 「The Cargo build directory」).
  // It is put back as it was found once the run ends.
  let held_lock = (!crates.is_empty()).then(|| HeldLock::aligned(root, &tests_build)).flatten();

  if let Some(said) = held_lock.as_ref().and_then(HeldLock::said) {
    say(&said);
  }

  // What the run's Rust stands on, built once before any guard is counted, through the repository's Cargo verb,
  // which removes what the build superseded; the code each build ended with.
  let prebuild = |building: &dyn Fn(&str) -> String, args: Vec<String>| -> i32 {
    let line = cargo_verb_line(&args);
    let built = Command::new(&line[0])
      .args(&line[1..])
      .current_dir(root)
      .envs(handed.iter().map(|(name, value)| (name, value)))
      .stdin(Stdio::null())
      .output();
    let (code, said) = match built {
      Ok(done) => (done.status.code().unwrap_or(1), String::from_utf8_lossy(&done.stderr).into_owned()),
      Err(why) => (127, why.to_string()),
    };

    say(&format!("{}\n{said}", building(&code.to_string())));
    code
  };
  let count = &reached.len().to_string();
  let building_tests = |what: &'static str| {
    move |code: &str| {
      fill_marks(&SUITE.lines.building_tests, &[("what", what), ("count", count), ("code", code)])
    }
  };

  // A library and its own tests compile the same source twice, a crate's two heaviest compiles, which Cargo
  // would start side by side; the libraries' tests go first and alone, so the two never meet in the fence.
  let tests_built: Option<i32> = (!crates.is_empty()).then(|| {
    if reached_crates.iter().any(|one| one.library) {
      prebuild(&building_tests(&SUITE.lines.library_tests), library_tests_build_args(&reached));
    }

    prebuild(&building_tests(&SUITE.lines.tests), tests_build.clone())
  });

  // The browser test files each project of its own the run checks declares for itself, each with the unit of
  // its project: they run in the browser lane once the project's own check, which readies the project, has
  // ended.
  let project_browser: Vec<(String, String)> = projects
    .iter()
    .flat_map(|project| {
      project_browser_files(root, project, &every_project)
        .into_iter()
        .map(|file| (file, unit_of(&project.dir)))
    })
    .collect();
  // Whether this run drives a browser for a file, as a path from the repository's root: one the declaration
  // names as a browser file, or one of the projects it checks declares for itself. Such a file runs in the
  // browser lane, and its runner is handed no per-test limit of its own.
  let drives_browser =
    |file: &str| browser_file(test, file) || project_browser.iter().any(|(one, _)| one == file);

  // The TypeScript test files the run starts. A file git names that is already gone stays in.
  let tests = test_files(repo);
  let in_scope: Vec<String> = match &selected {
    Some((layout, chosen)) => selected_tests(layout, chosen, &tests),
    None => tests,
  }
  .into_iter()
  .filter(|file| ask.picked.is_empty() || file_words.iter().any(|part| file.contains(part.as_str())))
  .collect();

  // Only for a run that starts a TypeScript test: the shared target directory has one lock, which another
  // build holds for minutes, and a run with nothing to drive a binary never queues behind it.
  if !in_scope.is_empty() {
    for one in driven_crates(ask, layout, marked) {
      let building = |code: &str| {
        fill_marks(
          &SUITE.lines.building,
          &[("what", &SUITE.lines.binaries), ("crate", &one.name), ("code", code)],
        )
      };

      prebuild(&building, crate_build_args(&one.name, &one.binaries));
    }
  }

  // Every file stands on a clone, so the template is made before the first starts.
  if let Some(template) = &template {
    let making = template_line("make", template);
    let made = Command::new(&making[0])
      .args(&making[1..])
      .current_dir(root)
      .envs(handed.iter().map(|(name, value)| (name, value)))
      .stdin(Stdio::null())
      .output();

    say(&match made {
      Ok(done) if done.status.success() => SUITE.lines.template_stands.clone(),
      Ok(done) => {
        fill_marks(&SUITE.lines.template_failed, &[("why", String::from_utf8_lossy(&done.stderr).trim())])
      }
      Err(why) => fill_marks(&SUITE.lines.template_failed, &[("why", &why.to_string())]),
    });
  }

  let began = Instant::now();
  let took = || began.elapsed().as_secs_f64();
  // Each unit holds connections of the suite's own server, which every run at the same time shares.
  let width = database.map_or(pool, |database| pool.min(database.max_files.max(1) as usize));
  let browser = if test.browser.is_empty() {
    SUITE.lines.no_browser.clone()
  } else {
    test.browser.join(", ")
  };
  let mut files: Vec<String> = project_units.keys().chain(tool_units.keys()).cloned().collect();

  files.sort();

  let mut crate_files: Vec<String> = crates.iter().map(|one| unit_of(&one.dir)).collect();

  files.append(&mut crate_files);
  files.extend(project_browser.iter().map(|(file, _)| file.clone()));
  files.extend(largest_first(in_scope, |file| {
    std::fs::metadata(root.join(file)).map_or(0, |held| held.len())
  }));

  say(&fill_marks(
    &SUITE.lines.plan,
    &[
      ("count", &files.len().to_string()),
      ("width", &width.to_string()),
      ("browser", &browser),
    ],
  ));

  let cargo_env = cargo_environment(root);
  let silence = test.silence();
  let per_test_seconds = test.per_test_seconds();
  // No unit inherits what a Cargo running this suite set for its own package: a unit that runs Cargo would
  // build as though for that package (docs/deploy.md, 「The Cargo build directory」). Nor does it inherit the
  // deployment's own variables, which name the live data rather than the test's (docs/suite.md, 「What a test
  // unit is handed」); what the suite names for the unit itself is handed after them.
  let unit_env = || -> Vec<(String, String)> {
    std::env::vars()
      .filter(|(name, _)| !set_by_cargo(name) && !SUITE.environment.stripped.contains(name))
      .chain(handed.iter().cloned())
      .collect()
  };
  // A crate's unit running the test binaries a record holds for it: Cargo itself, in the checkout's own build
  // environment, with a home of its own as every test file has. Rust's test harness keeps no per-test limit,
  // so the run keeps it over the crate's tests.
  let crate_command = |one: &CrateDeclaration, binaries: &Path| -> UnitCommand {
    let home =
      std::env::temp_dir().join(format!("bmo-testbed-crate-home-{}-{}", std::process::id(), one.name));
    let _ = std::fs::create_dir_all(&home);
    let mut env = unit_env();

    env.extend(cargo_env.iter().cloned());
    env.push(("HOME".to_owned(), home.to_string_lossy().into_owned()));

    UnitCommand {
      program: running_testbed().to_string_lossy().into_owned(),
      args: vec![
        "crate-tests".to_owned(),
        "run".to_owned(),
        binaries.to_string_lossy().into_owned(),
        one.name.clone(),
      ],
      cwd: root.to_path_buf(),
      env,
      home: Some(home),
      silence: Silence::default(),
      per_test: Some(PerTest {
        language: &Cargo,
        limit: Duration::from_secs(per_test_seconds),
      }),
    }
  };
  // Test files run from the folder of their runner's configuration under `under`, a folder of the repository,
  // with the run's words and its per-test limit, save where a browser runs among the files; none where one
  // command cannot run them all.
  let files_command = |under: &Path, files: &[String]| -> Option<UnitCommand> {
    let mut groups = file_groups(under, test, files).into_iter();
    let group = groups.next()?;

    if groups.next().is_some() || group.files.len() != files.len() {
      return None;
    }

    let within = under.strip_prefix(root).unwrap_or(Path::new(""));
    let browser = holds_browser(within, files, &drives_browser);
    let mut words = ask.extra.clone();

    words.extend(limit_words(group.language, &ask.extra, per_test_seconds, browser));

    let line = group.language.test_command(&group.files, &words)?;

    Some(UnitCommand::of(line, under.join(group.dir), unit_env(), None))
  };
  let unit_command = |file: &String| -> UnitCommand {
    let base = unit_env();

    if let Some(project) = project_units.get(file) {
      // A project with no runner of its script is said before its run is asked for.
      let line = script_runner(test)
        .and_then(|runner| runner.script_command(&repo.declaration.projects.check_script))
        .unwrap_or_default();

      return UnitCommand::of(line, root.join(&project.dir), base, None);
    }

    if let Some(tool) = tool_units.get(file)
      && let Ok((command, handed_to_it)) = tool.command()
      && let Some((program, args)) = command.split_first()
    {
      let mut env = base;

      env.extend(handed_to_it);

      return UnitCommand {
        program: program.clone(),
        args: args.to_vec(),
        cwd: root.join(&tool.dir),
        env,
        home: None,
        silence: Silence::default(),
        per_test: None,
      };
    }

    if let Some(one) = crate_units.get(file) {
      return crate_command(one, record.path());
    }

    // A test file listed has a runner, since a repository with none lists no test file; it runs from the
    // folder of its runner's configuration.
    files_command(root, std::slice::from_ref(file))
      .unwrap_or_else(|| UnitCommand::of(Vec::new(), root.to_path_buf(), base, None))
  };
  // What a unit that failed runs once more, read from its first run, and what the line before it says runs
  // (docs/suite.md, 「What failed runs once more」): a crate's failed binaries narrowed to their failed tests,
  // standing in a record of their own, a project's failed test files, or else the whole unit.
  let again_said = &SUITE.again;
  let once_more_of = |file: &String, first: &UnitRun| -> (UnitCommand, String, Option<RunRecord>) {
    let whole = || (unit_command(file), again_said.whole.clone(), None);

    if let Some(one) = crate_units.get(file) {
      let narrowed = read_record(record.path())
        .map(|binaries| binaries_once_more(&binaries, &one.name, &first.output))
        .unwrap_or_default();
      let own = record.beside(&one.name);

      if narrowed.is_empty() || narrowed.iter().any(|again| write_down(own.path(), &again.binary).is_err())
      {
        return whole();
      }

      let what: Vec<String> = narrowed
        .iter()
        .map(|again| {
          if again.tests.is_empty() {
            let executable = &again.binary.executable;
            let shown = Path::new(executable)
              .file_name()
              .map_or_else(|| executable.clone(), |name| name.to_string_lossy().into_owned());

            fill_marks(&again_said.binary, &[("binary", &shown)])
          } else {
            fill_marks(&again_said.tests, &[("names", &again.tests.join(&again_said.joiner))])
          }
        })
        .collect();

      return (crate_command(one, own.path()), what.join(&again_said.joiner), Some(own));
    }

    if let Some(project) = project_units.get(file) {
      let under = root.join(&project.dir);
      let mut failed: Vec<String> = Vec::new();

      for named in file_languages(test).iter().flat_map(|language| language.failed_files(&first.output)) {
        if !failed.contains(&named) {
          failed.push(named);
        }
      }

      let held = !failed.is_empty() && failed.iter().all(|one| under.join(one).is_file());

      if let Some(command) = held.then(|| files_command(&under, &failed)).flatten() {
        let what = fill_marks(&again_said.files, &[("names", &failed.join(&again_said.joiner))]);

        return (command, what, None);
      }
    }

    whole()
  };
  // Each unit that failed and ran once more.
  let ran_once_more: Mutex<Vec<UnitOnceMore>> = Mutex::new(Vec::new());
  // The tests the units skipped, by the counts their own output gives.
  let skipped = AtomicU64::new(0);
  let file_run = |file: &String| -> i32 {
    let no_check = match (project_units.get(file), tool_units.get(file)) {
      (Some(project), _) if !project.check => {
        Some(fill_marks(&SUITE.file.no_check, &[("script", &repo.declaration.projects.check_script)]))
      }
      (Some(_), _) if script_runner(test).is_none() => Some(fill_marks(
        &SUITE.file.no_runner,
        &[("script", &repo.declaration.projects.check_script)],
      )),
      (_, Some(tool)) => tool.check.clone().err(),
      _ => tests_built
        .filter(|code| *code != 0 && crate_units.contains_key(file))
        .map(|code| fill_marks(&SUITE.file.unbuilt_crate, &[("code", &code.to_string())])),
    };

    if let Some(why) = no_check {
      say(&format!(
        "\n{}\n{why}\n",
        fill_marks(&SUITE.file.heading, &[("mark", mark(1)), ("seconds", "0.0"), ("file", file)]),
      ));

      return 1;
    }

    let ran = run_unit(&UnitCommand {
      silence,
      ..unit_command(file)
    });

    if ran.code == 0 || !ask.again {
      skipped.fetch_add(tally_of(&ran.output).skip, Ordering::Relaxed);
      say(&unit_account(file, &ran));

      return ran.code;
    }

    // A unit that failed runs once more at once, in its own place of the sweep, and stands on that run.
    let (command, what, _own_record) = once_more_of(file, &ran);

    say(&format!(
      "{}\n{}",
      redo_account(file, &ran).trim_end(),
      fill_marks(&again_said.once_more, &[("mark", &SUITE.lines.mark), ("unit", file), ("what", &what)])
    ));

    let second = run_unit(&UnitCommand { silence, ..command });

    skipped.fetch_add(tally_of(&second.output).skip, Ordering::Relaxed);
    say(&unit_account(file, &second));

    if let Ok(mut ran_once_more) = ran_once_more.lock() {
      ran_once_more.push(UnitOnceMore {
        unit: file.clone(),
        failed_tests: failed_tests_of(&unit_account(file, &ran)),
        passed: second.code == 0,
      });
    }

    second.code
  };
  let lane = |file: &String| -> Option<String> {
    if drives_browser(file) {
      Some("browser".to_owned())
    } else if project_units.contains_key(file)
      || tool_units.contains_key(file)
      || crate_units.contains_key(file)
    {
      Some("heavy".to_owned())
    } else {
      None
    }
  };
  let after = |file: &String| -> Option<usize> {
    let (_, project) = project_browser.iter().find(|(one, _)| one == file)?;

    files.iter().position(|one| one == project)
  };
  let codes = sweep(
    &files,
    &SweepPlan {
      width,
      lane: &lane,
      after: &after,
    },
    &file_run,
  );
  let failed: Vec<&str> = files
    .iter()
    .zip(&codes)
    .filter(|(_, code)| **code != 0)
    .map(|(file, _)| file.as_str())
    .collect();
  let passed = codes.len() - failed.len();
  let said = &SUITE.ending;
  let joined = |names: &[&str]| names.join(&said.joiner);

  say(&format!(
    "\n{}",
    fill_marks(
      &said.summary,
      &[
        ("files", &files.len().to_string()),
        ("passed", &passed.to_string()),
        ("failed", &failed.len().to_string()),
      ],
    )
  ));

  if !failed.is_empty() {
    say(&fill_marks(&said.failed_files, &[("files", &joined(&failed))]));
  }

  let count = SweepCount {
    files: files.len(),
    passed,
    failed: failed.len(),
    skipped: skipped.load(Ordering::Relaxed),
  };
  let (ok, note, code) = sweep_ending(count);

  // The last thing written, always: what the run came to, on one line a caller can read on its own.
  if let Some(cleared) = clear_fence(&mut LiveClear) {
    say(&cleared);
  }

  say(&format!("\n{}", verdict_line(ok, took(), &note)));
  turn::record_turn_times(env, turn_command, Lane::Wide, turn.waited_seconds(), took());

  (code, ran_once_more.into_inner().unwrap_or_default())
}

// The crates whose binaries the run's test files start: the ones a package's `runs` names, every package's on a
// whole run and the marked ones' on a selected run; none on a run a word narrows (docs/suite.md, 「Heavy units run
// in a lane of one」).
fn driven_crates(ask: &SuiteAsk, layout: &Layout, marked: Option<&Selection>) -> Vec<CrateDeclaration> {
  if !ask.picked.is_empty() {
    return Vec::new();
  }

  let launched: Vec<String> = layout
    .packages
    .iter()
    .filter(|one| marked.is_none_or(|chosen| chosen.marked.contains_key(&one.name)))
    .flat_map(|one| one.runs.iter().map(|run| run.trim_end_matches('/').to_owned()))
    .collect();

  layout
    .crates
    .iter()
    .filter(|one| launched.contains(&one.dir) && !one.binaries.is_empty())
    .cloned()
    .collect()
}
