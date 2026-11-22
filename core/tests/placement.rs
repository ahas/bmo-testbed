//! A run placed elsewhere standing on the machine that placed it (docs/boundary.md, 「A run placed elsewhere
//! stands here too」): read back in its lane while it lasts, running once its turn there came, taken away as it
//! ends, and an entry whose process is gone left out; and the load at or above which a machine takes no run
//! (docs/testbed.md, 「A run on another machine」).

use std::path::Path;

use bmo_testbed_core::placement::{
  Closed, LOADED_VARIABLE, PlaceAsk, PlacedEntry, PlacedRun, lane_takes, load_figure, loaded,
  loaded_percent, read_placed,
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

// "a machine is loaded where its load figure stands at or above a share of its threads": the line the reason
// carries names the load, the threads and the share, and a load just under the bound is no load
#[test]
fn a_machine_is_loaded_from_its_share_of_threads_on() {
  // Sixty-seven percent of sixteen threads is 10.72, a hair under the two thirds one seat of a three-seat
  // wide lane asks for.
  let said = fill_marks(&LANES.placed.loaded, &[("load", "10.72"), ("threads", "16"), ("percent", "67")]);

  assert_eq!(loaded(10.72, 16, 67), Some(Closed::Loaded(said)));
  assert_eq!(loaded(10.71, 16, 67), None);
  assert!(loaded(0.0, 16, 0).is_some(), "a share of 0 makes any machine loaded");
}

// "its load figure, the larger of its last-minute and last-five-minute averages": the kernel's line is read for
// those two figures alone, and a line that gives neither reads as no load
#[test]
fn the_load_figure_is_the_larger_of_the_first_two_averages() {
  assert_eq!(load_figure("11.30 8.90 13.30 1/234 5678"), Some(11.3));
  assert_eq!(load_figure("8.90 11.30 13.30 1/234 5678"), Some(11.3));
  assert_eq!(load_figure("2.00 1.00"), Some(2.0));
  assert_eq!(load_figure("0.50"), Some(0.5));
  assert_eq!(load_figure("many 1.00"), None);
  assert_eq!(load_figure(""), None);
}

// "a machine whose every seat is held or waited for takes a run only from the last of the order, and a loaded
// machine takes none wherever it stands"; a lane that closes for no reason takes a run wherever it stands
#[test]
fn seats_turn_a_run_away_only_before_the_last_machine_and_load_turns_it_away_everywhere() {
  assert!(lane_takes(None, false) && lane_takes(None, true));
  assert!(
    !lane_takes(Some(Closed::Seats), false),
    "a machine with others behind it passes the run on"
  );
  assert!(lane_takes(Some(Closed::Seats), true), "the last machine waits the run in its line");
  let loaded = || Some(Closed::Loaded("loaded".to_owned()));

  assert!(
    !lane_takes(loaded(), false) && !lane_takes(loaded(), true),
    "a loaded machine takes no run wherever it stands"
  );
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
