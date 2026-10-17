//! The suite's template database on the test server, made, dropped or swept (docs/suite.md, 「The test
//! database」). The template is an empty database the repository's declared migrate command made, carrying
//! two functions a clone is reused with: one empties what the last test wrote, the other sets every identity
//! counting on past the rows a test then puts in.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::queue::process_alive;
use regex::Regex;
use sqlx::{Connection as _, PgConnection};

use crate::test_server::{CLONE_MARK, TEMPLATE_MARK, pool_prefix, test_database, test_server_up};

/// The variable the migrate command finds the database to migrate in.
pub const TEMPLATE_URL_VARIABLE: &str = "BMO_TEST_TEMPLATE_URL";

// Every table a test wrote emptied, and every scratch schema a test made dropped. Emptying only the tables
// that hold a row keeps it to milliseconds, where emptying every table costs a second; and a few rows go
// faster by DELETE than by TRUNCATE, which makes the table's files afresh, but for a table whose own
// triggers refuse the DELETE, which TRUNCATE fires none of, or one another table's key still points into,
// which TRUNCATE empties together with the tables pointing at it, all of them being emptied anyway.
const RESET_FUNCTION: &str = r"
CREATE FUNCTION public.bmo_test_reset() RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  one record;
  filled boolean;
BEGIN
  FOR one IN SELECT nspname FROM pg_namespace WHERE nspname LIKE 't\_%' LOOP
    EXECUTE format('DROP SCHEMA %I CASCADE', one.nspname);
  END LOOP;
  FOR one IN SELECT tablename FROM pg_tables WHERE schemaname = 'public' LOOP
    EXECUTE format('SELECT EXISTS (SELECT 1 FROM public.%I)', one.tablename) INTO filled;
    IF filled THEN
      BEGIN
        EXECUTE format('DELETE FROM public.%I', one.tablename);
      EXCEPTION WHEN OTHERS THEN
        EXECUTE format('TRUNCATE public.%I CASCADE', one.tablename);
      END;
    END IF;
  END LOOP;
END
$$;";

// Every identity counting on past the largest key its table holds; a key below one counts on from one.
const COUNT_ON_FUNCTION: &str = r"
CREATE FUNCTION public.bmo_test_count_on() RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  one record;
BEGIN
  FOR one IN SELECT table_name, column_name FROM information_schema.columns WHERE table_schema = 'public' AND is_identity = 'YES' LOOP
    EXECUTE format('SELECT setval(pg_get_serial_sequence(%L, %L), GREATEST(COALESCE((SELECT max(%I) FROM public.%I), 0), 0) + 1, false)',
      'public.' || quote_ident(one.table_name), one.column_name, one.column_name, one.table_name);
  END LOOP;
END
$$;";

// A database's name the verb takes, which it writes into its statements quoted as it stands.
static NAME: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^[a-z_][a-z0-9_]*$").expect("the name's pattern"));

// When a template was made, as its note says it.
static MADE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^made (\d+)$").expect("the made note's pattern"));

// A scratch database named by the second and the process that made it.
static SCRATCH: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(&format!(r"^{CLONE_MARK}(?:tmpl_)?\d{{9,}}_(\d+)_")).expect("the scratch name's pattern")
});

/// What the verb is asked to do with a template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateVerb {
  Make,
  Drop,
  Sweep,
}

/// Whether a name is one the verb writes into its statements.
pub fn is_database_name(name: &str) -> bool {
  NAME.is_match(name)
}

/// Whether a template's note says when it was made, as the sweep reads it.
pub fn is_made_note(note: &str) -> bool {
  MADE.is_match(note)
}

/// The templates a sweep drops, each with its pool: every one but the one named, made more than the kept
/// span before `now`, whose pool `held` does not say a living process holds. A template whose note names no
/// time it was made is kept.
pub fn superseded<'a>(
  templates: &'a [(String, Option<String>)],
  keep: &str,
  now: u64,
  held: impl Fn(&str) -> bool,
) -> Vec<&'a str> {
  let kept = SUITE.template.superseded_kept_seconds;

  templates
    .iter()
    .filter(|(name, note)| {
      let made = note
        .as_deref()
        .and_then(|note| MADE.captures(note))
        .and_then(|found| found[1].parse::<u64>().ok());

      name != keep && made.is_some_and(|made| now.saturating_sub(made) > kept) && !held(&pool_prefix(name))
    })
    .map(|(name, _)| name.as_str())
    .collect()
}

/// The scratch databases a sweep drops: each named by the process that made it, once `alive` says that
/// process is gone.
pub fn orphaned(databases: &[String], alive: impl Fn(i64) -> bool) -> Vec<&str> {
  databases
    .iter()
    .filter(|name| {
      SCRATCH
        .captures(name)
        .and_then(|found| found[1].parse::<i64>().ok())
        .is_some_and(|pid| !alive(pid))
    })
    .map(String::as_str)
    .collect()
}

/// Whether a living process holds a clone of the pool `prefix` begins: its claim, a file of the slot's own in
/// `dir` naming the process.
pub fn pool_held(dir: &Path, prefix: &str, alive: impl Fn(i64) -> bool) -> bool {
  let Ok(entries) = std::fs::read_dir(dir) else { return false };
  let slot = format!("{prefix}_s");

  entries.flatten().any(|entry| {
    let file = entry.file_name().to_string_lossy().into_owned();

    file.starts_with(&slot)
      && file.ends_with(".claim")
      && std::fs::read_to_string(entry.path())
        .ok()
        .and_then(|pid| pid.trim().parse::<i64>().ok())
        .is_some_and(&alive)
  })
}

/// A connection to one database on the test server.
pub async fn connect(database: &str) -> Result<PgConnection, String> {
  PgConnection::connect(&test_database(database)).await.map_err(|why| why.to_string())
}

/// A statement run as it stands.
pub async fn execute(connection: &mut PgConnection, sql: &str) -> Result<(), String> {
  sqlx::raw_sql(sql).execute(connection).await.map(|_| ()).map_err(|why| why.to_string())
}

/// Whether a database of this name stands on the server.
pub async fn standing(server: &mut PgConnection, database: &str) -> Result<bool, String> {
  sqlx::query("SELECT 1 FROM pg_database WHERE datname = $1")
    .bind(database)
    .fetch_optional(server)
    .await
    .map(|row| row.is_some())
    .map_err(|why| why.to_string())
}

async fn names(server: &mut PgConnection, sql: &str, bound: Option<&str>) -> Result<Vec<String>, String> {
  let mut query = sqlx::query_scalar::<_, String>(sql);

  if let Some(bound) = bound {
    query = query.bind(bound);
  }

  query.fetch_all(server).await.map_err(|why| why.to_string())
}

/// A template and every clone of its pool; a database another session still holds is left for a later sweep.
pub async fn drop_set(server: &mut PgConnection, template: &str) -> Result<(), String> {
  let pool = names(
    server,
    "SELECT datname::text FROM pg_database WHERE starts_with(datname, $1)",
    Some(&format!("{}_s", pool_prefix(template))),
  )
  .await?;

  for clone in pool.iter().map(String::as_str).chain([template]) {
    let _ = execute(server, &format!("DROP DATABASE IF EXISTS \"{clone}\"")).await;
  }

  Ok(())
}

// The declared migrate command run over the database at `address`, in the repository's root.
fn migrate(migrate: &str, root: &Path, address: &str) -> Result<(), String> {
  let done = Command::new("sh")
    .arg("-c")
    .arg(migrate)
    .current_dir(root)
    .env(TEMPLATE_URL_VARIABLE, address)
    .stdin(Stdio::null())
    .output()
    .map_err(|why| why.to_string())?;

  if done.status.success() {
    return Ok(());
  }

  let code = done.status.code().map_or_else(|| done.status.to_string(), |code| code.to_string());

  Err(fill_marks(
    &SUITE.template.migrate_failed,
    &[("code", &code), ("said", String::from_utf8_lossy(&done.stderr).trim())],
  ))
}

fn now_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |since| since.as_secs())
}

// The template made under another name and put in place only whole, so a making cut short is never taken for
// it.
async fn make(server: &mut PgConnection, name: &str, command: &str, root: &Path) -> Result<(), String> {
  let making = format!("{name}_making");

  if standing(server, &making).await? {
    execute(server, &format!("DROP DATABASE \"{making}\"")).await?;
  }

  execute(
    server,
    &format!("CREATE DATABASE \"{making}\" TEMPLATE template0 ENCODING 'UTF8' LC_COLLATE 'C' LC_CTYPE 'C'"),
  )
  .await?;

  let made = async {
    migrate(command, root, &test_database(&making))?;

    let mut template = connect(&making).await?;
    let functions = async {
      execute(&mut template, RESET_FUNCTION).await?;
      execute(&mut template, COUNT_ON_FUNCTION).await
    }
    .await;

    let _ = template.close().await;
    functions?;
    execute(server, &format!("ALTER DATABASE \"{making}\" RENAME TO \"{name}\"")).await?;

    // When it was made, which the sweep reads once another template has taken its place.
    execute(server, &format!("COMMENT ON DATABASE \"{name}\" IS 'made {}'", now_secs())).await
  }
  .await;

  if made.is_err() {
    let _ = execute(server, &format!("DROP DATABASE IF EXISTS \"{making}\"")).await;
  }

  made
}

/// The template made where it does not stand, by the migrate command in the repository's root. Every caller
/// may ask at once; one makes it while the rest wait on the server's lock, then find it.
pub async fn make_once(
  server: &mut PgConnection,
  name: &str,
  command: &str,
  root: &Path,
) -> Result<(), String> {
  let mut held = connect("postgres").await?;

  sqlx::query("SELECT pg_advisory_lock(hashtext($1))")
    .bind(name)
    .execute(&mut held)
    .await
    .map_err(|why| why.to_string())?;

  let made = match standing(server, name).await {
    Ok(true) => Ok(()),
    Ok(false) => make(server, name, command, root).await,
    Err(why) => Err(why),
  };

  let _ = sqlx::query("SELECT pg_advisory_unlock(hashtext($1))")
    .bind(name)
    .execute(&mut held)
    .await;
  let _ = held.close().await;

  made
}

/// Every template but the one named made more than the kept span ago whose pool no living process holds,
/// with its pool, and every scratch database whose process is gone.
pub async fn sweep(server: &mut PgConnection, keep: &str) -> Result<(), String> {
  let templates: Vec<(String, Option<String>)> = sqlx::query_as(
    "SELECT datname::text, shobj_description(oid, 'pg_database') FROM pg_database WHERE starts_with(datname, $1)",
  )
  .bind(TEMPLATE_MARK)
  .fetch_all(&mut *server)
  .await
  .map_err(|why| why.to_string())?;
  let claims = std::env::temp_dir();

  for template in
    superseded(&templates, keep, now_secs(), |prefix| pool_held(&claims, prefix, process_alive))
  {
    drop_set(server, template).await?;
  }

  let scratch = names(
    server,
    "SELECT datname::text FROM pg_database WHERE starts_with(datname, $1)",
    Some(CLONE_MARK),
  )
  .await?;

  for database in orphaned(&scratch, process_alive) {
    let _ = execute(server, &format!("DROP DATABASE IF EXISTS \"{database}\"")).await;
  }

  Ok(())
}

/// What the verb was asked, done on the test server, which is stood up before anything is asked of it; why
/// it could not be, where it could not.
pub fn template(verb: TemplateVerb, name: &str, repo: &Repository) -> Result<(), String> {
  let migrate = repo.declaration.test.database.as_ref().and_then(|database| database.migrate.clone());

  if verb == TemplateVerb::Make && migrate.is_none() {
    return Err(SUITE.template.no_migrate.clone());
  }

  test_server_up()?;

  let runtime = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .map_err(|why| why.to_string())?;

  runtime.block_on(async {
    let mut server = connect("postgres").await?;
    let done = match verb {
      TemplateVerb::Make => {
        make_once(&mut server, name, migrate.as_deref().unwrap_or_default(), &repo.root).await
      }
      TemplateVerb::Drop => drop_set(&mut server, name).await,
      TemplateVerb::Sweep => sweep(&mut server, name).await,
    };

    let _ = server.close().await;
    done
  })
}
