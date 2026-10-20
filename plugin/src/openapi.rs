//! The OpenAPI document (docs/boundary.md, 「The HTTP service」 and 「The harness plugin」): every route with
//! its parameters, its answer and the error codes it can give, the members as the client's types describe
//! them, the events, and the harness plugin's tools from the table the MCP server lists. The committed
//! `docs/openapi.json` is this document, which a test holds it to.

use bmo_testbed_client::{
  Deployment, ErrorCode, Event, EventKind, EventsQuery, Health, LogLine, Page, PageQuery, Queue, Restart,
  RestartsQuery, TestRun, TestRunsQuery, WireError,
};
use bmo_testbed_core::policy::plugin::PLUGIN;
use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::{JsonSchema, Schema};
use serde_json::{Map, Value, json};
use strum::VariantArray;

use crate::tools::{Calls, tools};

/// Where the schemas the routes refer to stand in the document.
const SCHEMAS: &str = "/components/schemas";

/// The plugin a harness takes the testbed in, and the command that starts its MCP server.
pub const PLUGIN_NAME: &str = "bmo-testbed";
pub const MCP_COMMAND: &str = "bmo-testbed mcp";

/// One route as the document states it.
struct Route {
  path: &'static str,
  id: &'static str,
  tag: &'static str,
  summary: &'static str,
  /// The key the path names, where it names one.
  key: Option<&'static str>,
  query: Option<fn(&mut SchemaGenerator) -> Schema>,
  answer: fn(&mut SchemaGenerator) -> Schema,
  codes: &'static [ErrorCode],
  /// Whether it also answers as an event stream.
  streams: bool,
}

fn routes() -> Vec<Route> {
  let said = &PLUGIN.document;

  vec![
    Route {
      path: "/health",
      id: "listHealth",
      tag: "health",
      summary: &said.health,
      key: None,
      query: None,
      answer: SchemaGenerator::subschema_for::<Health>,
      codes: &[ErrorCode::Unauthorized],
      streams: false,
    },
    Route {
      path: "/events",
      id: "listEvents",
      tag: "events",
      summary: &said.events,
      key: None,
      query: Some(EventsQuery::json_schema),
      answer: SchemaGenerator::subschema_for::<Page<Event>>,
      codes: &[ErrorCode::Unauthorized, ErrorCode::BadRequest, ErrorCode::Gone],
      streams: true,
    },
    Route {
      path: "/queues",
      id: "listQueues",
      tag: "queues",
      summary: &said.queues,
      key: None,
      query: Some(PageQuery::json_schema),
      answer: SchemaGenerator::subschema_for::<Page<Queue>>,
      codes: &[ErrorCode::Unauthorized, ErrorCode::BadRequest],
      streams: false,
    },
    Route {
      path: "/test-runs",
      id: "listTestRuns",
      tag: "test-runs",
      summary: &said.test_runs,
      key: None,
      query: Some(TestRunsQuery::json_schema),
      answer: SchemaGenerator::subschema_for::<Page<TestRun>>,
      codes: &[ErrorCode::Unauthorized, ErrorCode::BadRequest],
      streams: false,
    },
    Route {
      path: "/test-runs/{testRunId}",
      id: "readTestRunsByTestRunId",
      tag: "test-runs",
      summary: &said.test_run,
      key: Some("testRunId"),
      query: None,
      answer: SchemaGenerator::subschema_for::<TestRun>,
      codes: &[ErrorCode::Unauthorized, ErrorCode::BadRequest, ErrorCode::NotFound],
      streams: false,
    },
    Route {
      path: "/deployments",
      id: "listDeployments",
      tag: "deployments",
      summary: &said.deployments,
      key: None,
      query: Some(PageQuery::json_schema),
      answer: SchemaGenerator::subschema_for::<Page<Deployment>>,
      codes: &[ErrorCode::Unauthorized, ErrorCode::BadRequest],
      streams: false,
    },
    Route {
      path: "/deployments/{deploymentId}",
      id: "readDeploymentsByDeploymentId",
      tag: "deployments",
      summary: &said.deployment,
      key: Some("deploymentId"),
      query: None,
      answer: SchemaGenerator::subschema_for::<Deployment>,
      codes: &[ErrorCode::Unauthorized, ErrorCode::BadRequest, ErrorCode::NotFound],
      streams: false,
    },
    Route {
      path: "/deployments/{deploymentId}/log-lines",
      id: "listDeploymentsLogLinesByDeploymentId",
      tag: "deployments",
      summary: &said.log_lines,
      key: Some("deploymentId"),
      query: Some(PageQuery::json_schema),
      answer: SchemaGenerator::subschema_for::<Page<LogLine>>,
      codes: &[ErrorCode::Unauthorized, ErrorCode::BadRequest, ErrorCode::NotFound],
      streams: false,
    },
    Route {
      path: "/restarts",
      id: "listRestarts",
      tag: "restarts",
      summary: &said.restarts,
      key: None,
      query: Some(RestartsQuery::json_schema),
      answer: SchemaGenerator::subschema_for::<Page<Restart>>,
      codes: &[ErrorCode::Unauthorized, ErrorCode::BadRequest],
      streams: false,
    },
  ]
}

/// Every method and path the document states, for a caller holding the service to them.
pub fn route_paths() -> Vec<&'static str> {
  routes().iter().map(|route| route.path).collect()
}

// The query's fields as the route's parameters, each optional, the cursor described as a cursor.
fn parameters(query: Schema) -> Vec<Value> {
  let value = query.to_value();
  let properties = value.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();

  properties
    .into_iter()
    .map(|(name, mut schema)| {
      if let Some(object) = schema.as_object_mut() {
        if name == "after" {
          object.insert("description".into(), PLUGIN.document.after.clone().into());
        }
        if let Some(Value::Array(types)) = object.get("type").cloned() {
          let named: Vec<Value> = types.into_iter().filter(|kind| kind != "null").collect();

          if let [one] = named.as_slice() {
            object.insert("type".into(), one.clone());
          }
        }
      }
      json!({ "name": name, "in": "query", "required": false, "schema": schema })
    })
    .collect()
}

fn operation(route: &Route, generator: &mut SchemaGenerator) -> Value {
  let answer = (route.answer)(generator).to_value();
  let mut content = Map::new();

  if route.streams {
    content.insert("text/event-stream".into(), json!({ "schema": { "type": "string" } }));
  }
  content.insert("application/json".into(), json!({ "schema": answer }));

  let mut parameters_of: Vec<Value> = route
    .key
    .map(|key| json!({ "name": key, "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }))
    .into_iter()
    .collect();

  if let Some(query) = route.query {
    parameters_of.extend(parameters(query(generator)));
  }

  let mut operation = json!({
    "operationId": route.id,
    "tags": [route.tag],
    "summary": route.summary,
    "responses": {
      "200": { "description": route.summary, "content": content },
      "default": { "$ref": "#/components/responses/Error" },
    },
    "x-codes": route.codes.iter().map(|code| code.name()).collect::<Vec<_>>(),
  });

  if !parameters_of.is_empty() {
    operation["parameters"] = Value::Array(parameters_of);
  }
  operation
}

/// The document, its `x-commands` the binary's verbs as the caller names them.
pub fn openapi(commands: &[&str]) -> Value {
  let said = &PLUGIN.document;
  let mut generator = SchemaSettings::draft2020_12()
    .for_serialize()
    .with(|settings| {
      settings.definitions_path = SCHEMAS.into();
      settings.meta_schema = None;
    })
    .into_generator();
  let error = generator.subschema_for::<WireError>();
  let mut tags: Vec<&str> = Vec::new();
  let mut paths = Map::new();

  for route in routes() {
    if !tags.contains(&route.tag) {
      tags.push(route.tag);
    }
    paths.insert(route.path.to_owned(), json!({ "get": operation(&route, &mut generator) }));
  }

  let x_tools: Vec<Value> = tools()
    .iter()
    .map(|tool| {
      let (route, command) = match tool.calls {
        Calls::Route(route) => (Value::from(route), None),
        Calls::Command(command) => (Value::Null, Some(command)),
      };
      let mut listed = json!({
        "name": tool.name,
        "description": tool.description,
        "input": tool.input(&mut generator),
        "output": tool.output(&mut generator),
        "route": route,
      });

      if let Some(command) = command {
        listed["command"] = command.into();
      }
      listed
    })
    .collect();

  json!({
    "openapi": "3.1.0",
    "info": {
      "title": said.title,
      "version": env!("CARGO_PKG_VERSION"),
      "description": said.description,
    },
    "servers": [{ "url": "/", "description": said.server }],
    "security": [{ "bearer": [] }],
    "x-layer": "upstream",
    "x-stage": 2,
    "x-calls": [],
    "x-harness": { "plugin": PLUGIN_NAME, "mcp": MCP_COMMAND, "entry": said.entry },
    "tags": tags.iter().map(|tag| json!({ "name": tag })).collect::<Vec<_>>(),
    "paths": paths,
    "components": {
      "securitySchemes": { "bearer": { "type": "http", "scheme": "bearer", "description": said.bearer } },
      "responses": {
        "Error": {
          "description": said.error,
          "content": { "application/json": { "schema": error } },
        },
      },
      "schemas": generator.take_definitions(true),
    },
    "x-events": EventKind::VARIANTS.iter().map(|kind| kind.name()).collect::<Vec<_>>(),
    "x-tools": x_tools,
    "x-commands": commands,
  })
}

/// The document as the repository keeps it: two-space indented JSON with a closing newline.
pub fn openapi_json(commands: &[&str]) -> String {
  let mut text = serde_json::to_string_pretty(&openapi(commands)).unwrap_or_default();

  text.push('\n');
  text
}
