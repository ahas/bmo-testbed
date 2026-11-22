//! The `serve` and `mcp` verbs (docs/boundary.md, 「`serve`」 and 「`mcp`」), and the status as JSON, which
//! answers the lanes as the service's queue page does.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bmo_testbed_client::PageQuery;
use bmo_testbed_core::close_line::{CloseEntry, read_close_line};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::service::SERVICE;
use bmo_testbed_core::turn::Env;
use bmo_testbed_core::verb::REFUSED_EXIT_CODE;
use bmo_testbed_record::Record;
use bmo_testbed_service::{Ground, queue_page, router, watch_queues};
use serde::Serialize;

use bmo_testbed_core::verb::data_root_of;
use bmo_testbed_deploy::line::{DeployLine, Served, served};

use crate::placement::MachineState;
use crate::words::{McpArgs, ServeArgs};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CloseLineEntry {
  scope: Option<String>,
  checkout: String,
  asked_at: i64,
  held_at: Option<i64>,
  pid: i64,
}

impl From<&CloseEntry> for CloseLineEntry {
  fn from(entry: &CloseEntry) -> Self {
    Self {
      scope: entry.scope.clone(),
      checkout: entry.checkout.clone(),
      asked_at: entry.asked,
      held_at: entry.since,
      pid: entry.pid,
    }
  }
}

#[derive(Serialize)]
struct CloseLineAnswer {
  holder: Option<CloseLineEntry>,
  waiting: Vec<CloseLineEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusAnswer {
  queues: bmo_testbed_client::Page<bmo_testbed_client::Queue>,
  machines: Vec<MachineState>,
  close_line: Option<CloseLineAnswer>,
  deploy_line: Served,
}

/// Both lanes as `GET /queues` answers them, the placed machines as given, the close line where the caller
/// names one, then the deployment line under the environment's data root as it will serve its deployments.
pub fn status_json(env: &Env, close_line: Option<&Path>, machines: Vec<MachineState>) -> String {
  let answer = StatusAnswer {
    queues: queue_page(env, &PageQuery::default()),
    machines,
    close_line: close_line.map(read_close_line).map(|line| CloseLineAnswer {
      holder: line.holder.as_ref().map(CloseLineEntry::from),
      waiting: line.waiting.iter().map(CloseLineEntry::from).collect(),
    }),
    deploy_line: served(&DeployLine::under(&data_root_of(env)).lock),
  };

  serde_json::to_string_pretty(&answer).unwrap_or_default()
}

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
  tokio::runtime::Builder::new_multi_thread().enable_all().build()
}

/// Serves the HTTP service until a stop signal; the code it ends with.
pub fn serve(args: &ServeArgs, data_root: &Path, env: Env) -> u8 {
  let said = &SERVICE.lines;
  let address = args.listen.clone().unwrap_or_else(|| SERVICE.listen.clone());

  if args.token.is_empty() {
    eprintln!("{}", said.no_token);
    return REFUSED_EXIT_CODE;
  }

  if let Err(why) = Record::open(data_root) {
    eprintln!("{}", fill_marks(&said.no_record, &[("why", &why)]));
    return 1;
  }

  let ground = Arc::new(Ground {
    data_root: data_root.to_path_buf(),
    token: args.token.clone(),
    env,
    event_poll: Duration::from_millis(SERVICE.event_poll_milliseconds),
  });
  let unbound = |why: &dyn std::fmt::Display| {
    eprintln!("{}", fill_marks(&said.unbound, &[("address", &address), ("why", &why.to_string())]));
  };
  let runtime = match runtime() {
    Ok(runtime) => runtime,
    Err(why) => {
      unbound(&why);
      return 1;
    }
  };

  runtime.block_on(async {
    let listener = match tokio::net::TcpListener::bind(&address).await {
      Ok(listener) => listener,
      Err(why) => {
        unbound(&why);
        return 1;
      }
    };
    let bound = listener.local_addr().map_or_else(|_| address.clone(), |bound| bound.to_string());

    eprintln!("{}", fill_marks(&said.listening, &[("address", &bound)]));
    tokio::spawn(watch_queues(
      Arc::clone(&ground),
      Duration::from_millis(SERVICE.queue_poll_milliseconds),
    ));

    match axum::serve(listener, router(ground)).with_graceful_shutdown(stopped()).await {
      Ok(()) => 0,
      Err(why) => {
        unbound(&why);
        1
      }
    }
  })
}

// Resolves on the first stop signal: an interrupt or a termination.
async fn stopped() {
  let mut terminated = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
  let terminate = async {
    match terminated.as_mut() {
      Some(signal) => {
        signal.recv().await;
      }
      None => std::future::pending().await,
    }
  };

  tokio::select! {
    _ = tokio::signal::ctrl_c() => {}
    () = terminate => {}
  }
}

/// Serves the harness plugin's MCP server on standard input and output; the code it ends with.
pub fn mcp(args: &McpArgs) -> u8 {
  let service = args.service.clone().unwrap_or_else(|| format!("http://{}", SERVICE.listen));
  let binary = std::env::current_exe().unwrap_or_else(|_| crate::words::BINARY.into());

  match runtime() {
    Ok(runtime) => runtime.block_on(bmo_testbed_plugin::serve_stdio(&service, &args.token, binary)),
    Err(why) => {
      eprintln!("{why}");
      1
    }
  }
}
