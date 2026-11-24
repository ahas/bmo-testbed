//! `nightly`: the nightly round over the machine's repositories (docs/suite.md, 「The nightly round」). For
//! every checkout the machine's registry names, one at a time, it reads the main branch's head, skips a
//! repository whose head already has a passed whole suite in the record, and otherwise runs the whole suite at
//! that head in the repository's own tree of the daily whole run, so the gates that take a passed whole suite
//! find one. Once the round has ended, one `nightly-suites-failed` event names every repository that failed
//! or was not run, sent to the machine's own hook; a round that passed says nothing.
//!
//! It never runs in its caller's process but with `--dry-run`: called from anywhere but its own unit, it
//! starts that unit and ends. One round at a time on a machine.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::nightly::NIGHTLY;
use bmo_testbed_core::queue::{LockWait, now_ms, take_lock};
use bmo_testbed_core::turn::{Env, NOT_RUN_EXIT_CODE, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_deploy::deployment::passed_suite;
use bmo_testbed_deploy::run::stamp_for_name;
use bmo_testbed_deploy::unit::{DeployUnitStart, deploy_unit_of_process, start_deploy_unit, this_again};
use bmo_testbed_suite::dependents::{read_registry, registry_path};
use bmo_testbed_suite::fence::SUITE_SLICE;
use chrono::{DateTime, NaiveTime, TimeZone};
use serde_json::{Value, json};

use crate::env_file::assigned;
use crate::full::{FullHands, Machine, Round, own_tree, read_start, run_full, runs_folder, write_start};
use crate::placement::{MACHINE_ENV_FILE, machine_hook};
use crate::words::NightlyArgs;

/// What a round's unit is called by where it is started by hand; the timer's service is `bmo-nightly-suites`.
pub const UNIT_PREFIX: &str = "bmo-nightly-suites-";
/// The variable naming the time of day a round is due at.
pub const AT_VARIABLE: &str = "BMO_TESTBED_NIGHTLY_AT";
/// The event a round that did not pass sends.
pub const EVENT: &str = "nightly-suites-failed";

/// A fresh unit name for one round started by hand.
pub fn unit_name(now_ms: i64) -> String {
  format!("{UNIT_PREFIX}{now_ms}")
}

/// The round's own folder under the data root, where its start time stands.
pub fn round_folder(data_root: &Path) -> PathBuf {
  data_root.join("testbed").join(&NIGHTLY.folder)
}

/// The time of day a round is due at: the one `BMO_TESTBED_NIGHTLY_AT` names in the environment, else in the
/// machine's testbed environment file, else the policy's; a value that is no `HH:MM` means the policy's, and
/// the line saying so comes with it.
pub fn due_at(env: &Env, data_root: &Path) -> (NaiveTime, Option<String>) {
  let policy = NaiveTime::parse_from_str(&NIGHTLY.at, "%H:%M").unwrap_or_default();
  let named = env
    .get(AT_VARIABLE)
    .filter(|value| !value.trim().is_empty())
    .cloned()
    .or_else(|| assigned(&data_root.join(MACHINE_ENV_FILE), AT_VARIABLE));
  let Some(named) = named else {
    return (policy, None);
  };

  match NaiveTime::parse_from_str(named.trim(), "%H:%M") {
    Ok(at) => (at, None),
    Err(_) => (policy, Some(fill_marks(&NIGHTLY.run.bad_at, &[("value", &named), ("at", &NIGHTLY.at)]))),
  }
}

/// Whether a round is due at `now`: the moment it is due since, where the clock has shown `at` within the
/// policy's window and no round started since; otherwise the line saying why none starts.
pub fn due<Tz: TimeZone>(
  at: NaiveTime,
  now: &DateTime<Tz>,
  started: Option<i64>,
) -> Result<DateTime<Tz>, String> {
  let said = &NIGHTLY.run;
  let shown = at.format("%H:%M").to_string();
  let since = crate::full::latest_due(at, now).ok_or_else(|| {
    fill_marks(&said.window_passed, &[("hours", &NIGHTLY.window_hours.to_string()), ("due", &shown)])
  })?;

  if now.clone().signed_duration_since(since.clone())
    >= chrono::Duration::hours(i64::from(NIGHTLY.window_hours))
  {
    return Err(fill_marks(
      &said.window_passed,
      &[("hours", &NIGHTLY.window_hours.to_string()), ("due", &shown)],
    ));
  }

  if let Some(started) = started.filter(|started| *started >= since.timestamp_millis()) {
    let at = DateTime::from_timestamp_millis(started).unwrap_or_default().to_rfc3339();

    return Err(fill_marks(&said.started_since, &[("started", &at), ("due", &shown)]));
  }

  Ok(since)
}

/// A checkout of the registry whose declaration reads: its declared name and its root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
  pub name: String,
  pub root: PathBuf,
}

/// A checkout of the registry, or the path of one whose declaration does not read and why.
pub type Checkout = Result<Member, (PathBuf, String)>;

/// How one repository's run that did not pass ended: its verdict, the failing setup step where that was it,
/// its log, and each failing unit with its package.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Failure {
  pub verdict: String,
  pub step: Option<String>,
  pub log: Option<String>,
  pub units: Vec<(String, Option<String>)>,
}

/// What one repository's run came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Came {
  Passed,
  Failed(Failure),
  NotRun(String),
}

/// Everything a round reaches, so a test holds each path.
pub trait NightlyHands {
  /// The checkouts the machine's registry names, in its order; why where the registry does not read, with
  /// its path.
  fn checkouts(&self) -> Result<Vec<Checkout>, (String, String)>;
  /// The main branch's head of a repository, the remote asked first; why where it does not read.
  fn head(&self, member: &Member) -> Result<String, String>;
  /// The test run that passed the repository's whole suite on the commit, by its id; none where none did.
  fn passed(&self, member: &Member, commit: &str) -> Option<i64>;
  /// The repository's whole suite run at the commit in its own tree.
  fn run(&self, member: &Member, commit: &str) -> Came;
  /// The event sent.
  fn tell(&self, event: Event<'_>);
  fn say(&self, line: &str);
}

fn short(commit: &str) -> &str {
  &commit[..commit.len().min(12)]
}

// One repository in the event: who, where, at what, how it came out and what failed.
struct Account {
  repository: String,
  root: PathBuf,
  commit: Option<String>,
  outcome: &'static str,
  failure: Failure,
}

fn account_entry(account: &Account) -> String {
  let said = &NIGHTLY.account;
  let units = &account.failure.units;
  let mut files: Vec<String> = units
    .iter()
    .take(NIGHTLY.files_shown)
    .map(|(file, unit)| {
      fill_marks(&said.file, &[("file", file), ("unit", unit.as_deref().unwrap_or(&said.outside))])
    })
    .collect();
  let files = if files.is_empty() {
    String::new()
  } else {
    if units.len() > NIGHTLY.files_shown {
      let more = (units.len() - NIGHTLY.files_shown).to_string();

      files.push(fill_marks(&said.more, &[("count", &more)]).trim().to_owned());
    }

    fill_marks(&said.files, &[("files", &files.join(&said.joiner))])
  };

  fill_marks(
    &said.entry,
    &[
      ("name", &account.repository),
      (
        "at",
        &account
          .commit
          .as_deref()
          .map_or_else(String::new, |commit| fill_marks(&said.at, &[("commit", short(commit))])),
      ),
      ("verdict", account.failure.verdict.trim_end_matches('.')),
      ("files", &files),
      (
        "log",
        &account
          .failure
          .log
          .as_deref()
          .map_or_else(String::new, |log| fill_marks(&said.log, &[("log", log)])),
      ),
    ],
  )
}

fn round_event(
  count: usize,
  passed: &[String],
  skipped: &[String],
  accounts: &[Account],
) -> Event<'static> {
  let said = &NIGHTLY.account;
  let entries: Vec<String> = accounts.iter().map(account_entry).collect();

  Event {
    name: EVENT,
    commit: None,
    summary: fill_marks(
      &said.summary,
      &[
        ("count", &count.to_string()),
        ("failed", &accounts.len().to_string()),
        ("entries", &entries.join(&said.between)),
      ],
    ),
    detail: json!({
      "repositories": count,
      "passed": passed,
      "skipped": skipped,
      "failed": accounts.iter().map(|account| json!({
        "repository": account.repository,
        "root": account.root.to_string_lossy(),
        "commit": account.commit,
        "outcome": account.outcome,
        "verdict": account.failure.verdict,
        "step": account.failure.step,
        "log": account.failure.log,
        "failures": account
          .failure
          .units
          .iter()
          .map(|(unit, package)| json!({ "unit": unit, "package": package }))
          .collect::<Vec<_>>(),
      })).collect::<Vec<_>>(),
    }),
  }
}

/// One round, in the registry's order: each repository skipped where its head has a passed whole suite, said
/// where it is a dry run, and otherwise run; once every one has had its turn, one event where any failed or
/// was not run. Its exit code.
pub fn run_nightly(hands: &impl NightlyHands, dry_run: bool) -> u8 {
  let said = &NIGHTLY.run;
  let checkouts = match hands.checkouts() {
    Ok(checkouts) => checkouts,
    Err((path, why)) => {
      hands.say(&fill_marks(&said.registry_unread, &[("path", &path), ("why", &why)]));

      if !dry_run {
        hands.tell(Event {
          name: EVENT,
          commit: None,
          summary: fill_marks(&NIGHTLY.account.unread, &[("path", &path), ("why", &why)]),
          detail: json!({ "repositories": 0, "passed": [], "skipped": [], "failed": [] }),
        });
      }

      return REFUSED_EXIT_CODE;
    }
  };
  let count = checkouts.len();

  hands.say(&fill_marks(&said.begins, &[("count", &count.to_string())]));

  let mut passed: Vec<String> = Vec::new();
  let mut skipped: Vec<String> = Vec::new();
  let mut accounts: Vec<Account> = Vec::new();

  for checkout in checkouts {
    let member = match checkout {
      Ok(member) => member,
      Err((path, why)) => {
        hands.say(&fill_marks(&said.unread, &[("path", &path.to_string_lossy()), ("why", &why)]));
        accounts.push(Account {
          repository: path.to_string_lossy().into_owned(),
          root: path,
          commit: None,
          outcome: "failed",
          failure: Failure {
            verdict: why,
            ..Failure::default()
          },
        });
        continue;
      }
    };
    let head = match hands.head(&member) {
      Ok(head) => head,
      Err(why) => {
        hands.say(&fill_marks(&said.failed, &[("name", &member.name), ("verdict", &why)]));
        accounts.push(Account {
          repository: member.name,
          root: member.root,
          commit: None,
          outcome: "failed",
          failure: Failure {
            verdict: why,
            ..Failure::default()
          },
        });
        continue;
      }
    };
    let at = short(&head).to_owned();

    if let Some(id) = hands.passed(&member, &head) {
      hands.say(&fill_marks(
        &said.skip,
        &[("name", &member.name), ("commit", &at), ("id", &id.to_string())],
      ));
      skipped.push(member.name);
      continue;
    }

    if dry_run {
      hands.say(&fill_marks(&said.would_run, &[("name", &member.name), ("commit", &at)]));
      continue;
    }

    match hands.run(&member, &head) {
      Came::Passed => {
        hands.say(&fill_marks(&said.passed, &[("name", &member.name), ("commit", &at)]));
        passed.push(member.name);
      }
      Came::Failed(failure) => {
        hands.say(&fill_marks(&said.failed, &[("name", &member.name), ("verdict", &failure.verdict)]));
        accounts.push(Account {
          repository: member.name,
          root: member.root,
          commit: Some(head),
          outcome: "failed",
          failure,
        });
      }
      Came::NotRun(why) => {
        hands.say(&fill_marks(&said.not_run, &[("name", &member.name), ("why", &why)]));
        accounts.push(Account {
          repository: member.name,
          root: member.root,
          commit: Some(head),
          outcome: "not-run",
          failure: Failure {
            verdict: why,
            ..Failure::default()
          },
        });
      }
    }
  }

  let failed = accounts.iter().filter(|account| account.outcome == "failed").count();
  let unrun = accounts.len() - failed;

  hands.say(&fill_marks(
    &said.ended,
    &[
      ("passed", &passed.len().to_string()),
      ("skipped", &skipped.len().to_string()),
      ("failed", &failed.to_string()),
      ("unrun", &unrun.to_string()),
      ("count", &count.to_string()),
    ],
  ));

  if dry_run {
    return 0;
  }

  if !accounts.is_empty() {
    hands.tell(round_event(count, &passed, &skipped, &accounts));
  }

  if failed > 0 {
    1
  } else if unrun > 0 {
    NOT_RUN_EXIT_CODE
  } else {
    0
  }
}

// One repository's daily whole run's hands, held to the head the round read and to the suite alone: no static
// check, and what it would tell kept for the round's one event.
struct Pinned {
  machine: Machine,
  head: String,
  told: RefCell<Option<Value>>,
}

impl FullHands for Pinned {
  fn head(&self) -> Option<String> {
    Some(self.head.clone())
  }

  fn prepare(&self, commit: &str) -> Result<(), String> {
    self.machine.prepare(commit)
  }

  fn static_check(&self) -> Option<(String, String)> {
    None
  }

  fn suite(&self, only: &[String]) -> Round {
    self.machine.suite(only)
  }

  fn package_of(&self, unit: &str) -> Option<String> {
    self.machine.package_of(unit)
  }

  fn tell(&self, event: Event<'_>) {
    *self.told.borrow_mut() = Some(event.detail);
  }

  fn say(&self, line: &str) {
    self.machine.say(line);
  }
}

// What a daily whole run's event told, as one repository's failure.
fn failure_of(detail: &Value, log: &Path) -> Failure {
  let text = |key: &str| detail[key].as_str().map(str::to_owned);

  Failure {
    verdict: text("verdict").unwrap_or_default(),
    step: text("step"),
    log: text("log").or_else(|| Some(log.to_string_lossy().into_owned())),
    units: detail["failures"]
      .as_array()
      .map(|failures| {
        failures
          .iter()
          .map(|one| {
            (
              one["unit"].as_str().unwrap_or_default().to_owned(),
              one["package"].as_str().map(str::to_owned),
            )
          })
          .collect()
      })
      .unwrap_or_default(),
  }
}

// The round's hands on this machine: the registry, each checkout's main branch and tree, the record, and the
// machine's own hook.
struct Here {
  data_root: PathBuf,
  env: Env,
  registry: PathBuf,
  hook: Hook,
}

impl Here {
  fn repository(&self, member: &Member) -> Result<Repository, String> {
    Repository::read(&member.root).map_err(|refusal| refusal.to_string())
  }
}

impl NightlyHands for Here {
  fn checkouts(&self) -> Result<Vec<Checkout>, (String, String)> {
    let paths = read_registry(&self.registry, &self.env)
      .map_err(|fault| (fault.path.to_string_lossy().into_owned(), fault.why))?;
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut found = Vec::new();

    for path in paths {
      let same = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());

      if seen.contains(&same) {
        continue;
      }

      seen.push(same);
      found.push(match Repository::read(&path) {
        Ok(repo) => Ok(Member {
          name: repo.name(),
          root: repo.root,
        }),
        Err(refusal) => Err((path, refusal.to_string())),
      });
    }

    Ok(found)
  }

  fn head(&self, member: &Member) -> Result<String, String> {
    let repo = self.repository(member)?;
    let main = bmo_testbed_core::main_branch::MainBranch::new(&repo.root, &repo.declaration.check.main);

    // A remote that cannot be asked leaves the head to what this machine already knows of it.
    let _ = main.fetch();

    main.head()
  }

  fn passed(&self, member: &Member, commit: &str) -> Option<i64> {
    passed_suite(&self.data_root, &member.name, commit).map(|run| run.id)
  }

  fn run(&self, member: &Member, commit: &str) -> Came {
    let repo = match self.repository(member) {
      Ok(repo) => repo,
      Err(why) => {
        return Came::Failed(Failure {
          verdict: why,
          ..Failure::default()
        });
      }
    };
    let lock = self.data_root.join("locks").join(format!("full-{}.lock", member.name));
    let Some(_held) = take_lock(&lock, 0.0, LockWait::default()) else {
      return Came::NotRun(NIGHTLY.run.busy.clone());
    };
    let folder = runs_folder(&self.data_root, &member.name);
    let log = folder.join(format!("nightly-{}.log", stamp_for_name(now_ms())));

    let _ = std::fs::create_dir_all(&folder);
    let _ = std::fs::write(&log, "");
    self.say(&fill_marks(
      &NIGHTLY.run.running,
      &[("name", &member.name), ("commit", short(commit)), ("log", &log.to_string_lossy())],
    ));

    let pinned = Pinned {
      machine: Machine {
        source: repo.root.clone(),
        tree: own_tree(&repo, &self.env, &self.data_root),
        log: log.clone(),
        main: repo.declaration.check.main.clone(),
        hook: Hook::of(None, &self.env),
      },
      head: commit.to_owned(),
      told: RefCell::new(None),
    };
    let (code, _) = run_full(&pinned, &pinned.machine.main, &log.to_string_lossy());

    if code == 0 {
      return Came::Passed;
    }

    let detail = pinned.told.borrow().clone().unwrap_or(Value::Null);

    Came::Failed(failure_of(&detail, &log))
  }

  fn tell(&self, event: Event<'_>) {
    self.hook.send(&event);
  }

  fn say(&self, line: &str) {
    eprintln!("{line}");
  }
}

/// `nightly`: with `--dry-run`, what a round would run, said here; otherwise started in a unit of its own, or
/// with `--when-due` only where a round is due; inside its unit, the round, one at a time on a machine.
pub fn nightly(args: &NightlyArgs) -> u8 {
  let said = &NIGHTLY.run;
  let env = process_env();
  let root = data_root();
  let here = Here {
    registry: registry_path(&root, &env),
    hook: machine_hook(&root, &env),
    data_root: root.clone(),
    env: env.clone(),
  };

  if args.dry_run {
    return run_nightly(&here, true);
  }

  let folder = round_folder(&root);

  if args.when_due {
    let (at, bad) = due_at(&env, &root);

    if let Some(bad) = bad {
      eprintln!("{bad}");
    }

    if let Err(why) = due(at, &chrono::Local::now(), read_start(&folder)) {
      println!("{}", fill_marks(&said.not_due, &[("why", &why)]));
      return 0;
    }
  }

  if deploy_unit_of_process().is_none() {
    let now = now_ms();
    let unit = unit_name(now);
    let mut handed = vec![("BMO_DATA_DIR".to_owned(), root.to_string_lossy().into_owned())];

    for variable in [
      "BMO_TESTBED_CHECKOUTS",
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
        cwd: std::env::current_dir().unwrap_or_else(|_| root.clone()),
        // The unit runs the round itself, whatever `--when-due` it was started with.
        command: vec![this_again()[0].clone(), "nightly".to_owned()],
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

  let Some(_held) = take_lock(&root.join("locks").join(&NIGHTLY.lock), 0.0, LockWait::default()) else {
    eprintln!("{}", said.refused);
    return REFUSED_EXIT_CODE;
  };

  // The start counts as the night's round from here on, whatever the round comes to.
  write_start(&folder, now_ms());

  run_nightly(&here, false)
}
