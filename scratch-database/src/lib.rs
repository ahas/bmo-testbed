//! A Rust test's own database (docs/suite.md, 「The test database」): the database the migrator a test hands
//! in makes, migrated once per test binary into a template, and each test handed a copy of it, which the
//! server makes in a fraction of the time a migration takes. A copy is dropped when its test is done, and the
//! templates and copies a binary left behind are dropped by the next binary to make a template.

use std::any::TypeId;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

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

async fn server() -> PgConnection {
  let address = address_of(&SCRATCH.server.admin_database);
  let options: PgConnectOptions = address.parse().unwrap();

  PgConnection::connect_with(&options).await.unwrap()
}

// The process a template's or a copy's name says made it.
fn maker_of(database: &str) -> &str {
  let names = &SCRATCH.names;

  database
    .strip_prefix(names.template.as_str())
    .or_else(|| database.strip_prefix(names.copy.as_str()))
    .unwrap_or(database)
    .split('_')
    .next()
    .unwrap_or_default()
}

// The templates and copies whose process is gone, dropped once a binary, before its first template.
async fn sweep() {
  SWEPT
    .get_or_init(|| async {
      let names = &SCRATCH.names;
      let mut admin = server().await;
      let left: Vec<String> = sqlx::query_scalar(
        "SELECT datname::text FROM pg_database WHERE datname LIKE $1 OR datname LIKE $2",
      )
      .bind(format!("{}%", names.template.replace('_', "\\_")))
      .bind(format!("{}%", names.copy.replace('_', "\\_")))
      .fetch_all(&mut admin)
      .await
      .unwrap();

      for old in left {
        if !alive(maker_of(&old)) {
          let _ = admin.execute(format!("DROP DATABASE IF EXISTS \"{old}\" WITH (FORCE)").as_str()).await;
        }
      }

      admin.close().await.unwrap();
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

/// A new copy of this binary's database migrated by `M`, empty; its database's name.
pub async fn copy<M: MigratorTrait + 'static>() -> String {
  let template = template::<M>().await;
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

/// A copy dropped, its connections ended with it. It goes on a thread of its own, since a guard dropped inside
/// the runtime may not block on it.
pub fn drop_copy(database: String) {
  let _ = std::thread::spawn(move || {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    runtime.block_on(async move {
      let mut admin = server().await;

      let _ = admin
        .execute(format!("DROP DATABASE IF EXISTS \"{database}\" WITH (FORCE)").as_str())
        .await;
      let _ = admin.close().await;
    });
  })
  .join();
}

/// A migrated, empty database of the test's own, dropped when the guard goes.
pub struct Scratch {
  database: String,
  db: DatabaseConnection,
}

impl Scratch {
  /// A copy of the database `M` migrates, and a connection to it.
  pub async fn new<M: MigratorTrait + 'static>() -> Scratch {
    let database = copy::<M>().await;
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
  fn drop(&mut self) {
    drop_copy(std::mem::take(&mut self.database));
  }
}
