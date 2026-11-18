//! The machine's own event hook (docs/boundary.md, 「Events」): the hook the caller's environment names, else the
//! one the machine's testbed environment file names, the file the timed prune's service reads as its environment
//! (docs/deploy.md, 「The Cargo build directory」), else the running binary's `notice`, which tells the owner's
//! supervisor. The placed machine's events are the machine's and no repository's, so no declaration names their
//! hook.

use std::path::Path;

use bmo_testbed_core::events::{EVENT_HOOK_VARIABLE, Hook};
use bmo_testbed_core::turn::Env;

use crate::env_file::assigned;
use crate::words::BINARY;

/// The machine's testbed environment file, under the data root.
pub const MACHINE_ENV_FILE: &str = "env/testbed.env";

/// The hook the machine's events go to: the environment's `BMO_TESTBED_EVENT_HOOK`, else the one the machine's
/// testbed environment file names, else the running binary's `notice`. Where that cannot be named, none, and the
/// event is the line on standard error.
pub fn machine_hook(data_root: &Path, env: &Env) -> Hook {
  let mut hook = Hook::of(None, env);

  if hook.command.is_none() {
    hook.command = assigned(&data_root.join(MACHINE_ENV_FILE), EVENT_HOOK_VARIABLE).or_else(notice_hook);
  }

  hook
}

/// The hook that hands an event to the running binary's `notice` (docs/boundary.md, 「`notice`」): its path quoted
/// as one shell word, then the verb. None where the running program is not the testbed's binary, such as a test
/// harness calling the placement in its own process, or its path cannot be read or quoted.
pub fn notice_hook() -> Option<String> {
  let binary = std::env::current_exe().ok()?;

  if binary.file_name()? != BINARY {
    return None;
  }

  let quoted = shlex::try_quote(binary.to_str()?).ok()?;

  Some(format!("{quoted} notice"))
}
