//! What the testbed tells instead of telling anyone itself (docs/boundary.md, 「Events」): an event goes to
//! the hook the environment or the declaration names, run through `sh` with the event's name as its first
//! argument and the payload on its standard input. The hook never changes a verb's outcome: one that fails
//! or overruns its time is said on a line, and the verb goes on.

use std::io::Write as _;
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::Serialize;
use serde_json::Value;

use crate::declaration::{Events, Repository};
use crate::policy::events::EVENTS;
use crate::policy::fill_marks;
use crate::turn::Env;

/// The variable naming the event hook over the declaration's.
pub const EVENT_HOOK_VARIABLE: &str = "BMO_TESTBED_EVENT_HOOK";

/// One event: its name, the commit it concerns, one English line saying what happened, and its own fields.
pub struct Event<'a> {
  pub name: &'a str,
  pub commit: Option<String>,
  pub summary: String,
  pub detail: Value,
}

// The payload the hook reads.
#[derive(Serialize)]
struct Payload<'a> {
  version: u64,
  event: &'a str,
  at: String,
  repository: Option<&'a str>,
  root: Option<String>,
  commit: Option<&'a str>,
  summary: &'a str,
  detail: &'a Value,
}

/// Where events go: the hook, its time, and the repository and tree they happen in.
pub struct Hook {
  pub command: Option<String>,
  pub repository: Option<String>,
  pub root: Option<PathBuf>,
}

impl Hook {
  /// The hook the environment names, or the repository's declared one; a verb run outside any repository
  /// has the environment's alone.
  pub fn of(repo: Option<&Repository>, env: &Env) -> Self {
    let declared = repo.map_or_else(Events::default, |repo| repo.declaration.events.clone());
    let command = env
      .get(EVENT_HOOK_VARIABLE)
      .filter(|named| !named.is_empty())
      .cloned()
      .or(declared.hook);

    Self {
      command,
      repository: repo.map(Repository::name),
      root: repo.map(|repo| repo.root.clone()),
    }
  }

  /// The payload of an event, as the hook reads it.
  pub fn payload(&self, event: &Event<'_>) -> String {
    let payload = Payload {
      version: EVENTS.version,
      event: event.name,
      at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
      repository: self.repository.as_deref(),
      root: self.root.as_ref().map(|root| root.to_string_lossy().into_owned()),
      commit: event.commit.as_deref(),
      summary: &event.summary,
      detail: &event.detail,
    };

    serde_json::to_string(&payload).unwrap_or_default()
  }

  /// The event sent: to the hook, or onto standard error where there is none. What came of it is said on
  /// standard error and changes nothing else.
  pub fn send(&self, event: &Event<'_>) {
    let said = &EVENTS.lines;
    let payload = self.payload(event);
    let Some(command) = &self.command else {
      eprintln!("{}", fill_marks(&said.unhooked, &[("event", event.name), ("payload", &payload)]));
      return;
    };
    let failed =
      |why: &str| eprintln!("{}", fill_marks(&said.failed, &[("event", event.name), ("why", why)]));
    // The hook's own line, handed the event's name as its first argument.
    let started = Command::new("sh")
      .arg("-c")
      .arg(format!("{command} \"$@\""))
      .arg("sh")
      .arg(event.name)
      .stdin(Stdio::piped())
      .stdout(Stdio::null())
      .process_group(0)
      .spawn();
    let mut child = match started {
      Ok(child) => child,
      Err(why) => return failed(&why.to_string()),
    };

    if let Some(mut input) = child.stdin.take() {
      let _ = input.write_all(payload.as_bytes());
    }

    // The hook is waited for until it ends, however long that takes.
    match child.wait() {
      Ok(status) if status.success() => {}
      Ok(status) => failed(&status.to_string()),
      Err(why) => failed(&why.to_string()),
    }
  }
}
