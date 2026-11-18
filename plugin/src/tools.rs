//! The harness tools (docs/boundary.md, 「The harness plugin」): five reading tools, each a thin call of one
//! route through the client, and three running tools, each a thin call of one of the binary's verbs with
//! `--json` in the checkout named. The MCP server lists them and the OpenAPI document states them under
//! `x-tools`, both from this one table.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use bmo_testbed_client::{ClientFailure, CommandRun, LaneName, PageQuery, TestRunsQuery, TestbedClient};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::plugin::PLUGIN;
use schemars::generate::SchemaGenerator;
use schemars::{JsonSchema, Schema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;

/// The most of a verb's standard error a failure carries back, from its end.
const SAID_TAIL_CHARS: usize = 2000;

/// What a tool's call came to short of its answer.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolFailure {
  /// The arguments are not the shape the tool takes.
  Arguments(String),
  /// The route's call reached no answer of its shape.
  Client(ClientFailure),
  /// The verb could not be started, or answered nothing that reads.
  Verb(String),
}

impl ToolFailure {
  /// The failure as the tool error's content: the error shape the service answered with and its status, or
  /// the sentence saying why nothing answered.
  pub fn content(&self) -> Value {
    match self {
      Self::Arguments(why) => json!({ "code": "bad-request", "error": why }),
      Self::Client(ClientFailure::Answered {
        status,
        answer: Some(answer),
        ..
      }) => json!({ "status": status, "code": answer.code, "error": answer.error }),
      Self::Client(failure @ ClientFailure::Answered { status, .. }) => {
        json!({ "status": status, "error": failure.to_string() })
      }
      Self::Client(failure) => json!({ "error": failure.to_string() }),
      Self::Verb(why) => json!({ "error": why }),
    }
  }
}

/// What the tools call through: the client of the service, and the binary whose verbs run.
pub struct Hands {
  pub client: TestbedClient,
  pub binary: PathBuf,
}

/// A tool's answer: the route's or the verb's answer as JSON, or why there is none.
pub type Answered = Pin<Box<dyn Future<Output = Result<Value, ToolFailure>> + Send>>;

type Call = Box<dyn Fn(Arc<Hands>, Value) -> Answered + Send + Sync>;

/// What a tool is a thin call of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Calls {
  /// One route, by its method and path.
  Route(&'static str),
  /// One of the binary's verbs, by the words it opens with.
  Command(&'static str),
}

/// One tool: its name, what it does, what it calls, its input and output, and the call itself.
pub struct TestbedTool {
  pub name: &'static str,
  pub description: &'static str,
  pub calls: Calls,
  input: fn(&mut SchemaGenerator) -> Schema,
  output: fn(&mut SchemaGenerator) -> Schema,
  call: Call,
}

impl TestbedTool {
  /// The tool's input, its nested types referred to where `generator` keeps its definitions.
  pub fn input(&self, generator: &mut SchemaGenerator) -> Schema {
    (self.input)(generator)
  }

  /// The tool's output, referred to where `generator` keeps its definitions.
  pub fn output(&self, generator: &mut SchemaGenerator) -> Schema {
    (self.output)(generator)
  }

  /// Calls the tool through the hands, with the tool's arguments.
  pub fn call(&self, hands: Arc<Hands>, arguments: Value) -> Answered {
    (self.call)(hands, arguments)
  }
}

/// A tool whose input is `I` and whose answer is `O`, calling `call` with the arguments read as `I`.
fn tool<I, O, F, Fut>(name: &'static str, description: &'static str, calls: Calls, call: F) -> TestbedTool
where
  I: DeserializeOwned + JsonSchema,
  O: Serialize + JsonSchema,
  F: Fn(Arc<Hands>, I) -> Fut + Send + Sync + 'static,
  Fut: Future<Output = Result<O, ToolFailure>> + Send + 'static,
{
  TestbedTool {
    name,
    description,
    calls,
    input: I::json_schema,
    output: SchemaGenerator::subschema_for::<O>,
    call: Box::new(move |hands, arguments| {
      let input = match serde_json::from_value::<I>(arguments) {
        Ok(input) => input,
        Err(error) => {
          let why = fill_marks(&PLUGIN.lines.bad_arguments, &[("why", &error.to_string())]);
          return Box::pin(std::future::ready(Err(ToolFailure::Arguments(why))));
        }
      };
      let answer = call(hands, input);

      Box::pin(async move {
        let answer = answer.await?;
        Ok(serde_json::to_value(answer).unwrap_or(Value::Null))
      })
    }),
  }
}

/// One test run, by its id.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestRunKey {
  test_run_id: i64,
}

/// One deployment, by its id.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeploymentKey {
  deployment_id: i64,
}

/// A heavy command, the checkout it runs in, and the lane, the wide one where none is named.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunInput {
  /// The command's words.
  command: Vec<String>,
  /// The checkout the command runs in.
  checkout: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  queue: Option<LaneName>,
}

/// The checkout whose change is checked, and the commit the change is read from; or the checkouts of a change
/// check over several checkouts, each with its own since, read from the checkout the verb runs in.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CheckInput {
  checkout: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  since: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  checkouts: Option<Vec<SetCheckout>>,
}

/// One checkout of a change check over several checkouts: its path, and the commit its change is read from,
/// its merge base with its declared main branch where none is named.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetCheckout {
  path: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  since: Option<String>,
}

/// The checkout whose suite runs.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SuiteInput {
  checkout: String,
}

// The last characters of what a verb wrote on its standard error.
fn tail(said: &[u8]) -> String {
  let text = String::from_utf8_lossy(said);
  let start = text.char_indices().rev().nth(SAID_TAIL_CHARS).map_or(0, |(at, _)| at);

  text[start..].trim().to_owned()
}

/// One of the binary's verbs run in a checkout, its words ending in `--json`'s one command run.
async fn verb(hands: Arc<Hands>, checkout: String, words: Vec<String>) -> Result<CommandRun, ToolFailure> {
  let unstarted = |why: &dyn std::fmt::Display| {
    ToolFailure::Verb(fill_marks(&PLUGIN.lines.unstarted, &[("why", &why.to_string())]))
  };
  let mut child = Command::new(&hands.binary)
    .args(&words)
    .current_dir(&checkout)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true)
    .spawn()
    .map_err(|why| unstarted(&why))?;
  let mut out = Vec::new();
  let mut said = Vec::new();
  let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
    return Err(unstarted(&"no pipes"));
  };
  let (read_out, read_said) = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut said));

  read_out.and(read_said).map_err(|why| unstarted(&why))?;
  child.wait().await.map_err(|why| unstarted(&why))?;

  let text = String::from_utf8_lossy(&out);
  let last = text.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or_default();

  serde_json::from_str(last).map_err(|why| {
    ToolFailure::Verb(fill_marks(
      &PLUGIN.lines.unread,
      &[("why", &why.to_string()), ("said", &tail(&said))],
    ))
  })
}

fn client(failure: ClientFailure) -> ToolFailure {
  ToolFailure::Client(failure)
}

/// The eight tools, the reading ones first in the order of their routes.
pub fn tools() -> Vec<TestbedTool> {
  let said = &PLUGIN.tools;

  vec![
    tool(
      "testbed_queues",
      &said.testbed_queues,
      Calls::Route("GET /queues"),
      |hands, query: PageQuery| async move { hands.client.queues(&query).await.map_err(client) },
    ),
    tool(
      "testbed_test_runs",
      &said.testbed_test_runs,
      Calls::Route("GET /test-runs"),
      |hands, query: TestRunsQuery| async move { hands.client.test_runs(&query).await.map_err(client) },
    ),
    tool(
      "testbed_test_run",
      &said.testbed_test_run,
      Calls::Route("GET /test-runs/{testRunId}"),
      |hands, TestRunKey { test_run_id }| async move {
        hands.client.test_run(test_run_id).await.map_err(client)
      },
    ),
    tool(
      "testbed_deployments",
      &said.testbed_deployments,
      Calls::Route("GET /deployments"),
      |hands, query: PageQuery| async move { hands.client.deployments(&query).await.map_err(client) },
    ),
    tool(
      "testbed_deployment",
      &said.testbed_deployment,
      Calls::Route("GET /deployments/{deploymentId}"),
      |hands, DeploymentKey { deployment_id }| async move {
        hands.client.deployment(deployment_id).await.map_err(client)
      },
    ),
    tool(
      "testbed_run",
      &said.testbed_run,
      Calls::Command("bmo-testbed run --"),
      |hands,
       RunInput {
         command,
         checkout,
         queue,
       }| async move {
        if command.is_empty() {
          return Err(ToolFailure::Arguments(PLUGIN.lines.no_command.clone()));
        }

        let mut words = vec!["run".to_owned(), "--json".to_owned()];

        if queue == Some(LaneName::Narrow) {
          words.push("--narrow".to_owned());
        }
        words.push("--".to_owned());
        words.extend(command);
        verb(hands, checkout, words).await
      },
    ),
    tool(
      "testbed_check",
      &said.testbed_check,
      Calls::Command("bmo-testbed check"),
      |hands,
       CheckInput {
         checkout,
         since,
         checkouts,
       }| async move {
        let mut words = vec!["check".to_owned(), "--json".to_owned()];
        let checkouts = checkouts.unwrap_or_default();

        if since.is_some() && !checkouts.is_empty() {
          return Err(ToolFailure::Arguments(PLUGIN.lines.since_with_checkouts.clone()));
        }

        if let Some(since) = since {
          words.extend(["--since".to_owned(), since]);
        }

        for one in checkouts {
          let word = match one.since {
            Some(since) => format!("{}@{since}", one.path),
            None => one.path,
          };

          words.extend(["--checkout".to_owned(), word]);
        }

        verb(hands, checkout, words).await
      },
    ),
    tool(
      "testbed_suite",
      &said.testbed_suite,
      Calls::Command("bmo-testbed suite"),
      |hands, SuiteInput { checkout }| async move {
        verb(hands, checkout, vec!["suite".to_owned(), "--json".to_owned()]).await
      },
    ),
  ]
}
