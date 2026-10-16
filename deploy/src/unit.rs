//! Every deploy run runs in a transient user unit of its own (docs/deploy.md, 「Where a deployment runs」).
//! A deployment moves the deploy tree and restarts the repository's services, and a run that is its
//! caller's child dies with that caller — a shell's command timeout, a closed terminal — leaving it half
//! laid. Where the caller stands is never read: a run not already inside a deployment's unit starts one and
//! ends, whoever called it.
//!
//! This is the mechanism alone. What a verb's unit is called, what it is handed and which of its runs go
//! through here are the verb's own.

use std::path::PathBuf;

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::verb::REFUSED_EXIT_CODE;

use crate::run::{Ran, run_quiet};

/// The variable a deployment's unit is started with, naming the unit; every process in it inherits it.
pub const DEPLOY_UNIT_MARKER: &str = "BMO_DEPLOY_UNIT";

/// What every deployment's unit is called by, which is how a reader finds the ones at work.
pub const DEPLOY_UNIT_PREFIX: &str = "bmo-deploy-";

/// What the unit a deployment starts its backup upload in is called by.
pub const UPLOAD_UNIT_PREFIX: &str = "bmo-backup-upload-";

/// The unit this process stands in as a deployment, or none where it stands in none.
pub fn deploy_unit_of_process() -> Option<String> {
  std::env::var(DEPLOY_UNIT_MARKER).ok().filter(|unit| !unit.is_empty())
}

/// A fresh deployment unit's name: `bmo-deploy-<stamp>`, the stamp in milliseconds.
pub fn deploy_unit_name(now_ms: i64) -> String {
  format!("{DEPLOY_UNIT_PREFIX}{now_ms}")
}

/// A fresh backup upload unit's name: `bmo-backup-upload-<stamp>`, the stamp in milliseconds.
pub fn upload_unit_name(now_ms: i64) -> String {
  format!("{UPLOAD_UNIT_PREFIX}{now_ms}")
}

/// What a deployment's unit is started with.
#[derive(Debug, Clone)]
pub struct DeployUnitStart {
  pub unit: String,
  /// The tree the run starts in.
  pub cwd: PathBuf,
  /// The command the unit runs, the program first.
  pub command: Vec<String>,
  /// What the unit is handed beyond `PATH` and the marker: the roots the caller resolved.
  pub env: Vec<(String, String)>,
}

/// The whole systemd-run line. A transient unit is started by the user manager rather than by the caller,
/// so it inherits nothing of the caller's environment but what is named here.
pub fn deploy_unit_line(start: &DeployUnitStart, path: &str) -> Vec<String> {
  let mut line = vec![
    "systemd-run".to_owned(),
    "--user".to_owned(),
    "--quiet".to_owned(),
    "--collect".to_owned(),
    format!("--unit={}", start.unit),
    format!("--setenv={DEPLOY_UNIT_MARKER}={}", start.unit),
    format!("--setenv=PATH={path}"),
  ];

  line.extend(start.env.iter().map(|(name, value)| format!("--setenv={name}={value}")));
  line.push(format!("--working-directory={}", start.cwd.display()));
  line.extend(start.command.iter().cloned());

  line
}

/// How a caller is told where its run went.
pub fn deploy_unit_notice(unit: &str) -> String {
  format!(
    "running as {unit}, which outlives this call; read it with: journalctl --user -u {unit} --no-pager"
  )
}

/// Starts the unit with the `PATH` it is handed, reaching the user bus from whatever line the caller was
/// run on.
pub fn start_deploy_unit(start: &DeployUnitStart, path: &str) -> Ran {
  run_quiet(&deploy_unit_line(start, path), None)
}

/// The one ending of a run that was called outside a deployment's unit: the unit is started and the caller
/// told where to read it, exiting 0, or the start failed, nothing runs in the caller's place, and the run
/// is refused.
pub fn hand_off(start: &DeployUnitStart, path: &str, say: &dyn Fn(&str)) -> u8 {
  let started = start_deploy_unit(start, path);

  if !started.ok {
    say(&format!(
      "refused: its unit {} could not be started, and it does not run in this shell instead: {}",
      start.unit,
      first_chars(&started.out, DEPLOY.numbers.quoted_chars)
    ));

    return REFUSED_EXIT_CODE;
  }

  say(&deploy_unit_notice(&start.unit));

  0
}

/// The command a verb runs itself again with in its unit: this binary and the words it was given.
pub fn this_again() -> Vec<String> {
  let binary = std::env::current_exe()
    .map(|path| path.to_string_lossy().into_owned())
    .unwrap_or_else(|_| std::env::args().next().unwrap_or_default());

  std::iter::once(binary).chain(std::env::args().skip(1)).collect()
}

/// The first characters of a text, as a line quotes what a command said.
pub fn first_chars(text: &str, count: usize) -> String {
  text.chars().take(count).collect()
}
