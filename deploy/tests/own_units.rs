//! This repository's own units (docs/deploy.md, 「The Cargo build directory」, 「The unit links」 and 「The
//! slices」; docs/backup.md): its declaration reaches them, its deployment keeps the prune's and the data
//! backup's timers with their services by their marks, each service runs its verb of the binary its own
//! deploy tree holds, every slice a unit runs in is one it ships, linked by its mark, and the placed machine
//! is laid the prune's timer, its service and its slice and nothing else.

use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::turn::Env;
use bmo_testbed_deploy::deployment::DeployPlaces;
use bmo_testbed_deploy::tree::{
  PlacedUnits, UnitMark, marked_units, placed_units, timer_service, unit_program,
};
use bmo_testbed_deploy::unit::{BACKUP_SLICE, DEPLOY_SLICE};

fn root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn unit_files() -> Vec<String> {
  let mut found: Vec<String> = std::fs::read_dir(root().join("units"))
    .unwrap()
    .flatten()
    .map(|entry| format!("units/{}", entry.file_name().to_string_lossy()))
    .collect();

  found.sort();
  found
}

#[test]
fn the_repositorys_deployment_keeps_the_prune_the_data_backup_and_their_timers() {
  let repo = Repository::read(&root()).unwrap();
  let files = unit_files();
  let read = |path: &str| std::fs::read_to_string(root().join(path)).unwrap_or_default();
  let globs: Vec<glob::Pattern> = repo
    .declaration
    .deploy
    .units
    .clone()
    .unwrap()
    .iter()
    .map(|one| glob::Pattern::new(one).unwrap())
    .collect();
  let timers = [
    ("units/bmo-cargo-prune.timer", "bmo-cargo-prune.service", " prune"),
    ("units/bmo-data-backup-drill.timer", "bmo-data-backup-drill.service", " backup drill"),
    ("units/bmo-data-backup-media.timer", "bmo-data-backup-media.service", " backup media"),
    ("units/bmo-data-backup.timer", "bmo-data-backup.service", " backup snapshot"),
  ];

  assert_eq!(
    files,
    [
      "units/bmo-backup.slice",
      "units/bmo-cargo-prune.service",
      "units/bmo-cargo-prune.timer",
      "units/bmo-check.slice",
      "units/bmo-data-backup-drill.service",
      "units/bmo-data-backup-drill.timer",
      "units/bmo-data-backup-media.service",
      "units/bmo-data-backup-media.timer",
      "units/bmo-data-backup.service",
      "units/bmo-data-backup.timer",
      "units/bmo-deploy.slice",
      "units/bmo-suite.slice",
      "units/bmo-upkeep.slice",
    ]
  );
  assert!(files.iter().all(|file| globs.iter().any(|glob| glob.matches(file))), "{files:?}");
  assert_eq!(
    marked_units(&files, &read, ".timer", UnitMark::Timer),
    timers.iter().map(|(timer, _, _)| *timer).collect::<Vec<_>>()
  );

  let places = DeployPlaces::of(&repo, &Env::from([("HOME".to_owned(), "/account".to_owned())])).unwrap();

  for (timer, service, verb) in timers {
    assert_eq!(timer_service(timer.trim_start_matches("units/")), service);

    let unit = read(&format!("units/{service}"));
    let program = unit_program(&unit).unwrap().replace("%h", "/account");
    let started = unit.lines().find(|line| line.starts_with("ExecStart=")).unwrap();

    assert_eq!(PathBuf::from(program), places.deploy_dir.join("bin/bmo-testbed"));
    assert!(started.ends_with(verb), "{started}");
  }

  assert!(repo.declaration.build.binaries.unwrap().contains(&"bmo-testbed".to_owned()));
}

#[test]
fn every_slice_a_unit_runs_in_is_one_the_repository_ships_and_links() {
  let files = unit_files();
  let read = |path: &str| std::fs::read_to_string(root().join(path)).unwrap_or_default();
  let slices: Vec<String> = files.iter().filter(|file| file.ends_with(".slice")).cloned().collect();
  let shipped = |slice: &str| slices.contains(&format!("units/{slice}"));

  assert_eq!(marked_units(&files, &read, ".slice", UnitMark::Link), slices);
  assert!(
    slices
      .iter()
      .all(|slice| read(slice).lines().any(|line| line.starts_with("Description=")))
  );
  assert!(shipped(DEPLOY_SLICE) && shipped(BACKUP_SLICE), "{slices:?}");

  for (service, slice) in [
    ("bmo-cargo-prune.service", "bmo-upkeep.slice"),
    ("bmo-data-backup-drill.service", BACKUP_SLICE),
    ("bmo-data-backup-media.service", BACKUP_SLICE),
    ("bmo-data-backup.service", BACKUP_SLICE),
  ] {
    let unit = read(&format!("units/{service}"));

    assert!(unit.lines().any(|line| line == format!("Slice={slice}")), "{service}");
    assert!(shipped(slice), "{slice}");
  }
}

// "every deployment of the testbed lays them on the placed machine and starts the timer there"
#[test]
fn the_placed_machine_is_laid_the_prune_s_timer_its_service_and_its_slice_alone() {
  let files = unit_files();
  let read = |path: &str| std::fs::read_to_string(root().join(path)).unwrap_or_default();
  let placed = placed_units(&files, &read);

  assert_eq!(
    placed,
    PlacedUnits {
      laid: ["units/bmo-cargo-prune.timer", "units/bmo-cargo-prune.service", "units/bmo-upkeep.slice"]
        .map(str::to_owned)
        .to_vec(),
      timers: vec!["bmo-cargo-prune.timer".to_owned()],
    }
  );

  // The timer laid there is the file kept here, so its schedule is this machine's.
  assert!(marked_units(&files, &read, ".timer", UnitMark::Timer).contains(&placed.laid[0]));
}
