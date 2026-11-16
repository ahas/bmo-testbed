//! A test's own database on the test server (docs/suite.md, 「The test database」): migrated by the migrator
//! the test hands in, empty, apart from every other test's, and gone once its guard is.

use bmo_testbed_scratch_database::{SCRATCH, Scratch, connect};
use sea_orm::{ConnectionTrait, DbBackend, Statement};

use crate::migrators::{Kept, Other};

// Whether the test server holds a database of that name, asked of the server's own listing.
async fn stands(database: &str) -> bool {
  let admin = connect(&SCRATCH.server.admin_database).await;
  let row = admin
    .query_one(Statement::from_sql_and_values(
      DbBackend::Postgres,
      "SELECT count(*) FROM pg_database WHERE datname = $1",
      [database.into()],
    ))
    .await
    .unwrap()
    .unwrap();
  let count: i64 = row.try_get_by_index(0).unwrap();

  admin.close().await.unwrap();
  count == 1
}

// Whether the database holds a table of that name.
async fn holds_table(scratch: &Scratch, table: &str) -> bool {
  scratch
    .count(
      "SELECT count(*) FROM information_schema.tables WHERE table_name = $1",
      vec![table.into()],
    )
    .await
    == 1
}

#[tokio::test]
async fn a_scratch_database_is_migrated_and_empty() {
  let scratch = Scratch::new::<Kept>().await;

  assert!(holds_table(&scratch, "kept").await);
  assert_eq!(scratch.count("SELECT count(*) FROM kept", vec![]).await, 0);
  assert!(scratch.address().contains(scratch.name()));
}

#[tokio::test]
async fn two_scratch_databases_stand_apart() {
  let one = Scratch::new::<Kept>().await;
  let two = Scratch::new::<Kept>().await;

  one.run("INSERT INTO kept (id) VALUES ($1)", vec![1.into()]).await;

  assert_ne!(one.name(), two.name());
  assert_eq!(one.count("SELECT count(*) FROM kept", vec![]).await, 1);
  assert_eq!(two.count("SELECT count(*) FROM kept", vec![]).await, 0);
}

#[tokio::test]
async fn each_migrator_handed_in_makes_a_template_of_its_own() {
  let kept = Scratch::new::<Kept>().await;
  let other = Scratch::new::<Other>().await;

  assert!(holds_table(&kept, "kept").await);
  assert!(!holds_table(&kept, "other").await);
  assert!(holds_table(&other, "other").await);
  assert!(!holds_table(&other, "kept").await);
}

#[tokio::test]
async fn a_blank_database_holds_no_table_and_goes_by_the_copies_name() {
  let blank = Scratch::blank().await;

  assert_eq!(
    blank
      .count("SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'", vec![])
      .await,
    0
  );
  assert!(blank.name().starts_with(&SCRATCH.names.copy), "{}", blank.name());
}

// The guard's own connection is still open as it drops the database, so the drop ends it.
#[tokio::test]
async fn a_scratch_database_is_dropped_with_its_guard() {
  let scratch = Scratch::new::<Kept>().await;
  let name = scratch.name().to_owned();

  assert!(stands(&name).await);

  drop(scratch);

  assert!(!stands(&name).await);
}
