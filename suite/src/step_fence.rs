//! The fence a check step runs in (docs/checks.md, 「The static check」): a transient scope of its own, which
//! the user's service manager starts around a keeper the step holds, and which the step's command moves
//! itself into before it executes. The command and everything it starts then count against the scope's own
//! ceiling, never against the unit the step's caller stands in.

use std::ffi::CString;
use std::io::Read as _;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use bmo_testbed_core::memory::{CGROUP_ROOT, MemoryFence, fence_reached, read_fence};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::static_check::STATIC_CHECK;

use crate::fence::{FenceLimits, FenceSeams, LiveFence, start_scope, user_bus_refusal};

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
}

/// Whether the command started with a fence's `join` moved itself into the scope.
pub struct Joining {
  unit: String,
  told: Option<OwnedFd>,
  telling: Option<OwnedFd>,
}

impl Joining {
  /// Read once the command was started: why it does not stand in the scope, none where it does. A command
  /// that never started tells nothing.
  pub fn missed(mut self) -> Option<String> {
    drop(self.telling.take());

    let mut errno = [0u8; 4];
    let mut told = std::fs::File::from(self.told.take()?);

    told.read_exact(&mut errno).ok()?;

    let why = std::io::Error::from_raw_os_error(i32::from_ne_bytes(errno));

    Some(fill_marks(
      &STATIC_CHECK.fence_unjoined,
      &[("unit", &self.unit), ("why", &why.to_string())],
    ))
  }
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
    })
  }

  /// Opens one on the machine's own manager, saying on standard error why none could be had.
  pub fn open_for(name: &str) -> Option<Self> {
    Self::open(step_limits(), &mut LiveFence).map_err(|why| say_none(name, &why)).ok()
  }

  /// Has `command`, once started, move itself into the scope before it executes, so it is born inside. The
  /// answer, read once it has started, says whether it did; a command that could not still runs.
  pub fn join(&self, command: &mut Command) -> Joining {
    let mut ends = [-1; 2];

    // SAFETY: pipe2 fills the two descriptors it is handed, which are owned from here on.
    if unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
      return Joining {
        unit: self.unit.clone(),
        told: None,
        telling: None,
      };
    }

    // SAFETY: both descriptors were just opened and nothing else owns them.
    let (told, telling) = unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
    let procs = self.procs.clone();
    let tell = telling.as_raw_fd();

    // SAFETY: between fork and exec the closure calls only open, write and close, each safe there, on memory
    // made before the fork. The pipe's writing end closes at exec, so the reader hears nothing where the move
    // was made and the error number where it was not.
    unsafe {
      command.pre_exec(move || {
        let fd = libc::open(procs.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        let moved = fd >= 0 && libc::write(fd, b"0".as_ptr().cast(), 1) == 1;

        if !moved {
          let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0).to_ne_bytes();

          libc::write(tell, errno.as_ptr().cast(), errno.len());
        }

        if fd >= 0 {
          libc::close(fd);
        }

        Ok(())
      });
    }

    Joining {
      unit: self.unit.clone(),
      told: Some(told),
      telling: Some(telling),
    }
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

/// Says on standard error that the step `name` runs where its caller stands, and why.
pub fn say_none(name: &str, why: &str) {
  eprintln!("{}", fill_marks(&STATIC_CHECK.fence_none, &[("name", name), ("why", why)]));
}
