//! What the tool's tests stand on: checkouts declaring a deploy root, a registry naming them, stand-ins on
//! the path for the testbed binary and the service manager, and the executable handed one request.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::turn::Env;
use serde_json::{Value, json};

/// A commit's whole name, as a deployment record holds it.
pub const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// The line the deploy verb prints as it hands its run to a unit of its own.
pub const HANDED_OFF: &str = "running as bmo-deploy-1793328193818, which outlives this call; read it with: journalctl --user -u bmo-deploy-1793328193818 --no-pager";

pub fn write(path: &Path, text: &str) {
  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
}

/// A script on the stand-ins' path, made executable.
pub fn script(bin: &Path, name: &str, body: &str) -> PathBuf {
  let path = bin.join(name);

  write(&path, &format!("#!/bin/sh\n{body}\n"));
  std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
  path
}

/// A checkout under `dir` declaring the name given, its deploy root `<dir>/<name>-deploy`.
pub fn checkout(dir: &Path, folder: &str, name: &str) -> PathBuf {
  let root = dir.join(folder);
  let deploy = dir.join(format!("{folder}-deploy"));

  write(
    &root.join("testbed.toml"),
    &format!(
      "testbed = 0\nname = \"{name}\"\n\n[deploy]\nroot = \"{}\"\ndata_root = \"{}\"\n",
      deploy.display(),
      dir.join(format!("{folder}-data")).display()
    ),
  );
  root
}

/// The deployment record a checkout made by `checkout` is read from.
pub fn record_path(dir: &Path, folder: &str) -> PathBuf {
  dir.join(format!("{folder}-deploy")).join("deployed.json")
}

/// A deployment record of the commit given.
pub fn record(commit: &str, subject: &str, message: &str, at: &str) -> String {
  json!({
    "commit": commit,
    "subject": subject,
    "message": message,
    "version": "",
    "at": at,
    "backup": "",
  })
  .to_string()
}

/// A registry under `dir` naming the checkouts given.
pub fn registry(dir: &Path, checkouts: &[&Path]) -> PathBuf {
  let at = dir.join("checkouts.toml");
  let listed = checkouts
    .iter()
    .map(|path| format!("\"{}\"", path.display()))
    .collect::<Vec<_>>()
    .join(", ");

  write(&at, &format!("checkouts = [{listed}]\n"));
  at
}

/// The environment a call is made in: the stand-ins first on the path, the registry named, and a harness's
/// process named, which the verb must never see.
pub fn env(dir: &Path, bin: &Path, registry: &Path) -> Env {
  HashMap::from([
    ("PATH".to_owned(), format!("{}:/usr/bin:/bin", bin.display())),
    ("HOME".to_owned(), dir.to_string_lossy().into_owned()),
    ("BMO_DATA_DIR".to_owned(), dir.join("data").to_string_lossy().into_owned()),
    ("BMO_TESTBED_CHECKOUTS".to_owned(), registry.to_string_lossy().into_owned()),
    ("CLAUDE_PID".to_owned(), "4242".to_owned()),
    ("BMO_DEPLOY_UNIT".to_owned(), "bmo-deploy-1".to_owned()),
    ("STAND_LOG".to_owned(), dir.join("log").to_string_lossy().into_owned()),
  ])
}

/// The testbed binary's stand-in: it writes its words, where it ran and the harness variable it saw under
/// `STAND_LOG`, then runs `body`.
pub fn testbed(bin: &Path, body: &str) -> PathBuf {
  script(
    bin,
    "bmo-testbed",
    &format!(
      "mkdir -p \"$STAND_LOG\"\nprintf '%s\\n' \"$@\" > \"$STAND_LOG/words\"\npwd > \"$STAND_LOG/cwd\"\nprintf '%s' \"${{CLAUDE_PID-none}} ${{BMO_DEPLOY_UNIT-none}}\" > \"$STAND_LOG/harness\"\n{body}"
    ),
  )
}

/// What the stand-in wrote under `STAND_LOG`.
pub fn logged(dir: &Path, name: &str) -> String {
  std::fs::read_to_string(dir.join("log").join(name)).unwrap_or_default()
}

/// A call of the entry, as the tool host hands it.
pub fn call(arguments: Value) -> Value {
  json!({
    "protocol": 1,
    "kind": "call",
    "entry": "testbed/deploy",
    "arguments": arguments,
    "session": { "holderName": "chat", "capabilities": ["deploy"], "variables": {} },
    "callback": "/run/calls/1.sock",
  })
}

/// How a run of the executable ended.
pub struct Ran {
  pub code: i32,
  pub out: String,
  pub err: String,
}

impl Ran {
  /// Its answer, read as the tool host reads it.
  pub fn answer(&self) -> Value {
    serde_json::from_str(&self.out).unwrap_or_else(|why| panic!("{why}: {} {}", self.out, self.err))
  }
}

/// The executable handed `request` on standard input in the environment given and nothing else.
pub fn tool(request: &str, env: &Env) -> Ran {
  let mut started = Command::new(env!("CARGO_BIN_EXE_deploy-tool"))
    .env_clear()
    .envs(env)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("the tool starts");

  started.stdin.take().unwrap().write_all(request.as_bytes()).unwrap();

  let done = started.wait_with_output().expect("the tool ends");

  Ran {
    code: done.status.code().unwrap_or(-1),
    out: String::from_utf8_lossy(&done.stdout).into_owned(),
    err: String::from_utf8_lossy(&done.stderr).into_owned(),
  }
}

/// The text of an answer's pieces, joined.
pub fn text(answer: &Value) -> String {
  answer["content"]
    .as_array()
    .unwrap()
    .iter()
    .map(|piece| piece["text"].as_str().unwrap().to_owned())
    .collect::<Vec<_>>()
    .join("\n")
}
