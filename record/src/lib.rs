//! The record (docs/boundary.md, 「The record」): one SQLite file under the data root's `testbed/` that every
//! verb writes and the HTTP service reads. It holds the test runs, the deployments with their log lines,
//! the restarts, the gates' passes a deployment reads, and an event for each change of the members among them, each member kept in the shape the client answers it
//! in. Every write that makes or changes a member writes its event in the same transaction, so the stream
//! never tells of a change the collections do not hold.
//!
//! Each process opens its own connection; SQLite's write-ahead log lets the service read while a verb
//! writes, and a writer meeting another's lock waits the policy's milliseconds for it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_client::{
  Deployment, DeploymentState, Event, EventKind, LogLine, Page, PageQuery, Restart, RestartsQuery, TestRun,
  TestRunState, TestRunsQuery,
};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::record::RECORD;
use bmo_testbed_core::queue::{now_ms, process_alive};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// How long a writer or a reader waits for a lock another one holds. SQLite's busy handler is told a span and
/// counts it in an `int` of milliseconds, so this is the longest it can be told: a writer waits for the holder
/// to let go.
const LOCK_WAIT: Duration = Duration::from_millis(i32::MAX as u64);

const SCHEMA: &str = "
  PRAGMA journal_mode = WAL;
  CREATE TABLE IF NOT EXISTS test_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    checkout TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    member TEXT NOT NULL
  );
  CREATE INDEX IF NOT EXISTS test_runs_by_checkout ON test_runs (checkout, id);
  CREATE TABLE IF NOT EXISTS deployments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pid INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    member TEXT NOT NULL
  );
  CREATE INDEX IF NOT EXISTS deployments_by_run ON deployments (pid, started_at);
  CREATE TABLE IF NOT EXISTS log_lines (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deployment_id INTEGER NOT NULL,
    at INTEGER NOT NULL,
    text TEXT NOT NULL
  );
  CREATE INDEX IF NOT EXISTS log_lines_by_deployment ON log_lines (deployment_id, id);
  CREATE TABLE IF NOT EXISTS restarts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    unit_name TEXT NOT NULL,
    deployment_id INTEGER NOT NULL,
    at INTEGER NOT NULL,
    reason TEXT NOT NULL
  );
  CREATE INDEX IF NOT EXISTS restarts_by_unit ON restarts (unit_name, id);
  CREATE TABLE IF NOT EXISTS gate_passes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository TEXT NOT NULL,
    git_commit TEXT NOT NULL,
    range_from TEXT NOT NULL,
    heads TEXT NOT NULL,
    toolchains TEXT NOT NULL,
    machine TEXT NOT NULL,
    at INTEGER NOT NULL
  );
  CREATE INDEX IF NOT EXISTS gate_passes_by_commit ON gate_passes (repository, git_commit, id);
  CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    at INTEGER NOT NULL,
    data TEXT NOT NULL
  );
";

/// The record's file under a data root.
pub fn record_path(data_root: &Path) -> PathBuf {
  data_root.join("testbed").join(&RECORD.dir).join(&RECORD.file)
}

/// Says on standard error that the record was not written, and nothing more: a verb never stops over it.
pub fn say_unwritten(why: &str) {
  eprintln!("{}", fill_marks(&RECORD.lines.unwritten, &[("why", why)]));
}

/// A page's limit: the query's, the policy's default where it names none, never past the ceiling.
fn limit_of(asked: Option<i64>) -> i64 {
  asked.unwrap_or(RECORD.default_limit).clamp(1, RECORD.limit_ceiling)
}

fn failed(why: impl std::fmt::Display) -> String {
  why.to_string()
}

fn member_of<T: DeserializeOwned>(text: &str) -> Result<T, String> {
  serde_json::from_str(text).map_err(failed)
}

fn text_of(member: &impl Serialize) -> Result<String, String> {
  serde_json::to_string(member).map_err(failed)
}

/// A test run as it reads now: one still running whose process is gone is gone.
fn test_run_now(mut run: TestRun) -> TestRun {
  if run.state == TestRunState::Running && !process_alive(run.pid) {
    run.state = TestRunState::Gone;
  }
  run
}

/// A deployment as it reads now: one still running whose process is gone is gone.
fn deployment_now(mut deployment: Deployment) -> Deployment {
  if deployment.state == DeploymentState::Running && !process_alive(deployment.pid) {
    deployment.state = DeploymentState::Gone;
  }
  deployment
}

/// A page out of one more row than the limit asked: the rows up to the limit, and whether one stood past.
fn paged<T>(mut items: Vec<T>, limit: i64) -> Page<T> {
  let more = items.len() as i64 > limit;

  items.truncate(limit as usize);
  Page { items, more }
}

/// What a resume of the events asked for.
pub enum EventsRead {
  /// The events after the number, of the kinds named (every kind where none is), up to the limit.
  Page(Page<Event>),
  /// Events after the number asked from have gone past the window; the number is the last of them.
  Gone(i64),
}

/// One connection to the record.
pub struct Record {
  connection: Connection,
}

impl Record {
  /// The record under a data root, made where it does not stand yet.
  pub fn open(data_root: &Path) -> Result<Self, String> {
    Self::open_file(&record_path(data_root))
  }

  /// The record at a file, made where it does not stand yet.
  pub fn open_file(path: &Path) -> Result<Self, String> {
    if let Some(dir) = path.parent() {
      std::fs::create_dir_all(dir).map_err(failed)?;
    }

    let connection = Connection::open(path).map_err(failed)?;

    connection.busy_timeout(LOCK_WAIT).map_err(failed)?;
    connection.execute_batch(SCHEMA).map_err(failed)?;

    Ok(Self { connection })
  }

  fn writing<T>(&mut self, write: impl FnOnce(&Transaction<'_>) -> Result<T, String>) -> Result<T, String> {
    let transaction = self.connection.transaction().map_err(failed)?;
    let written = write(&transaction)?;

    prune(&transaction, now_ms())?;
    transaction.commit().map_err(failed)?;

    Ok(written)
  }

  /// A test run begun: written with its own id and as running, with its `test-run.created` event.
  pub fn begin_test_run(&mut self, run: &TestRun) -> Result<TestRun, String> {
    self.writing(|transaction| {
      transaction
        .execute(
          "INSERT INTO test_runs (checkout, started_at, member) VALUES (?1, ?2, '{}')",
          params![run.checkout, run.started_at],
        )
        .map_err(failed)?;

      let begun = TestRun {
        id: transaction.last_insert_rowid(),
        state: TestRunState::Running,
        ended_at: None,
        ..run.clone()
      };

      write_test_run(transaction, &begun)?;
      post(transaction, EventKind::TestRunCreated, &begun)?;
      Ok(begun)
    })
  }

  /// A test run ended with its state and counts, with its `test-run.ended` event. A run the record no
  /// longer holds is none.
  pub fn end_test_run(&mut self, id: i64, end: &TestRunEnd) -> Result<Option<TestRun>, String> {
    self.writing(|transaction| {
      let Some(standing) = read_test_run(transaction, id)? else {
        return Ok(None);
      };
      let ended = TestRun {
        state: end.state,
        passed_count: end.passed_count,
        failed_count: end.failed_count,
        skipped_count: end.skipped_count,
        ended_at: Some(end.ended_at),
        ..standing
      };

      write_test_run(transaction, &ended)?;
      post(transaction, EventKind::TestRunEnded, &ended)?;
      Ok(Some(ended))
    })
  }

  /// One test run, as it reads now.
  pub fn test_run(&self, id: i64) -> Result<Option<TestRun>, String> {
    Ok(read_test_run(&self.connection, id)?.map(test_run_now))
  }

  /// The test runs, newest first, older than the cursor, of one checkout where the query names one.
  pub fn test_runs(&self, query: &TestRunsQuery) -> Result<Page<TestRun>, String> {
    let limit = limit_of(query.limit);
    let mut statement = self
      .connection
      .prepare(
        "SELECT member FROM test_runs WHERE id < ?1 AND (?2 IS NULL OR checkout = ?2) ORDER BY id DESC LIMIT ?3",
      )
      .map_err(failed)?;
    let rows = statement
      .query_map(params![query.after.unwrap_or(i64::MAX), query.checkout, limit + 1], |row| {
        row.get::<_, String>(0)
      })
      .map_err(failed)?;
    let mut items = Vec::new();

    for row in rows {
      items.push(test_run_now(member_of(&row.map_err(failed)?)?));
    }

    Ok(paged(items, limit))
  }

  /// The run of a repository's whole suite a deployment's gate takes in place of its own (docs/deploy.md,
  /// 「What the deploy run does, in order」): the newest test run of kind `suite` of the repository, whole,
  /// begun on the commit from a clean tree no earlier than the moment given, as it reads now, where it
  /// passed; none where that run did not pass or none stands. A run whose commit, wholeness or tree the
  /// record does not hold is none of them.
  pub fn passed_suite(
    &self,
    repository: &str,
    commit: &str,
    since: i64,
  ) -> Result<Option<TestRun>, String> {
    let member: Option<String> = self
      .connection
      .query_row(
        "SELECT member FROM test_runs
          WHERE started_at >= ?1
            AND json_extract(member, '$.kind') = 'suite'
            AND json_extract(member, '$.repositoryName') = ?2
            AND json_extract(member, '$.commit') = ?3
            AND json_extract(member, '$.whole') = 1
            AND json_extract(member, '$.clean') = 1
          ORDER BY id DESC LIMIT 1",
        params![since, repository, commit],
        |row| row.get(0),
      )
      .optional()
      .map_err(failed)?;
    let Some(member) = member else {
      return Ok(None);
    };
    let run = test_run_now(member_of(&member)?);

    Ok((run.state == TestRunState::Passed).then_some(run))
  }

  /// A gate's pass written as it stands (docs/boundary.md, 「The record」); no event tells of it.
  pub fn write_gate_pass(&mut self, pass: &GatePass) -> Result<(), String> {
    self.writing(|transaction| {
      transaction
        .execute(
          "INSERT INTO gate_passes (repository, git_commit, range_from, heads, toolchains, machine, at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
          params![
            pass.ground.repository,
            pass.ground.commit,
            pass.ground.since.as_deref().unwrap_or_default(),
            pass.ground.heads.join("\n"),
            pass.ground.toolchains,
            pass.machine.as_deref().unwrap_or_default(),
            pass.at,
          ],
        )
        .map_err(failed)?;
      Ok(())
    })
  }

  /// The newest gate's pass on exactly the ground given, written no earlier than the moment given; none where
  /// none stands (docs/deploy.md, 「What the deploy run does, in order」, step 3).
  pub fn gate_passed(&self, ground: &GateGround, since: i64) -> Result<Option<GatePass>, String> {
    self
      .connection
      .query_row(
        "SELECT machine, at FROM gate_passes
          WHERE repository = ?1 AND git_commit = ?2 AND range_from = ?3 AND heads = ?4 AND toolchains = ?5
            AND at >= ?6
          ORDER BY id DESC LIMIT 1",
        params![
          ground.repository,
          ground.commit,
          ground.since.as_deref().unwrap_or_default(),
          ground.heads.join("\n"),
          ground.toolchains,
          since,
        ],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
      )
      .optional()
      .map_err(failed)
      .map(|found| {
        found.map(|(machine, at)| GatePass {
          ground: ground.clone(),
          machine: (!machine.is_empty()).then_some(machine),
          at,
        })
      })
  }

  /// The deployment a run began, by the process that began it and when; none where the record holds none.
  pub fn deployment_of(&self, pid: i64, started_at: i64) -> Result<Option<i64>, String> {
    self
      .connection
      .query_row(
        "SELECT id FROM deployments WHERE pid = ?1 AND started_at = ?2 ORDER BY id DESC LIMIT 1",
        params![pid, started_at],
        |row| row.get(0),
      )
      .optional()
      .map_err(failed)
  }

  /// A deployment written as it stands now: made, with its `deployment.created` event, where no deployment
  /// of its run stands yet, or else written over that one, with a `deployment.changed` event. Its id is the
  /// record's own.
  pub fn write_deployment(&mut self, deployment: &Deployment) -> Result<Deployment, String> {
    let standing = self.deployment_of(deployment.pid, deployment.started_at)?;

    self.writing(|transaction| {
      let (id, kind) = match standing {
        Some(id) => (id, EventKind::DeploymentChanged),
        None => {
          transaction
            .execute(
              "INSERT INTO deployments (pid, started_at, member) VALUES (?1, ?2, '{}')",
              params![deployment.pid, deployment.started_at],
            )
            .map_err(failed)?;
          (transaction.last_insert_rowid(), EventKind::DeploymentCreated)
        }
      };
      let written = Deployment {
        id,
        ..deployment.clone()
      };

      transaction
        .execute("UPDATE deployments SET member = ?1 WHERE id = ?2", params![text_of(&written)?, id])
        .map_err(failed)?;
      post(transaction, kind, &written)?;
      Ok(written)
    })
  }

  /// One deployment, as it reads now.
  pub fn deployment(&self, id: i64) -> Result<Option<Deployment>, String> {
    let member: Option<String> = self
      .connection
      .query_row("SELECT member FROM deployments WHERE id = ?1", params![id], |row| row.get(0))
      .optional()
      .map_err(failed)?;

    member.map(|member| member_of(&member).map(deployment_now)).transpose()
  }

  /// The deployments, newest first, older than the cursor.
  pub fn deployments(&self, query: &PageQuery) -> Result<Page<Deployment>, String> {
    let limit = limit_of(query.limit);
    let mut statement = self
      .connection
      .prepare("SELECT member FROM deployments WHERE id < ?1 ORDER BY id DESC LIMIT ?2")
      .map_err(failed)?;
    let rows = statement
      .query_map(params![query.after.unwrap_or(i64::MAX), limit + 1], |row| row.get::<_, String>(0))
      .map_err(failed)?;
    let mut items = Vec::new();

    for row in rows {
      items.push(deployment_now(member_of(&row.map_err(failed)?)?));
    }

    Ok(paged(items, limit))
  }

  /// The deployments of the repository named still under way: written running, their process alive.
  pub fn deployments_under_way(&self, repository: &str) -> Result<Vec<Deployment>, String> {
    let mut statement = self
      .connection
      .prepare(
        "SELECT member FROM deployments
          WHERE json_extract(member, '$.repositoryName') = ?1
            AND json_extract(member, '$.state') = 'running'
          ORDER BY id",
      )
      .map_err(failed)?;
    let rows = statement
      .query_map(params![repository], |row| row.get::<_, String>(0))
      .map_err(failed)?;
    let mut under_way = Vec::new();

    for row in rows {
      let deployment = deployment_now(member_of(&row.map_err(failed)?)?);

      if deployment.state == DeploymentState::Running {
        under_way.push(deployment);
      }
    }

    Ok(under_way)
  }

  /// One line a deployment said.
  pub fn add_log_line(&mut self, deployment_id: i64, text: &str) -> Result<LogLine, String> {
    let at = now_ms();

    self.writing(|transaction| {
      transaction
        .execute(
          "INSERT INTO log_lines (deployment_id, at, text) VALUES (?1, ?2, ?3)",
          params![deployment_id, at, text],
        )
        .map_err(failed)?;

      Ok(LogLine {
        id: transaction.last_insert_rowid(),
        at,
        text: text.to_owned(),
      })
    })
  }

  /// A deployment's log lines, oldest first, newer than the cursor; none where the deployment is not held.
  pub fn log_lines(&self, deployment_id: i64, query: &PageQuery) -> Result<Option<Page<LogLine>>, String> {
    if self.deployment(deployment_id)?.is_none() {
      return Ok(None);
    }

    let limit = limit_of(query.limit);
    let mut statement = self
      .connection
      .prepare(
        "SELECT id, at, text FROM log_lines WHERE deployment_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
      )
      .map_err(failed)?;
    let rows = statement
      .query_map(params![deployment_id, query.after.unwrap_or(0), limit + 1], |row| {
        Ok(LogLine {
          id: row.get(0)?,
          at: row.get(1)?,
          text: row.get(2)?,
        })
      })
      .map_err(failed)?;
    let items = rows.collect::<Result<Vec<_>, _>>().map_err(failed)?;

    Ok(Some(paged(items, limit)))
  }

  /// Why a deployment stops a unit, with its `restart.created` event.
  pub fn add_restart(
    &mut self,
    unit_name: &str,
    deployment_id: i64,
    reason: &str,
  ) -> Result<Restart, String> {
    let at = now_ms();

    self.writing(|transaction| {
      transaction
        .execute(
          "INSERT INTO restarts (unit_name, deployment_id, at, reason) VALUES (?1, ?2, ?3, ?4)",
          params![unit_name, deployment_id, at, reason],
        )
        .map_err(failed)?;

      let restart = Restart {
        id: transaction.last_insert_rowid(),
        unit_name: unit_name.to_owned(),
        deployment_id,
        reason: reason.to_owned(),
        at,
      };

      post(transaction, EventKind::RestartCreated, &restart)?;
      Ok(restart)
    })
  }

  /// The restarts, newest first, older than the cursor, of one unit where the query names one.
  pub fn restarts(&self, query: &RestartsQuery) -> Result<Page<Restart>, String> {
    let limit = limit_of(query.limit);
    let mut statement = self
      .connection
      .prepare(
        "SELECT id, unit_name, deployment_id, reason, at FROM restarts
          WHERE id < ?1 AND (?2 IS NULL OR unit_name = ?2) ORDER BY id DESC LIMIT ?3",
      )
      .map_err(failed)?;
    let rows = statement
      .query_map(params![query.after.unwrap_or(i64::MAX), query.unit_name, limit + 1], |row| {
        Ok(Restart {
          id: row.get(0)?,
          unit_name: row.get(1)?,
          deployment_id: row.get(2)?,
          reason: row.get(3)?,
          at: row.get(4)?,
        })
      })
      .map_err(failed)?;
    let items = rows.collect::<Result<Vec<_>, _>>().map_err(failed)?;

    Ok(paged(items, limit))
  }

  /// An event of the service's own, its data the member as a `GET` answers it now.
  pub fn post_event(&mut self, kind: EventKind, data: &impl Serialize) -> Result<Event, String> {
    self.writing(|transaction| post(transaction, kind, data))
  }

  /// The number of the last event written, or zero where none ever was.
  pub fn last_event_id(&self) -> Result<i64, String> {
    let last: Option<i64> = self
      .connection
      .query_row("SELECT seq FROM sqlite_sequence WHERE name = 'events'", [], |row| row.get(0))
      .optional()
      .map_err(failed)?;

    Ok(last.unwrap_or(0))
  }

  /// The number the window starts after: every event past it is still held, and none before it.
  pub fn window_start(&self) -> Result<i64, String> {
    let first_kept: Option<i64> = self
      .connection
      .query_row("SELECT MIN(id) FROM events", [], |row| row.get(0))
      .map_err(failed)?;

    first_kept.map_or_else(|| self.last_event_id(), |first| Ok(first - 1))
  }

  /// The events after a number, oldest first, of the kinds named, every kind where none is; or the last
  /// number past the window, where events after the one asked from have gone.
  pub fn events(&self, after: i64, kinds: &[EventKind], limit: Option<i64>) -> Result<EventsRead, String> {
    let past = self.window_start()?;

    if after < past {
      return Ok(EventsRead::Gone(past));
    }

    let limit = limit_of(limit);
    let names: Vec<&str> = kinds.iter().map(|kind| kind.name()).collect();
    let wanted = serde_json::to_string(&names).map_err(failed)?;
    let mut statement = self
      .connection
      .prepare(
        "SELECT id, kind, at, data FROM events
          WHERE id > ?1 AND (?2 = '[]' OR kind IN (SELECT value FROM json_each(?2))) ORDER BY id LIMIT ?3",
      )
      .map_err(failed)?;
    let rows = statement
      .query_map(params![after, wanted, limit + 1], |row| {
        Ok((
          row.get::<_, i64>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, i64>(2)?,
          row.get::<_, String>(3)?,
        ))
      })
      .map_err(failed)?;
    let mut items = Vec::new();

    for row in rows {
      let (id, kind, at, data) = row.map_err(failed)?;

      items.push(Event {
        id,
        kind: kind.parse().map_err(failed)?,
        at,
        data: member_of(&data)?,
      });
    }

    Ok(EventsRead::Page(paged(items, limit)))
  }
}

/// What a deployment's gate judged beside the commit (docs/deploy.md, 「What the deploy run does, in order」,
/// step 3): the repository, the commit, the commit its range began at, none where nothing was live, the head of
/// every repository laid beside the tree it judged, in order, and the toolchains its machine answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GateGround {
  pub repository: String,
  pub commit: String,
  pub since: Option<String>,
  pub heads: Vec<String>,
  pub toolchains: String,
}

/// A gate's pass: the ground it passed on, the machine it ran on where it is not the deploying one, and when
/// it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatePass {
  pub ground: GateGround,
  pub machine: Option<String>,
  pub at: i64,
}

/// How a test run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestRunEnd {
  pub state: TestRunState,
  pub passed_count: i64,
  pub failed_count: i64,
  pub skipped_count: i64,
  pub ended_at: i64,
}

fn read_test_run(connection: &Connection, id: i64) -> Result<Option<TestRun>, String> {
  let member: Option<String> = connection
    .query_row("SELECT member FROM test_runs WHERE id = ?1", params![id], |row| row.get(0))
    .optional()
    .map_err(failed)?;

  member.map(|member| member_of(&member)).transpose()
}

fn write_test_run(transaction: &Transaction<'_>, run: &TestRun) -> Result<(), String> {
  transaction
    .execute("UPDATE test_runs SET member = ?1 WHERE id = ?2", params![text_of(run)?, run.id])
    .map_err(failed)?;
  Ok(())
}

fn post(transaction: &Transaction<'_>, kind: EventKind, data: &impl Serialize) -> Result<Event, String> {
  let at = now_ms();
  let data = serde_json::to_value(data).map_err(failed)?;

  transaction
    .execute(
      "INSERT INTO events (kind, at, data) VALUES (?1, ?2, ?3)",
      params![kind.name(), at, text_of(&data)?],
    )
    .map_err(failed)?;

  Ok(Event {
    id: transaction.last_insert_rowid(),
    kind,
    at,
    data,
  })
}

// What has stood past the policy's days goes: test runs and deployments with their log lines and restarts, the
// gates' passes, and the events past the window.
fn prune(transaction: &Transaction<'_>, now: i64) -> Result<(), String> {
  let kept_from = now - RECORD.keep_days * DAY_MS;
  let window_from = now - RECORD.event_window_days * DAY_MS;

  transaction
    .execute_batch(&format!(
      "DELETE FROM test_runs WHERE started_at < {kept_from};
       DELETE FROM log_lines WHERE deployment_id IN (SELECT id FROM deployments WHERE started_at < {kept_from});
       DELETE FROM restarts WHERE at < {kept_from};
       DELETE FROM deployments WHERE started_at < {kept_from};
       DELETE FROM gate_passes WHERE at < {kept_from};
       DELETE FROM events WHERE at < {window_from};"
    ))
    .map_err(failed)
}
