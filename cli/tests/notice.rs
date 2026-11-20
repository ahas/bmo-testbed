//! `notice` (docs/boundary.md, 「`notice`」): an event, taken as a hook takes it, queued for the owner's supervisor as
//! a quiet instruction in the owner's private room, the receiver found as its own tools find it.

use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

use bmo_testbed_core::policy::events::EVENTS;
use bmo_testbed_core::policy::fill_marks;
use serde_json::{Value, json};

use crate::stand::{Ran, ReceiverStand, finished, testbed};

const TOKEN: &str = "stand-token";

// The verb run in `dir` on the data root under it, handed `payload` on standard input and `env` over its own.
fn noticed(dir: &Path, event: &str, payload: &str, env: &[(&str, &str)]) -> Ran {
  let mut command = testbed(dir, &dir.join("data"), &["notice", event]);

  command.envs(env.iter().copied()).stdin(Stdio::piped());

  let mut run = command.spawn().expect("the binary starts");

  run.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
  finished(run)
}

fn payload() -> Value {
  json!({
    "version": 1,
    "event": "placed-unreachable",
    "at": "2026-01-01T00:00:00.000Z",
    "repository": null,
    "root": null,
    "commit": null,
    "summary": "machine-2 does not answer",
    "detail": { "machine": "machine-2", "since": "2026-01-01T00:00:00.000Z" },
  })
}

// "The room is the one the receiver's `GET /owner` names ... said as a fact: `code`, `testbed:` and the event's name,
// with the payload's fields standing beside it. Its `sort` is `notice`, its `kind` and `summary` the code, and its
// `publisher` `testbed`"; the receiver found out of `<data root>/env/receiver.env`, and the environment's
// `RECEIVER_ADDRESS` and `RECEIVER_TOKEN` over it; the token in the requests' authorization alone
#[test]
fn an_event_is_queued_in_the_owner_s_room_at_the_receiver_its_env_file_or_the_environment_names() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let filed = ReceiverStand::answering(201);
  let named = ReceiverStand::answering(201);

  filed.written_under(&dir.join("data"), TOKEN);

  let run = noticed(&dir, "placed-unreachable", &payload().to_string(), &[]);

  assert_eq!(run.code, 0, "{}\n{}", run.out, run.err);
  assert_eq!(
    run.out.trim(),
    fill_marks(&EVENTS.notice.lines.queued, &[("event", "placed-unreachable"), ("id", "1")])
  );
  assert!(!run.out.contains(TOKEN) && !run.err.contains(TOKEN), "{}\n{}", run.out, run.err);

  let [one] = filed.noticed().try_into().unwrap();
  let mut fact = payload();

  fact["code"] = json!("testbed:placed-unreachable");
  assert_eq!(one.authorization, format!("Bearer {TOKEN}"));
  assert_eq!(
    one.body,
    json!({
      "room": ReceiverStand::OWNER_ROOM,
      "said": { "fact": fact },
      "sort": "notice",
      "kind": "testbed:placed-unreachable",
      "summary": "testbed:placed-unreachable",
      "publisher": "testbed",
    })
  );

  let run = noticed(
    &dir,
    "placed-recovered",
    &payload().to_string(),
    &[("RECEIVER_ADDRESS", &named.address), ("RECEIVER_TOKEN", "other-token")],
  );

  assert_eq!(run.code, 0, "{}\n{}", run.out, run.err);
  assert_eq!(filed.noticed().len(), 1, "the environment's receiver is asked over the file's");

  let [one] = named.noticed().try_into().unwrap();

  assert_eq!(one.authorization, "Bearer other-token");
  assert_eq!(one.body["kind"], "testbed:placed-recovered");
}

// "Exits 1 where the receiver is not found, does not answer, refuses, or names no private room for the owner: the
// reason goes on a line, then the event's line as an event with no hook writes it ... Exits 2 where the payload is
// not one JSON object"
#[test]
fn a_notice_not_queued_says_why_and_leaves_the_event_on_standard_error() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let data = dir.join("data");
  let event_line = fill_marks(
    &EVENTS.lines.unhooked,
    &[("event", "placed-unreachable"), ("payload", &payload().to_string())],
  );

  let unfound = noticed(&dir, "placed-unreachable", &payload().to_string(), &[]);

  assert_eq!(unfound.code, 1, "{}", unfound.err);
  assert_eq!(
    unfound.err.lines().collect::<Vec<_>>(),
    [
      fill_marks(
        &EVENTS.notice.lines.unfound,
        &[
          ("event", "placed-unreachable"),
          ("file", &data.join("env/receiver.env").to_string_lossy()),
          ("variable", "RECEIVER_ADDRESS"),
        ],
      ),
      event_line.clone(),
    ]
  );

  let roomless = ReceiverStand::roomless();

  roomless.written_under(&data, TOKEN);

  let unroomed = noticed(&dir, "placed-unreachable", &payload().to_string(), &[]);

  assert_eq!(unroomed.code, 1, "{}", unroomed.err);
  assert!(unroomed.err.contains("names no private room for the owner"), "{}", unroomed.err);
  assert!(unroomed.err.contains(&event_line), "{}", unroomed.err);
  assert!(roomless.noticed().is_empty(), "no instruction is queued without the owner's room");

  let refusing = ReceiverStand::answering(403);

  refusing.written_under(&data, TOKEN);

  let refused = noticed(&dir, "placed-unreachable", &payload().to_string(), &[]);

  assert_eq!(refused.code, 1, "{}", refused.err);
  assert!(refused.err.contains("the receiver answered 403"), "{}", refused.err);
  assert!(refused.err.contains(&event_line), "{}", refused.err);

  let gone = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
  let unreached = noticed(
    &dir,
    "placed-unreachable",
    &payload().to_string(),
    &[("RECEIVER_ADDRESS", &format!("http://{gone}"))],
  );

  assert_eq!(unreached.code, 1, "{}", unreached.err);
  assert!(unreached.err.contains("did not answer"), "{}", unreached.err);
  assert!(unreached.err.contains(&event_line), "{}", unreached.err);

  let unread = noticed(&dir, "placed-unreachable", "[1, 2]", &[]);

  assert_eq!(unread.code, 2, "{}", unread.err);
  assert_eq!(refusing.noticed().len(), 1, "a payload that does not read is not sent");
}
