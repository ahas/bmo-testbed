//! The request the tool host hands the executable and the answer it reads back (bmo-pocket's
//! `docs/mcp-host.md`, 「A call」): protocol 1, a call of the one entry, and an answer of text pieces.

use deploy_tool::{Asked, read_handed};
use serde_json::json;

use crate::stand::{call, env, registry, text, tool};

// "a call is read with its entry and arguments, the record and the sockets beside it passed over"
#[test]
fn a_call_is_read_with_its_entry_and_arguments() {
  let request = call(json!({ "repository": "agency", "commit": "abc1234", "why": "Ship it" }));
  let handed = read_handed(request.to_string().as_bytes()).unwrap();

  assert_eq!(handed.protocol, 1);
  assert!(matches!(&handed.asked, Asked::Call { entry, .. } if entry == "testbed/deploy"));
}

// "a request in another protocol does not read"
#[test]
fn a_request_in_another_protocol_does_not_read() {
  let mut request = call(json!({}));

  request["protocol"] = json!(2);

  let refusal = read_handed(request.to_string().as_bytes()).unwrap_err();

  assert!(refusal.contains("protocol 2"), "{refusal}");
}

// "arguments that are not the schema's answer an error, and nothing runs"
#[test]
fn arguments_that_are_not_the_schemas_answer_an_error() {
  let dir = tempfile::tempdir().unwrap();
  let at = registry(dir.path(), &[]);
  let env = env(dir.path(), &dir.path().join("bin"), &at);

  for (arguments, said) in [
    (json!({ "repository": "agency", "commit": "abc1234" }), "why"),
    (
      json!({ "repository": " ", "commit": "abc1234", "why": "Ship it" }),
      "repository must not be empty",
    ),
    (
      json!({ "repository": "agency", "commit": "--no-check", "why": "Ship it" }),
      "hexadecimal",
    ),
    (json!({ "repository": "agency", "commit": "abc", "why": "Ship it" }), "hexadecimal"),
  ] {
    let ran = tool(&call(arguments).to_string(), &env);
    let answer = ran.answer();

    assert_eq!(ran.code, 0, "{}", ran.err);
    assert_eq!(answer["isError"], true);
    assert!(text(&answer).contains(said), "{}", text(&answer));
  }

  assert!(!dir.path().join("log").exists());
}

// "a call of another entry answers an error naming the one it answers"
#[test]
fn a_call_of_another_entry_answers_an_error() {
  let dir = tempfile::tempdir().unwrap();
  let at = registry(dir.path(), &[]);
  let mut request = call(json!({}));

  request["entry"] = json!("testbed/other");

  let answer = tool(&request.to_string(), &env(dir.path(), &dir.path().join("bin"), &at)).answer();

  assert_eq!(answer["isError"], true);
  assert!(text(&answer).contains("testbed/deploy"), "{}", text(&answer));
}

// "it serves no resources, and a kind it does not answer ends the call unclean"
#[test]
fn it_serves_no_resources() {
  let dir = tempfile::tempdir().unwrap();
  let at = registry(dir.path(), &[]);
  let env = env(dir.path(), &dir.path().join("bin"), &at);
  let listed = tool(&json!({ "protocol": 1, "kind": "resources_list", "session": {} }).to_string(), &env);

  assert_eq!(listed.answer(), json!({ "resources": [] }));

  let claimed = tool(
    &json!({ "protocol": 1, "kind": "claim", "url": "https://example.com", "session": {} }).to_string(),
    &env,
  );

  assert_ne!(claimed.code, 0);
  assert!(claimed.out.is_empty(), "{}", claimed.out);
}
