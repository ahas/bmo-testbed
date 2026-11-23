//! The supervisor's places: the runtime directory it hands on.

use std::os::unix::fs::PermissionsExt;

use crate::support::Bench;

#[test]
fn a_runtime_directory_that_stands_is_handed_on_as_it_is() {
  let bench = Bench::new();
  let named = bench.path("run-user");

  std::fs::create_dir_all(&named).unwrap();

  assert_eq!(bench.places.runtime(&named).unwrap(), named);
}

#[test]
fn where_none_stands_the_supervisor_makes_its_own_for_the_account_alone() {
  let bench = Bench::new();
  let own = bench.places.runtime(&bench.path("no-such-runtime")).unwrap();

  assert_eq!(own, bench.places.folder.join("run"));
  assert_eq!(std::fs::metadata(&own).unwrap().permissions().mode() & 0o777, 0o700);
  assert_eq!(bench.places.runtime(&bench.path("no-such-runtime")).unwrap(), own);
}
