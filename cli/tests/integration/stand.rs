//! What the binary's tests stand on: the binary run in a directory of the test's own, its lanes, logs and
//! close line there, standing for no job, inside no turn and placed nowhere unless the test names an
//! endpoint; and a git checkout made for a test.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How a run of the binary ended.
pub struct Ran {
  pub code: i32,
  pub out: String,
  pub err: String,
}

/// The binary as a test starts it: in `dir`, on the test's own lock and data root under `data`.
pub fn testbed(dir: &Path, data: &Path, args: &[&str]) -> Command {
  let mut command = Command::new(env!("CARGO_BIN_EXE_bmo-testbed"));

  command
    .args(args)
    .current_dir(dir)
    .env("BMO_TESTBED_LOCK", data.join("testbed.lock"))
    .env("BMO_DATA_DIR", data)
    // A runtime directory holding no socket of the service manager's, as a test home's is.
    .env("XDG_RUNTIME_DIR", data)
    .env_remove("DBUS_SESSION_BUS_ADDRESS")
    .env_remove("CLAUDE_PID")
    .env_remove("BMO_AGENT_PLACE")
    .env_remove("BMO_TESTBED_HELD")
    .env_remove("BMO_TESTBED_FRONT")
    .env_remove("BMO_TESTBED_NEXT")
    .env_remove("BMO_TESTBED_PLACEMENT")
    .env_remove("BMO_DEPLOY_UNIT")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
  command
}

/// A started run waited out.
pub fn finished(run: Child) -> Ran {
  let done = run.wait_with_output().expect("the binary ends");

  Ran {
    code: done.status.code().unwrap_or(-1),
    out: String::from_utf8_lossy(&done.stdout).into_owned(),
    err: String::from_utf8_lossy(&done.stderr).into_owned(),
  }
}

/// The binary run to its end.
pub fn ran(command: &mut Command) -> Ran {
  finished(command.spawn().expect("the binary starts"))
}

/// A git command in a directory, which must succeed; what it printed.
pub fn git(dir: &Path, args: &[&str]) -> String {
  let done = Command::new("git")
    .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
    .args(args)
    .current_dir(dir)
    .stdin(Stdio::null())
    .output()
    .expect("git runs");

  assert!(done.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&done.stderr));
  String::from_utf8_lossy(&done.stdout).trim().to_owned()
}

/// A checkout with one commit, holding the declaration given.
pub fn checkout(dir: &Path, declaration: &str) {
  std::fs::create_dir_all(dir).unwrap();
  git(dir, &["init", "-q", "-b", "main"]);
  std::fs::write(dir.join("testbed.toml"), declaration).unwrap();
  git(dir, &["add", "."]);
  git(dir, &["commit", "-q", "-m", "start"]);
}

/// An executable put in place by a process of its own: a test of this process forking while the file was
/// written here would hold it open for writing, and running it would then fail as a busy text file.
pub fn install(script: &str, at: &Path) {
  let source = at.with_extension("source");

  std::fs::write(&source, script).unwrap();
  assert!(
    Command::new("install")
      .arg("-m")
      .arg("755")
      .arg(&source)
      .arg(at)
      .status()
      .unwrap()
      .success()
  );
}

/// A bounded wait on a condition.
pub fn until(what: &str, condition: impl Fn() -> bool, within: Duration) {
  let began = Instant::now();

  while !condition() {
    assert!(began.elapsed() < within, "never came: {what}");
    std::thread::sleep(Duration::from_millis(20));
  }
}
