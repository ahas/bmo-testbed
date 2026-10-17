//! A checkout of the testbed runs its own (docs/boundary.md, 「The binary」): `suite`, `check` and `template`
//! run by any other `bmo-testbed` inside a checkout of this repository build that checkout's own binary
//! through the Cargo verb, in the machine's turn like any build, and hand it the same words in the checkout's
//! root, so the code that checks a change is the code the change carries.

use std::os::unix::fs::MetadataExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::root_of;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_suite::cargo::run_cargo;
use bmo_testbed_suite::crates::checkout_target_dir;

use crate::words::BINARY;

/// The variable the own binary finds the directory its words were given in.
pub const CALLED_FROM_VARIABLE: &str = "BMO_TESTBED_CALLED_FROM";

// The package the binary is built from.
const CLI_PACKAGE: &str = "bmo-testbed-cli";

// The words that build a checkout's own binary, quietly, on the lockfile the checkout holds.
const BUILD_WORDS: [&str; 7] = ["build", "--locked", "-q", "-p", CLI_PACKAGE, "--bin", BINARY];

fn manifest(path: &Path) -> Option<toml::Table> {
  std::fs::read_to_string(path).ok()?.parse::<toml::Table>().ok()
}

/// Whether the tree at `root` is a checkout of the testbed: its Cargo workspace has a member whose package is
/// the one the binary is built from.
pub fn builds_testbed(root: &Path) -> bool {
  let Some(workspace) = manifest(&root.join("Cargo.toml")) else { return false };
  let members = workspace
    .get("workspace")
    .and_then(|workspace| workspace.get("members"))
    .and_then(toml::Value::as_array);

  members.into_iter().flatten().filter_map(toml::Value::as_str).any(|member| {
    manifest(&root.join(member).join("Cargo.toml"))
      .and_then(|package| package.get("package")?.get("name")?.as_str().map(str::to_owned))
      .is_some_and(|name| name == CLI_PACKAGE)
  })
}

/// The checkout's own binary, where its build leaves it.
pub fn own_binary(root: &Path) -> PathBuf {
  checkout_target_dir(root).join("debug").join(BINARY)
}

// Whether the running binary is the one at `own`, by the file rather than the path: Cargo leaves the build in
// the shared build directory and links that same file into the checkout's target directory.
fn running(own: &Path) -> bool {
  let Ok(current) = std::env::current_exe() else { return false };
  let file = |path: &Path| std::fs::metadata(path).ok().map(|meta| (meta.dev(), meta.ino()));

  file(&current).is_some_and(|current| file(own) == Some(current))
}

/// Hands the command over to the checkout's own binary where the verb runs inside a checkout of the testbed
/// and this binary is not that checkout's: builds it, then replaces this process with it, given the same
/// words in the checkout's root, with where they were given kept in `BMO_TESTBED_CALLED_FROM`. Returns where
/// the verb runs here; ends the process where the build failed, `unbuilt` saying so by the build's code.
pub fn run_own(unbuilt: impl Fn(u8) -> String) {
  let here = std::env::current_dir().unwrap_or_default();
  let Some(root) = root_of(&here) else { return };

  if !builds_testbed(&root) {
    return;
  }

  let own = own_binary(&root);

  if running(&own) {
    return;
  }

  let words: Vec<String> = BUILD_WORDS.iter().map(|word| (*word).to_owned()).collect();
  let built = run_cargo(&root, &words);

  if built != 0 {
    eprintln!("{}", unbuilt(built));
    std::process::exit(1);
  }

  let why = std::process::Command::new(&own)
    .args(std::env::args_os().skip(1))
    .current_dir(&root)
    .env(CALLED_FROM_VARIABLE, &here)
    .exec();

  eprintln!(
    "{}",
    fill_marks(&VERBS.own.unstarted, &[("binary", &own.to_string_lossy()), ("why", &why.to_string())])
  );
  std::process::exit(1);
}

/// The directory a verb's words were given in: the one a hand-over names, or where it runs.
pub fn called_from() -> PathBuf {
  std::env::var_os(CALLED_FROM_VARIABLE)
    .filter(|named| !named.is_empty())
    .map_or_else(|| std::env::current_dir().unwrap_or_default(), PathBuf::from)
}
