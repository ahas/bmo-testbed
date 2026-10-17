//! One unit of a run in a process of its own, heading a process group of its own, under the file guard
//! (docs/suite.md, 「One sweep, a process per file」). Its output is held until it ends, since the
//! units beside it would otherwise interleave with it, and is then written out whole with the verdict on
//! the unit ahead of it. Whatever the unit started and left behind goes with the group once it ends.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bmo_testbed_core::policy::suite::SUITE;

// How often a running unit is looked at.
fn look_every() -> Duration {
  Duration::from_millis(SUITE.numbers.unit_poll_ms)
}

/// How a unit is started.
pub struct UnitCommand {
  pub program: String,
  pub args: Vec<String>,
  pub cwd: PathBuf,
  /// The whole environment it runs in.
  pub env: Vec<(String, String)>,
  /// A home of its own, removed once the unit ends.
  pub home: Option<PathBuf>,
}

impl UnitCommand {
  /// A unit running a command line, its program first.
  pub fn of(line: Vec<String>, cwd: PathBuf, env: Vec<(String, String)>, home: Option<PathBuf>) -> Self {
    let mut words = line.into_iter();

    Self {
      program: words.next().unwrap_or_default(),
      args: words.collect(),
      cwd,
      env,
      home,
    }
  }
}

/// What one unit came to.
#[derive(Clone, Debug)]
pub struct UnitRun {
  pub code: i32,
  pub seconds: f64,
  pub cut: bool,
  pub output: String,
  /// Whether a process the unit started outside its own group still held its output past the grace.
  pub held: bool,
}

struct Gathered {
  bytes: Arc<Mutex<Vec<u8>>>,
  reader: JoinHandle<()>,
}

fn gather(mut stream: impl Read + Send + 'static) -> Gathered {
  let bytes = Arc::new(Mutex::new(Vec::new()));
  let into = Arc::clone(&bytes);
  let reader = std::thread::spawn(move || {
    let mut chunk = [0u8; 64 * 1024];

    while let Ok(read) = stream.read(&mut chunk) {
      if read == 0 {
        break;
      }

      into
        .lock()
        .expect("the output is held only to add to it")
        .extend_from_slice(&chunk[..read]);
    }
  });

  Gathered { bytes, reader }
}

// Everything a stream said, once it ended or once the grace after the unit's end is spent; and whether it
// ended.
fn gathered(stream: Gathered, until: Instant) -> (String, bool) {
  while !stream.reader.is_finished() && Instant::now() < until {
    std::thread::sleep(look_every());
  }

  let ended = stream.reader.is_finished();
  let text =
    String::from_utf8_lossy(&stream.bytes.lock().expect("the output is held only to read it")).into_owned();

  (text, ended)
}

fn signal_group(leader: u32, signal: i32) {
  // SAFETY: a signal to the unit's own process group, which nothing else stands in.
  unsafe { libc::kill(-(leader as i32), signal) };
}

fn ended(child: &mut Child) -> Option<i32> {
  match child.try_wait() {
    Ok(Some(status)) => Some(
      status
        .code()
        .unwrap_or_else(|| 128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0)),
    ),
    Ok(None) => None,
    Err(_) => Some(1),
  }
}

/// Runs one unit to its end, cut where it outlasts the guard.
pub fn run_unit(unit: &UnitCommand, guard: Duration) -> UnitRun {
  let started = Instant::now();
  let spawned = Command::new(&unit.program)
    .args(&unit.args)
    .current_dir(&unit.cwd)
    .env_clear()
    .envs(unit.env.iter().map(|(name, value)| (name, value)))
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .process_group(0)
    .spawn();
  let mut child = match spawned {
    Ok(child) => child,
    Err(why) => {
      return UnitRun {
        code: 127,
        seconds: 0.0,
        cut: false,
        output: why.to_string(),
        held: false,
      };
    }
  };
  let leader = child.id();
  let out = gather(child.stdout.take().expect("the output is piped"));
  let err = gather(child.stderr.take().expect("the errors are piped"));
  let mut cut = false;
  let mut asked_at: Option<Instant> = None;
  let code = loop {
    if let Some(code) = ended(&mut child) {
      break code;
    }

    if !cut && started.elapsed() >= guard {
      cut = true;
      asked_at = Some(Instant::now());
      signal_group(leader, libc::SIGTERM);
    }

    if asked_at
      .is_some_and(|asked| asked.elapsed() >= Duration::from_secs(SUITE.numbers.unit_kill_after_seconds))
    {
      signal_group(leader, libc::SIGKILL);
    }

    std::thread::sleep(look_every());
  };

  signal_group(leader, libc::SIGKILL);

  // A process that left the group on its own can still hold the output open, and the run does not wait on it.
  let until = Instant::now() + Duration::from_secs(SUITE.numbers.unit_output_grace_seconds);
  let (said, said_whole) = gathered(out, until);
  let (warned, warned_whole) = gathered(err, until);

  if let Some(home) = &unit.home {
    let _ = std::fs::remove_dir_all(home);
  }

  UnitRun {
    code,
    seconds: started.elapsed().as_secs_f64(),
    cut,
    output: format!("{said}{warned}"),
    held: !(said_whole && warned_whole),
  }
}
