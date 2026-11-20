//! Where the suite's databases stand (docs/suite.md, 「The test database」): the suite's own PostgreSQL
//! server, a cluster of the user's own apart from any server the repository's services run on, stood up
//! before the first unit starts and kept on the settings `policy/suite.toml` lists; and the template a
//! checkout's declared migrations make, named by what those migrations are.

use std::path::PathBuf;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::{SUITE, ServerSetting};
use bmo_testbed_core::queue::{HeldLock, LockWait, now_ms, take_lock};

use crate::fence::{fence_units, other_fence_listed};
use crate::files::checkout_files;
use crate::selection::glob_matches;
use std::process::{Command, Stdio};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::crates::account_home;

/// What the names of a template begin with.
pub const TEMPLATE_MARK: &str = "bmo_test_tmpl_";

const OWN_UNIT: &str = "bmo-test-postgres";

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

fn quoted(text: &str) -> String {
  format!("'{}'", text.replace('\'', "''"))
}

/// The query a running server answers `t` to where every one of these settings stands as given, and `f`
/// where one does not; none where there is nothing to compare.
pub fn settings_query(settings: &[&ServerSetting]) -> Option<String> {
  if settings.is_empty() {
    return None;
  }

  let rows: Vec<String> = settings
    .iter()
    .map(|setting| format!("({}, {})", quoted(&setting.name), quoted(&setting.value)))
    .collect();

  Some(format!(
    "SELECT bool_and(current_setting(name) = value) FROM (VALUES {}) AS own(name, value)",
    rows.join(", ")
  ))
}

// The sessions on the server other than the one asking: whatever a test, a template or a sweep holds.
const OTHER_SESSIONS: &str =
  "SELECT count(*) FROM pg_stat_activity WHERE backend_type = 'client backend' AND pid <> pg_backend_pid()";

// Whether the server's unit still stands active or on its way there.
fn coming_up() -> bool {
  Command::new("systemctl")
    .args(["--user", "is-active", OWN_UNIT])
    .envs(crate::fence::user_bus())
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output()
    .is_ok_and(|done| {
      matches!(String::from_utf8_lossy(&done.stdout).trim(), "active" | "activating" | "reloading")
    })
}

// Whether the scope of a run other than this process's stands; a listing that cannot be read is taken for
// one that names another.
fn other_fence_stands() -> bool {
  Command::new("systemctl")
    .args(["--user", "list-units", "--plain", "--no-legend"])
    .arg(fence_units())
    .envs(crate::fence::user_bus())
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output()
    .ok()
    .filter(|done| done.status.success())
    .is_none_or(|done| other_fence_listed(&String::from_utf8_lossy(&done.stdout), std::process::id()))
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

/// A wait for another run standing the test server up (docs/suite.md, 「The test database」), which says so once
/// every `every` milliseconds of it; the moments are handed in, so a test sets them.
pub struct StandWait {
  began: i64,
  every: i64,
  said_at: i64,
}

impl StandWait {
  /// A wait begun at `began`, in milliseconds since the epoch.
  pub fn new(began: i64, every: i64) -> Self {
    Self {
      began,
      every,
      said_at: began,
    }
  }

  /// The line the wait says at `now`, where a whole `every` has passed since it last said one; none otherwise.
  pub fn line_at(&mut self, now: i64) -> Option<String> {
    if now - self.said_at < self.every {
      return None;
    }

    self.said_at = now;

    let seconds = ((now - self.began) / 1000).to_string();

    Some(fill_marks(&SUITE.lines.stand_waiting, &[("seconds", &seconds)]))
  }
}

/// The suite's own server: the programs of the machine's PostgreSQL and the cluster under the account's home.
struct OwnServer {
  bin: PathBuf,
  dir: PathBuf,
  data: PathBuf,
  socket: PathBuf,
}

impl OwnServer {
  fn here() -> Result<Self, String> {
    let bin = PathBuf::from(ran(Command::new("pg_config").arg("--bindir"))?.trim());
    let dir = own_server_dir();

    Ok(Self {
      bin,
      data: dir.join("data"),
      socket: dir.join("socket"),
      dir,
    })
  }

  fn program(&self, name: &str) -> String {
    self.bin.join(name).to_string_lossy().into_owned()
  }

  fn answers(&self) -> bool {
    Command::new(self.program("pg_isready"))
      .args(["-q", "-t", "1", "-h"])
      .arg(&self.socket)
      .args(["-d", "postgres"])
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status()
      .is_ok_and(|status| status.success())
  }

  fn asked(&self, query: &str) -> Result<String, String> {
    let server = format!("postgres:///postgres?host={}", self.socket.display());

    ran(Command::new("psql").args(["-X", "-At", "-d", &server, "-c", query]))
      .map(|said| said.trim().to_owned())
  }

  // The major version of the cluster, which the server running on it is.
  fn major(&self) -> Result<u32, String> {
    let marked = self.data.join("PG_VERSION");
    let read = std::fs::read_to_string(&marked).map_err(|why| format!("{}: {why}", marked.display()))?;

    read.trim().parse().map_err(|_| format!("{}: no major version", marked.display()))
  }

  fn settings_stand(&self) -> Result<bool, String> {
    let major = self.major()?;

    match settings_query(&SUITE.server.settings_for(major)) {
      Some(query) => self.asked(&query).map(|said| said == "t"),
      None => Ok(true),
    }
  }

  fn in_use(&self) -> Result<bool, String> {
    self.asked(OTHER_SESSIONS).map(|said| said != "0")
  }

  // Another run standing the server up is waited out, however long it takes, said once a minute.
  fn stand_lock(&self) -> Result<HeldLock, String> {
    std::fs::create_dir_all(&self.dir).map_err(|why| why.to_string())?;

    let every = i64::try_from(SUITE.numbers.stand_waiting_seconds * 1000).unwrap_or(i64::MAX);
    let mut standing = StandWait::new(now_ms(), every);
    let mut wait_on = |_: Option<&str>| -> Option<i64> {
      let now = now_ms();

      if let Some(line) = standing.line_at(now) {
        eprintln!("{line}");
      }

      Some(now + 1000)
    };
    let wait = LockWait {
      wait_on: Some(&mut wait_on),
      ..LockWait::default()
    };

    take_lock(&self.dir.join("stand.lock"), 1.0, wait)
      .ok_or_else(|| format!("{}: its stand lock could not be taken", self.dir.display()))
  }

  // Made where it was never made, started as the transient unit and asked until it answers, with the stand
  // lock held.
  fn start(&self) -> Result<(), String> {
    // Made under another name and put in place only whole, so a making cut short is never taken for the
    // cluster.
    if !self.data.join("PG_VERSION").exists() {
      let making = self.dir.join("data.making");
      let _ = std::fs::remove_dir_all(&making);

      ran(Command::new(self.program("initdb")).arg("-D").arg(&making).args([
        "--locale=C",
        "--encoding=UTF8",
        "--auth-local=trust",
        "--auth-host=reject",
        "--no-instructions",
      ]))?;
      std::fs::rename(&making, &self.data).map_err(|why| why.to_string())?;
    }

    std::fs::create_dir_all(&self.socket).map_err(|why| why.to_string())?;

    let mut start = Command::new("systemd-run");

    start
      .args([
        "--user",
        &format!("--unit={OWN_UNIT}"),
        &format!("--slice={}", crate::fence::SUITE_SLICE),
        "--collect",
        "--quiet",
        "--",
      ])
      .arg(self.program("postgres"))
      .arg("-D")
      .arg(&self.data)
      .arg("-k")
      .arg(&self.socket)
      .env_clear()
      .envs(crate::fence::user_bus());

    for setting in SUITE.server.settings_for(self.major()?) {
      start.args(["-c", &format!("{}={}", setting.name, setting.value)]);
    }

    // A unit that already stands is one still coming up, which the wait below waits for as well.
    let started = start.stdin(Stdio::null()).output();

    // Asked until it answers, however long it takes, or until its unit is no longer coming up.
    while !self.answers() && coming_up() {
      std::thread::sleep(Duration::from_millis(SUITE.numbers.server_start_poll_ms));
    }

    if !self.answers() {
      let said = started
        .map(|done| String::from_utf8_lossy(&done.stderr).trim().to_owned())
        .unwrap_or_default();

      return Err(format!("{OWN_UNIT}: {said}"));
    }

    if self.asked("SELECT 1 FROM pg_database WHERE datname = 'bmo_test'")? != "1" {
      self.asked("CREATE DATABASE bmo_test")?;
    }

    Ok(())
  }
}

/// The server the suite tests on, answering: where it is the suite's own and does not answer, made where it
/// was never made and started as a transient user unit that outlives the run. Another server the
/// environment names is taken as it stands.
pub fn test_server_up() -> Result<(), String> {
  if test_postgres() != own_test_postgres() {
    return Ok(());
  }

  let own = OwnServer::here()?;

  if own.answers() {
    return Ok(());
  }

  let lock = own.stand_lock()?;

  if own.answers() {
    drop(lock);
    return Ok(());
  }

  let started = own.start();

  drop(lock);
  started
}

/// The server answering as `test_server_up` stands it, and then, where it is the suite's own unit answering
/// with settings other than its own, restarted onto them; but only while no other session is connected to it
/// and no other run's fence stands, and otherwise used as it stands, said once.
pub fn test_server_settled() -> Result<(), String> {
  test_server_up()?;

  if test_postgres() != own_test_postgres() {
    return Ok(());
  }

  let own = OwnServer::here()?;

  if !coming_up() || own.settings_stand()? {
    return Ok(());
  }

  let lock = own.stand_lock()?;
  let settled = settle(&own);

  drop(lock);
  settled
}

// With the stand lock held: another run may have restarted it meanwhile.
fn settle(own: &OwnServer) -> Result<(), String> {
  if !coming_up() || own.settings_stand()? {
    return Ok(());
  }

  if own.in_use()? || other_fence_stands() {
    eprintln!("{}", SUITE.lines.server_settings_held);
    return Ok(());
  }

  eprintln!("{}", SUITE.lines.server_restarting);
  ran(
    Command::new("systemctl")
      .args(["--user", "stop", OWN_UNIT])
      .envs(crate::fence::user_bus()),
  )?;
  own.start()
}
