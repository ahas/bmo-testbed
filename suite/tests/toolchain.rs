//! A toolchain a Cargo command names given the component the command runs (docs/testbed.md, 「Languages」):
//! asked of rustup first, installed only where it is missing.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_suite::toolchain::provide_component;

// A rustup standing in for the machine's: it writes each call's words as a line of `calls` and answers
// `asked` to the asking and `installed` to the install.
fn rustup(dir: &Path, asked: u8, installed: u8) -> PathBuf {
  let program = dir.join("rustup");
  let calls = dir.join("calls");
  let script = format!(
    "#!/bin/sh\necho \"$* auto=${{RUSTUP_AUTO_INSTALL:-}}\" >> '{}'\ncase \"$1\" in\n  run) exit {asked} ;;\n  \
     toolchain) exit {installed} ;;\nesac\nexit 9\n",
    calls.display()
  );

  std::fs::write(&program, script).unwrap();
  std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

  // A process another test forks while the file is still open for writing holds it open until it execs, and the
  // program cannot be started meanwhile; it is started here until it can be, within a bound, and the line that
  // start wrote is taken away again.
  for _ in 0..400 {
    let started = std::process::Command::new(&program).stderr(std::process::Stdio::null()).status();

    if !started.is_err_and(|why| why.raw_os_error() == Some(libc::ETXTBSY)) {
      break;
    }

    std::thread::sleep(std::time::Duration::from_millis(5));
  }

  let _ = std::fs::remove_file(&calls);
  program
}

fn provided(asked: u8, installed: u8) -> (Vec<String>, Vec<String>) {
  let dir = tempfile::tempdir().unwrap();
  let program = rustup(dir.path(), asked, installed);
  let mut said = Vec::new();

  provide_component(&program, "nightly", "rustfmt", &[], &mut |line| said.push(line.to_owned()));

  let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap_or_default();

  (calls.lines().map(str::to_owned).collect(), said)
}

// "asked first whether the component's program runs on it, with rustup's own installing on demand held off"
#[test]
fn a_toolchain_that_holds_the_component_is_only_asked() {
  let (calls, said) = provided(0, 0);

  assert_eq!(calls, ["run nightly rustfmt --version auto=0"]);
  assert!(said.is_empty(), "{said:?}");
}

// "installed only where it does not, with the minimal profile and that component alone"
#[test]
fn a_toolchain_without_the_component_is_given_it_with_the_minimal_profile_and_says_so() {
  let marks = [("toolchain", "nightly"), ("component", "rustfmt")];
  let (calls, said) = provided(1, 0);

  assert!(
    calls[1]
      .starts_with("toolchain install nightly --profile minimal --component rustfmt --no-self-update "),
    "{calls:?}"
  );
  assert_eq!(said, [fill_marks(&SUITE.cargo.installing, &marks)]);

  let (_, said) = provided(1, 3);

  assert_eq!(said[1], fill_marks(&SUITE.cargo.not_installed, &[marks[0], marks[1], ("code", "3")]));
}

// "a machine with no rustup is left to Cargo as it stands"
#[test]
fn a_machine_with_no_rustup_installs_nothing_and_says_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let mut said = Vec::new();

  provide_component(&dir.path().join("no-rustup"), "nightly", "rustfmt", &[], &mut |line| {
    said.push(line.to_owned());
  });

  assert!(said.is_empty(), "{said:?}");
}
