use bmo_testbed_suite::crates::set_by_cargo;

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
