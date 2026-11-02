//! A run placed through the placement endpoint (docs/boundary.md, 「Placement」), against a stand-in
//! endpoint the test serves: an answer of local, or one that does not come or does not read, runs here; an
//! answer of remote makes the bench ready over a stand-in `ssh` that runs its line on this machine, and the
//! run's output and result come back off its stream.

use std::convert::Infallible;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::turn::NOT_RUN_EXIT_CODE;
use serde_json::{Value, json};

use crate::stand::{Ran, checkout, git, install, ran, testbed};

// A stand-in endpoint: what it was asked, and the address it serves on, which ends with the runtime.
struct Endpoint {
  _runtime: tokio::runtime::Runtime,
  base: String,
  asked: Arc<Mutex<Vec<Value>>>,
}

// An endpoint answering `answer` to every run, `{base}` in it standing for its own address, and serving the
// run's stream as the events given.
fn endpoint(answer: Value, events: Vec<(&'static str, String)>) -> Endpoint {
  let runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(1)
    .enable_all()
    .build()
    .unwrap();
  let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let asked = Arc::new(Mutex::new(Vec::new()));
  let answer: Value = serde_json::from_str(&answer.to_string().replace("{base}", &base)).unwrap();
  let keeping = Arc::clone(&asked);
  let app = Router::new()
    .route(
      "/place",
      post(move |axum::Json(body): axum::Json<Value>| {
        keeping.lock().unwrap().push(body);
        let answer = answer.clone();

        async move { axum::Json(answer) }
      }),
    )
    .route(
      "/run",
      get(move || {
        let events = events.clone();

        async move {
          Sse::new(futures_util::stream::iter(
            events
              .into_iter()
              .map(|(name, data)| Ok::<_, Infallible>(Event::default().event(name).data(data))),
          ))
        }
      }),
    );

  runtime.spawn(async move { axum::serve(listener, app).await });
  Endpoint {
    _runtime: runtime,
    base,
    asked,
  }
}

// A stand-in `ssh` that runs the line it is handed on this machine, past its options and its destination,
// the way git and rsync hand it theirs.
const SSH: &str = r#"#!/bin/sh
while [ $# -gt 0 ]; do
  case "$1" in
    -o|-p|-i|-l|-F) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
shift
exec sh -c "$*"
"#;

// A run of the binary in a checkout, placed through the endpoint.
fn placed(repo: &Path, data: &Path, endpoint: &str, args: &[&str]) -> Ran {
  ran(testbed(repo, data, args).env("BMO_TESTBED_PLACEMENT", format!("{endpoint}/place")))
}

fn bench_answer(dir: &Path, ssh: &Path) -> Value {
  json!({
    "place": "remote",
    "machine": "machine-2",
    "bench": { "destination": "stand", "path": dir.join("bench"), "ssh": [ssh] },
    "run": "{base}/run",
  })
}

// "an answer of local runs the command here and says nothing of it, and the endpoint is told the run"
#[test]
fn an_answer_of_local_runs_here_and_the_endpoint_is_told_the_run() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let local = endpoint(json!({ "place": "local" }), Vec::new());

  checkout(&repo, "testbed = 0\n");

  let done = placed(&repo, dir.path(), &local.base, &["run", "--narrow", "--", "sh", "-c", "exit 0"]);
  let asked = local.asked.lock().unwrap().clone();

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(!done.err.contains("placement endpoint"), "{}", done.err);
  assert_eq!(asked.len(), 1);
  assert_eq!(asked[0]["command"], json!(["sh", "-c", "exit 0"]));
  assert_eq!(asked[0]["lane"], "narrow");
  assert!(asked[0].get("wait_seconds").is_none() && asked[0].get("guard_seconds").is_none());
  assert_eq!(asked[0]["commit"], git(&repo, &["rev-parse", "HEAD"]));
  assert_eq!(asked[0]["checkout"], git(&repo, &["rev-parse", "--show-toplevel"]));
  assert_eq!(asked[0]["remote"], "");
  assert_eq!((asked[0]["front"].clone(), asked[0]["next"].clone()), (json!(false), json!(false)));
}

// "an endpoint that does not answer, or answers what does not read, leaves the run here, said on a line"
#[test]
fn an_endpoint_that_does_not_answer_or_does_not_read_leaves_the_run_here() {
  let dir = tempfile::tempdir().unwrap();
  let gone = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
  let unread = endpoint(json!({ "place": "moon" }), Vec::new());
  let opening = fill_marks(&LANES.placed.local, &[("endpoint", "")]);
  let opening = opening.split("{why}").next().unwrap().split(" gave").next().unwrap();

  for base in [format!("http://{gone}"), unread.base.clone()] {
    let done = placed(dir.path(), dir.path(), &base, &["run", "--", "sh", "-c", "exit 0"]);

    assert_eq!(done.code, 0, "{base}: {}", done.err);
    assert!(done.err.contains(opening) && done.err.contains(&base), "{base}: {}", done.err);
  }
}

// "a run placed elsewhere runs in a bench copy of the checkout, and ends with the code its stream gives"
#[test]
fn a_run_placed_elsewhere_runs_in_a_bench_and_ends_with_the_code_its_stream_gives() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let ssh = dir.path().join("ssh");
  let result = "testbed: FAILED in 3s (waited 0s) — cargo: 1 passed, 1 failed, 0 skipped; log: /x.log";
  let remote = endpoint(
    bench_answer(dir.path(), &ssh),
    vec![
      ("output", "remote says hi".to_owned()),
      ("result", json!({ "line": result, "code": 3 }).to_string()),
    ],
  );

  install(SSH, &ssh);
  checkout(&repo, "testbed = 0\n");
  std::fs::write(repo.join(".gitignore"), "ignored.txt\n").unwrap();
  std::fs::write(repo.join("ignored.txt"), "left out").unwrap();
  std::fs::write(repo.join("open.txt"), "not yet committed").unwrap();

  let done = placed(&repo, dir.path(), &remote.base, &["run", "--", "cargo", "test"]);
  let bench = dir.path().join("bench");
  let running =
    fill_marks(&LANES.placed.running, &[("machine", "machine-2"), ("bench", &bench.to_string_lossy())]);

  assert_eq!(done.code, 3, "{}\n{}", done.out, done.err);
  assert_eq!(done.out, format!("{result}\n"));
  assert!(done.err.contains(&running) && done.err.contains("remote says hi\n"), "{}", done.err);
  assert_eq!(git(&bench, &["rev-parse", "HEAD"]), git(&repo, &["rev-parse", "HEAD"]));
  assert_eq!(std::fs::read_to_string(bench.join("open.txt")).unwrap(), "not yet committed");
  assert!(!bench.join("ignored.txt").exists());
  assert_eq!(remote.asked.lock().unwrap()[0]["command"], json!(["cargo", "test"]));
}

// "a stream that ends with no result, or a bench that cannot be made ready, ends the run as not run"
#[test]
fn a_run_dropped_before_its_result_ends_as_not_run() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let ssh = dir.path().join("ssh");
  let refusing = dir.path().join("refusing-ssh");
  let silent = endpoint(bench_answer(dir.path(), &ssh), vec![("output", "starting".to_owned())]);
  let unready = endpoint(bench_answer(dir.path(), &refusing), Vec::new());
  let dropped = |said: &str| fill_marks(&LANES.run.dropped, &[("machine", "machine-2"), ("said", said)]);

  install(SSH, &ssh);
  install("#!/bin/sh\necho 'no route to stand' >&2\nexit 255\n", &refusing);
  checkout(&repo, "testbed = 0\n");

  let ended = placed(&repo, dir.path(), &silent.base, &["run", "--", "cargo", "test"]);

  assert_eq!(ended.code, i32::from(NOT_RUN_EXIT_CODE), "{}", ended.err);
  assert_eq!(ended.out, format!("{}\n", dropped(&LANES.placed.no_result)));

  let never = placed(&repo, dir.path(), &unready.base, &["run", "--", "cargo", "test"]);
  let failed = fill_marks(&LANES.placed.failed, &[("said", "no route to stand")]);

  assert_eq!(never.code, i32::from(NOT_RUN_EXIT_CODE), "{}", never.err);
  assert_eq!(never.out, format!("{}\n", dropped(&failed)));
}
