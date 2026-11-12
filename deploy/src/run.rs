//! What every part of a deployment stands on beside its own work: a command run to its end on the user's
//! own service manager, and a moment written for a name.

use std::path::Path;
use std::process::{Command, Stdio};

/// What a command run to its end came to: whether it ended well; the code it exited with, none where it did not
/// start or a signal ended it; and its output, standard error after standard output, trimmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
  pub ok: bool,
  pub code: Option<i32>,
  pub out: String,
}

/// A command on the user's own service manager, whatever line this process was started on: its runtime
/// directory named where it is missing, and a disabled session bus address taken off.
pub fn command(words: &[String], cwd: Option<&Path>) -> Command {
  let mut command = Command::new(&words[0]);

  command.args(&words[1..]).stdin(Stdio::null());

  if std::env::var_os("XDG_RUNTIME_DIR").is_none_or(|dir| dir.is_empty()) {
    // SAFETY: getuid has no preconditions and cannot fail.
    command.env("XDG_RUNTIME_DIR", format!("/run/user/{}", unsafe { libc::getuid() }));
  }

  if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|address| address == "disabled:") {
    command.env_remove("DBUS_SESSION_BUS_ADDRESS");
  }

  if let Some(cwd) = cwd {
    command.current_dir(cwd);
  }

  command
}

/// A command run to its end with its output taken.
pub fn run_quiet(words: &[String], cwd: Option<&Path>) -> Ran {
  match command(words, cwd).output() {
    Ok(done) => Ran {
      ok: done.status.success(),
      code: done.status.code(),
      out: format!("{}{}", String::from_utf8_lossy(&done.stdout), String::from_utf8_lossy(&done.stderr))
        .trim()
        .to_owned(),
    },
    Err(why) => Ran {
      ok: false,
      code: None,
      out: why.to_string(),
    },
  }
}

/// A command run to its end with its output shown as it comes, answering whether it ended well.
pub fn run_loud(words: &[String], cwd: Option<&Path>) -> bool {
  command(words, cwd)
    .stdout(Stdio::inherit())
    .stderr(Stdio::inherit())
    .status()
    .is_ok_and(|status| status.success())
}

/// Words of a command line, from what a call writes as literals.
pub fn words<const N: usize>(words: [&str; N]) -> Vec<String> {
  words.iter().map(|word| (*word).to_owned()).collect()
}

/// A declared command line, run by the shell as the declaration writes it.
pub fn shell_words(line: &str) -> Vec<String> {
  words(["sh", "-c", line])
}

/// A moment as the files and the names of the machine write it: `2026-10-11T07-24-00-123Z`.
pub fn stamp_for_name(at_ms: i64) -> String {
  chrono::DateTime::from_timestamp_millis(at_ms)
    .unwrap_or_default()
    .format("%Y-%m-%dT%H-%M-%S-%3fZ")
    .to_string()
}
