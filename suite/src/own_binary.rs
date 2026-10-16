//! The running `bmo-testbed` as what the code starts: a verb the code names as a process of its own is a verb
//! of the binary running that code, so the code naming a verb is always a binary that answers it
//! (docs/boundary.md, 「The binary」).

use std::path::PathBuf;

// The binary's name, as a person reads a line it starts.
const BINARY: &str = "bmo-testbed";

/// The running `bmo-testbed`, or the one on the path where it cannot be told. A binary a deployment replaced
/// while it ran has no path left, and is still started through the process's own link to it.
pub fn running_testbed() -> PathBuf {
  match std::env::current_exe() {
    Ok(path) if path.exists() => path,
    Ok(_) => PathBuf::from("/proc/self/exe"),
    Err(_) => PathBuf::from(BINARY),
  }
}

/// A line as a person reads it, the running `bmo-testbed` written by its name rather than its path.
pub fn shown(line: &[String]) -> String {
  let running = running_testbed().to_string_lossy().into_owned();

  line
    .iter()
    .enumerate()
    .map(|(at, word)| if at == 0 && *word == running { BINARY } else { word.as_str() })
    .collect::<Vec<_>>()
    .join(" ")
}

/// The words running `words` as a verb of the running `bmo-testbed`.
pub fn testbed_line(words: &[impl AsRef<str>]) -> Vec<String> {
  std::iter::once(running_testbed().to_string_lossy().into_owned())
    .chain(words.iter().map(|word| word.as_ref().to_owned()))
    .collect()
}
