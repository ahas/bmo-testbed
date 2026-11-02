//! This repository's own units (docs/deploy.md, 「The Cargo build directory」 and 「The unit links」;
//! docs/backup.md): its declaration reaches them, its deployment keeps the prune's and the data backup's
//! timers with their services by their marks, and each service runs its verb of the binary its own deploy
//! tree holds.

use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::turn::Env;
use bmo_testbed_deploy::deployment::DeployPlaces;
use bmo_testbed_deploy::tree::{UnitMark, marked_units, timer_service, unit_program};

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
      "units/bmo-cargo-prune.service",
      "units/bmo-cargo-prune.timer",
      "units/bmo-data-backup-drill.service",
      "units/bmo-data-backup-drill.timer",
      "units/bmo-data-backup-media.service",
      "units/bmo-data-backup-media.timer",
      "units/bmo-data-backup.service",
      "units/bmo-data-backup.timer",
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
