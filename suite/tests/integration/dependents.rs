//! The dependents of a major change (docs/suite.md, 「The dependents of a major change」): the machine's
//! registry of checkouts, the paths it makes whole, and the consumers whose own declaration names an upstream.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::turn::Env;
use bmo_testbed_suite::dependents::{
  CHECKOUTS_VARIABLE, FaultKind, consumers, read_registry, registry_path,
};

fn write(path: &Path, text: &str) {
  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
}

fn declaration(name: &str, upstreams: &[&str]) -> String {
  let listed = upstreams.iter().map(|one| format!("\"{one}\"")).collect::<Vec<_>>().join(", ");

  format!("testbed = 0\nname = \"{name}\"\nupstreams = [{listed}]\n")
}

fn env(home: &Path) -> Env {
  HashMap::from([("HOME".to_owned(), home.to_string_lossy().into_owned())])
}

// "the file is checkouts.toml under the data root's testbed, unless the variable names another"
#[test]
fn the_registry_is_the_data_roots_file_unless_the_variable_names_another() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path().join("data");
  let named = dir.path().join("elsewhere.toml");

  assert_eq!(registry_path(&root, &env(dir.path())), root.join("testbed").join("checkouts.toml"));
  assert_eq!(
    registry_path(
      &root,
      &HashMap::from([(CHECKOUTS_VARIABLE.to_owned(), named.to_string_lossy().into_owned())]),
    ),
    named
  );
}

// "a path is absolute, begins with ~/, or is read against the registry file's folder"
#[test]
fn a_registrys_paths_are_made_whole_against_its_own_folder() {
  let dir = tempfile::tempdir().unwrap();
  let registry = dir.path().join("registry").join("checkouts.toml");
  let home = dir.path().join("home");

  write(
    &registry,
    &format!("checkouts = [\"../first\", \"{}/second\", \"~/third\"]\n", dir.path().display()),
  );

  let found = read_registry(&registry, &env(&home)).unwrap();

  assert_eq!(found, [dir.path().join("first"), dir.path().join("second"), home.join("third")]);
}

// "the registry names checkouts in its list or in its checkout tables"
#[test]
fn a_registry_names_checkouts_in_its_list_or_its_tables() {
  let dir = tempfile::tempdir().unwrap();
  let registry = dir.path().join("checkouts.toml");

  write(
    &registry,
    "checkouts = [\"first\", { path = \"second\" }]\n[[checkout]]\npath = \"third\"\n",
  );

  let found = read_registry(&registry, &env(dir.path())).unwrap();

  assert_eq!(found, [dir.path().join("first"), dir.path().join("second"), dir.path().join("third")]);
}

// "a registry that does not stand holds no checkouts; one that does not read is refused"
#[test]
fn a_registry_that_does_not_stand_holds_none_and_one_that_does_not_read_is_a_fault() {
  let dir = tempfile::tempdir().unwrap();
  let missing = dir.path().join("checkouts.toml");

  assert!(read_registry(&missing, &env(dir.path())).unwrap().is_empty());

  write(&missing, "checkouts = \n");
  let fault = read_registry(&missing, &env(dir.path())).unwrap_err();

  assert_eq!(fault.kind, FaultKind::Registry);
  assert_eq!(fault.path, missing);
}

// "the checkouts whose declaration names the upstream, and no consumer of a consumer"
#[test]
fn the_consumers_are_the_checkouts_that_name_the_upstream_and_only_directly() {
  let dir = tempfile::tempdir().unwrap();
  let upstream = dir.path().join("upstream");
  let first = dir.path().join("first");
  let second = dir.path().join("second");
  let later = dir.path().join("later");

  write(&upstream.join("testbed.toml"), &declaration("shared", &[]));
  write(&first.join("testbed.toml"), &declaration("alpha", &["shared", "other"]));
  write(&second.join("testbed.toml"), &declaration("beta", &["other"]));
  write(&later.join("testbed.toml"), &declaration("gamma", &["alpha"]));

  let registry = dir.path().join("checkouts.toml");
  write(
    &registry,
    &format!(
      "checkouts = [\"{}\", \"{}\", \"{}\", \"{}\"]\n",
      upstream.display(),
      later.display(),
      second.display(),
      first.display()
    ),
  );

  let upstream = Repository::read(&upstream).unwrap();
  let found = consumers(&upstream, &registry, &env(dir.path())).unwrap();

  assert_eq!(found.iter().map(Repository::name).collect::<Vec<_>>(), ["alpha"]);
  assert_eq!(found[0].root, first);
}

// The upstream may also be named beside the dependency check, and both lists are read.
#[test]
fn an_upstream_named_beside_the_dependency_check_is_found_too() {
  let dir = tempfile::tempdir().unwrap();
  let upstream = dir.path().join("upstream");
  let consumer = dir.path().join("consumer");

  write(&upstream.join("testbed.toml"), &declaration("shared", &[]));
  write(
    &consumer.join("testbed.toml"),
    "testbed = 0\nname = \"consumer\"\n[check]\nupstreams = [\"shared\"]\n",
  );
  let registry = dir.path().join("checkouts.toml");

  write(&registry, &format!("checkouts = [\"{}\"]\n", consumer.display()));

  let upstream = Repository::read(&upstream).unwrap();
  let found = consumers(&upstream, &registry, &env(dir.path())).unwrap();

  assert_eq!(found.len(), 1);
  assert_eq!(found[0].root, consumer);
}

// A checkout in the registry whose declaration does not read is a fault naming its declaration.
#[test]
fn a_consumers_declaration_that_does_not_read_is_a_fault() {
  let dir = tempfile::tempdir().unwrap();
  let upstream = dir.path().join("upstream");
  let broken = dir.path().join("broken");

  write(&upstream.join("testbed.toml"), &declaration("shared", &[]));
  write(&broken.join("testbed.toml"), "testbed = \n");
  let registry = dir.path().join("checkouts.toml");
  write(&registry, &format!("checkouts = [\"{}\"]\n", broken.display()));

  let upstream = Repository::read(&upstream).unwrap();
  let fault = consumers(&upstream, &registry, &env(dir.path())).unwrap_err();

  assert_eq!(fault.kind, FaultKind::Declaration);
  assert_eq!(fault.path, broken);
}

// The path a repository is named by is its declaration's name, not its folder's.
#[test]
fn a_consumer_naming_a_declared_upstream_is_found_by_that_declaration() {
  let dir = tempfile::tempdir().unwrap();
  let upstream = dir.path().join("folder-name");
  let consumer = dir.path().join("consumer");

  write(&upstream.join("testbed.toml"), &declaration("declared-upstream", &[]));
  write(&consumer.join("testbed.toml"), &declaration("consumer", &["declared-upstream"]));
  let registry = dir.path().join("checkouts.toml");
  write(&registry, &format!("checkouts = [\"{}\"]\n", consumer.display()));

  let upstream = Repository::read(&upstream).unwrap();
  let found = consumers(&upstream, &registry, &env(dir.path())).unwrap();

  assert_eq!(found.len(), 1);
  assert_eq!(found[0].root, PathBuf::from(&consumer));
}
