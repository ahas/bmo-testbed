//! The supervisor's control socket (docs/supervise.md, 「The control socket」): one JSON request a line, answered
//! by one JSON line, and the connection closed. The stand-ins, the `supervise` verb and a packager's screen
//! all ask it so.

use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::supervise::SUPERVISE;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::state::UnitState;

/// What a caller asks of the supervisor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "kebab-case")]
pub enum Request {
  Start {
    units: Vec<String>,
  },
  Stop {
    units: Vec<String>,
  },
  Restart {
    units: Vec<String>,
  },
  /// Restarted where it runs, left alone where it does not.
  TryRestart {
    units: Vec<String>,
  },
  ResetFailed {
    units: Vec<String>,
  },
  /// The unit files read again.
  DaemonReload,
  /// Each unit linked into its target's wants folder, a unit named by a path linked into the unit directory
  /// first; started too with `now`.
  Enable {
    units: Vec<String>,
    now: bool,
  },
  /// Each unit's wants link taken away; stopped too with `now`.
  Disable {
    units: Vec<String>,
    now: bool,
  },
  /// A signal sent to each unit's processes.
  Kill {
    units: Vec<String>,
    signal: i32,
  },
  /// Each named unit's state, every unit's where none is named.
  Show {
    units: Vec<String>,
  },
  /// The environment every unit starts from.
  ShowEnvironment,
  /// A transient service of `systemd-run`, answered once it started, or once it ended with `wait`.
  Run {
    unit: Option<String>,
    words: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    collect: bool,
    wait: bool,
  },
  /// Every unit stopped and the supervisor ended.
  Shutdown,
}

/// What the supervisor answers: a code as systemd's tools end with, what they would print, and the states asked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
  pub code: i32,
  pub out: String,
  pub err: String,
  pub states: Vec<UnitState>,
}

impl Answer {
  pub fn ok() -> Self {
    Self::default()
  }

  pub fn failed(code: i32, err: String) -> Self {
    Self {
      code,
      err,
      ..Self::default()
    }
  }

  pub fn said(out: String) -> Self {
    Self {
      out,
      ..Self::default()
    }
  }

  /// Two answers as one, the worse code kept and both outputs joined.
  pub fn and(mut self, other: Answer) -> Self {
    if self.code == 0 {
      self.code = other.code;
    }

    for (mine, theirs) in [(&mut self.out, other.out), (&mut self.err, other.err)] {
      if !theirs.is_empty() {
        if !mine.is_empty() && !mine.ends_with('\n') {
          mine.push('\n');
        }

        mine.push_str(&theirs);
      }
    }

    self.states.extend(other.states);
    self
  }
}

/// Asks the supervisor at `socket` and waits for its answer, however long the request takes; the line saying why
/// where the socket cannot be reached.
pub async fn ask(socket: &Path, request: &Request) -> Result<Answer, String> {
  let unreachable = |why: String| {
    fill_marks(&SUPERVISE.said.unreachable, &[("socket", &socket.to_string_lossy()), ("why", &why)])
  };
  let mut stream = UnixStream::connect(socket).await.map_err(|why| unreachable(why.to_string()))?;
  let mut line = serde_json::to_string(request).map_err(|why| unreachable(why.to_string()))?;

  line.push('\n');
  stream.write_all(line.as_bytes()).await.map_err(|why| unreachable(why.to_string()))?;

  let mut answer = String::new();

  BufReader::new(stream)
    .read_line(&mut answer)
    .await
    .map_err(|why| unreachable(why.to_string()))?;
  serde_json::from_str(&answer).map_err(|why| unreachable(why.to_string()))
}

/// `ask` from a caller with no runtime of its own.
pub fn ask_blocking(socket: &Path, request: &Request) -> Result<Answer, String> {
  tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .map_err(|why| why.to_string())?
    .block_on(ask(socket, request))
}
