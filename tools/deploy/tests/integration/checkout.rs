//! The checkout a repository is deployed from (docs/deploy.md, 「The hosted deploy tool」): the one the
//! machine's registry lists whose declaration names the repository, and the refusals where there is not
//! one.

use std::collections::HashMap;

use bmo_testbed_core::turn::Env;
use deploy_tool::{find_checkout, same_name};

use crate::stand::{checkout, registry, write};

fn env() -> Env {
  HashMap::from([("HOME".to_owned(), "/home".to_owned())])
}

// "a name is the same with or without a leading bmo-"
#[test]
fn a_name_is_the_same_with_or_without_a_leading_bmo() {
  assert!(same_name("bmo-gate", "gate"));
  assert!(same_name("agency", "bmo-agency"));
  assert!(same_name("receiver", " receiver "));
  assert!(!same_name("bmo-gate", "gate-harvest"));
  assert!(!same_name("agency", "agency-w2"));
}

// "the checkout is the one whose declared name is the repository asked for"
#[test]
fn the_checkout_is_the_one_whose_declared_name_is_asked_for() {
  let dir = tempfile::tempdir().unwrap();
  let gate = checkout(dir.path(), "gate", "bmo-gate");
  let agency = checkout(dir.path(), "agency", "agency");
  let at = registry(dir.path(), &[&gate, &agency]);

  assert_eq!(find_checkout("gate", &at, &env()).unwrap().root, gate);
  assert_eq!(find_checkout("bmo-agency", &at, &env()).unwrap().root, agency);
}

// "a registry that does not stand is refused, naming its path"
#[test]
fn a_registry_that_does_not_stand_is_refused_naming_its_path() {
  let dir = tempfile::tempdir().unwrap();
  let at = dir.path().join("checkouts.toml");
  let refusal = find_checkout("agency", &at, &env()).unwrap_err();

  assert!(refusal.contains(&at.to_string_lossy().into_owned()), "{refusal}");
  assert!(refusal.contains("does not stand"), "{refusal}");
}

// "a registry that does not read is refused, naming its path"
#[test]
fn a_registry_that_does_not_read_is_refused_naming_its_path() {
  let dir = tempfile::tempdir().unwrap();
  let at = dir.path().join("checkouts.toml");

  write(&at, "checkouts = 7\n");

  let refusal = find_checkout("agency", &at, &env()).unwrap_err();

  assert!(refusal.contains(&at.to_string_lossy().into_owned()), "{refusal}");
  assert!(refusal.contains("does not read"), "{refusal}");
}

// "a name no checkout declares is refused, naming the names the registry holds and the unread checkouts"
#[test]
fn a_name_no_checkout_declares_is_refused_naming_the_names_held() {
  let dir = tempfile::tempdir().unwrap();
  let gate = checkout(dir.path(), "gate", "bmo-gate");
  let agency = checkout(dir.path(), "agency", "agency");
  let broken = dir.path().join("broken");

  write(&broken.join("testbed.toml"), "testbed = 0\nunknown_key = 1\n");

  let at = registry(dir.path(), &[&gate, &agency, &broken]);
  let refusal = find_checkout("receiver", &at, &env()).unwrap_err();

  assert!(refusal.contains("receiver"), "{refusal}");
  assert!(refusal.contains("agency, bmo-gate"), "{refusal}");
  assert!(refusal.contains(&broken.to_string_lossy().into_owned()), "{refusal}");
}

// "a name two checkouts declare is refused, naming both"
#[test]
fn a_name_two_checkouts_declare_is_refused_naming_both() {
  let dir = tempfile::tempdir().unwrap();
  let one = checkout(dir.path(), "one", "agency");
  let other = checkout(dir.path(), "other", "bmo-agency");
  let at = registry(dir.path(), &[&one, &other, &one]);
  let refusal = find_checkout("agency", &at, &env()).unwrap_err();

  assert!(refusal.contains("more than one"), "{refusal}");
  assert!(refusal.contains(&one.to_string_lossy().into_owned()), "{refusal}");
  assert!(refusal.contains(&other.to_string_lossy().into_owned()), "{refusal}");
}
