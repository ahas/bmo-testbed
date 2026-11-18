//! `notice` (docs/boundary.md, 「`notice`」): an event, taken as a hook takes it, queued for the owner's supervisor
//! through the receiver's owner notices, the receiver found as its own tools find it.

use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use bmo_testbed_core::events::unhooked_line;
use bmo_testbed_core::policy::events::{EVENTS, Notice};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::turn::{Env, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use serde_json::{Map, Value};

use crate::env_file::assigned;

fn policy() -> &'static Notice {
  &EVENTS.notice
}

/// Where the receiver is reached: its base address and the bearer token it takes. The token is handed to the
/// request alone, so the type shows nothing of itself.
pub struct Receiver {
  pub address: String,
  token: String,
}

impl Receiver {
  /// The receiver as its own tools find it: its address and its token each from the environment, else out of the
  /// receiver's env file under the data root; the line naming the first of the two neither names.
  pub fn found(env: &Env, data_root: &Path, event: &str) -> Result<Self, String> {
    let said = policy();
    let file = data_root.join(&said.env_file);
    let named = |variable: &str| {
      env
        .get(variable)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_owned())
        .or_else(|| assigned(&file, variable))
        .ok_or_else(|| {
          fill_marks(
            &said.lines.unfound,
            &[("event", event), ("file", &file.to_string_lossy()), ("variable", variable)],
          )
        })
    };

    Ok(Self {
      address: named(&said.address_variable)?,
      token: named(&said.token_variable)?,
    })
  }

  /// The notice queued at the receiver's owner notices; the id of the row it was queued as, or the line saying why
  /// it was not.
  pub async fn queued(&self, event: &str, body: &Value) -> Result<String, String> {
    let said = policy();
    let route = format!("{}{}", self.address.trim_end_matches('/'), said.route);
    let unreached = |why: String| {
      fill_marks(&said.lines.unreached, &[("event", event), ("address", &self.address), ("why", &why)])
    };
    let client = reqwest::Client::builder()
      .timeout(Duration::from_secs(said.seconds))
      .build()
      .map_err(|why| unreached(why.to_string()))?;
    let answer = client
      .post(route)
      .bearer_auth(&self.token)
      .json(body)
      .send()
      .await
      .map_err(|why| unreached(why.without_url().to_string()))?;
    let status = answer.status();
    let text = answer.text().await.map_err(|why| unreached(why.without_url().to_string()))?;

    if !status.is_success() {
      let quoted: String = text.trim().chars().take(said.quoted_chars).collect();

      return Err(fill_marks(
        &said.lines.refused,
        &[("event", event), ("status", &status.as_u16().to_string()), ("said", &quoted)],
      ));
    }

    let id = serde_json::from_str::<Value>(&text)
      .ok()
      .and_then(|answer| answer.get("id").cloned())
      .map(|id| id.to_string())
      .unwrap_or_default();

    Ok(id)
  }
}

/// The body the receiver's owner notices take for an event: its code, the event's name under the policy's mark,
/// and the payload's fields beside it.
pub fn notice_body(event: &str, mut payload: Map<String, Value>) -> Value {
  payload.insert("code".to_owned(), Value::String(fill_marks(&policy().code, &[("event", event)])));

  Value::Object(payload)
}

/// The payload a hook is handed, read as one JSON object; the line saying why where it is not one.
pub fn read_payload(event: &str, text: &str) -> Result<Map<String, Value>, String> {
  let unread = |why: String| fill_marks(&policy().lines.unread, &[("event", event), ("why", &why)]);

  match serde_json::from_str::<Value>(text) {
    Ok(Value::Object(payload)) => Ok(payload),
    Ok(_) => Err(unread("not an object".to_owned())),
    Err(why) => Err(unread(why.to_string())),
  }
}

/// The verb: the event's payload read from standard input and queued as a notice; the code it ends with.
pub fn notice(event: &str) -> u8 {
  let mut text = String::new();
  let read = std::io::stdin().read_to_string(&mut text);
  let payload = match read.map_err(|why| why.to_string()).and_then(|_| read_payload(event, &text)) {
    Ok(payload) => payload,
    Err(why) => {
      eprintln!("{why}");
      return REFUSED_EXIT_CODE;
    }
  };
  let body = notice_body(event, payload.clone());
  let sent = Receiver::found(&process_env(), &data_root(), event).and_then(|receiver| {
    let runtime = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .map_err(|why| why.to_string())?;

    runtime.block_on(receiver.queued(event, &body))
  });

  match sent {
    Ok(id) => {
      println!("{}", fill_marks(&policy().lines.queued, &[("event", event), ("id", &id)]));
      0
    }
    Err(why) => {
      eprintln!("{why}");
      eprintln!("{}", unhooked_line(event, &Value::Object(payload).to_string()));
      1
    }
  }
}
