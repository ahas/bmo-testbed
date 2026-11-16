//! A Rust test's own database (docs/suite.md, 「The test database」): the database the migrator a test hands
//! in makes, migrated once per test binary into a template, and each test handed a copy of it, which the
//! server makes in a fraction of the time a migration takes. A test that needs nothing migrated is handed a
//! blank database under the copies' name. A copy is dropped when its test is done, failing the test where it
//! cannot be, and the templates and copies a binary left behind are dropped by the next binary to make a
//! template.

use std::any::TypeId;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use bmo_testbed_core::policy::fill_shown;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement, Value};
use sea_orm_migration::MigratorTrait;
use serde::Deserialize;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::{Connection as _, Executor};
use tokio::sync::OnceCell;

/// The variable the suite names its test server's address in (docs/boundary.md, 「The environment」).
pub const TEST_POSTGRES_VARIABLE: &str = "BMO_TEST_POSTGRES";

/// The scratch databases' names and server, read from the testbed's policy.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchPolicy {
  pub names: Names,
  pub server: Server,
  pub drop: Dropping,
  pub words: Words,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Names {
  pub template: String,
  pub copy: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
  pub address: String,
  pub admin_database: String,
  pub connections: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dropping {
  pub tries: u32,
  pub session_end_milliseconds: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Words {
  pub not_dropped: String,
  pub unreachable: String,
}

pub static SCRATCH: LazyLock<ScratchPolicy> = LazyLock::new(|| {
  bmo_testbed_core::policy::read(
    "scratch-database.toml",
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../policy/scratch-database.toml")),
  )
});

static COPIES: AtomicU32 = AtomicU32::new(0);
static SWEPT: OnceCell<()> = OnceCell::const_new();
// This binary's templates, one for each migrator handed in, each made the first time a test asks for it.
static TEMPLATES: Mutex<Vec<(TypeId, Arc<OnceCell<String>>)>> = Mutex::new(Vec::new());

/// The suite's PostgreSQL server, as the run names it, and the machine's own socket where nothing does.
pub fn test_address() -> String {
  std::env::var(TEST_POSTGRES_VARIABLE)
    .ok()
    .filter(|named| !named.is_empty())
    .unwrap_or_else(|| SCRATCH.server.address.clone())
}

/// Whether a process is alive, read off the kernel's own listing.
pub fn alive(pid: &str) -> bool {
  std::path::Path::new("/proc").join(pid).exists()
}

/// A server's address with another database in place of its own, everything else as it stands.
pub fn address_in(address: &str, database: &str) -> String {
  let mut named = url::Url::parse(address)
    .unwrap_or_else(|error| panic!("the test server's address {address} does not read: {error}"));

  named.set_path(&format!("/{database}"));
  named.to_string()
}

/// The test server's address with another database in place of its own.
pub fn address_of(database: &str) -> String {
  address_in(&test_address(), database)
}

async fn reach_server() -> Result<PgConnection, String> {
  let address = address_of(&SCRATCH.server.admin_database);
  let unreachable = |why: &dyn std::fmt::Display| {
    fill_shown(&SCRATCH.words.unreachable, &[("address", &address), ("why", why)])
  };
  let options: PgConnectOptions = address.parse().map_err(|why| unreachable(&why))?;

  PgConnection::connect_with(&options).await.map_err(|why| unreachable(&why))
}

async fn server() -> PgConnection {
  reach_server().await.unwrap_or_else(|why| panic!("{why}"))
}

// The process a template's or a copy's name says made it, where the name is one of theirs.
fn maker_of(database: &str) -> Option<u32> {
  let names = &SCRATCH.names;

  database
    .strip_prefix(names.template.as_str())
    .or_else(|| database.strip_prefix(names.copy.as_str()))?
    .split('_')
    .next()?
    .parse()
    .ok()
}

// One drop asked: every session of the test role on the database ended, then the database dropped without
// force, which asks the server's own maintenance on it to stop.
async fn drop_once(admin: &mut PgConnection, database: &str) -> Result<(), sqlx::Error> {
  sqlx::query(
    "SELECT pg_terminate_backend(pid, $2) FROM pg_stat_activity \
     WHERE datname = $1 AND usename = current_user AND pid <> pg_backend_pid()",
  )
  .bind(database)
  .bind(SCRATCH.drop.session_end_milliseconds)
  .execute(&mut *admin)
  .await?;
  admin.execute(format!("DROP DATABASE IF EXISTS \"{database}\"").as_str()).await?;

  Ok(())
}

/// A database of the test server dropped, its role's sessions on it ended first. The drop is not forced: a
/// forced one must end every session on the database, and a role that is no superuser may not end the
/// server's own maintenance, which a plain drop asks to stop. It is asked again a few times; the reason the
/// database still stands, where it does.
pub async fn drop_database(database: &str) -> Result<(), String> {
  let mut admin = reach_server().await?;
  let mut dropped = Ok(());

  for _ in 0..SCRATCH.drop.tries.max(1) {
    dropped = drop_once(&mut admin, database).await;

    if dropped.is_ok() {
      break;
    }
  }

  let _ = admin.close().await;

  dropped.map_err(|why| fill_shown(&SCRATCH.words.not_dropped, &[("database", &database), ("why", &why)]))
}

// The templates and copies whose process is gone, dropped once a binary, before its first template: only
// those the test role may drop and no client is connected to, so a database another machine's test holds
// on a shared server stands.
async fn sweep() {
  SWEPT
    .get_or_init(|| async {
      let names = &SCRATCH.names;
      let mut admin = server().await;
      let left: Vec<String> = sqlx::query_scalar(
        "SELECT datname::text FROM pg_database d \
         WHERE (datname LIKE $1 OR datname LIKE $2) AND pg_has_role(datdba, 'MEMBER') \
         AND NOT EXISTS (SELECT 1 FROM pg_stat_activity a \
                         WHERE a.datname = d.datname AND a.backend_type = 'client backend')",
      )
      .bind(format!("{}%", names.template.replace('_', "\\_")))
      .bind(format!("{}%", names.copy.replace('_', "\\_")))
      .fetch_all(&mut admin)
      .await
      .unwrap();

      admin.close().await.unwrap();

      for old in left {
        if maker_of(&old).is_some_and(|maker| !alive(&maker.to_string())) {
          drop_database(&old).await.unwrap_or_else(|why| panic!("{why}"));
        }
      }
    })
    .await;
}

// The template this binary's copies for one migrator are made from, migrated the first time a test asks.
async fn template<M: MigratorTrait + 'static>() -> String {
  let (index, made) = {
    let mut templates = TEMPLATES.lock().unwrap();
    let index = match templates.iter().position(|(migrator, _)| *migrator == TypeId::of::<M>()) {
      Some(index) => index,
      None => {
        templates.push((TypeId::of::<M>(), Arc::new(OnceCell::new())));
        templates.len() - 1
      }
    };

    (index, templates[index].1.clone())
  };

  made
    .get_or_init(|| async {
      sweep().await;

      let name = format!("{}{}_{index}", SCRATCH.names.template, std::process::id());
      let mut admin = server().await;

      admin
        .execute(
          format!(
            "CREATE DATABASE \"{name}\" TEMPLATE template0 ENCODING 'UTF8' LC_COLLATE 'C' LC_CTYPE 'C'"
          )
          .as_str(),
        )
        .await
        .unwrap();
      admin.close().await.unwrap();

      let db = connect(&name).await;

      M::up(&db, None).await.unwrap();
      db.close().await.unwrap();

      name
    })
    .await
    .clone()
}

/// A connection to a database of the test server.
pub async fn connect(database: &str) -> DatabaseConnection {
  let mut options = ConnectOptions::new(address_of(database));

  options.sqlx_logging(false).max_connections(SCRATCH.server.connections);

  Database::connect(options).await.unwrap()
}

// A new database of this process's under the copies' name, made from the template given; its name.
async fn copy_of(template: &str) -> String {
  let database =
    format!("{}{}_{}", SCRATCH.names.copy, std::process::id(), COPIES.fetch_add(1, Ordering::SeqCst));
  let mut admin = server().await;

  admin
    .execute(format!("CREATE DATABASE \"{database}\" TEMPLATE \"{template}\"").as_str())
    .await
    .unwrap();
  admin.close().await.unwrap();

  database
}

/// A new copy of this binary's database migrated by `M`, empty; its database's name.
pub async fn copy<M: MigratorTrait + 'static>() -> String {
  let template = template::<M>().await;

  copy_of(&template).await
}

/// A new blank database, nothing migrated, under the copies' name; its database's name. A binary's blank
/// databases are swept as its copies are.
pub async fn blank() -> String {
  sweep().await;
  copy_of("template0").await
}

/// A copy dropped, its connections ended with it; it panics with the server's reason where the copy still
/// stands. It goes on a thread of its own, since a guard dropped inside the runtime may not block on it.
pub fn drop_copy(database: String) {
  dropped_apart(database).unwrap_or_else(|why| panic!("{why}"));
}

// A database dropped on a thread and a runtime of its own; the reason it still stands, where it does.
fn dropped_apart(database: String) -> Result<(), String> {
  std::thread::spawn(move || {
    let runtime = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .map_err(|why| why.to_string())?;

    runtime.block_on(drop_database(&database))
  })
  .join()
  .unwrap_or_else(|_| Err("the thread dropping it panicked".to_owned()))
}

/// A migrated, empty database of the test's own, dropped when the guard goes.
pub struct Scratch {
  database: String,
  db: DatabaseConnection,
}

impl Scratch {
  /// A copy of the database `M` migrates, and a connection to it.
  pub async fn new<M: MigratorTrait + 'static>() -> Scratch {
    Scratch::opened(copy::<M>().await).await
  }

  /// A blank database, nothing migrated, and a connection to it.
  pub async fn blank() -> Scratch {
    Scratch::opened(blank().await).await
  }

  async fn opened(database: String) -> Scratch {
    let db = connect(&database).await;

    Scratch { database, db }
  }

  /// The database's name on the test server.
  pub fn name(&self) -> &str {
    &self.database
  }

  /// The database's address, as a reader handed a place opens it.
  pub fn address(&self) -> String {
    address_of(&self.database)
  }

  pub fn db(&self) -> &DatabaseConnection {
    &self.db
  }

  /// A statement run as written.
  pub async fn run(&self, sql: &str, values: Vec<Value>) {
    self
      .db
      .execute(Statement::from_sql_and_values(DbBackend::Postgres, sql, values))
      .await
      .unwrap();
  }

  /// A count a statement answers.
  pub async fn count(&self, sql: &str, values: Vec<Value>) -> i64 {
    let row = self
      .db
      .query_one(Statement::from_sql_and_values(DbBackend::Postgres, sql, values))
      .await
      .unwrap()
      .unwrap();

    row.try_get_by_index::<i64>(0).unwrap()
  }
}

impl Drop for Scratch {
  // A copy that still stands fails its test; where the test is already failing, a second panic would abort
  // the binary, so the reason is written out instead.
  fn drop(&mut self) {
    if let Err(why) = dropped_apart(std::mem::take(&mut self.database)) {
      if std::thread::panicking() {
        eprintln!("{why}");
      } else {
        panic!("{why}");
      }
    }
  }
}
