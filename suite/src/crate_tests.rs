//! A crate's tests run from the one build of every crate a run reaches (docs/suite.md, 「Heavy units run in a lane
//! of one」): while the build runs the tests, Cargo's runner writes each test binary down as Cargo would run it,
//! and a crate's unit runs the binaries written down for its package (docs/boundary.md, 「crate-tests」).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_core::policy::suite::SUITE;
use serde::{Deserialize, Serialize};

use crate::crates::set_by_cargo;

/// The library path Cargo hands a binary it runs, beside the variables it sets for the binary's package.
const LIBRARY_PATH: &str = "LD_LIBRARY_PATH";

/// The variable naming the package a binary belongs to, as Cargo sets it.
const PACKAGE_NAME: &str = "CARGO_PKG_NAME";

/// The `--config` key naming the runner for whatever target the build is for.
const RUNNER_KEY: &str = "target.'cfg(all())'.runner";

/// One test binary as Cargo would run it: the executable and the words Cargo handed it, the folder it runs from,
/// and the variables Cargo sets for it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestBinary {
  pub executable: String,
  pub args: Vec<String>,
  pub dir: String,
  pub env: BTreeMap<String, String>,
}

impl TestBinary {
  /// What a runner Cargo started was handed: of the whole environment, only the variables Cargo sets.
  pub fn seen(
    executable: String,
    args: Vec<String>,
    dir: String,
    env: impl IntoIterator<Item = (String, String)>,
  ) -> Self {
    Self {
      executable,
      args,
      dir,
      env: env.into_iter().filter(|(name, _)| set_by_cargo(name) || name == LIBRARY_PATH).collect(),
    }
  }

  /// The package it belongs to, where Cargo named one.
  pub fn package(&self) -> Option<&str> {
    self.env.get(PACKAGE_NAME).map(String::as_str)
  }
}

/// The record of one run's test binaries, in the temporary directory under the run's process, begun empty and
/// taken away with the run.
pub struct RunRecord(PathBuf);

impl RunRecord {
  pub fn fresh() -> Self {
    let path = std::env::temp_dir().join(format!("bmo-testbed-crate-tests-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);

    Self(path)
  }

  pub fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for RunRecord {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
  }
}

/// The `--config` words making the testbed's `crate-tests record` Cargo's runner, writing into `record`.
pub fn runner_config(testbed: &Path, record: &Path) -> String {
  let runner: toml_edit::Array = [
    testbed.to_string_lossy().into_owned(),
    "crate-tests".to_owned(),
    "record".to_owned(),
    record.to_string_lossy().into_owned(),
  ]
  .into_iter()
  .collect();

  format!("{RUNNER_KEY}={runner}")
}

/// One binary added to the record, as one line of JSON.
pub fn write_down(record: &Path, binary: &TestBinary) -> std::io::Result<()> {
  let mut line = serde_json::to_string(binary).map_err(std::io::Error::other)?;

  line.push('\n');
  std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(record)?
    .write_all(line.as_bytes())
}

/// Every binary the record holds, in the order they were written down.
pub fn read_record(record: &Path) -> Result<Vec<TestBinary>, String> {
  let text = std::fs::read_to_string(record).map_err(|why| why.to_string())?;

  text
    .lines()
    .filter(|line| !line.trim().is_empty())
    .map(|line| serde_json::from_str(line).map_err(|why| why.to_string()))
    .collect()
}

/// `crate-tests record`: the binary Cargo handed this runner written down, and nothing run.
pub fn record(record: &Path, executable: String, args: Vec<String>) -> u8 {
  let dir = std::env::current_dir().unwrap_or_default().to_string_lossy().into_owned();
  let binary = TestBinary::seen(executable, args, dir, std::env::vars());

  match write_down(record, &binary) {
    Ok(()) => 0,
    Err(why) => {
      eprintln!(
        "{}",
        fill_shown(&SUITE.crate_tests.unwritten, &[("record", &record.display()), ("why", &why)])
      );
      1
    }
  }
}

// A binary run to its end from its folder with its variables over this process's own, its output passed through;
// the code it ended with, a signal that ended it read as the shell reads it.
fn run_one(binary: &TestBinary) -> i32 {
  eprintln!("{}", fill_shown(&SUITE.crate_tests.running, &[("executable", &binary.executable)]));

  let ran = Command::new(&binary.executable)
    .args(&binary.args)
    .current_dir(&binary.dir)
    .envs(&binary.env)
    .stdin(Stdio::null())
    .status();

  match ran {
    Ok(status) => status
      .code()
      .unwrap_or_else(|| 128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0)),
    Err(why) => {
      eprintln!(
        "{}",
        fill_shown(&SUITE.crate_tests.unstarted, &[("executable", &binary.executable), ("why", &why)])
      );
      1
    }
  }
}

/// `crate-tests run`: every binary the record holds for `package`, each to its end; the first failing one's code.
pub fn run(record: &Path, package: &str) -> u8 {
  let binaries = match read_record(record) {
    Ok(binaries) => binaries,
    Err(why) => {
      eprintln!(
        "{}",
        fill_shown(&SUITE.crate_tests.unread, &[("record", &record.display()), ("why", &why)])
      );
      return 1;
    }
  };
  let own: Vec<&TestBinary> = binaries.iter().filter(|binary| binary.package() == Some(package)).collect();

  if own.is_empty() {
    println!("{}", fill_shown(&SUITE.crate_tests.none, &[("package", &package)]));
    return 0;
  }

  let first_failure = own
    .into_iter()
    .map(run_one)
    .fold(0, |first, code| if first == 0 { code } else { first });

  u8::try_from(first_failure).unwrap_or(1)
}
