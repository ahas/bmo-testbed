//! The toolchains a checkout is built and tested with (docs/boundary.md, 「`toolchain`」): each language that
//! names a command saying its toolchain answers through that command, run in the checkout so a toolchain file
//! it carries is read. A deployment's gate reads them as part of the ground a pass stands on. A toolchain a
//! Cargo command names is first given the component the command runs where this machine lacks it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::languages::{LANGUAGES, cargo_named_component};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;

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

/// Gives the toolchain a Cargo command `args` names with its leading `+` the component the command runs, where
/// this machine's rustup holds the toolchain without it or does not hold the toolchain (docs/testbed.md,
/// 「Languages」). A command naming no toolchain, or a machine with no rustup, is left to Cargo as it stands.
pub fn provide_named_component(args: &[String], say: &mut dyn FnMut(&str)) {
  let Some((toolchain, component)) = cargo_named_component(args) else { return };
  let homes = toolchain_environment();

  provide_component(&program_of("rustup", &homes), toolchain, component, &homes, say);
}

/// Gives `toolchain` its `component` through the rustup at `rustup`: asked first whether the component's
/// program of that name runs on it, with rustup's own installing on demand held off so the asking changes
/// nothing, and installed only where it does not, with the minimal profile and that component alone. Says a
/// line before installing and one where the install ended otherwise than 0; Cargo then says why it cannot run.
pub fn provide_component(
  rustup: &Path,
  toolchain: &str,
  component: &str,
  homes: &[(String, String)],
  say: &mut dyn FnMut(&str),
) {
  let rustup_command = |args: &[&str]| {
    let mut command = Command::new(rustup);

    command
      .args(args)
      .envs(homes.iter().map(|(name, value)| (name, value)))
      .stdin(Stdio::null());
    command
  };
  let asked = rustup_command(&["run", toolchain, component, "--version"])
    .env("RUSTUP_AUTO_INSTALL", "0")
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status();

  // A rustup that does not start installs nothing either; one that answers 0 holds the component.
  if asked.is_err() || asked.is_ok_and(|status| status.success()) {
    return;
  }

  let marks = [("toolchain", toolchain), ("component", component)];

  say(&fill_marks(&SUITE.cargo.installing, &marks));

  let installed = rustup_command(&[
    "toolchain",
    "install",
    toolchain,
    "--profile",
    "minimal",
    "--component",
    component,
    "--no-self-update",
  ])
  .stdout(Stdio::inherit())
  .status();
  let code = match installed {
    Ok(status) if status.success() => return,
    Ok(status) => status.code().map_or_else(|| SUITE.cargo.no_code.clone(), |code| code.to_string()),
    Err(why) => why.to_string(),
  };

  say(&fill_marks(&SUITE.cargo.not_installed, &[marks[0], marks[1], ("code", code.as_str())]));
}
