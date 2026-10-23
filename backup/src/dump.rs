//! The databases (docs/backup.md, 「What is backed up」 and 「The restore drill」): those on the server the
//! plan dumps, each dumped by `pg_dump` from a snapshot its rows are counted in, and a dump restored by
//! `pg_restore` into a scratch database and counted there.

use std::path::Path;
use std::process::Stdio;

use bmo_testbed_core::policy::fill_shown;
use sqlx::postgres::PgConnection;
use sqlx::{Connection as _, Executor as _};

use crate::destination::quoted;
use crate::manifest::Tables;
use crate::policy::BACKUP;

/// A server's address with another database in place of its own, everything else as it stands.
pub fn address_in(address: &str, database: &str) -> Result<String, String> {
  let mut named = url::Url::parse(address).map_err(|why| format!("{address}: {why}"))?;

  named.set_path(&format!("/{database}"));
  Ok(named.to_string())
}

async fn connect(address: &str) -> Result<PgConnection, String> {
  PgConnection::connect(address).await.map_err(|why| why.to_string())
}

// A name written into a statement as an identifier.
fn identifier(name: &str) -> String {
  format!("\"{}\"", name.replace('"', "\"\""))
}

/// Every database on the server, by name.
pub async fn databases(server: &str) -> Result<Vec<String>, String> {
  let mut admin = connect(server).await?;
  let names: Vec<String> = sqlx::query_scalar(
    "SELECT datname::text FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY 1",
  )
  .fetch_all(&mut admin)
  .await
  .map_err(|why| why.to_string())?;
  let _ = admin.close().await;

  Ok(names)
}

/// Every table's rows, as the connection's transaction sees them; a table an extension owns is left out,
/// since a dump holds none of its rows.
pub async fn count_tables(connection: &mut PgConnection) -> Result<Tables, String> {
  let tables: Vec<(String, String)> = sqlx::query_as(
    "SELECT n.nspname::text, c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relkind IN ('r', 'p') AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
     AND n.nspname NOT LIKE 'pg\\_toast%' AND n.nspname NOT LIKE 'pg\\_temp%' \
     AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass \
     AND d.objid = c.oid AND d.deptype = 'e') ORDER BY 1, 2",
  )
  .fetch_all(&mut *connection)
  .await
  .map_err(|why| why.to_string())?;
  let mut counts = Tables::new();

  for (schema, table) in tables {
    let statement = format!("SELECT count(*) FROM {}.{}", identifier(&schema), identifier(&table));
    let rows: i64 = sqlx::query_scalar(&statement)
      .fetch_one(&mut *connection)
      .await
      .map_err(|why| format!("{schema}.{table}: {why}"))?;

    counts.insert(format!("{schema}.{table}"), rows);
  }

  Ok(counts)
}

/// One database dumped into the file, from a snapshot exported by a transaction that counts its rows in
/// that same snapshot while the dump runs.
pub async fn dump(server: &str, database: &str, file: &Path) -> Result<Tables, String> {
  let address = address_in(server, database)?;
  let mut connection = connect(&address).await?;

  connection
    .execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
    .await
    .map_err(|why| why.to_string())?;

  let snapshot: String = sqlx::query_scalar("SELECT pg_export_snapshot()")
    .fetch_one(&mut connection)
    .await
    .map_err(|why| why.to_string())?;
  let child = tokio::process::Command::new("pg_dump")
    .arg("--format=custom")
    .arg(format!("--compress={}", BACKUP.numbers.dump_compress))
    .arg(format!("--snapshot={snapshot}"))
    .arg(format!("--file={}", file.display()))
    .arg(format!("--dbname={address}"))
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|why| fill_shown(&BACKUP.words.unstarted, &[("what", &"pg_dump"), ("why", &why)]))?;
  let counted = count_tables(&mut connection).await;
  // The transaction stays open until the dump has ended, since the dump reads the snapshot it exported.
  let done = child.wait_with_output().await.map_err(|why| why.to_string())?;
  let _ = connection.execute("COMMIT").await;
  let _ = connection.close().await;

  if !done.status.success() {
    return Err(fill_shown(
      &BACKUP.words.exited,
      &[
        ("what", &"pg_dump"),
        ("code", &done.status.code().unwrap_or(-1)),
        ("said", &quoted(&String::from_utf8_lossy(&done.stderr))),
      ],
    ));
  }

  counted
}

/// What a restore came to: the restored database's rows, and what `pg_restore` complained of, where it
/// did, said by how many errors it counted and never by its own lines, which may quote a row.
#[derive(Debug, Default)]
pub struct Restored {
  pub tables: Tables,
  pub complaint: Option<String>,
}

/// A dump restored into a new scratch database of the given name, its rows counted, and the scratch
/// database dropped whatever came of it.
pub async fn restore_and_count(server: &str, scratch: &str, file: &Path) -> Result<Restored, String> {
  let mut admin = connect(server).await?;

  admin
    .execute(format!("CREATE DATABASE {} TEMPLATE template0", identifier(scratch)).as_str())
    .await
    .map_err(|why| why.to_string())?;

  let restored = restore_into(server, scratch, file).await;
  let _ = admin
    .execute(format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", identifier(scratch)).as_str())
    .await;
  let _ = admin.close().await;

  restored
}

async fn restore_into(server: &str, scratch: &str, file: &Path) -> Result<Restored, String> {
  let address = address_in(server, scratch)?;
  let done = tokio::process::Command::new("pg_restore")
    .args(["--no-owner", "--no-privileges"])
    .arg(format!("--jobs={}", BACKUP.numbers.restore_jobs))
    .arg(format!("--dbname={address}"))
    .arg(file)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .output()
    .await
    .map_err(|why| fill_shown(&BACKUP.words.unstarted, &[("what", &"pg_restore"), ("why", &why)]))?;
  let complaint = (!done.status.success()).then(|| {
    let errors = String::from_utf8_lossy(&done.stderr)
      .lines()
      .filter(|line| line.contains("error:"))
      .count();

    fill_shown(
      &BACKUP.words.restore_complained,
      &[("code", &done.status.code().unwrap_or(-1)), ("errors", &errors)],
    )
  });
  let mut connection = connect(&address).await?;
  let tables = count_tables(&mut connection).await;
  let _ = connection.close().await;

  Ok(Restored {
    tables: tables?,
    complaint,
  })
}
