//! The Cargo profile check (docs/checks.md, 「The quick checks」): the workspace manifest holds every setting
//! that keeps each build small, a setting left out or set otherwise named, and a repository with no
//! manifest has nothing to hold.

use bmo_testbed_suite::quick::Findings;
use bmo_testbed_suite::quick::cargo_profile::checkout_cargo_profile;

use crate::quick::checkout;

const SMALL: &str = "[workspace]\nmembers = []\n\n[profile.dev]\ndebug = \"line-tables-only\"\nsplit-debuginfo = \"unpacked\"\nincremental = false\n\n[profile.test]\ndebug = \"line-tables-only\"\nstrip = \"debuginfo\"\nincremental = false\n\n[profile.release]\ndebug = false\nstrip = \"debuginfo\"\n";

// "a manifest holding every setting passes, each setting read"
#[test]
fn a_manifest_holding_every_setting_passes() {
  let dir = checkout(&[("Cargo.toml", SMALL)]);

  assert_eq!(
    checkout_cargo_profile(dir.path()),
    Findings {
      paths: 8,
      findings: vec![],
    }
  );
}

// "a setting set otherwise or left out is named with what it should be"
#[test]
fn a_setting_set_otherwise_or_left_out_is_named_with_what_it_should_be() {
  let dir = checkout(&[(
    "Cargo.toml",
    "[workspace]\n\n[profile.dev]\ndebug = true\nsplit-debuginfo = \"unpacked\"\nincremental = false\n\n[profile.test]\ndebug = \"line-tables-only\"\nincremental = true\n\n[profile.release]\ndebug = false\n",
  )]);

  assert_eq!(
    checkout_cargo_profile(dir.path()).findings,
    [
      "Cargo.toml: [profile.dev] debug is true where it should be \"line-tables-only\"",
      "Cargo.toml: [profile.release] strip is not set where it should be \"debuginfo\"",
      "Cargo.toml: [profile.test] incremental is true where it should be false",
      "Cargo.toml: [profile.test] strip is not set where it should be \"debuginfo\"",
    ]
  );
}

// "a repository with no workspace manifest has nothing to hold, and one that does not read is said to"
#[test]
fn no_manifest_has_nothing_to_hold_and_one_that_does_not_read_is_said_to() {
  let empty = checkout(&[("README.md", "nothing built\n")]);
  let broken = checkout(&[("Cargo.toml", "[profile.dev\n")]);

  assert_eq!(checkout_cargo_profile(empty.path()), Findings::default());
  assert!(
    checkout_cargo_profile(broken.path()).findings[0]
      .starts_with("Cargo.toml does not read as a Cargo manifest: ")
  );
}
