//! The harness plugin and the HTTP contract as the binary serves them (docs/boundary.md, 「The harness
//! plugin」): the committed OpenAPI document is the one the code generates, `run --json` answers one command
//! run and keeps its test run in the record, and `mcp` lists exactly the eight tools and calls each kind
//! through: a reading tool through the service, a running tool through the binary's own verb. The MCP
//! server runs as its own process, spoken to over its pipes, and is taken down before the test ends.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use bmo_testbed_client::{CommandRun, TestRunKind, TestRunState};
use bmo_testbed_core::turn::Env;
use bmo_testbed_record::Record;
use bmo_testbed_service::{Ground, router};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;

use crate::stand::{ran, testbed};

const TOKEN: &str = "the-service-token";

/// How long a test lets the server take to answer one request before it fails; a guard, never a wait.
const PATIENCE: Duration = Duration::from_secs(60);

// "the committed OpenAPI document is the one the code generates"
#[test]
fn the_committed_openapi_document_is_the_one_the_code_generates() {
  let committed =
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/openapi.json"))
      .expect("docs/openapi.json stands");

  assert!(
    committed == bmo_testbed_cli::openapi_document(),
    "docs/openapi.json differs from what the code generates: \
     cargo run -q -p bmo-testbed-cli --example openapi > docs/openapi.json"
  );
}

// "run --json prints one command run, its lines on standard error, and keeps its test run in the record"
#[test]
fn run_json_prints_one_command_run_and_keeps_its_test_run() {
  let dir = tempfile::tempdir().unwrap();
  let done = ran(&mut testbed(
    dir.path(),
    dir.path(),
    &["run", "--json", "--", "sh", "-c", "echo ' 2 pass'"],
  ));
  let answer: CommandRun = serde_json::from_str(done.out.trim()).expect("one command run");
  let kept = Record::open(dir.path())
    .unwrap()
    .test_run(answer.test_run_id.expect("a test run is kept"))
    .unwrap();
  let kept = kept.expect("the record holds it");

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(done.err.contains("testbed:"), "{}", done.err);
  assert_eq!(answer.exit_code, 0);
  assert!(Path::new(&answer.log_path).is_file());
  assert_eq!(
    (kept.kind, kept.state, kept.passed_count),
    (TestRunKind::Command, TestRunState::Passed, 2)
  );
  assert_eq!(kept.log_path, answer.log_path);
}

/// The service over the test's data root, on a loopback port of its own.
struct Service {
  base: String,
  served: JoinHandle<()>,
}

impl Service {
  async fn start(data: &Path) -> Self {
    let ground = Arc::new(Ground {
      data_root: data.to_path_buf(),
      token: TOKEN.to_owned(),
      env: Env::from([("BMO_TESTBED_LOCK".to_owned(), data.join("testbed.lock").to_string_lossy().into())]),
      event_poll: Duration::from_millis(10),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let served = tokio::spawn(async move {
      let _ = axum::serve(listener, router(ground)).await;
    });

    Self { base, served }
  }
}

impl Drop for Service {
  fn drop(&mut self) {
    self.served.abort();
  }
}

/// The MCP server, spoken to line by line.
struct Mcp {
  child: Child,
  input: ChildStdin,
  output: Lines<BufReader<ChildStdout>>,
  next: u64,
}

impl Mcp {
  async fn start(dir: &Path, service: &str) -> Self {
    let mut command = tokio::process::Command::from(testbed(dir, dir, &["mcp", "--service", service]));

    command
      .env("BMO_TESTBED_TOKEN", TOKEN)
      .stdin(Stdio::piped())
      .stderr(Stdio::null())
      .kill_on_drop(true);

    let mut child = command.spawn().expect("the server starts");
    let input = child.stdin.take().expect("the server's input is piped");
    let output = BufReader::new(child.stdout.take().expect("the server's output is piped")).lines();
    let mut server = Self {
      child,
      input,
      output,
      next: 1,
    };
    let info = server
      .ask(
        "initialize",
        json!({
          "protocolVersion": "2025-11-25",
          "capabilities": {},
          "clientInfo": { "name": "tester", "version": "1.0.0" },
        }),
      )
      .await;

    assert_eq!(info["serverInfo"]["name"], "bmo-testbed");
    assert_eq!(info["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    server.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    server
  }

  async fn send(&mut self, message: Value) {
    let mut line = message.to_string();

    line.push('\n');
    self.input.write_all(line.as_bytes()).await.expect("the server reads its input");
    self.input.flush().await.expect("the server reads its input");
  }

  async fn ask(&mut self, method: &str, params: Value) -> Value {
    let id = self.next;

    self.next += 1;
    self
      .send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
      .await;

    loop {
      let line = tokio::time::timeout(PATIENCE, self.output.next_line())
        .await
        .expect("the server answers in time")
        .expect("the server's output reads")
        .expect("the server answers before it closes its output");
      let answer: Value = serde_json::from_str(&line).expect("the server writes JSON lines");

      if answer["id"] == id {
        assert!(answer["error"].is_null(), "{method} is answered without a protocol error: {answer}");
        return answer["result"].clone();
      }
    }
  }

  async fn call(&mut self, tool: &str, arguments: Value) -> Value {
    self.ask("tools/call", json!({ "name": tool, "arguments": arguments })).await
  }

  async fn stop(mut self) {
    drop(self.input);
    self.child.kill().await.ok();
    self.child.wait().await.expect("the server is taken down");
  }
}

// "mcp lists exactly the eight tools, a reading tool answers through the service and a running tool
// through the binary's own verb"
#[tokio::test(flavor = "multi_thread")]
async fn mcp_lists_exactly_the_eight_tools_and_calls_them_through() {
  let dir = tempfile::tempdir().unwrap();
  let service = Service::start(dir.path()).await;
  let mut mcp = Mcp::start(dir.path(), &service.base).await;
  let listed = mcp.ask("tools/list", json!({})).await;
  let names: Vec<&str> = listed["tools"]
    .as_array()
    .unwrap()
    .iter()
    .map(|tool| tool["name"].as_str().unwrap())
    .collect();

  assert_eq!(
    names,
    vec![
      "testbed_queues",
      "testbed_test_runs",
      "testbed_test_run",
      "testbed_deployments",
      "testbed_deployment",
      "testbed_run",
      "testbed_check",
      "testbed_suite",
    ]
  );

  let queues = mcp.call("testbed_queues", json!({})).await;

  assert_eq!(queues["structuredContent"]["items"][0]["name"], "wide", "{queues}");

  let checkout = dir.path().to_string_lossy().into_owned();
  let run = mcp
    .call(
      "testbed_run",
      json!({ "command": ["sh", "-c", "echo ' 3 pass'"], "checkout": checkout }),
    )
    .await;
  let answered: CommandRun =
    serde_json::from_value(run["structuredContent"].clone()).expect("a command run");
  let read = mcp.call("testbed_test_run", json!({ "testRunId": answered.test_run_id })).await;

  assert_eq!(answered.exit_code, 0, "{run}");
  assert_eq!(read["structuredContent"]["passedCount"], 3, "{read}");
  assert_eq!(read["structuredContent"]["state"], "passed", "{read}");

  let missing = mcp.call("testbed_test_run", json!({ "testRunId": 99_999 })).await;
  let error: Value = serde_json::from_str(missing["content"][0]["text"].as_str().unwrap()).unwrap();

  assert_eq!(missing["isError"], true);
  assert_eq!((error["status"].clone(), error["code"].clone()), (json!(404), json!("not-found")));

  let unread = mcp.call("testbed_run", json!({ "command": [], "checkout": checkout })).await;

  assert_eq!(unread["isError"], true);
  mcp.stop().await;
}
