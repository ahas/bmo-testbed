use bmo_testbed_core::turn::{Env, FRONT_VARIABLE};
use bmo_testbed_suite::crates::{
  DEPLOYMENT_MARK_NAME, cargo_environment, checkout_target_dir, mark_deployment_checkout, set_by_cargo,
};

#[test]
fn what_cargo_sets_for_a_package_is_told_apart_from_what_names_cargo_s_own_places() {
  for set in [
    "CARGO",
    "CARGO_MANIFEST_DIR",
    "CARGO_PKG_NAME",
    "CARGO_BIN_EXE_sample",
    "CARGO_TARGET_TMPDIR",
    "OUT_DIR",
  ] {
    assert!(set_by_cargo(set), "{set}");
  }

  for kept in ["CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_BUILD_BUILD_DIR", "RUSTUP_HOME", "PATH"] {
    assert!(!set_by_cargo(kept), "{kept}");
  }
}

#[test]
fn a_checkout_builds_in_a_directory_of_its_own_so_builds_of_two_checkouts_take_no_lock_of_each_other() {
  let first = tempfile::tempdir().unwrap();
  let second = tempfile::tempdir().unwrap();
  let built_in = |root: &std::path::Path, name: &str| {
    cargo_environment(root).into_iter().find(|(key, _)| key == name).map(|(_, value)| value)
  };

  for root in [first.path(), second.path()] {
    let own = checkout_target_dir(root).to_string_lossy().into_owned();

    assert_eq!(built_in(root, "CARGO_BUILD_BUILD_DIR").as_deref(), Some(own.as_str()));
    assert_eq!(built_in(root, "CARGO_TARGET_DIR").as_deref(), Some(own.as_str()));
  }

  assert_ne!(
    built_in(first.path(), "CARGO_BUILD_BUILD_DIR"),
    built_in(second.path(), "CARGO_BUILD_BUILD_DIR")
  );
}

// "the Cargo verb, run for a command standing at the front ..., writes a mark into the checkout's directory under
// the build root": a deployment's caller marks it, any other caller leaves it unmarked
#[test]
fn a_build_standing_at_the_front_marks_its_checkout_s_directory_as_a_deployment_s() {
  let dir = tempfile::tempdir().unwrap();
  let working = dir.path().join("working");
  let deployment = dir.path().join("deployment");

  mark_deployment_checkout(&working, &Env::new());
  mark_deployment_checkout(&deployment, &Env::from([(FRONT_VARIABLE.to_owned(), "1".to_owned())]));

  assert!(!working.join(DEPLOYMENT_MARK_NAME).exists());
  assert!(deployment.join(DEPLOYMENT_MARK_NAME).exists());
}
