//! The machine's own event hook (docs/boundary.md, 「Events」): the hook the caller's environment names, else the
//! one the machine's testbed environment file names, the file the timed prune's service reads as its environment
//! (docs/deploy.md, 「The Cargo build directory」). The placed machine's events are the machine's and no
//! repository's, so no declaration names their hook.

use std::path::Path;

use bmo_testbed_core::events::{EVENT_HOOK_VARIABLE, Hook};
use bmo_testbed_core::turn::Env;

/// The machine's testbed environment file, under the data root.
pub const MACHINE_ENV_FILE: &str = "env/testbed.env";

/// The hook the machine's events go to: the environment's `BMO_TESTBED_EVENT_HOOK`, else the one the machine's
/// testbed environment file names, else none, and the event is the line on standard error.
pub fn machine_hook(data_root: &Path, env: &Env) -> Hook {
  let mut hook = Hook::of(None, env);

  if hook.command.is_none() {
    hook.command = named_in(&data_root.join(MACHINE_ENV_FILE));
  }

  hook
}

// The hook an environment file names, read as the service manager reads the file for a unit: the last line
// assigning the variable, its value trimmed, and a value standing in quotes unquoted. Only the one variable is
// read, and its value goes nowhere but the hook's own command. The file is a list of assignments the service
// manager reads, for which no maintained reader of the service manager's own takes one variable out without
// taking the rest into this process's environment; a quoted value is split as a shell word by `shlex`.
fn named_in(file: &Path) -> Option<String> {
  let text = std::fs::read_to_string(file).ok()?;
  let value = text
    .lines()
    .filter_map(|line| line.trim_start().split_once('='))
    .filter(|(name, _)| name.trim_end() == EVENT_HOOK_VARIABLE)
    .map(|(_, value)| value.trim())
    .next_back()?;
  let unquoted = if value.starts_with(['"', '\'']) {
    match shlex::split(value)?.as_slice() {
      [one] => one.clone(),
      _ => return None,
    }
  } else {
    value.to_owned()
  };

  Some(unquoted).filter(|command| !command.is_empty())
}
