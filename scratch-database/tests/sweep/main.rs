//! What the first template a binary makes takes away (docs/suite.md, 「The test database」): every
//! template and copy whose process is gone, and nothing a living process or a connected client holds. A
//! binary of its own, so its one test makes the binary's first template.

// The integration tests' migrators, of which the sweep hands in one.
#[allow(dead_code)]
#[path = "../migrators.rs"]
mod migrators;

use bmo_testbed_scratch_database::{SCRATCH, Scratch, connect, drop_database};
use sea_orm::{ConnectionTrait, DbBackend, Statement};

use migrators::Kept;

// A process number no process has: above the largest the kernel hands out.
const GONE: u32 = 999_999_999;

async fn admin_run(sql: &str) {
  let admin = connect(&SCRATCH.server.admin_database).await;

  admin.execute(Statement::from_string(DbBackend::Postgres, sql)).await.unwrap();
  admin.close().await.unwrap();
}

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

#[tokio::test]
async fn the_first_template_drops_what_a_gone_process_left_and_keeps_what_is_held() {
  let names = &SCRATCH.names;
  let left_template = format!("{}{GONE}_0", names.template);
  let left_copy = format!("{}{GONE}_3", names.copy);
  let living = format!("{}{}_999", names.copy, std::process::id());
  // A gone process's name, as a test on another machine sharing the server would leave it while it runs.
  let connected = format!("{}{GONE}_4", names.copy);

  for database in [&left_template, &left_copy, &living, &connected] {
    admin_run(&format!("CREATE DATABASE \"{database}\"")).await;
  }

  let client = connect(&connected).await;
  let scratch = Scratch::new::<Kept>().await;

  assert!(!stands(&left_template).await);
  assert!(!stands(&left_copy).await);
  assert!(stands(&living).await);
  assert!(stands(&connected).await);

  drop(scratch);
  client.close().await.unwrap();

  for database in [&living, &connected] {
    drop_database(database).await.unwrap();
  }
}
