//! What the first template a binary makes takes away (docs/suite.md, 「The test database」): every
//! template and copy whose process is gone, and nothing a living process holds. A binary of its own, so
//! its one test makes the binary's first template.

// The integration tests' migrators, of which the sweep hands in one.
#[allow(dead_code)]
#[path = "../integration/migrators.rs"]
mod migrators;

use bmo_testbed_scratch_database::{SCRATCH, Scratch, connect};
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
async fn the_first_template_drops_what_a_gone_process_left_and_keeps_a_living_ones() {
  let names = &SCRATCH.names;
  let left_template = format!("{}{GONE}_0", names.template);
  let left_copy = format!("{}{GONE}_3", names.copy);
  let living = format!("{}{}_999", names.copy, std::process::id());

  for database in [&left_template, &left_copy, &living] {
    admin_run(&format!("CREATE DATABASE \"{database}\"")).await;
  }

  let scratch = Scratch::new::<Kept>().await;

  assert!(!stands(&left_template).await);
  assert!(!stands(&left_copy).await);
  assert!(stands(&living).await);

  drop(scratch);
  admin_run(&format!("DROP DATABASE \"{living}\" WITH (FORCE)")).await;
}
