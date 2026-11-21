//! A run placed elsewhere standing on the machine that placed it (docs/boundary.md, 「A run placed elsewhere
//! stands here too」): read back in its lane while it lasts, running once its turn there came, taken away as it
//! ends, and an entry whose process is gone left out; and the load at or above which a machine takes no run
//! (docs/testbed.md, 「A run on another machine」).

use std::path::Path;

use bmo_testbed_core::placement::{
  LOADED_VARIABLE, PlaceAsk, PlacedEntry, PlacedRun, loaded, loaded_percent, read_placed,
};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::turn::{Env, Lane, TurnEntry, lock_path};

fn ask<'a>(command: &'a [String], env: &'a Env) -> PlaceAsk<'a> {
  PlaceAsk {
    command,
    adapter: None,
    lane: Lane::Narrow,
    cwd: "/checkout/sub".into(),
    env,
  }
}

// "a placed run stands in its lane while it lasts, running once its turn there came, and goes as it ends"
#[test]
fn a_placed_run_stands_in_its_lane_until_it_ends() {
  let dir = tempfile::tempdir().unwrap();
  let env = Env::from([
    (
      "BMO_TESTBED_LOCK".to_owned(),
      dir.path().join("wide.lock").to_string_lossy().into_owned(),
    ),
    ("BMO_TESTBED_SCOPE".to_owned(), "scope-3".to_owned()),
  ]);
  let command = vec!["bmo-testbed".to_owned(), "suite".to_owned()];
  let narrow = lock_path(&env, Lane::Narrow);
  let run = PlacedRun::stand(&ask(&command, &env), Path::new("/checkout"), "machine-2");
  let stood = read_placed(&narrow);

  assert_eq!(stood.len(), 1);
  assert_eq!(stood[0].machine, "machine-2");
  assert_eq!(
    (stood[0].entry.checkout.as_str(), stood[0].entry.command.as_str()),
    ("/checkout", "bmo-testbed suite")
  );
  assert_eq!(stood[0].entry.scope.as_deref(), Some("scope-3"));
  assert_eq!(stood[0].entry.pid, i64::from(std::process::id()));
  assert_eq!(stood[0].entry.since, None, "it waits until its turn there comes");
  assert!(
    read_placed(&lock_path(&env, Lane::Wide)).is_empty(),
    "it stands in the lane it asked for alone"
  );

  run.turn_came();

  assert!(read_placed(&narrow)[0].entry.since.is_some());

  drop(run);

  assert!(read_placed(&narrow).is_empty());
}

// "an entry whose process is gone is nobody's, and the next run placed takes it away"
#[test]
fn an_entry_whose_process_is_gone_is_left_out_and_taken_away() {
  let dir = tempfile::tempdir().unwrap();
  let env = Env::from([(
    "BMO_TESTBED_LOCK".to_owned(),
    dir.path().join("wide.lock").to_string_lossy().into_owned(),
  )]);
  let narrow = lock_path(&env, Lane::Narrow);
  let placed_dir = dir.path().join("wide-narrow.lock.placed");
  // A process that has ended: one this test started and waited for.
  let mut ended = std::process::Command::new("true").spawn().unwrap();
  let gone_pid = i64::from(ended.id());

  ended.wait().unwrap();
  std::fs::create_dir_all(&placed_dir).unwrap();

  let gone = PlacedEntry {
    entry: TurnEntry {
      checkout: "/gone".to_owned(),
      pid: gone_pid,
      ..TurnEntry::default()
    },
    machine: "machine-2".to_owned(),
  };

  std::fs::write(placed_dir.join("0-gone"), serde_json::to_string(&gone).unwrap()).unwrap();

  assert!(read_placed(&narrow).is_empty());

  let command = vec!["cargo".to_owned()];
  let _run = PlacedRun::stand(&ask(&command, &env), Path::new("/checkout"), "machine-2");

  assert!(!placed_dir.join("0-gone").exists());
  assert_eq!(read_placed(&narrow).len(), 1);
}

// "a machine is loaded where its load average over the last minute stands at or above a share of its threads":
// the line names the load, the threads and the share, and a load just under the bound is no load
#[test]
fn a_machine_is_loaded_from_its_share_of_threads_on() {
  let said = fill_marks(&LANES.placed.loaded, &[("load", "12.00"), ("threads", "16"), ("percent", "75")]);

  assert_eq!(loaded(12.0, 16, 75), Some(said));
  assert_eq!(loaded(11.99, 16, 75), None);
  assert!(loaded(0.0, 16, 0).is_some(), "a share of 0 makes any machine loaded");
}

// "`loaded_percent` in the lane policy, and `BMO_TESTBED_LOADED_PERCENT` names another whole number"; anything
// that is not a whole number of 0 or more means the policy's
#[test]
fn the_loaded_share_is_the_policys_unless_the_variable_names_a_whole_number() {
  let named = |given: &str| loaded_percent(&Env::from([(LOADED_VARIABLE.to_owned(), given.to_owned())]));

  assert_eq!(loaded_percent(&Env::new()), LANES.numbers.loaded_percent);
  assert_eq!(named("0"), 0);
  assert_eq!(named("150"), 150);
  assert_eq!(named("-1"), LANES.numbers.loaded_percent);
  assert_eq!(named("many"), LANES.numbers.loaded_percent);
}
