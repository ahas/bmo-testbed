//! One unit of a run in a process of its own, heading a process group of its own, run until it ends
//! (docs/suite.md, 「One sweep, a process per file」). Its output is held until it ends, since the
//! units beside it would otherwise interleave with it, and is then written out whole with the verdict on
//! the unit ahead of it. Whatever the unit started and left behind goes once it ends: its group, and any
//! process outside the group still holding its output.

use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;
use std::time::Instant;

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
  pub output: String,
}

// A stream read to its end beside the unit, and the pipe it is read from, as the kernel names it.
struct Gathered {
  pipe: Option<PathBuf>,
  reader: JoinHandle<Vec<u8>>,
}

fn gather(mut stream: impl Read + AsRawFd + Send + 'static) -> Gathered {
  let pipe = std::fs::read_link(format!("/proc/self/fd/{}", stream.as_raw_fd())).ok();
  let reader = std::thread::spawn(move || {
    let mut bytes = Vec::new();
    let _ = stream.read_to_end(&mut bytes);

    bytes
  });

  Gathered { pipe, reader }
}

// Every process but this one holding the pipe open: what the unit started and left outside its group.
fn holders_of(pipe: &PathBuf) -> Vec<i32> {
  let me = std::process::id().to_string();
  let Ok(processes) = std::fs::read_dir("/proc") else { return Vec::new() };

  processes
    .flatten()
    .filter_map(|process| {
      let pid = process.file_name().to_string_lossy().into_owned();

      if pid == me || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
      }

      let fds = std::fs::read_dir(process.path().join("fd")).ok()?;
      let holds = fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|link| link == *pipe));

      holds.then(|| pid.parse().ok()).flatten()
    })
    .collect()
}

// Everything a stream said, read to its end once whatever still held it open has been ended.
fn gathered(stream: Gathered) -> String {
  if let Some(pipe) = &stream.pipe {
    for pid in holders_of(pipe) {
      // SAFETY: a signal to a process holding this unit's own output pipe, which the unit started.
      unsafe { libc::kill(pid, libc::SIGKILL) };
    }
  }

  String::from_utf8_lossy(&stream.reader.join().unwrap_or_default()).into_owned()
}

fn signal_group(leader: u32, signal: i32) {
  // SAFETY: a signal to the unit's own process group, which nothing else stands in.
  unsafe { libc::kill(-(leader as i32), signal) };
}

// The code the unit ended with, a signal that ended it read as the shell reads it.
fn ended(child: &mut Child) -> i32 {
  match child.wait() {
    Ok(status) => status
      .code()
      .unwrap_or_else(|| 128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0)),
    Err(_) => 1,
  }
}

/// Runs one unit to its end, however long that takes.
pub fn run_unit(unit: &UnitCommand) -> UnitRun {
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
        output: why.to_string(),
      };
    }
  };
  let leader = child.id();
  let out = gather(child.stdout.take().expect("the output is piped"));
  let err = gather(child.stderr.take().expect("the errors are piped"));
  let code = ended(&mut child);

  // What the unit left standing in its group goes at once, and so does anything outside it still holding the
  // output, so the output closes.
  signal_group(leader, libc::SIGKILL);

  let said = gathered(out);
  let warned = gathered(err);

  if let Some(home) = &unit.home {
    let _ = std::fs::remove_dir_all(home);
  }

  UnitRun {
    code,
    seconds: started.elapsed().as_secs_f64(),
    output: format!("{said}{warned}"),
  }
}
