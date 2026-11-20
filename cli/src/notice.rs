//! `notice` (docs/boundary.md, 「`notice`」): an event, taken as a hook takes it, queued for the owner's supervisor as
//! a quiet instruction in the owner's private room, the receiver found as its own tools find it.

use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use bmo_testbed_core::events::unhooked_line;
use bmo_testbed_core::policy::events::{EVENTS, Notice};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::turn::{Env, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use serde_json::{Map, Value, json};

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

  /// The event queued as an instruction in the owner's private room, `fact` what it is said as; the id of the row it
  /// was queued as, or the line saying why it was not.
  pub async fn queued(&self, event: &str, fact: Value) -> Result<String, String> {
    let said = policy();
    let client = reqwest::Client::builder()
      .timeout(Duration::from_secs(said.seconds))
      .build()
      .map_err(|why| self.unreached(event, why.to_string()))?;
    let owner = self.answered(event, client.get(self.at(&said.owner_route))).await?;
    let Some(room) = owner.get("room").and_then(Value::as_i64) else {
      return Err(fill_marks(
        &said.lines.roomless,
        &[("event", event), ("said", &quoted(&owner.to_string()))],
      ));
    };
    let queued = self
      .answered(event, client.post(self.at(&said.route)).json(&instruction(event, room, fact)))
      .await?;

    Ok(queued.get("id").map(Value::to_string).unwrap_or_default())
  }

  fn at(&self, route: &str) -> String {
    format!("{}{route}", self.address.trim_end_matches('/'))
  }

  fn unreached(&self, event: &str, why: String) -> String {
    fill_marks(
      &policy().lines.unreached,
      &[("event", event), ("address", &self.address), ("why", &why)],
    )
  }

  /// The receiver's answer to one request behind its token, read as JSON (`null` where its body is none); the line
  /// saying why where it was not reached or refused.
  async fn answered(&self, event: &str, request: reqwest::RequestBuilder) -> Result<Value, String> {
    let answer = request
      .bearer_auth(&self.token)
      .send()
      .await
      .map_err(|why| self.unreached(event, why.without_url().to_string()))?;
    let status = answer.status();
    let text = answer
      .text()
      .await
      .map_err(|why| self.unreached(event, why.without_url().to_string()))?;

    if !status.is_success() {
      return Err(fill_marks(
        &policy().lines.refused,
        &[("event", event), ("status", &status.as_u16().to_string()), ("said", &quoted(&text))],
      ));
    }

    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
  }
}

/// The start of a body the receiver answered, as long as the policy quotes.
fn quoted(text: &str) -> String {
  text.trim().chars().take(policy().quoted_chars).collect()
}

/// What an event is said as: its code, the event's name under the policy's mark, and the payload's fields beside it.
pub fn fact(event: &str, mut payload: Map<String, Value>) -> Value {
  payload.insert("code".to_owned(), Value::String(code(event)));

  Value::Object(payload)
}

fn code(event: &str) -> String {
  fill_marks(&policy().code, &[("event", event)])
}

/// The instruction the receiver queues for an event in `room`: said as `fact`, under the event's code as its kind
/// and summary, and published by the testbed.
fn instruction(event: &str, room: i64, fact: Value) -> Value {
  let said = policy();

  json!({
    "room": room,
    "said": { "fact": fact },
    "sort": said.sort,
    "kind": code(event),
    "summary": code(event),
    "publisher": said.publisher,
  })
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

/// The verb: the event's payload read from standard input and queued as an instruction; the code it ends with.
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
  let fact = fact(event, payload.clone());
  let sent = Receiver::found(&process_env(), &data_root(), event).and_then(|receiver| {
    let runtime = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .map_err(|why| why.to_string())?;

    runtime.block_on(receiver.queued(event, fact))
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
