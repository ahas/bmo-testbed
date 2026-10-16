//! The test server's address with another database in place of its own (docs/suite.md, 「The test
//! database」), whatever else the address says left as it stands.

use std::sync::LazyLock;

use bmo_testbed_scratch_database::{SCRATCH, address_in};

#[test]
fn the_policy_reads_as_its_type() {
  LazyLock::force(&SCRATCH);

  assert!(SCRATCH.names.template != SCRATCH.names.copy);
  assert!(SCRATCH.server.connections > 0);
}

#[test]
fn another_database_takes_the_place_of_the_addresss_own() {
  assert_eq!(
    address_in("postgres:///bmo_test?host=/var/run/postgresql", "postgres"),
    "postgres:///postgres?host=/var/run/postgresql"
  );
  assert_eq!(
    address_in("postgres://tester:secret@localhost:5433/bmo_test", "bmo_rs_copy_1_0"),
    "postgres://tester:secret@localhost:5433/bmo_rs_copy_1_0"
  );
}

#[test]
fn an_address_naming_no_database_is_given_one() {
  assert_eq!(
    address_in("postgres://localhost:5432", "postgres"),
    "postgres://localhost:5432/postgres"
  );
}
