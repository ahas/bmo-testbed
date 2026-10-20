//! The testbed's harness plugin (docs/boundary.md, 「The harness plugin」): the tool table, the MCP server
//! the `mcp` verb starts over it, and the OpenAPI document generated from the client's types and the
//! table. The `ahas-testbed` skill stands beside them under `skills/`.

mod openapi;
mod server;
mod tools;

pub use openapi::{MCP_COMMAND, PLUGIN_NAME, openapi, openapi_json, route_paths};
pub use server::{TestbedServer, serve_stdio};
pub use tools::{Calls, Hands, TestbedTool, ToolFailure, tools};
