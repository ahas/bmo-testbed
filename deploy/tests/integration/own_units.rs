//! This repository's own units (docs/deploy.md, 「The Cargo build directory」 and 「The unit links」): its
//! declaration reaches them, its deployment keeps the prune's timer with its service by their marks, and the
//! service runs the prune of the binary its own deploy tree holds.

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
fn the_repositorys_deployment_keeps_the_prune_and_its_timer() {
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

  assert_eq!(files, ["units/bmo-cargo-prune.service", "units/bmo-cargo-prune.timer"]);
  assert!(files.iter().all(|file| globs.iter().any(|glob| glob.matches(file))), "{files:?}");
  assert_eq!(marked_units(&files, &read, ".timer", UnitMark::Timer), ["units/bmo-cargo-prune.timer"]);
  assert_eq!(timer_service("bmo-cargo-prune.timer"), "bmo-cargo-prune.service");

  let service = read("units/bmo-cargo-prune.service");
  let places =
    DeployPlaces::of(&repo, &Env::from([("HOME".to_owned(), "/home/someone".to_owned())])).unwrap();
  let program = unit_program(&service).unwrap().replace("%h", "/home/someone");
  let started = service.lines().find(|line| line.starts_with("ExecStart=")).unwrap();

  assert_eq!(PathBuf::from(program), places.deploy_dir.join("bin/bmo-testbed"));
  assert!(started.ends_with(" prune"), "{started}");
  assert!(repo.declaration.build.binaries.unwrap().contains(&"bmo-testbed".to_owned()));
}
