//! The toolchains a checkout is built and tested with (docs/boundary.md, 「`toolchain`」): each language that
//! names a command saying its toolchain answers through that command, run in the checkout so a toolchain file
//! it carries is read. A deployment's gate reads them as part of the ground a pass stands on.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::languages::LANGUAGES;

use crate::crates::{on_path, toolchain_environment};

/// One language's toolchain: its name, and what its command printed, none where the command did not start or
/// did not end 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
  pub language: &'static str,
  pub said: Option<String>,
}

/// The toolchains of the checkout at `root`, one for every language naming a command, in the order of their
/// names.
pub fn toolchains(root: &Path) -> Vec<Toolchain> {
  let homes = toolchain_environment();

  LANGUAGES
    .iter()
    .filter_map(|language| {
      let command = language.toolchain_command()?;

      Some(Toolchain {
        language: language.name(),
        said: said_by(command, root, &homes),
      })
    })
    .collect()
}

/// The toolchains as one text, a line naming each language and what it said, `none` where it said nothing: the
/// verb's thin answer, and the ground a gate's pass is matched on.
pub fn toolchain_text(toolchains: &[Toolchain]) -> String {
  toolchains
    .iter()
    .map(|one| format!("{}: {}\n", one.language, one.said.as_deref().unwrap_or("none")))
    .collect()
}

// What a toolchain command printed in the checkout, the program the Cargo home holds taken first, as the Cargo
// verb takes its `cargo` from there.
fn said_by(command: &[&str], root: &Path, homes: &[(String, String)]) -> Option<String> {
  let (program, args) = command.split_first()?;
  let done = Command::new(program_of(program, homes))
    .args(args)
    .current_dir(root)
    .envs(homes.iter().map(|(name, value)| (name, value)))
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output()
    .ok()?;

  done.status.success().then(|| String::from_utf8_lossy(&done.stdout).trim().to_owned())
}

fn program_of(program: &str, homes: &[(String, String)]) -> PathBuf {
  homes
    .iter()
    .find(|(name, _)| name == "CARGO_HOME")
    .map(|(_, home)| Path::new(home).join("bin").join(program))
    .filter(|installed| installed.is_file())
    .or_else(|| on_path(program))
    .unwrap_or_else(|| PathBuf::from(program))
}
