//! One unit of a run in a process of its own, heading a process group of its own, run until it ends or falls
//! silent (docs/suite.md, 「One sweep, a process per file」 and 「Every unit runs to its end」). Its output is
//! held until it ends, since the units beside it would otherwise interleave with it, and is then written out
//! whole with the verdict on the unit ahead of it; while it writes, the holder above the run is told so.
//! Whatever the unit started and left behind goes once it ends: its group, and any process outside the group
//! still holding its output.

use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

use bmo_testbed_core::silence::{Output, Silence, Waited, Watch};

/// How a unit is started.
pub struct UnitCommand {
  pub program: String,
  pub args: Vec<String>,
  pub cwd: PathBuf,
  /// The whole environment it runs in.
  pub env: Vec<(String, String)>,
  /// A home of its own, removed once the unit ends.
  pub home: Option<PathBuf>,
  /// How long it may write nothing before it is ended as hung.
  pub silence: Silence,
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
      silence: Silence::default(),
    }
  }
}

/// What one unit came to.
#[derive(Clone, Debug)]
pub struct UnitRun {
  /// The code it ended with, a signal that ended it read as the shell reads it.
  pub code: i32,
  /// The signal that ended it, where one did.
  pub signal: Option<i32>,
  pub seconds: f64,
  pub output: String,
}

// A stream read to its end beside the unit, and the pipe it is read from, as the kernel names it.
struct Gathered {
  pipe: Option<PathBuf>,
  reader: JoinHandle<Vec<u8>>,
}

// The stream read beside the unit, every byte taken added to `heard` as it comes.
fn gather(mut stream: impl Read + AsRawFd + Send + 'static, heard: Arc<AtomicU64>) -> Gathered {
  let pipe = std::fs::read_link(format!("/proc/self/fd/{}", stream.as_raw_fd())).ok();
  let reader = std::thread::spawn(move || {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 16 * 1024];

    while let Ok(read) = stream.read(&mut chunk) {
      if read == 0 {
        break;
      }

      heard.fetch_add(read as u64, Ordering::Relaxed);
      bytes.extend_from_slice(&chunk[..read]);
    }

    bytes
  });

  Gathered { pipe, reader }
}

// The process that started this one, from the `PPid` field of its status file.
fn parent_of(process: &Path) -> Option<u32> {
  let status = std::fs::read_to_string(process.join("status")).ok()?;

  status.lines().find_map(|line| line.strip_prefix("PPid:")?.trim().parse().ok())
}

/// Every process holding the pipe open but this one and the ones it started itself: what the unit started
/// and left outside its group. A process this one is still starting holds every pipe open here until it
/// becomes its own program, so it is no holder; and this process is no subreaper, so nothing a unit left
/// behind is its child.
pub fn holders_of(pipe: &Path) -> Vec<i32> {
  let me = std::process::id();
  let Ok(processes) = std::fs::read_dir("/proc") else { return Vec::new() };

  processes
    .flatten()
    .filter_map(|process| {
      let pid: u32 = process.file_name().to_str()?.parse().ok()?;

      if pid == me || parent_of(&process.path()) == Some(me) {
        return None;
      }

      let fds = std::fs::read_dir(process.path().join("fd")).ok()?;
      let holds = fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|link| link == *pipe));

      holds.then_some(pid as i32)
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

// The code the unit ended with, and the signal that ended it, where one did; a unit ended as hung ends on the
// hung code, its own signal no part of it.
fn ended(waited: &Waited) -> (i32, Option<i32>) {
  if waited.hung.is_some() {
    return (waited.code().unwrap_or(1), None);
  }

  match waited.status {
    Some(status) => {
      let signal = std::os::unix::process::ExitStatusExt::signal(&status);

      (status.code().unwrap_or_else(|| 128 + signal.unwrap_or(0)), signal)
    }
    None => (1, None),
  }
}

/// Runs one unit to its end, however long that takes while it writes, or until it has written nothing for
/// its silence.
pub fn run_unit(unit: &UnitCommand) -> UnitRun {
  let started = Instant::now();
  let heard = Arc::new(AtomicU64::new(0));
  let watch = Watch::new(Output::Read(Arc::clone(&heard)), unit.silence);
  let spawned = Command::new(&unit.program)
    .args(&unit.args)
    .current_dir(&unit.cwd)
    .env_clear()
    .envs(unit.env.iter().map(|(name, value)| (name, value)))
    .envs(watch.variable())
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
        signal: None,
        seconds: 0.0,
        output: why.to_string(),
      };
    }
  };
  let leader = child.id();
  let out = gather(child.stdout.take().expect("the output is piped"), Arc::clone(&heard));
  let err = gather(child.stderr.take().expect("the errors are piped"), heard);
  let what = std::iter::once(&unit.program).chain(&unit.args).cloned().collect::<Vec<_>>().join(" ");
  let waited = watch.wait(&mut child, &what);
  let (code, signal) = ended(&waited);

  // What the unit left standing in its group goes at once, and so does anything outside it still holding the
  // output, so the output closes.
  signal_group(leader, libc::SIGKILL);

  let said = gathered(out);
  let warned = gathered(err);

  if let Some(home) = &unit.home {
    let _ = std::fs::remove_dir_all(home);
  }

  let mut output = format!("{said}{warned}");

  if let Some(hung) = &waited.hung {
    if !output.is_empty() && !output.ends_with('\n') {
      output.push('\n');
    }

    output.push_str(hung);
    output.push('\n');
  }

  UnitRun {
    code,
    signal,
    seconds: started.elapsed().as_secs_f64(),
    output,
  }
}
