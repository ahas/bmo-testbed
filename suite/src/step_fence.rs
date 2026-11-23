//! The fence a check step runs in (docs/checks.md, 「The static check」): a transient scope of its own, which
//! the user's service manager starts around a keeper the step holds, and which the step's command moves
//! itself into before it executes, or where it may not, as from a login session's scope, is moved into by the
//! manager while it waits to execute. The command and everything it starts then count against the scope's own
//! ceiling, never against the unit the step's caller stands in.

use std::ffi::CString;
use std::io::{Read as _, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use bmo_testbed_core::memory::{CGROUP_ROOT, MemoryFence, fence_reached, read_fence};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::static_check::STATIC_CHECK;

use crate::fence::{FenceLimits, FenceSeams, LiveFence, attach_call, start_scope, user_bus_refusal};

/// The slice every step's fence runs in, one of this repository's `units/` (docs/deploy.md, 「The slices」).
pub const CHECK_SLICE: &str = "bmo-check.slice";

/// The scope of the `count`th fence the process `pid` opened.
pub fn step_unit(pid: u32, count: u64) -> String {
  format!("bmo-check-{pid}-{count}.scope")
}

/// What a step's fence holds it to, as the policy keeps it.
pub fn step_limits() -> FenceLimits {
  let ceilings = &STATIC_CHECK.fence;

  FenceLimits {
    memory_gb: ceilings.memory_gb,
    swap_gb: ceilings.swap_gb,
    tasks: ceilings.tasks,
  }
}

/// A step's scope, standing while the step holds it: the keeper it was started around is ended when it goes.
pub struct StepFence {
  pub unit: String,
  /// The scope's control group, by its path under the cgroup root.
  pub group: String,
  procs: CString,
  opened: Option<MemoryFence>,
  keeper: Child,
  // The environment the manager is asked in, and whether a command tries to move itself before it is moved.
  env: Vec<(String, String)>,
  moves_itself: bool,
}

impl StepFence {
  /// A scope of its own, started around a keeper this process holds, or why none could be had.
  pub fn open(limits: FenceLimits, seams: &mut dyn FenceSeams) -> Result<Self, String> {
    static OPENED: AtomicU64 = AtomicU64::new(0);

    let env = seams.env();

    if let Some(refused) = user_bus_refusal(&env) {
      return Err(refused);
    }

    // The keeper reads a pipe this process holds, so it ends with this process however that ends.
    let mut keeper = Command::new("cat")
      .stdin(Stdio::piped())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()
      .map_err(|why| why.to_string())?;
    let unit = step_unit(seams.pid(), OPENED.fetch_add(1, Ordering::Relaxed));
    let group = match start_scope(&unit, CHECK_SLICE, keeper.id(), limits, &env, seams) {
      Ok(group) => group,
      Err(why) => {
        let _ = keeper.kill();
        let _ = keeper.wait();

        return Err(why);
      }
    };
    let procs = Path::new(CGROUP_ROOT).join(group.trim_start_matches('/')).join("cgroup.procs");

    Ok(Self {
      procs: CString::new(procs.to_string_lossy().into_owned()).map_err(|why| why.to_string())?,
      opened: read_fence(Path::new(CGROUP_ROOT), &group),
      unit,
      group,
      keeper,
      moves_itself: seams.moves_itself(),
      env,
    })
  }

  /// Opens one on the machine's own manager, saying on standard error why none could be had.
  pub fn open_for(name: &str) -> Option<Self> {
    Self::open(step_limits(), &mut LiveFence).map_err(|why| say_none(name, &why)).ok()
  }

  /// `command` started inside the scope, born there: it moves itself in before it executes, and where it may not,
  /// it waits while the manager moves it, then executes. The answer is the start, and why the command does not
  /// stand in the scope where it does not; a command that could not be moved still runs.
  pub fn spawn(&self, command: &mut Command) -> (std::io::Result<Child>, Option<String>) {
    let (Some((told, telling)), Some((waiting, going))) = (pipe(), pipe()) else {
      return (command.spawn(), Some(self.unjoined("no pipe could be made to tell the move")));
    };
    let procs = self.procs.clone();
    let tell = telling.as_raw_fd();
    let wait = waiting.as_raw_fd();
    let itself = self.moves_itself;

    // SAFETY: between fork and exec the closure calls only getpid, open, read, write and close, each safe there,
    // on memory made before the fork. Every pipe end closes at exec, so the reader hears nothing where the command
    // moved itself, and its error number and process number where it did not, after which it waits for the
    // manager's move to be told before it executes.
    unsafe {
      command.pre_exec(move || {
        let mut errno = 0;

        if itself {
          let fd = libc::open(procs.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
          let moved = fd >= 0 && libc::write(fd, b"0".as_ptr().cast(), 1) == 1;

          if moved {
            libc::close(fd);
            return Ok(());
          }

          errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);

          if fd >= 0 {
            libc::close(fd);
          }
        }

        let mut message = [0u8; 8];

        message[..4].copy_from_slice(&errno.to_ne_bytes());
        message[4..].copy_from_slice(&libc::getpid().to_ne_bytes());
        libc::write(tell, message.as_ptr().cast(), message.len());

        let mut go = [0u8; 1];

        libc::read(wait, go.as_mut_ptr().cast(), 1);
        Ok(())
      });
    }

    std::thread::scope(|scope| {
      let moving = scope.spawn(move || self.moved_by_manager(told, going));
      let child = command.spawn();

      // This process's ends go, so the reader hears the end of the pipe where the command told nothing.
      drop(telling);
      drop(waiting);

      let missed = moving.join().unwrap_or_else(|_| Some(self.unjoined("the move could not be told")));

      (child, missed)
    })
  }

  // What a command started into the scope told: none where it moved itself or never started; where it did not,
  // the manager asked to move it, the command then let go on, and why it stands outside where the manager did
  // not move it either.
  fn moved_by_manager(&self, told: OwnedFd, going: OwnedFd) -> Option<String> {
    let mut message = [0u8; 8];

    std::fs::File::from(told).read_exact(&mut message).ok()?;

    let errno = i32::from_ne_bytes(message[..4].try_into().unwrap_or_default());
    let pid = u32::from_ne_bytes(message[4..].try_into().unwrap_or_default());
    let (moved, said) = LiveFence.call(&attach_call(&self.unit, pid), &self.env);

    let _ = std::fs::File::from(going).write_all(&[1]);

    if moved {
      return None;
    }

    let own = (errno != 0).then(|| std::io::Error::from_raw_os_error(errno).to_string());

    Some(self.unjoined(&fill_marks(
      &STATIC_CHECK.fence_unmoved,
      &[("own", own.as_deref().unwrap_or("its own move was not tried")), ("said", &said)],
    )))
  }

  fn unjoined(&self, why: &str) -> String {
    fill_marks(&STATIC_CHECK.fence_unjoined, &[("unit", &self.unit), ("why", why)])
  }

  /// The scope as it stands, where its own ceiling has been reached since it was opened.
  pub fn reached(&self) -> Option<MemoryFence> {
    let now = read_fence(Path::new(CGROUP_ROOT), &self.group)?;
    let opened = self.opened.clone()?;

    fence_reached(&[opened], std::slice::from_ref(&now)).cloned()
  }
}

impl Drop for StepFence {
  fn drop(&mut self) {
    let _ = self.keeper.kill();
    let _ = self.keeper.wait();
  }
}

// A pipe whose two ends close at exec: its reading end, then its writing end.
fn pipe() -> Option<(OwnedFd, OwnedFd)> {
  let mut ends = [-1; 2];

  // SAFETY: pipe2 fills the two descriptors it is handed, which are owned from here on.
  if unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
    return None;
  }

  // SAFETY: both descriptors were just opened and nothing else owns them.
  Some(unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) })
}

/// Says on standard error that the step `name` runs where its caller stands, and why.
pub fn say_none(name: &str, why: &str) {
  eprintln!("{}", fill_marks(&STATIC_CHECK.fence_none, &[("name", name), ("why", why)]));
}
