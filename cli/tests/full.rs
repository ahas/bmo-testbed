//! The daily whole run (docs/suite.md, 「The daily whole run」), over stand-in hands: the main branch's head,
//! its own tree, the static check and every unit, units never started run again by name, and a run that
//! fails sent as a `full-suite-failed` event naming each failing unit with its package.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use bmo_testbed_cli::full::{
  FullHands, FullOutcome, Round, latest_due, prepare_tree, read_start, run_full, unit_name, write_start,
};
use bmo_testbed_core::events::Event;
use bmo_testbed_core::policy::daily::DAILY;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::turn::NOT_RUN_EXIT_CODE;
use bmo_testbed_suite::run::verdict_line;
use chrono::{NaiveTime, TimeZone};
use serde_json::{Value, json};

use crate::stand::git;

const HEAD: &str = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";

type Suite = Box<dyn Fn(&[String]) -> Round>;
type Prepare = Box<dyn Fn(&str) -> Result<(), String>>;

// Hands whose every part answers as the test sets it, and which write down what was asked and told.
struct Hands {
  head: Option<String>,
  prepare: Prepare,
  check: Option<(String, String)>,
  suite: Suite,
  order: RefCell<Vec<String>>,
  told: RefCell<Vec<(String, Value)>>,
}

fn passed() -> String {
  verdict_line(true, 12.0, "3 of 3 files passed")
}

fn hands() -> Hands {
  hands_ending(0, &passed())
}

fn hands_ending(code: i32, output: &str) -> Hands {
  let output = output.to_owned();

  Hands {
    head: Some(HEAD.to_owned()),
    prepare: Box::new(|_| Ok(())),
    check: None,
    suite: Box::new(move |_| Round {
      code,
      output: output.clone(),
    }),
    order: RefCell::new(Vec::new()),
    told: RefCell::new(Vec::new()),
  }
}

impl FullHands for Hands {
  fn head(&self) -> Option<String> {
    self.head.clone()
  }

  fn prepare(&self, commit: &str) -> Result<(), String> {
    self.order.borrow_mut().push("prepare".to_owned());
    (self.prepare)(commit)
  }

  fn static_check(&self) -> Option<(String, String)> {
    self.order.borrow_mut().push("check".to_owned());
    self.check.clone()
  }

  fn suite(&self, only: &[String]) -> Round {
    self.order.borrow_mut().push(format!("suite {}", only.join(",")));
    (self.suite)(only)
  }

  fn package_of(&self, unit: &str) -> Option<String> {
    unit.starts_with("server/").then(|| "@sample/server".to_owned())
  }

  fn tell(&self, event: Event<'_>) {
    self
      .told
      .borrow_mut()
      .push((event.summary.clone(), json!({ "name": event.name, "detail": event.detail })));
  }

  fn say(&self, _: &str) {}
}

fn told(hands: &Hands) -> Vec<Value> {
  hands.told.borrow().iter().map(|(_, event)| event.clone()).collect()
}

// "its unit is its own, not a deployment's"
#[test]
fn its_unit_is_its_own_not_a_deployments() {
  assert_eq!(unit_name(7), "bmo-full-suite-7");
  assert!(!unit_name(7).starts_with(bmo_testbed_deploy::unit::DEPLOY_UNIT_PREFIX));
}

// "the start is written down and read back, and one that will not read is none"
#[test]
fn the_start_is_written_down_and_read_back() {
  let dir = tempfile::tempdir().unwrap();
  let at = 1_790_278_205_000;

  assert_eq!(read_start(dir.path()), None);
  write_start(dir.path(), at);
  assert_eq!(read_start(dir.path()), Some(at));
  std::fs::write(dir.path().join("started"), "not a time\n").unwrap();
  assert_eq!(read_start(dir.path()), None);
}

// "the time a run is due since is today's where it has passed, yesterday's otherwise"
#[test]
fn the_time_due_since_is_todays_once_passed_and_yesterdays_before() {
  let zone = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
  let due = NaiveTime::from_hms_opt(4, 30, 0).unwrap();
  let after = zone.with_ymd_and_hms(2026, 10, 17, 6, 0, 0).unwrap();
  let before = zone.with_ymd_and_hms(2026, 10, 17, 3, 0, 0).unwrap();

  assert_eq!(latest_due(due, &after), Some(zone.with_ymd_and_hms(2026, 10, 17, 4, 30, 0).unwrap()));
  assert_eq!(latest_due(due, &before), Some(zone.with_ymd_and_hms(2026, 10, 16, 4, 30, 0).unwrap()));
}

// "puts its own tree on the head, checks, runs every unit, and tells nothing where it passed"
#[test]
fn puts_its_tree_on_the_head_checks_runs_every_unit_and_tells_nothing_where_it_passed() {
  let hands = hands();

  assert_eq!(run_full(&hands, "main", "/log"), (0, FullOutcome::Passed));
  assert_eq!(*hands.order.borrow(), ["prepare", "check", "suite "]);
  assert!(hands.told.borrow().is_empty());
}

// "its tree is made at the head, moved to the next head with a run's leftovers cleared, and set up each time"
#[test]
fn its_tree_is_made_at_the_head_moved_with_leftovers_cleared_and_set_up_each_time() {
  let root = tempfile::tempdir().unwrap();
  let repo = root.path().join("repo");
  let tree = root.path().join("full-check");
  let set_up = RefCell::new(Vec::<PathBuf>::new());
  let setup = |tree: &Path| {
    set_up.borrow_mut().push(tree.to_path_buf());
    Ok(())
  };

  std::fs::create_dir_all(&repo).unwrap();
  git(&repo, &["init", "-q", "-b", "main"]);
  std::fs::write(repo.join("a.ts"), "one\n").unwrap();
  git(&repo, &["add", "."]);
  git(&repo, &["commit", "-q", "-m", "one"]);
  // A half-written file in the checkout never reaches the run's tree.
  std::fs::write(repo.join("a.ts"), "half written\n").unwrap();

  let first = git(&repo, &["rev-parse", "main"]);

  prepare_tree(&repo, &first, &tree, root.path(), &setup).unwrap();
  assert_eq!(git(&tree, &["rev-parse", "HEAD"]), first);
  assert_eq!(std::fs::read_to_string(tree.join("a.ts")).unwrap(), "one\n");

  std::fs::write(tree.join("left-by-a-run.txt"), "x\n").unwrap();
  git(&repo, &["checkout", "-q", "--", "a.ts"]);
  std::fs::write(repo.join("b.ts"), "two\n").unwrap();
  git(&repo, &["add", "."]);
  git(&repo, &["commit", "-q", "-m", "two"]);

  let second = git(&repo, &["rev-parse", "main"]);

  prepare_tree(&repo, &second, &tree, root.path(), &setup).unwrap();
  assert_eq!(git(&tree, &["rev-parse", "HEAD"]), second);
  assert!(!tree.join("left-by-a-run.txt").exists());
  assert_eq!(*set_up.borrow(), [tree.clone(), tree.clone()]);
}

// "a tree another repository laid is taken away and laid again from this one"
#[test]
fn a_tree_another_repository_laid_is_laid_again_from_this_one() {
  let root = tempfile::tempdir().unwrap();
  let before = root.path().join("before");
  let repo = root.path().join("repo");
  let tree = root.path().join("full-check");

  for (dir, file) in [(&before, "old.ts"), (&repo, "new.ts")] {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join(file), "x\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "one"]);
  }

  let theirs = git(&before, &["rev-parse", "main"]);
  prepare_tree(&before, &theirs, &tree, root.path(), &|_| Ok(())).unwrap();

  let ours = git(&repo, &["rev-parse", "main"]);
  prepare_tree(&repo, &ours, &tree, root.path(), &|_| Ok(())).unwrap();

  assert_eq!(git(&tree, &["rev-parse", "HEAD"]), ours);
  assert!(tree.join("new.ts").exists());
  assert!(!tree.join("old.ts").exists());
}

// "a run that fails before its first unit writes why into its log": a tree that could not be set up.
#[test]
fn a_tree_that_could_not_be_set_up_is_written_into_the_run_s_log() {
  let dir = tempfile::tempdir().unwrap();
  let log = dir.path().join("full.log");
  let mut hands = hands();

  hands.prepare =
    Box::new(|_| Err("the tree's setup `cd packages && bun install` failed: bun: not found".to_owned()));

  assert_eq!(run_full(&hands, "main", log.to_str().unwrap()), (1, FullOutcome::TreeFailed));

  let written = std::fs::read_to_string(&log).unwrap();

  assert!(written.contains("bun: not found"), "{written}");
  assert!(written.contains("full-suite: FAILED"), "{written}");
}

// "a failing run tells the failing units, the package each stands in and the log"
#[test]
fn a_failing_run_tells_the_failing_units_the_package_each_stands_in_and_the_log() {
  let verdict = verdict_line(false, 64.0, "637 of 640 files passed, 2 failed");
  let output = [
    "fail 1.2s  server/a.test.ts",
    "fail 0.4s  tools/loose.test.ts",
    "failed: server/a.test.ts, tools/loose.test.ts",
    &verdict,
  ]
  .join("\n");
  let hands = hands_ending(1, &output);

  assert_eq!(run_full(&hands, "main", "/data/full-1.log"), (1, FullOutcome::SuiteFailed));

  let told = told(&hands);

  assert_eq!(told.len(), 1);
  assert_eq!(told[0]["name"], "full-suite-failed");
  assert_eq!(told[0]["detail"]["head"], HEAD);
  assert_eq!(told[0]["detail"]["log"], "/data/full-1.log");
  assert_eq!(told[0]["detail"]["step"], Value::Null);
  assert_eq!(
    told[0]["detail"]["failures"],
    json!([
      { "unit": "server/a.test.ts", "package": "@sample/server" },
      { "unit": "tools/loose.test.ts", "package": null },
    ])
  );

  let summary = &hands.told.borrow()[0].0;

  assert!(summary.contains(&fill_marks(
    &DAILY.account.file,
    &[("file", "server/a.test.ts"), ("unit", "@sample/server")]
  )));
  assert!(summary.contains("/data/full-1.log"), "{summary}");
}

// Units a round never started failed nothing: they run again by name, and the run stands on what that round
// came to.
mod units_a_round_never_started {
  use super::*;

  fn short() -> String {
    [
      format!(
        "never started within 240s: {}",
        ["client/a.test.ts", "core/"].join(&SUITE.ending.joiner)
      ),
      verdict_line(false, 241.0, "8 of 10 files passed, 2 never started within 240s"),
    ]
    .join("\n")
  }

  fn rounds(again_code: i32, again: String) -> Hands {
    let mut hands = hands();

    hands.suite = Box::new(move |only| {
      if only.is_empty() {
        Round {
          code: i32::from(NOT_RUN_EXIT_CODE),
          output: short(),
        }
      } else {
        Round {
          code: again_code,
          output: again.clone(),
        }
      }
    });
    hands
  }

  // "run again by name, and a run whose second round passed passes"
  #[test]
  fn run_again_by_name_and_a_run_whose_second_round_passed_passes() {
    let hands = rounds(0, verdict_line(true, 20.0, "2 of 2 files passed"));

    assert_eq!(run_full(&hands, "main", "/log"), (0, FullOutcome::Passed));
    assert_eq!(hands.order.borrow()[2..], ["suite ", "suite client/a.test.ts,core/"]);
    assert!(hands.told.borrow().is_empty());
  }

  // "fail where the second round failed, naming only what failed"
  #[test]
  fn fail_where_the_second_round_failed_naming_only_what_failed() {
    let failed = [
      "fail 1.2s  client/a.test.ts".to_owned(),
      "failed: client/a.test.ts".to_owned(),
      verdict_line(false, 20.0, "1 of 2 files passed, 1 failed"),
    ]
    .join("\n");
    let hands = rounds(1, failed);

    assert_eq!(run_full(&hands, "main", "/log").1, FullOutcome::SuiteFailed);
    assert_eq!(
      told(&hands)[0]["detail"]["failures"],
      json!([{ "unit": "client/a.test.ts", "package": null }])
    );
  }

  // "stop running again once a round starts no more of them than the one before"
  #[test]
  fn stop_running_again_once_a_round_starts_no_more_than_the_one_before() {
    let hands = rounds(i32::from(NOT_RUN_EXIT_CODE), short());

    assert_eq!(run_full(&hands, "main", "/log").1, FullOutcome::SuiteFailed);
    assert_eq!(hands.order.borrow().len(), 4);
    assert_eq!(told(&hands)[0]["detail"]["failures"], json!([]));
  }
}

// "a run that wrote no verdict is a failed one"
#[test]
fn a_run_that_wrote_no_verdict_is_a_failed_one() {
  let hands = hands_ending(0, "the run was killed\n");

  assert_eq!(run_full(&hands, "main", "/log").1, FullOutcome::SuiteFailed);
  assert!(hands.told.borrow()[0].0.contains(&fill_marks(&DAILY.run.no_verdict, &[("code", "0")])));
}

// "a tree that could not be set up, a failed static check and an unreadable head each tell and run no test"
#[test]
fn a_tree_not_set_up_a_failed_check_and_an_unreadable_head_each_tell_and_run_no_test() {
  let mut broken = hands();
  let why = "the check tree /x could not be made at aaaaaaaaaaaa: fatal";

  broken.prepare = Box::new(move |_| Err(why.to_owned()));
  assert_eq!(run_full(&broken, "main", "/log").1, FullOutcome::TreeFailed);
  assert_eq!(*broken.order.borrow(), ["prepare"]);
  assert_eq!(told(&broken)[0]["detail"]["step"], DAILY.run.setup_step.as_str());

  let mut unchecked = hands();

  unchecked.check = Some(("types".to_owned(), "static check: FAILED, types exited 2".to_owned()));
  assert_eq!(run_full(&unchecked, "main", "/log").1, FullOutcome::CheckFailed);
  assert_eq!(*unchecked.order.borrow(), ["prepare", "check"]);
  assert_eq!(told(&unchecked)[0]["detail"]["step"], "types");

  let mut headless = hands();

  headless.head = None;
  assert_eq!(run_full(&headless, "main", "/log").1, FullOutcome::NoHead);
  assert!(headless.order.borrow().is_empty());
  assert_eq!(told(&headless)[0]["detail"]["head"], Value::Null);
}
