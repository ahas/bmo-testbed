//! The snapshot and the drill over a database of the test server (docs/backup.md, 「How a snapshot runs」
//! and 「The restore drill」): what a snapshot sends where, the manifest's counts the dump's, the drill
//! restoring it and matching every count, and a count that differs failing the drill.

use bmo_testbed_backup::drill::drill;
use bmo_testbed_backup::dump::address_in;
use bmo_testbed_backup::manifest::Manifest;
use bmo_testbed_backup::policy::BACKUP;
use bmo_testbed_backup::snapshot::{slot_of, snapshot};
use bmo_testbed_scratch_database::{SCRATCH, test_address};
use sqlx::postgres::PgConnection;
use sqlx::{Connection as _, Executor as _};

use crate::stand::Ground;

fn server() -> String {
  address_in(&test_address(), &SCRATCH.server.admin_database).unwrap()
}

async fn statements(address: &str, sql: &[&str]) {
  let mut connection = PgConnection::connect(address).await.unwrap();

  for one in sql {
    connection.execute(*one).await.unwrap();
  }

  connection.close().await.unwrap();
}

// A database of the test's own, with rows in two schemas; dropped when the guard goes.
struct Source(String);

impl Source {
  async fn new() -> Self {
    let name = format!("backup_source_{}", std::process::id());
    let drop = format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)");
    let create = format!("CREATE DATABASE \"{name}\" TEMPLATE template0");

    statements(&server(), &[&drop, &create]).await;
    statements(
      &address_in(&server(), &name).unwrap(),
      &[
        "CREATE TABLE notes (id serial PRIMARY KEY, body text)",
        "INSERT INTO notes (body) SELECT 'line ' || n FROM generate_series(1, 25) n",
        "CREATE SCHEMA side",
        "CREATE TABLE side.\"Mixed Case\" (id int)",
        "INSERT INTO side.\"Mixed Case\" VALUES (1), (2), (3)",
        "CREATE TABLE empty (id int)",
      ],
    )
    .await;

    Self(name)
  }
}

impl Drop for Source {
  fn drop(&mut self) {
    let name = self.0.clone();

    let _ = std::thread::spawn(move || {
      let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

      runtime
        .block_on(statements(&server(), &[&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)")]));
    })
    .join();
  }
}

fn manifest_at(path: std::path::PathBuf) -> Manifest {
  serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_snapshot_is_sent_whole_and_the_drill_restores_every_count() {
  let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
  let source = runtime.block_on(Source::new());

  drop(runtime);

  let ground = Ground::new();

  ground.file("data/brain/notes.txt", 10);
  ground.file("data/brain/big.bin", 5000);
  ground.file("data/env/app.env", 10);

  let plan = ground.plan(&ground.settings(&format!(
    "folders = [\"data/brain\", \"data/env\"]\ndatabases = [\"{}\"]\nserver = \"{}\"\nlarge_file_bytes = 1000",
    source.0,
    server()
  )));
  let now = chrono::Local::now();
  let slot = slot_of(now, plan.hourly_slots);
  let day = now.format("%Y-%m-%d").to_string();
  let mut said = Vec::new();
  let outcome = snapshot(&plan, now, &mut |line| said.push(line.to_owned()));

  assert!(outcome.passed(), "{:?} {said:?}", outcome.failed);

  let dump = format!("{}{}", source.0, BACKUP.names.dump_suffix);
  let expected: Vec<String> = [format!("daily/{day}"), format!("hourly/{slot}")]
    .iter()
    .flat_map(|base| {
      [&dump, &BACKUP.names.data_archive, &BACKUP.names.manifest]
        .map(|file| format!("data-backup/{base}/{file}"))
    })
    .collect();
  let mut expected_sorted = expected.clone();

  expected_sorted.sort();

  assert_eq!(ground.in_bucket(), expected_sorted);
  assert_eq!(ground.on_remote(), expected_sorted);

  let manifest = manifest_at(ground.path("staging/latest").join(&BACKUP.names.manifest));

  assert_eq!(manifest.slot, slot);
  assert_eq!(manifest.databases.len(), 1);
  assert_eq!(manifest.databases[0].tables["public.notes"], 25);
  assert_eq!(manifest.databases[0].tables["side.Mixed Case"], 3);
  assert_eq!(manifest.databases[0].tables["public.empty"], 0);
  // The data folders, their parent and the two small files; the large one is the weekly archive's.
  assert_eq!(manifest.data.as_ref().unwrap().entries, 5);

  let mut said = Vec::new();
  let drilled = drill(&plan, now, &mut |line| said.push(line.to_owned()));

  assert!(drilled.passed(), "{:?} {said:?}", drilled.failed);
  assert!(
    said.iter().any(|line| line.starts_with("drill: the newest snapshot is bucket:")),
    "{said:?}"
  );

  // A count in the first destination's manifest that differs from what restores fails the drill.
  let tampered = ground
    .path("s3/bucket")
    .join(format!("data-backup/hourly/{slot}/{}", BACKUP.names.manifest));
  let mut altered = manifest_at(tampered.clone());

  altered.databases[0].tables.insert("public.notes".to_owned(), 24);
  std::fs::write(&tampered, serde_json::to_string(&altered).unwrap()).unwrap();

  let failed = drill(&plan, now, &mut |_| {});

  assert_eq!(failed.failed.len(), 1, "{:?}", failed.failed);
  assert_eq!(failed.failed[0].step, "count");
  assert!(
    failed.failed[0]
      .said
      .contains("public.notes restored 25 rows where the dump counted 24")
  );

  // The drill leaves no scratch database behind.
  let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
  let left: Vec<String> = runtime.block_on(async {
    let mut admin = PgConnection::connect(&server()).await.unwrap();
    let left = sqlx::query_scalar("SELECT datname::text FROM pg_database WHERE datname LIKE $1")
      .bind(format!("{}{}\\_%", BACKUP.names.drill_database.replace('_', "\\_"), std::process::id()))
      .fetch_all(&mut admin)
      .await
      .unwrap();

    admin.close().await.unwrap();
    left
  });

  assert!(left.is_empty(), "{left:?}");
}

#[test]
fn a_drill_with_no_snapshot_anywhere_fails() {
  let ground = Ground::new();
  let plan = ground.plan(&ground.settings("folders = []\ndatabases = []"));
  let outcome = drill(&plan, chrono::Local::now(), &mut |_| {});

  assert_eq!(outcome.failed.len(), 1, "{:?}", outcome.failed);
  assert_eq!(outcome.failed[0].step, "find");
}
