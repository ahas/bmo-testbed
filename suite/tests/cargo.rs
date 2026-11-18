use bmo_testbed_core::languages::cargo_compiles as compiles;
use bmo_testbed_suite::cargo::{CargoInvocation, cargo_invocation};
use bmo_testbed_suite::crates::checkout_target_dir;

fn words(given: &[&str]) -> Vec<String> {
  given.iter().map(|one| (*one).to_owned()).collect()
}

fn named<'a>(invocation: &'a CargoInvocation, variable: &str) -> &'a str {
  invocation.named(variable).unwrap_or_default()
}

#[test]
fn clippy_runs_as_a_check_through_the_checkouts_own_wrapper_with_the_lint_arguments_after_the_mark() {
  let dir = tempfile::tempdir().unwrap();
  let invocation = cargo_invocation(dir.path(), &words(&["clippy", "--workspace", "--", "-D", "warnings"]));

  assert_eq!(invocation.command[1..], words(&["check", "--workspace"]));
  assert!(invocation.command[0].ends_with("cargo"));
  assert_eq!(named(&invocation, "CLIPPY_ARGS"), "-D__CLIPPY_HACKERY__warnings__CLIPPY_HACKERY__");
  assert_eq!(
    named(&invocation, "RUSTC_WORKSPACE_WRAPPER"),
    checkout_target_dir(dir.path()).join("workspace-clippy/clippy-driver").to_string_lossy()
  );
  assert_eq!(invocation.env.iter().filter(|(name, _)| name == "RUSTC_WORKSPACE_WRAPPER").count(), 1);
}

#[test]
fn any_other_command_goes_to_cargo_as_it_stands_in_the_checkouts_own_target_directory() {
  let dir = tempfile::tempdir().unwrap();
  let invocation =
    cargo_invocation(dir.path(), &words(&["test", "-p", "sample-crate", "--", "--nocapture"]));

  assert_eq!(invocation.command[1..], words(&["test", "-p", "sample-crate", "--", "--nocapture"]));
  assert_eq!(
    named(&invocation, "CARGO_TARGET_DIR"),
    checkout_target_dir(dir.path()).to_string_lossy()
  );
  assert!(invocation.named("CLIPPY_ARGS").is_none());
}

#[test]
fn a_cargo_started_under_another_cargos_run_is_handed_nothing_that_cargo_set_for_its_own_package() {
  let dir = tempfile::tempdir().unwrap();
  let invocation = cargo_invocation(dir.path(), &words(&["fmt"]));
  let command = invocation.command(dir.path());
  let removed: Vec<String> = command
    .get_envs()
    .filter(|(_, value)| value.is_none())
    .map(|(name, _)| name.to_string_lossy().into_owned())
    .collect();

  // This test runs under Cargo, which set its own package's variables for it.
  assert!(removed.iter().any(|name| name == "CARGO_MANIFEST_DIR"), "{removed:?}");
  assert!(removed.iter().any(|name| name == "CARGO_PKG_NAME"), "{removed:?}");
}

#[test]
fn a_command_compiles_past_a_toolchain_and_the_options_before_it_and_a_reading_verb_does_not() {
  assert!(compiles(&words(&["+nightly", "--locked", "clippy"])));
  assert!(compiles(&words(&["test", "-p", "sample-crate"])));
  assert!(!compiles(&words(&["fmt", "--all", "--check"])));
  assert!(!compiles(&words(&["metadata", "--no-deps"])));
  assert!(!compiles(&[]));
}

// "the Cargo verb sets CARGO_PROFILE_DEV_INCREMENTAL and CARGO_PROFILE_TEST_INCREMENTAL to true ..., unless its
// caller already names them", and "the verb names it as RUSTC_WRAPPER ... a caller that names RUSTC_WRAPPER
// keeps its own, and a machine without the program builds as it always did".
#[test]
fn a_build_keeps_incremental_state_in_its_development_profiles_and_goes_through_the_machine_s_compile_cache()
 {
  use bmo_testbed_suite::cargo::{INCREMENTAL_VARIABLES, WRAPPER_VARIABLE, build_cache};

  let set = |name: &str, value: &str| (name.to_owned(), value.to_owned());
  let none = std::collections::HashMap::new();
  let cache = std::path::Path::new("/opt/tools/sccache");
  let incremental = INCREMENTAL_VARIABLES.map(|name| set(name, "true"));

  assert_eq!(build_cache(&none, None), incremental);
  assert_eq!(
    build_cache(&none, Some(cache)),
    [
      incremental[0].clone(),
      incremental[1].clone(),
      set(WRAPPER_VARIABLE, "/opt/tools/sccache")
    ]
  );

  let own: std::collections::HashMap<String, String> =
    [set(INCREMENTAL_VARIABLES[0], "false"), set(WRAPPER_VARIABLE, "other")]
      .into_iter()
      .collect();

  assert_eq!(build_cache(&own, Some(cache)), [incremental[1].clone()]);
}
