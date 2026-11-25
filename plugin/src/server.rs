//! The MCP server: the harness tools over one testbed, each call made through the client or the binary.

use std::path::PathBuf;
use std::sync::Arc;

use bmo_testbed_client::TestbedClient;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::plugin::PLUGIN;
use rmcp::model::{
  CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, InitializeResult,
  JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::stdio;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use schemars::Schema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

use crate::tools::{Hands, TestbedTool, ToolFailure, tools};

/// The server over the testbed the hands reach.
pub struct TestbedServer {
  hands: Arc<Hands>,
  tools: Arc<Vec<TestbedTool>>,
}

impl TestbedServer {
  pub fn new(hands: Hands) -> Self {
    Self {
      hands: Arc::new(hands),
      tools: Arc::new(tools()),
    }
  }

  /// The tools as MCP lists them, each input and output a schema of its own with its definitions inside.
  pub fn listed(&self) -> Vec<Tool> {
    self.tools.iter().map(listed).collect()
  }
}

fn listed(tool: &TestbedTool) -> Tool {
  let mut generator = SchemaSettings::draft2020_12()
    .for_serialize()
    .with(|settings| settings.meta_schema = None)
    .into_generator();
  let input = tool.input(&mut generator);
  let output = tool.output(&mut generator);
  let definitions = generator.take_definitions(true);
  let mut listed = Tool::new(tool.name, tool.description, standing_alone(input, &definitions));

  listed.output_schema = Some(Arc::new(standing_alone(output, &definitions)));
  listed
}

/// A schema made to stand on its own: an object at its root, with every definition it refers to beside it.
fn standing_alone(schema: Schema, definitions: &Map<String, Value>) -> JsonObject {
  let mut object = match schema.to_value() {
    Value::Object(object) if object.contains_key("$ref") => {
      let mut wrapped = Map::new();

      wrapped.insert("allOf".into(), Value::Array(vec![Value::Object(object)]));
      wrapped.insert("type".into(), "object".into());
      wrapped
    }
    Value::Object(object) => object,
    _ => Map::new(),
  };

  if !definitions.is_empty() {
    object.insert("$defs".into(), Value::Object(definitions.clone()));
  }
  object
}

impl ServerHandler for TestbedServer {
  fn get_info(&self) -> InitializeResult {
    let mut info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build());
    let mut server = Implementation::default();

    server.name = "bmo-testbed".to_owned();
    server.version = bmo_testbed_core::version::VERSION.to_owned();
    info.server_info = server;
    info.instructions = Some(PLUGIN.instructions.clone());
    info
  }

  async fn list_tools(
    &self,
    _: Option<PaginatedRequestParams>,
    _: RequestContext<RoleServer>,
  ) -> Result<ListToolsResult, ErrorData> {
    Ok(ListToolsResult::with_all_items(self.listed()))
  }

  async fn call_tool(
    &self,
    request: CallToolRequestParams,
    _: RequestContext<RoleServer>,
  ) -> Result<CallToolResponse, ErrorData> {
    let Some(tool) = self.tools.iter().find(|tool| tool.name == request.name) else {
      return Err(ErrorData::invalid_params(
        fill_marks(&PLUGIN.lines.no_tool, &[("name", request.name.as_ref())]),
        None,
      ));
    };
    let arguments = Value::Object(request.arguments.unwrap_or_default());
    let result = match tool.call(Arc::clone(&self.hands), arguments).await {
      Ok(answer) => CallToolResult::structured(answer),
      Err(failure) => failed(&failure),
    };

    Ok(CallToolResponse::Complete(result))
  }
}

/// A failed call as a tool error whose text is the failure's content. It carries no structured content, which
/// would be read against the tool's output schema.
fn failed(failure: &ToolFailure) -> CallToolResult {
  CallToolResult::error(vec![ContentBlock::text(failure.content().to_string())])
}

/// Serves the tools on standard input and output until the harness closes the session; the code the verb
/// ends with.
pub async fn serve_stdio(service: &str, token: &str, binary: PathBuf) -> u8 {
  let said = &PLUGIN.lines;
  let server = TestbedServer::new(Hands {
    client: TestbedClient::new(service, token),
    binary,
  });
  let served = match server.serve(stdio()).await {
    Ok(served) => served,
    Err(why) => {
      eprintln!("{}", fill_marks(&said.unserved, &[("why", &why.to_string())]));
      return 1;
    }
  };

  match served.waiting().await {
    Ok(_) => 0,
    Err(why) => {
      eprintln!("{}", fill_marks(&said.ended, &[("why", &why.to_string())]));
      1
    }
  }
}
