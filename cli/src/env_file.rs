//! One variable read out of an environment file of the machine's, the way the service manager reads the file for a
//! unit (docs/boundary.md, 「Events」, 「`notice`」): the machine's testbed environment file for its hook, and the
//! receiver's own for where `notice` finds it.

use std::path::Path;

/// The value the file assigns the variable: the last line assigning it, its value trimmed, and a value standing in
/// quotes unquoted; none where the file does not read, assigns it nothing, or its value is empty. Only the one
/// variable is read, and its value goes nowhere but where the caller hands it. The file is a list of assignments the
/// service manager reads, for which no maintained reader of the service manager's own takes one variable out
/// without taking the rest into this process's environment; a quoted value is split as a shell word by `shlex`.
pub fn assigned(file: &Path, variable: &str) -> Option<String> {
  let text = std::fs::read_to_string(file).ok()?;
  let value = text
    .lines()
    .filter_map(|line| line.trim_start().split_once('='))
    .filter(|(name, _)| name.trim_end() == variable)
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

  Some(unquoted).filter(|value| !value.is_empty())
}
