//! The testbed's `run` and `status` verbs over their words (docs/boundary.md, 「The verbs」): what they
//! refuse, with the code they end on and nothing placed or run; where a run is asked to be placed; and the
//! close line `status` reads.

use std::cell::Cell;
use std::process::Command;

use bmo_testbed_core::close_line::{close_line_path, lock_file_name};
use bmo_testbed_core::placement::{DEPLOY_UNIT_VARIABLE, Local, PlaceAsk, Placement, runs_here};
use bmo_testbed_core::turn::{Env, HELD_VARIABLE, Lane};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, RunWords, VerbGround, run};
use bmo_testbed_core::witness::Unwitnessed;

fn words(line: &[&str]) -> Vec<String> {
  line.iter().map(|word| (*word).to_owned()).collect()
}

// A placement that answers with a code of its own, and says what it was asked.
struct Elsewhere {
  asked: Cell<Option<(Vec<String>, Lane)>>,
}

impl Placement for Elsewhere {
  fn place(&self, ask: &PlaceAsk<'_>) -> Option<u8> {
    self.asked.set(Some((ask.command.to_vec(), ask.lane)));
    Some(9)
  }
}

// "refuses a language it does not know, naming the ones there are, and places nothing"
#[test]
fn a_language_it_does_not_know_is_refused_and_nothing_is_placed() {
  let dir = tempfile::tempdir().unwrap();
  let elsewhere = Elsewhere {
    asked: Cell::new(None),
  };
  let code = run(
    &RunWords {
      command: words(&["true"]),
      adapter: Some("cobol".to_owned()),
      ..RunWords::default()
    },
    &VerbGround {
      data_root: dir.path().to_path_buf(),
      env: Env::new(),
      place: &elsewhere,
      witness: &Unwitnessed,
    },
  );

  assert_eq!(code.code, REFUSED_EXIT_CODE);
  assert_eq!(elsewhere.asked.take(), None);
}

// "a run is placed before its turn, and ends with the code the placement answers"
#[test]
fn a_run_ends_with_the_code_its_placement_answers() {
  let dir = tempfile::tempdir().unwrap();
  let elsewhere = Elsewhere {
    asked: Cell::new(None),
  };
  let asked = RunWords {
    command: words(&["cargo", "test"]),
    narrow: true,
    ..RunWords::default()
  };
  // Started outside any turn and any unit, whatever turn the test itself runs in.
  let ground = VerbGround {
    data_root: dir.path().to_path_buf(),
    env: Env::new(),
    place: &elsewhere,
    witness: &Unwitnessed,
  };

  assert_eq!(run(&asked, &ground).code, 9);
  assert_eq!(elsewhere.asked.take(), Some((words(&["cargo", "test"]), Lane::Narrow)));
}

// "a run inside a held turn or a deployment's unit runs where it stands"
#[test]
fn a_run_inside_a_held_turn_or_a_unit_is_never_placed() {
  let with = |name: &str, value: &str| Env::from([(name.to_owned(), value.to_owned())]);

  assert!(runs_here(&with(HELD_VARIABLE, "4242")));
  assert!(runs_here(&with(DEPLOY_UNIT_VARIABLE, "bmo-deploy-1")));
  assert!(!runs_here(&with(HELD_VARIABLE, "")));
  assert!(!runs_here(&Env::new()));
  assert_eq!(Local.place(&placed(&Env::new())), None);
}

fn placed(env: &Env) -> PlaceAsk<'_> {
  PlaceAsk {
    command: &[],
    lane: Lane::Wide,
    cwd: std::env::temp_dir(),
    env,
  }
}

// "every worktree of one repository shares its close line, and outside a repository there is none"
#[test]
fn every_worktree_of_a_repository_shares_one_close_line() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let git = |args: &[&str]| {
    assert!(
      Command::new("git").args(args).current_dir(&repo).output().unwrap().status.success(),
      "{args:?}"
    );
  };

  std::fs::create_dir_all(repo.join("sub")).unwrap();
  git(&["init", "-q"]);
  git(&[
    "-c",
    "user.name=t",
    "-c",
    "user.email=t@example.com",
    "commit",
    "-q",
    "--allow-empty",
    "-m",
    "start",
  ]);
  git(&["worktree", "add", "-q", "../other"]);

  let data = dir.path().join("data");
  let main = close_line_path(&data, &repo.join("sub")).unwrap();

  assert_eq!(close_line_path(&data, &dir.path().join("other")), Some(main.clone()));
  assert!(main.starts_with(data.join("locks")));
  assert_eq!(close_line_path(&data, &data), None);
}

// "a lock's file name keeps the tail of a long name, and the digest of the whole tells two apart"
#[test]
fn a_lock_file_name_is_safe_short_and_apart() {
  let long = format!("close:/{}/repo/.git", "deep".repeat(30));
  let name = lock_file_name(&long);

  assert!(name.ends_with(".lock"));
  assert!(name.starts_with("repo-.git-") || name.contains("-repo-.git-"), "{name}");
  assert!(name.chars().all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '-')));
  assert_ne!(lock_file_name("close:/a/b"), lock_file_name("close:/a-b"));
  assert!(lock_file_name("::").starts_with("root-"));
}

// The usage names the binary by its mark, and no span of the turn's or the run's.
#[test]
fn the_run_usage_names_the_binary_by_its_mark_and_no_span() {
  let usage = &bmo_testbed_core::policy::verbs::VERBS.usage.run;

  assert!(usage.contains("{verb}"));

  for mark in ["{guard}", "{harness_wait}", "{unit_wait}", "{kept}"] {
    assert!(!usage.contains(mark), "{mark}");
  }
}
