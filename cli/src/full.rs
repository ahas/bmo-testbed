//! `full`: the daily whole run of a repository's suite (docs/suite.md, 「The daily whole run」). It takes the
//! main branch's commit, never what the checkout it is run in stands on, puts its own tree on it and sets
//! it up, runs the static check there, then every unit through that tree's suite, waiting its turn like
//! any run. A selected run trusts what each package declares; this run catches a dependency no declaration
//! names within a day. A run that fails sends a `full-suite-failed` event.
//!
//! It never runs in its caller's process: called from anywhere but its own unit, it starts that unit and
//! ends. One run at a time to a repository; a second meeting a live one is refused.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::main_branch::MainBranch;
use bmo_testbed_core::policy::daily::DAILY;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::queue::{LockWait, now_ms, take_lock};
use bmo_testbed_core::turn::{Env, NOT_RUN_EXIT_CODE, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_deploy::expansion::{TreeStore, bring_submodules, lay_sibling_trees};
use bmo_testbed_deploy::release::beside;
use bmo_testbed_deploy::run::{run_quiet, stamp_for_name, words};
use bmo_testbed_deploy::unit::{DeployUnitStart, deploy_unit_of_process, start_deploy_unit, this_again};
use bmo_testbed_suite::crates::{owning_crate, workspace_crates};
use bmo_testbed_suite::fence::SUITE_SLICE;
use bmo_testbed_suite::log::{failed_files_of, unstarted_of, verdict_of};
use bmo_testbed_suite::static_check::{check_steps, run_static_check, spawn_step};
use bmo_testbed_suite::workspace::{owning_package, workspace_declarations};
use chrono::{DateTime, Days, NaiveTime, TimeZone};
use serde_json::json;

use crate::suite::repository;
use crate::words::FullArgs;

/// What a run's unit is called by. It is not a deployment's, so it does not carry a deployment's prefix.
pub const UNIT_PREFIX: &str = "bmo-full-suite-";
/// The variable naming another tree for the run.
pub const FULL_CHECK_VARIABLE: &str = "BMO_FULL_CHECK_DIR";

/// A fresh unit name for one run.
pub fn unit_name(now_ms: i64) -> String {
  format!("{UNIT_PREFIX}{now_ms}")
}

/// The repository's own folder of runs: its logs and the moment the last run started.
pub fn runs_folder(data_root: &Path, repository: &str) -> PathBuf {
  data_root.join("testbed").join("suite-full").join(repository)
}

/// The moment a run started, written where `--due` reads it.
pub fn write_start(folder: &Path, at_ms: i64) {
  let at = DateTime::from_timestamp_millis(at_ms).unwrap_or_default();
  let _ = std::fs::create_dir_all(folder);
  let _ = std::fs::write(folder.join("started"), format!("{}\n", at.to_rfc3339()));
}

/// The moment the last run started, or none where none is written or what is written does not read.
pub fn read_start(folder: &Path) -> Option<i64> {
  let text = std::fs::read_to_string(folder.join("started")).ok()?;

  DateTime::parse_from_rfc3339(text.trim()).ok().map(|at| at.timestamp_millis())
}

/// The latest moment at or before `now` the clock showed `due`: today's where it has passed, yesterday's
/// otherwise.
pub fn latest_due<Tz: TimeZone>(due: NaiveTime, now: &DateTime<Tz>) -> Option<DateTime<Tz>> {
  let today = now.date_naive().and_time(due);
  let at = if today <= now.naive_local() { today } else { today.checked_sub_days(Days::new(1))? };

  now.timezone().from_local_datetime(&at).earliest()
}

/// One round of the suite: its exit code and what it wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Round {
  pub code: i32,
  pub output: String,
}

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullOutcome {
  Passed,
  NoHead,
  TreeFailed,
  CheckFailed,
  SuiteFailed,
}

/// Everything a run reaches, so a test holds each path.
pub trait FullHands {
  /// The main branch's head (docs/declaration.md, 「The main branch's head」), which the run checks; none where
  /// it could not be read.
  fn head(&self) -> Option<String>;
  /// The run's own tree put on the commit and set up.
  fn prepare(&self, commit: &str) -> Result<(), String>;
  /// The static check over the whole tree: the failing step's id and its line, none where it passed.
  fn static_check(&self) -> Option<(String, String)>;
  /// Every unit of the tree, or with `only` those alone, by name: the ones a round before never started.
  fn suite(&self, only: &[String]) -> Round;
  /// The package or crate a unit stands in, by name; none where it stands in none.
  fn package_of(&self, unit: &str) -> Option<String>;
  /// The event sent.
  fn tell(&self, event: Event<'_>);
  fn say(&self, line: &str);
}

fn short(commit: &str) -> &str {
  &commit[..commit.len().min(12)]
}

// The `full-suite-failed` event: where it ran, what failed, each failing unit with its package, and the log.
fn failed_event(
  main: &str,
  head: Option<&str>,
  verdict: &str,
  step: Option<&str>,
  failed: &[(String, Option<String>)],
  log: &str,
) -> Event<'static> {
  let said = &DAILY.account;
  let files: Vec<String> = failed
    .iter()
    .map(|(unit, package)| {
      fill_marks(&said.file, &[("file", unit), ("unit", package.as_deref().unwrap_or(&said.outside))])
    })
    .collect();
  let at = head.map_or_else(
    || fill_marks(&said.no_head, &[("main", main)]),
    |head| fill_marks(&said.at_head, &[("main", main), ("head", short(head))]),
  );
  let summary = fill_marks(
    &said.summary,
    &[
      ("where", &at),
      ("verdict", verdict),
      (
        "files",
        &if files.is_empty() {
          String::new()
        } else {
          fill_marks(&said.files, &[("files", &files.join(&said.joiner))])
        },
      ),
      ("log", log),
    ],
  );

  Event {
    name: "full-suite-failed",
    commit: head.map(str::to_owned),
    summary,
    detail: json!({
      "head": head,
      "log": log,
      "step": step,
      "verdict": verdict,
      "failures": failed.iter().map(|(unit, package)| json!({ "unit": unit, "package": package })).collect::<Vec<_>>(),
    }),
  }
}

/// One line the run writes into its own log, so a run that failed before its suite wrote anything still
/// names why (docs/suite.md, 「The lines the run writes about itself」).
fn record(log: &str, line: &str) {
  if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
    let _ = writeln!(file, "{line}");
  }
}

/// One whole run, in order: the main branch's head, the tree, the static check, every unit with the units a
/// round never started run again by name; a failure is sent as an event. Its exit code and how it ended.
pub fn run_full(hands: &impl FullHands, main: &str, log: &str) -> (u8, FullOutcome) {
  let said = &DAILY.run;
  let Some(head) = hands.head() else {
    let line = fill_marks(&said.no_head, &[("main", main)]);

    hands.say(&line);
    record(log, &line);
    hands.tell(failed_event(
      main,
      None,
      &fill_marks(&said.no_head_verdict, &[("main", main)]),
      None,
      &[],
      log,
    ));

    return (1, FullOutcome::NoHead);
  };

  hands.say(&fill_marks(&said.begins, &[("head", short(&head)), ("log", log)]));

  if let Err(why) = hands.prepare(&head) {
    let line = fill_marks(&said.failed, &[("why", &why)]);

    hands.say(&line);
    record(log, &line);
    hands.tell(failed_event(main, Some(&head), &why, Some(&DAILY.run.setup_step), &[], log));

    return (1, FullOutcome::TreeFailed);
  }

  if let Some((step, line)) = hands.static_check() {
    hands.say(&line);
    record(log, &line);
    hands.tell(failed_event(main, Some(&head), &line, Some(&step), &[], log));

    return (1, FullOutcome::CheckFailed);
  }

  let mut rounds = vec![hands.suite(&[])];
  let mut unstarted = unstarted_of(&rounds[0].output);

  // Units that never started failed nothing and have no result yet, so they run again by name, for as long
  // as each round starts more of them than the one before.
  while !unstarted.is_empty() {
    hands.say(&fill_marks(
      &said.again,
      &[("count", &unstarted.len().to_string()), ("files", &unstarted.join(", "))],
    ));
    rounds.push(hands.suite(&unstarted));

    let left = unstarted_of(&rounds[rounds.len() - 1].output);

    if left.len() >= unstarted.len() {
      break;
    }

    unstarted = left;
  }

  let last = &rounds[rounds.len() - 1];
  let line = rounds
    .iter()
    .map(|round| {
      verdict_of(&round.output)
        .unwrap_or_else(|| fill_marks(&said.no_verdict, &[("code", &round.code.to_string())]))
    })
    .collect::<Vec<_>>()
    .join(&said.joiner);
  // Every round but the last fell short only of units it never started, and the last passed.
  let ok = last.code == 0
    && verdict_of(&last.output).is_some()
    && rounds[..rounds.len() - 1]
      .iter()
      .all(|round| round.code == i32::from(NOT_RUN_EXIT_CODE));

  hands.say(&fill_marks(&said.pass, &[("line", &line)]));

  if ok {
    return (0, FullOutcome::Passed);
  }

  let mut units: Vec<String> = Vec::new();

  for unit in rounds.iter().flat_map(|round| failed_files_of(&round.output)) {
    if !units.contains(&unit) {
      units.push(unit);
    }
  }

  let failed: Vec<(String, Option<String>)> =
    units.into_iter().map(|unit| (unit.clone(), hands.package_of(&unit))).collect();

  hands.tell(failed_event(main, Some(&head), &line, None, &failed, log));
  (1, FullOutcome::SuiteFailed)
}

/// A tree put on a commit of the repository at `source`: made as a detached worktree where it is not there
/// yet, moved to the commit where it is, with whatever an earlier run left untracked cleared, ignored files
/// staying; its submodules forced onto their pins; and set up by `setup`.
pub fn prepare_tree(
  source: &Path,
  commit: &str,
  tree: &Path,
  data_root: &Path,
  setup: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
  let said = &DAILY.tree;
  let at = tree.to_string_lossy().into_owned();

  // A tree laid by another repository, as when a merged repository takes over a deploy root another one laid
  // its tree beside, can never move to this repository's commit: it is taken away and laid again from here.
  if tree.join(".git").exists() && !same_repository(tree, source) {
    let owner = common_dir(tree).and_then(|dir| dir.parent().map(Path::to_path_buf));
    let cwd = owner.as_deref().unwrap_or(source);

    run_quiet(&words(["git", "worktree", "remove", "--force", &at]), Some(cwd));

    if tree.exists() {
      let _ = std::fs::remove_dir_all(tree);
    }
  }

  if tree.join(".git").exists() {
    let moved = run_quiet(&words(["git", "checkout", "-q", "--force", "--detach", commit]), Some(tree));

    if !moved.ok {
      return Err(fill_marks(
        &said.not_moved,
        &[("tree", &at), ("commit", short(commit)), ("said", &moved.out)],
      ));
    }

    run_quiet(&words(["git", "clean", "-fdq"]), Some(tree));
  } else {
    run_quiet(&words(["git", "worktree", "prune"]), Some(source));

    let made =
      run_quiet(&words(["git", "worktree", "add", "--force", "--detach", &at, commit]), Some(source));

    if !made.ok {
      return Err(fill_marks(
        &said.not_made,
        &[("tree", &at), ("commit", short(commit)), ("said", &made.out)],
      ));
    }
  }

  let run = |line: &[String], cwd: &Path| run_quiet(line, Some(cwd));

  bring_submodules(tree, data_root, &run).map_err(|why| {
    fill_marks(&said.not_moved, &[("tree", &at), ("commit", short(commit)), ("said", &why)])
  })?;

  setup(tree)
}

// The git directory a checkout's history is kept in, shared by every worktree of one repository.
fn common_dir(checkout: &Path) -> Option<PathBuf> {
  let ran = run_quiet(
    &words(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"]),
    Some(checkout),
  );

  ran.ok.then(|| PathBuf::from(ran.out.trim())).and_then(|dir| dir.canonicalize().ok())
}

// Whether two checkouts are worktrees of one repository, so one's commits stand in the other's history.
fn same_repository(one: &Path, other: &Path) -> bool {
  match (common_dir(one), common_dir(other)) {
    (Some(one), Some(other)) => one == other,
    _ => false,
  }
}

/// The committed trees the daily whole run's tree reads its sibling trees from: `-trees` beside the tree, its
/// own, apart from a deployment's.
pub fn sibling_store(tree: &Path) -> PathBuf {
  beside(tree, "-trees")
}

/// The tree a repository's daily whole run stands in where `BMO_FULL_CHECK_DIR` names none: `full-check` beside
/// its deploy tree, or `tree` in its own folder of runs where it declares no deploy root.
pub fn own_tree(repo: &Repository, env: &Env, data_root: &Path) -> PathBuf {
  repo
    .declaration
    .deploy
    .root
    .as_ref()
    .map(|root| repo.path(root, env).join("full-check"))
    .unwrap_or_else(|| runs_folder(data_root, &repo.name()).join("tree"))
}

/// The run's hands on this machine: the main branch read in the repository, its own tree, the log, and the hook.
pub(crate) struct Machine {
  pub(crate) source: PathBuf,
  pub(crate) tree: PathBuf,
  pub(crate) log: PathBuf,
  pub(crate) main: String,
  pub(crate) hook: Hook,
}

impl Machine {
  fn append(&self, text: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&self.log) {
      let _ = file.write_all(text.as_bytes());
    }
  }

  // The tree's own declaration, which its setup, its static check and its units are read from.
  fn tree_repository(&self) -> Result<Repository, String> {
    Repository::read(&self.tree).map_err(|refusal| refusal.to_string())
  }
}

impl FullHands for Machine {
  fn head(&self) -> Option<String> {
    let main = MainBranch::new(&self.source, &self.main);

    // A remote that cannot be asked leaves the head to what this machine already knows of it.
    let _ = main.fetch();

    main.head().inspect_err(|why| self.append(&format!("{why}\n"))).ok()
  }

  fn prepare(&self, commit: &str) -> Result<(), String> {
    let data_root = data_root();

    prepare_tree(&self.source, commit, &self.tree, &data_root, &|tree| {
      let repo = self.tree_repository()?;
      let run = |line: &[String], cwd: &Path| run_quiet(line, Some(cwd));

      // The trees its tests read by a path no manifest names stand beside the tree as they stand beside the
      // checkout, at their main heads, before the setup, which may read them too.
      lay_sibling_trees(
        tree,
        &self.source,
        &repo.declaration.test.sibling_trees,
        &TreeStore {
          store: &sibling_store(tree),
          data_root: &data_root,
        },
        &run,
      )?;

      for line in &repo.declaration.test.setup {
        let ran = run_quiet(&words(["sh", "-c", line]), Some(tree));

        if !ran.ok {
          return Err(fill_marks(&DAILY.tree.setup_failed, &[("command", line), ("said", &ran.out)]));
        }
      }

      Ok(())
    })
  }

  fn static_check(&self) -> Option<(String, String)> {
    let repo = match self.tree_repository() {
      Ok(repo) => repo,
      Err(why) => return Some((DAILY.run.setup_step.clone(), why)),
    };
    let mut failing = String::new();
    let line = run_static_check(
      &self.tree,
      &check_steps(&repo, None),
      &mut |text| self.append(text),
      &spawn_step,
      &mut |done| {
        if done.code != 0 && failing.is_empty() {
          failing = done.id.clone();
        }
      },
    )?;

    Some((failing, line))
  }

  fn suite(&self, only: &[String]) -> Round {
    let mut line = vec![this_again()[0].clone(), "suite".to_owned()];

    line.extend(only.iter().cloned());

    let done = std::process::Command::new(&line[0])
      .args(&line[1..])
      .current_dir(&self.tree)
      .stdin(Stdio::null())
      .output();
    let (code, output) = match done {
      Ok(done) => (
        done.status.code().unwrap_or(1),
        format!("{}{}", String::from_utf8_lossy(&done.stdout), String::from_utf8_lossy(&done.stderr)),
      ),
      Err(why) => (127, why.to_string()),
    };

    self.append(&output);
    Round { code, output }
  }

  fn package_of(&self, unit: &str) -> Option<String> {
    let path = unit.trim_end_matches('/');

    owning_package(&workspace_declarations(&self.tree), path)
      .map(|package| package.name.clone())
      .or_else(|| {
        let crates = workspace_crates(&self.tree).ok()?;

        owning_crate(&crates, path).map(|one| one.name.clone())
      })
  }

  fn tell(&self, event: Event<'_>) {
    self.hook.send(&event);
  }

  fn say(&self, line: &str) {
    eprintln!("{line}");
  }
}

/// `full`: started in a unit of its own, or with `--due` only where no run started since that time of day;
/// inside its unit, the run, one at a time to a repository.
pub fn full(args: &FullArgs) -> u8 {
  let said = &DAILY.run;
  let env = process_env();
  let repo = match repository() {
    Ok(repo) => repo,
    Err(code) => return code,
  };
  let name = repo.name();
  let folder = runs_folder(&data_root(), &name);
  let tree = env
    .get(FULL_CHECK_VARIABLE)
    .filter(|named| !named.is_empty())
    .map_or_else(|| own_tree(&repo, &env, &data_root()), PathBuf::from);
  let now = now_ms();

  if deploy_unit_of_process().is_none() {
    if let Some(due) = args.due {
      let since = latest_due(due, &chrono::Local::now()).map_or(i64::MAX, |at| at.timestamp_millis());

      if let Some(started) = read_start(&folder).filter(|started| *started >= since) {
        let at = DateTime::from_timestamp_millis(started).unwrap_or_default().to_rfc3339();

        println!(
          "{}",
          fill_marks(&said.not_due, &[("started", &at), ("due", &due.format("%H:%M").to_string())])
        );
        return 0;
      }
    }

    let unit = unit_name(now);
    let mut handed = vec![
      ("BMO_DATA_DIR".to_owned(), data_root().to_string_lossy().into_owned()),
      (FULL_CHECK_VARIABLE.to_owned(), tree.to_string_lossy().into_owned()),
    ];

    for variable in [
      "BMO_TESTBED_EVENT_HOOK",
      "BMO_CARGO_TARGET_DIR",
      "BMO_TESTBED_LOCK",
      "BMO_TEST_POSTGRES",
    ] {
      if let Some(value) = env.get(variable).filter(|value| !value.is_empty()) {
        handed.push((variable.to_owned(), value.clone()));
      }
    }

    let started = start_deploy_unit(
      &DeployUnitStart {
        unit: unit.clone(),
        slice: SUITE_SLICE.to_owned(),
        cwd: repo.root.clone(),
        // The unit runs the run itself, whatever `--due` it was started with.
        command: vec![this_again()[0].clone(), "full".to_owned()],
        env: handed,
      },
      env.get("PATH").map_or("", String::as_str),
    );

    if !started.ok {
      eprintln!("{}", fill_marks(&said.unit_failed, &[("unit", &unit), ("said", &started.out)]));
      return REFUSED_EXIT_CODE;
    }

    println!("{}", fill_marks(&said.started, &[("unit", &unit)]));
    return 0;
  }

  let lock = data_root().join("locks").join(format!("full-{name}.lock"));
  let Some(_held) = take_lock(&lock, 0.0, LockWait::default()) else {
    eprintln!("{}", said.refused);
    return REFUSED_EXIT_CODE;
  };
  let log = folder.join(format!("full-{}.log", stamp_for_name(now)));

  // The start counts as the day's run from here on, whatever the run comes to.
  write_start(&folder, now);
  let _ = std::fs::write(&log, "");

  let machine = Machine {
    source: repo.root.clone(),
    tree,
    log: log.clone(),
    main: repo.declaration.check.main.clone(),
    hook: Hook::of(Some(&repo), &env),
  };

  run_full(&machine, &machine.main, &log.to_string_lossy()).0
}
