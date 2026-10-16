//! Where the suite's databases stand (docs/suite.md, 「The test database」): the suite's own PostgreSQL
//! server, a cluster of the user's own apart from any server the repository's services run on, stood up
//! before the first unit starts; and the template a checkout's declared migrations make, named by what those
//! migrations are.

use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::queue::{LockWait, take_lock};

use crate::files::checkout_files;
use crate::selection::glob_matches;
use std::process::{Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::crates::account_home;

/// What the names of a template begin with.
pub const TEMPLATE_MARK: &str = "bmo_test_tmpl_";

const OWN_UNIT: &str = "bmo-test-postgres";

// What it holds is thrown away with every test, so it waits for no disk; and every file of every run at
// the same time, with the scripts each starts, holds connections out of it.
const OWN_SETTINGS: [(&str, &str); 5] = [
  ("listen_addresses", ""),
  ("max_connections", "300"),
  ("fsync", "off"),
  ("synchronous_commit", "off"),
  ("full_page_writes", "off"),
];

fn own_server_dir() -> PathBuf {
  account_home().join(".local").join("state").join("bmo-test-postgres")
}

/// The suite's own server's address, read off the account's own home.
pub fn own_test_postgres() -> String {
  format!("postgres:///bmo_test?host={}", own_server_dir().join("socket").display())
}

/// The server the suite tests on: its own, unless the environment names another.
pub fn test_postgres() -> String {
  std::env::var("BMO_TEST_POSTGRES")
    .ok()
    .filter(|named| !named.is_empty())
    .unwrap_or_else(own_test_postgres)
}

/// What the names of a template's clones, and of the scratch databases a test makes, begin with.
pub const CLONE_MARK: &str = "bmo_test_";

/// What the names of a template's pool of clones begin with, read off the template's name.
pub fn pool_prefix(template: &str) -> String {
  format!("{CLONE_MARK}{}", template.strip_prefix(TEMPLATE_MARK).unwrap_or(template))
}

/// A database on the test server, by name: the server's address with that database in place of its own.
pub fn test_database(database: &str) -> String {
  let address = test_postgres();

  match url::Url::parse(&address) {
    Ok(mut named) => {
      named.set_path(&format!("/{database}"));
      named.to_string().replace("%2F", "/").replace("%2f", "/")
    }
    Err(_) => address,
  }
}

/// The template the declared migrations of a checkout make: every run whose checkout migrates the same way
/// shares it, and a checkout whose migrations differ has a template of its own. None where the repository
/// declares no migrations, and then its tests are handed the server alone.
pub fn template_name(repo: &Repository) -> Option<String> {
  let globs = &repo.declaration.test.database.as_ref()?.migrations;

  if globs.is_empty() {
    return None;
  }

  let mut files: Vec<String> = checkout_files(&repo.root)
    .into_iter()
    .filter(|file| globs.iter().any(|glob| glob_matches(glob, file)))
    .collect();
  let mut hash = Sha256::new();

  files.sort();

  for file in files {
    hash.update(file.as_bytes());
    hash.update([0]);
    hash.update(std::fs::read(repo.root.join(&file)).unwrap_or_default());
    hash.update([0]);
  }

  Some(format!("{TEMPLATE_MARK}{}", &format!("{:x}", hash.finalize())[..16]))
}

fn program(bin: &Path, name: &str) -> String {
  bin.join(name).to_string_lossy().into_owned()
}

fn answers(bin: &Path, socket: &Path) -> bool {
  Command::new(program(bin, "pg_isready"))
    .args(["-q", "-t", "1", "-h"])
    .arg(socket)
    .args(["-d", "postgres"])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|status| status.success())
}

fn ran(command: &mut Command) -> Result<String, String> {
  let done = command.stdin(Stdio::null()).output().map_err(|why| why.to_string())?;

  if done.status.success() {
    Ok(String::from_utf8_lossy(&done.stdout).into_owned())
  } else {
    let said = String::from_utf8_lossy(&done.stderr).trim().to_owned();

    Err(if said.is_empty() {
      String::from_utf8_lossy(&done.stdout).trim().to_owned()
    } else {
      said
    })
  }
}

/// The server the suite tests on, answering: where it is the suite's own and does not answer, made where it
/// was never made and started as a transient user unit that outlives the run. Another server the
/// environment names is taken as it stands.
pub fn test_server_up() -> Result<(), String> {
  if test_postgres() != own_test_postgres() {
    return Ok(());
  }

  let bin = PathBuf::from(ran(Command::new("pg_config").arg("--bindir"))?.trim());
  let dir = own_server_dir();
  let (data, socket) = (dir.join("data"), dir.join("socket"));

  if answers(&bin, &socket) {
    return Ok(());
  }

  std::fs::create_dir_all(&dir).map_err(|why| why.to_string())?;

  let lock = take_lock(&dir.join("stand.lock"), 120.0, LockWait::default())
    .ok_or_else(|| format!("{} was being stood up by another run for two minutes", dir.display()))?;

  if answers(&bin, &socket) {
    drop(lock);
    return Ok(());
  }

  // Made under another name and put in place only whole, so a making cut short is never taken for the
  // cluster.
  if !data.join("PG_VERSION").exists() {
    let making = dir.join("data.making");
    let _ = std::fs::remove_dir_all(&making);

    ran(Command::new(program(&bin, "initdb")).arg("-D").arg(&making).args([
      "--locale=C",
      "--encoding=UTF8",
      "--auth-local=trust",
      "--auth-host=reject",
      "--no-instructions",
    ]))?;
    std::fs::rename(&making, &data).map_err(|why| why.to_string())?;
  }

  std::fs::create_dir_all(&socket).map_err(|why| why.to_string())?;

  let mut start = Command::new("systemd-run");

  start
    .args(["--user", &format!("--unit={OWN_UNIT}"), "--collect", "--quiet", "--"])
    .arg(program(&bin, "postgres"))
    .arg("-D")
    .arg(&data)
    .arg("-k")
    .arg(&socket)
    .env_clear()
    .envs(crate::fence::user_bus());

  for (name, value) in OWN_SETTINGS {
    start.args(["-c", &format!("{name}={value}")]);
  }

  // A unit that already stands is one still coming up, which the wait below waits for as well.
  let started = start.stdin(Stdio::null()).output();

  for _ in 0..60 {
    if answers(&bin, &socket) {
      break;
    }

    std::thread::sleep(Duration::from_millis(500));
  }

  if !answers(&bin, &socket) {
    let said = started
      .map(|done| String::from_utf8_lossy(&done.stderr).trim().to_owned())
      .unwrap_or_default();

    return Err(format!("{OWN_UNIT}: {said}"));
  }

  let server = format!("postgres:///postgres?host={}", socket.display());
  let exists = ran(
    Command::new("psql")
      .args(["-X", "-At", "-d", &server, "-c"])
      .arg("SELECT 1 FROM pg_database WHERE datname = 'bmo_test'"),
  )?;

  if exists.trim() != "1" {
    ran(Command::new("psql").args(["-X", "-q", "-d", &server, "-c", "CREATE DATABASE bmo_test"]))?;
  }

  drop(lock);
  Ok(())
}
