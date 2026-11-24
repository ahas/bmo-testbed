//! The nightly round (docs/suite.md, 「The nightly round」): over a stand-in set, a repository whose head has a
//! passed whole suite is skipped, the rest run, and one event names every one that failed or was not run; when
//! a round is due; and the binary over a registry of real checkouts, whose whole passes the record keeps so
//! `passed --commit` answers them and the next round skips them.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use bmo_testbed_cli::nightly::{
  AT_VARIABLE, Came, Checkout, Failure, Member, NightlyHands, due, due_at, run_nightly, unit_name,
};
use bmo_testbed_core::events::Event;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::nightly::NIGHTLY;
use bmo_testbed_core::turn::{Env, NOT_RUN_EXIT_CODE};
use chrono::{NaiveTime, TimeZone};
use serde_json::Value;

use crate::stand::{Ran, checkout, git, install, ran, testbed};

fn member(name: &str) -> Member {
  Member {
    name: name.to_owned(),
    root: PathBuf::from(format!("/checkouts/{name}")),
  }
}

fn head_of(name: &str) -> String {
  format!("{name:0<40}")
}

// A stand-in set: each repository's head is its name padded, a record of the commits whose whole suite passed,
// and what each run comes to; what was run and told is written down, and a passed run is recorded as the suite
// records it.
struct Set {
  checkouts: Vec<Checkout>,
  heads: Vec<(String, Result<String, String>)>,
  record: RefCell<Vec<(String, String)>>,
  outcomes: Vec<(String, Came)>,
  ran: RefCell<Vec<String>>,
  told: RefCell<Vec<(String, String, Value)>>,
  said: RefCell<Vec<String>>,
}

impl Set {
  fn of(names: &[&str]) -> Self {
    Self {
      checkouts: names.iter().map(|name| Ok(member(name))).collect(),
      heads: Vec::new(),
      record: RefCell::new(Vec::new()),
      outcomes: Vec::new(),
      ran: RefCell::new(Vec::new()),
      told: RefCell::new(Vec::new()),
      said: RefCell::new(Vec::new()),
    }
  }

  fn passed_before(self, name: &str) -> Self {
    self.record.borrow_mut().push((name.to_owned(), head_of(name)));
    self
  }

  fn ending(mut self, name: &str, came: Came) -> Self {
    self.outcomes.push((name.to_owned(), came));
    self
  }

  fn told(&self) -> Vec<(String, String, Value)> {
    self.told.borrow().clone()
  }
}

impl NightlyHands for Set {
  fn checkouts(&self) -> Result<Vec<Checkout>, (String, String)> {
    Ok(self.checkouts.clone())
  }

  fn head(&self, member: &Member) -> Result<String, String> {
    self
      .heads
      .iter()
      .find(|(name, _)| *name == member.name)
      .map_or_else(|| Ok(head_of(&member.name)), |(_, head)| head.clone())
  }

  fn passed(&self, member: &Member, commit: &str) -> Option<i64> {
    let record = self.record.borrow();

    record
      .iter()
      .position(|(name, at)| *name == member.name && at == commit)
      .map(|index| index as i64 + 1)
  }

  fn run(&self, member: &Member, commit: &str) -> Came {
    self.ran.borrow_mut().push(member.name.clone());

    let came = self
      .outcomes
      .iter()
      .find(|(name, _)| *name == member.name)
      .map_or(Came::Passed, |(_, came)| came.clone());

    if came == Came::Passed {
      self.record.borrow_mut().push((member.name.clone(), commit.to_owned()));
    }

    came
  }

  fn tell(&self, event: Event<'_>) {
    self
      .told
      .borrow_mut()
      .push((event.name.to_owned(), event.summary.clone(), event.detail));
  }

  fn say(&self, line: &str) {
    self.said.borrow_mut().push(line.to_owned());
  }
}

fn broken() -> Came {
  Came::Failed(Failure {
    verdict: "suite: FAILED in 3s: 2 of 4 files passed, 2 failed".to_owned(),
    step: None,
    log: Some("/logs/beta.log".to_owned()),
    units: vec![
      ("server/a.test.ts".to_owned(), Some("@beta/server".to_owned())),
      ("tools/loose.test.ts".to_owned(), None),
    ],
  })
}

// "a repository whose head already has a passed whole suite is skipped, the rest run one at a time, and a round
// that passed says nothing"
#[test]
fn a_repository_already_passed_is_skipped_the_rest_run_and_a_passed_round_says_nothing() {
  let set = Set::of(&["alpha", "beta", "gamma"]).passed_before("beta");

  assert_eq!(run_nightly(&set, false), 0);
  assert_eq!(*set.ran.borrow(), ["alpha", "gamma"]);
  assert!(set.told().is_empty(), "{:?}", set.told());
  assert!(set.said.borrow().iter().any(|line| line.contains("skip") && line.contains("beta")));
}

// "a whole run that passed is the one the record keeps, so the next round skips it"
#[test]
fn a_whole_pass_is_recorded_and_the_next_round_skips_it() {
  let set = Set::of(&["alpha", "beta"]);

  assert_eq!(run_nightly(&set, false), 0);
  assert_eq!(*set.ran.borrow(), ["alpha", "beta"]);

  set.ran.borrow_mut().clear();
  assert_eq!(run_nightly(&set, false), 0);
  assert!(set.ran.borrow().is_empty(), "{:?}", set.ran.borrow());
}

// "once the round has ended, one event names every repository that failed or was not run, each with the
// commit, the verdict, the failing units with their packages and the log"
#[test]
fn failures_are_summarised_in_one_event_once_the_round_has_ended() {
  let mut set = Set::of(&["alpha", "beta", "gamma", "delta", "epsilon"])
    .passed_before("alpha")
    .ending("beta", broken())
    .ending("delta", Came::NotRun(NIGHTLY.run.busy.clone()));

  set
    .heads
    .push(("epsilon".to_owned(), Err("the main branch main does not stand".to_owned())));
  set
    .checkouts
    .push(Err((PathBuf::from("/checkouts/zeta"), "no testbed.toml".to_owned())));

  assert_eq!(run_nightly(&set, false), 1);
  assert_eq!(*set.ran.borrow(), ["beta", "gamma", "delta"]);

  let told = set.told();

  assert_eq!(told.len(), 1, "{told:?}");

  let (name, summary, detail) = &told[0];

  assert_eq!(name, "nightly-suites-failed");
  assert_eq!(detail["repositories"], 6);
  assert_eq!(detail["passed"], serde_json::json!(["gamma"]));
  assert_eq!(detail["skipped"], serde_json::json!(["alpha"]));

  let failed = detail["failed"].as_array().unwrap();
  let names: Vec<&str> = failed.iter().map(|one| one["repository"].as_str().unwrap()).collect();

  assert_eq!(names, ["beta", "delta", "epsilon", "/checkouts/zeta"]);
  assert_eq!(failed[0]["commit"], head_of("beta"));
  assert_eq!(failed[0]["outcome"], "failed");
  assert_eq!(failed[0]["log"], "/logs/beta.log");
  assert_eq!(
    failed[0]["failures"],
    serde_json::json!([
      { "unit": "server/a.test.ts", "package": "@beta/server" },
      { "unit": "tools/loose.test.ts", "package": null },
    ])
  );
  assert_eq!(failed[1]["outcome"], "not-run");
  assert_eq!(failed[2]["commit"], Value::Null);

  let said = &NIGHTLY.account;

  assert!(summary.contains(&format!("beta at {}", &head_of("beta")[..12])), "{summary}");
  assert!(summary.contains("server/a.test.ts (@beta/server)"), "{summary}");
  assert!(summary.contains(&format!("tools/loose.test.ts ({})", said.outside)), "{summary}");
  assert!(summary.contains("Log: /logs/beta.log"), "{summary}");
  assert!(summary.contains(&NIGHTLY.run.busy), "{summary}");
  assert!(summary.contains("the main branch main does not stand"), "{summary}");
  assert!(!summary.contains("gamma"), "{summary}");
}

// "a round where none failed but one was not run ends with 75"
#[test]
fn a_round_where_one_was_not_run_and_none_failed_ends_not_run() {
  let set = Set::of(&["alpha"]).ending("alpha", Came::NotRun(NIGHTLY.run.busy.clone()));

  assert_eq!(run_nightly(&set, false), NOT_RUN_EXIT_CODE);
  assert_eq!(set.told().len(), 1);
}

// "the summary names the policy's number of failing files of a repository and how many more"
#[test]
fn the_summary_names_a_few_failing_files_and_how_many_more() {
  let units: Vec<(String, Option<String>)> =
    (0..NIGHTLY.files_shown + 2).map(|n| (format!("f{n}.test.ts"), None)).collect();
  let set = Set::of(&["alpha"]).ending(
    "alpha",
    Came::Failed(Failure {
      verdict: "suite: FAILED".to_owned(),
      units,
      ..Failure::default()
    }),
  );

  assert_eq!(run_nightly(&set, false), 1);

  let (_, summary, detail) = &set.told()[0];

  assert!(summary.contains(&format!("f{}.test.ts", NIGHTLY.files_shown - 1)), "{summary}");
  assert!(!summary.contains(&format!("f{}.test.ts", NIGHTLY.files_shown)), "{summary}");
  assert!(
    summary.contains(&fill_marks(NIGHTLY.account.more.trim(), &[("count", "2")])),
    "{summary}"
  );
  assert_eq!(detail["failed"][0]["failures"].as_array().unwrap().len(), NIGHTLY.files_shown + 2);
}

// "a dry run says for each repository whether it would run or be skipped, and runs and tells nothing"
#[test]
fn a_dry_run_says_what_it_would_run_and_runs_nothing() {
  let set = Set::of(&["alpha", "beta"]).passed_before("beta").ending("alpha", broken());

  assert_eq!(run_nightly(&set, true), 0);
  assert!(set.ran.borrow().is_empty());
  assert!(set.told().is_empty());

  let said = set.said.borrow();

  assert!(
    said.contains(&fill_marks(
      &NIGHTLY.run.would_run,
      &[("name", "alpha"), ("commit", &head_of("alpha")[..12])]
    )),
    "{said:?}"
  );
  assert!(said.iter().any(|line| line.contains("skip") && line.contains("beta")), "{said:?}");
}

// "a round is due within the window after its time of day, once a night"
#[test]
fn a_round_is_due_within_the_window_after_its_time_once_a_night() {
  let zone = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
  let at = NaiveTime::from_hms_opt(3, 0, 0).unwrap();
  let night = zone.with_ymd_and_hms(2026, 11, 25, 3, 15, 0).unwrap();
  let since = zone.with_ymd_and_hms(2026, 11, 25, 3, 0, 0).unwrap();

  assert_eq!(due(at, &night, None), Ok(since));
  // One that started before tonight's time leaves tonight's due.
  assert_eq!(due(at, &night, Some(since.timestamp_millis() - 1)), Ok(since));
  assert!(due(at, &night, Some(since.timestamp_millis() + 60_000)).is_err());

  let evening = zone.with_ymd_and_hms(2026, 11, 24, 19, 0, 0).unwrap();
  let late = since + chrono::Duration::hours(i64::from(NIGHTLY.window_hours));
  let before = zone.with_ymd_and_hms(2026, 11, 25, 2, 45, 0).unwrap();

  assert!(due(at, &evening, None).is_err());
  assert!(due(at, &late, None).is_err());
  assert!(due(at, &before, None).is_err());
}

// "the time of day is the environment's, else the machine's testbed environment file's, else the policy's"
#[test]
fn the_time_of_day_is_the_environments_the_files_or_the_policys() {
  let dir = tempfile::tempdir().unwrap();
  let policy = NaiveTime::parse_from_str(&NIGHTLY.at, "%H:%M").unwrap();

  assert_eq!(due_at(&Env::new(), dir.path()), (policy, None));

  std::fs::create_dir_all(dir.path().join("env")).unwrap();
  std::fs::write(dir.path().join("env/testbed.env"), format!("{AT_VARIABLE}=\"01:30\"\n")).unwrap();
  assert_eq!(due_at(&Env::new(), dir.path()).0, NaiveTime::from_hms_opt(1, 30, 0).unwrap());

  let named = Env::from([(AT_VARIABLE.to_owned(), "04:10".to_owned())]);

  assert_eq!(due_at(&named, dir.path()).0, NaiveTime::from_hms_opt(4, 10, 0).unwrap());

  let (at, said) = due_at(&Env::from([(AT_VARIABLE.to_owned(), "late".to_owned())]), dir.path());

  assert_eq!(at, policy);
  assert!(said.unwrap().contains("late"));
}

#[test]
fn a_round_started_by_hand_has_a_unit_of_its_own() {
  assert_eq!(unit_name(7), "bmo-nightly-suites-7");
}

// The binary over a registry of real checkouts.

// A hook that writes each event's name and payload into a file beside it.
fn hook(dir: &Path) -> (String, PathBuf) {
  let told = dir.join("told.jsonl");
  let script = dir.join("hook.sh");

  install(
    &format!("#!/bin/sh\n{{ printf '%s ' \"$1\"; cat; echo; }} >> '{}'\n", told.display()),
    &script,
  );
  (script.display().to_string(), told)
}

fn nightly(dir: &Path, data: &Path, registry: &Path, hook: &str, args: &[&str]) -> Ran {
  let mut command = testbed(dir, data, &[&["nightly"][..], args].concat());

  command
    .env("BMO_TESTBED_CHECKOUTS", registry)
    .env("BMO_TESTBED_EVENT_HOOK", hook)
    .env("BMO_DEPLOY_UNIT", unit_name(1));
  ran(&mut command)
}

const FAILING: &str = r#"import { expect, test } from "bun:test";

test("fails", () => {
  expect(1).toBe(2);
});
"#;

// "over the machine's registry, a head already passed is skipped, the rest run in a tree of their own, a whole
// pass is recorded so `passed --commit` answers it, and the failures are told in one event"
#[test]
fn the_round_over_real_checkouts_skips_runs_records_and_tells_once() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let settled = dir.path().join("settled");
  let fresh = dir.path().join("fresh");
  let failing = dir.path().join("failing");

  std::fs::create_dir_all(&data).unwrap();
  checkout(&settled, "testbed = 0\nname = \"settled\"\n");
  checkout(&fresh, "testbed = 0\nname = \"fresh\"\n");
  checkout(&failing, "testbed = 0\nname = \"failing\"\n\n[test]\nlanguages = [\"bun\"]\n");
  std::fs::write(failing.join("broken.test.ts"), FAILING).unwrap();
  git(&failing, &["add", "."]);
  git(&failing, &["commit", "-q", "-m", "a failing test"]);

  let before = ran(&mut testbed(&settled, &data, &["suite"]));

  assert_eq!(before.code, 0, "{}", before.err);

  // The person's own checkout carries a change of its own, which the round never runs.
  std::fs::write(fresh.join("testbed.toml"), "testbed = 0\nname = \"fresh\"\n# edited\n").unwrap();

  let registry = dir.path().join("checkouts.toml");

  std::fs::write(
    &registry,
    format!(
      "checkouts = [\"{}\", \"{}\", \"{}\"]\n",
      settled.display(),
      fresh.display(),
      failing.display()
    ),
  )
  .unwrap();

  let (hook, told) = hook(dir.path());
  let dry = nightly(dir.path(), &data, &registry, &hook, &["--dry-run"]);

  assert_eq!(dry.code, 0, "{}", dry.err);
  assert!(dry.err.contains("skip") && dry.err.contains("settled"), "{}", dry.err);
  assert!(!told.exists());

  let round = nightly(dir.path(), &data, &registry, &hook, &[]);

  assert_eq!(round.code, 1, "{}", round.err);

  let fresh_head = git(&fresh, &["rev-parse", "HEAD"]);
  let answered = ran(&mut testbed(&fresh, &data, &["passed", "--commit", &fresh_head]));

  assert_eq!(answered.code, 0, "{}\n{}", answered.out, answered.err);
  assert!(answered.out.contains("tree"), "the pass stands in the round's tree: {}", answered.out);

  let events = std::fs::read_to_string(&told).unwrap();
  let lines: Vec<&str> = events.lines().filter(|line| !line.trim().is_empty()).collect();

  assert_eq!(lines.len(), 1, "{events}");

  let (name, payload) = lines[0].split_once(' ').unwrap();
  let payload: Value = serde_json::from_str(payload).unwrap();

  assert_eq!(name, "nightly-suites-failed");
  assert_eq!(payload["detail"]["skipped"], serde_json::json!(["settled"]));
  assert_eq!(payload["detail"]["passed"], serde_json::json!(["fresh"]));
  assert_eq!(payload["detail"]["failed"][0]["repository"], "failing");
  assert_eq!(payload["detail"]["failed"][0]["failures"][0]["unit"], "broken.test.ts");
  assert!(payload["summary"].as_str().unwrap().contains("broken.test.ts"), "{payload}");

  // The next round skips what passed, and runs again only what did not.
  std::fs::remove_file(&told).unwrap();

  let again = nightly(dir.path(), &data, &registry, &hook, &["--dry-run"]);

  assert!(again.err.contains("skip") && again.err.contains("fresh"), "{}", again.err);
  assert!(
    again.err.contains(&fill_marks(
      &NIGHTLY.run.would_run,
      &[("name", "failing"), ("commit", &git(&failing, &["rev-parse", "HEAD"])[..12])]
    )),
    "{}",
    again.err
  );
}

// "a registry that does not read fails the round, told once"
#[test]
fn a_registry_that_does_not_read_fails_the_round() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let registry = dir.path().join("checkouts.toml");

  std::fs::create_dir_all(&data).unwrap();
  std::fs::write(&registry, "checkouts = 3\n").unwrap();

  let (hook, told) = hook(dir.path());
  let round = nightly(dir.path(), &data, &registry, &hook, &[]);

  assert_eq!(round.code, 2, "{}", round.err);
  assert!(std::fs::read_to_string(&told).unwrap().starts_with("nightly-suites-failed "));
}
