//! What the binary's tests stand on: the binary run in a directory of the test's own, its lanes, logs and
//! close line there, standing for no harness, inside no turn and placed nowhere unless the test names an
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
    .env_remove("BMO_TESTBED_SCOPE")
    .env_remove("BMO_TESTBED_SERVICE")
    .env_remove("BMO_TESTBED_TOKEN")
    .env_remove("BMO_TESTBED_HELD")
    .env_remove("BMO_TESTBED_FRONT")
    .env_remove("BMO_TESTBED_NEXT")
    .env_remove("BMO_TESTBED_PLACEMENT")
    .env_remove("BMO_TESTBED_PLACED")
    // The folder a testbed running the tests was called from is that run's, and a binary under test stands
    // in the folder its test gives it.
    .env_remove("BMO_TESTBED_CALLED_FROM")
    .env_remove("BMO_TESTBED_CHECKOUTS")
    .env_remove("BMO_DEPLOY_UNIT")
    // An event the binary under test sends reaches no hook and no receiver but one its test names.
    .env_remove("BMO_TESTBED_EVENT_HOOK")
    .env_remove("RECEIVER_ADDRESS")
    .env_remove("RECEIVER_TOKEN")
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

/// One instruction a stand-in receiver was handed: the authorization it carried and its body.
#[derive(Clone, Debug)]
pub struct Noticed {
  pub authorization: String,
  pub body: serde_json::Value,
}

/// A stand-in receiver: `GET /owner` answers `owner`, and `POST /instructions` answers `status` with the row it would
/// have queued. What it was handed, and the address it serves on, which ends with the runtime.
pub struct ReceiverStand {
  _runtime: tokio::runtime::Runtime,
  pub address: String,
  noticed: std::sync::Arc<std::sync::Mutex<Vec<Noticed>>>,
}

impl ReceiverStand {
  /// The owner's private room the stand names.
  pub const OWNER_ROOM: i64 = 7;

  pub fn answering(status: u16) -> Self {
    Self::serving(status, serde_json::json!({ "senderId": 1, "room": Self::OWNER_ROOM }))
  }

  /// A stand whose answer for the owner names no private room.
  pub fn roomless() -> Self {
    Self::serving(201, serde_json::json!({ "senderId": 1, "room": null }))
  }

  fn serving(status: u16, owner: serde_json::Value) -> Self {
    use axum::http::{HeaderMap, StatusCode};

    let runtime = tokio::runtime::Builder::new_multi_thread()
      .worker_threads(1)
      .enable_all()
      .build()
      .unwrap();
    let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let noticed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let keeping = std::sync::Arc::clone(&noticed);
    let status = StatusCode::from_u16(status).unwrap();
    let app = axum::Router::new()
      .route(
        "/owner",
        axum::routing::get(move || {
          let owner = owner.clone();

          async move { axum::Json(owner) }
        }),
      )
      .route(
        "/instructions",
        axum::routing::post(move |headers: HeaderMap, axum::Json(body): axum::Json<serde_json::Value>| {
          let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
          let mut kept = keeping.lock().unwrap();

          kept.push(Noticed {
            authorization,
            body,
          });

          let id = kept.len();

          async move { (status, axum::Json(serde_json::json!({ "id": id }))) }
        }),
      );

    runtime.spawn(async move { axum::serve(listener, app).await });
    Self {
      _runtime: runtime,
      address,
      noticed,
    }
  }

  /// The instructions handed so far, in order.
  pub fn noticed(&self) -> Vec<Noticed> {
    self.noticed.lock().unwrap().clone()
  }

  /// The receiver's own env file written under a data root, naming this stand and the token.
  pub fn written_under(&self, data_root: &Path, token: &str) {
    let file = data_root.join("env/receiver.env");

    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, format!("RECEIVER_ADDRESS={}\nRECEIVER_TOKEN={token}\n", self.address)).unwrap();
  }
}
