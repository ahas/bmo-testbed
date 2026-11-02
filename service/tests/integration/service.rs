//! The HTTP service (docs/boundary.md, 「The HTTP service」), served on a loopback port of its own and asked
//! through the published client: the token on every route, the members read from the record, the error
//! shape and its codes, the lanes as they stand, and the event stream with the service's own
//! `queue.changed`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bmo_testbed_client::{
  ClientFailure, ErrorCode, EventKind, EventsQuery, LaneName, PageQuery, TestRun, TestRunKind,
  TestRunState, TestRunsQuery, TestbedClient,
};
use bmo_testbed_core::queue::now_ms;
use bmo_testbed_core::turn::{Env, Lane, TurnAsk, take_turn};
use bmo_testbed_record::Record;
use bmo_testbed_service::{Ground, router, watch_queues};
use tokio::task::JoinHandle;

const TOKEN: &str = "the-service-token";

/// How long a test lets a condition take before it fails; a guard, never a wait.
const PATIENCE: Duration = Duration::from_secs(10);

/// The service over a data root and a lane lock of its own, stopped as the test ends.
struct Served {
  dir: tempfile::TempDir,
  base: String,
  tasks: Vec<JoinHandle<()>>,
}

impl Drop for Served {
  fn drop(&mut self) {
    for task in &self.tasks {
      task.abort();
    }
  }
}

impl Served {
  async fn start() -> Self {
    let dir = tempfile::tempdir().unwrap();
    let env =
      Env::from([("BMO_TESTBED_LOCK".to_owned(), dir.path().join("wide.lock").to_string_lossy().into())]);
    let ground = Arc::new(Ground {
      data_root: dir.path().to_path_buf(),
      token: TOKEN.to_owned(),
      env,
      event_poll: Duration::from_millis(10),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let serving = tokio::spawn(async move {
      let _ = axum::serve(listener, router(Arc::clone(&ground))).await;
    });

    Self {
      dir,
      base,
      tasks: vec![serving],
    }
  }

  fn watching(mut self, every: Duration) -> Self {
    let ground = Arc::new(Ground {
      data_root: self.dir.path().to_path_buf(),
      token: TOKEN.to_owned(),
      env: Env::from([(
        "BMO_TESTBED_LOCK".to_owned(),
        self.dir.path().join("wide.lock").to_string_lossy().into(),
      )]),
      event_poll: every,
    });

    self.tasks.push(tokio::spawn(watch_queues(ground, every)));
    self
  }

  fn client(&self) -> TestbedClient {
    TestbedClient::new(&self.base, TOKEN)
  }

  fn record(&self) -> Record {
    Record::open(self.dir.path()).unwrap()
  }
}

fn test_run(checkout: &str) -> TestRun {
  TestRun {
    id: 0,
    kind: TestRunKind::Suite,
    checkout: checkout.to_owned(),
    repository_name: Some("repository".to_owned()),
    commit: None,
    command: "bmo-testbed suite".to_owned(),
    scope: None,
    unit_name: None,
    pid: i64::from(std::process::id()),
    state: TestRunState::Running,
    passed_count: 0,
    failed_count: 0,
    skipped_count: 0,
    log_path: "/logs/suite.log".to_owned(),
    started_at: now_ms(),
    ended_at: None,
  }
}

fn code_of(failure: ClientFailure) -> (u16, Option<ErrorCode>) {
  match failure {
    ClientFailure::Answered { status, answer, .. } => (status, answer.map(|answer| answer.code)),
    other => panic!("the service answers: {other}"),
  }
}

// "every route takes the token, the health and a path no route answers among them"
#[tokio::test]
async fn every_route_takes_the_token() {
  let served = Served::start().await;
  let wrong = TestbedClient::new(&served.base, "another-token");

  assert_eq!(code_of(wrong.health().await.unwrap_err()), (401, Some(ErrorCode::Unauthorized)));
  assert_eq!(
    code_of(wrong.test_runs(&TestRunsQuery::default()).await.unwrap_err()),
    (401, Some(ErrorCode::Unauthorized))
  );

  let health = served.client().health().await.unwrap();

  assert!(health.ready && health.reason.is_none());
  assert_eq!(health.version, env!("CARGO_PKG_VERSION"));
}

// "a path or a method no route answers is no-route, a key nothing stands at not-found, a bad query bad-request"
#[tokio::test]
async fn failures_answer_in_the_error_shape_with_their_codes() {
  let served = Served::start().await;
  let http = reqwest::Client::new();
  let asked = |method: reqwest::Method, path: &str| {
    http.request(method, format!("{}{path}", served.base)).bearer_auth(TOKEN).send()
  };
  let no_route = asked(reqwest::Method::GET, "/nothing").await.unwrap();
  let posted = asked(reqwest::Method::POST, "/health").await.unwrap();
  let bad_limit = asked(reqwest::Method::GET, "/test-runs?limit=many").await.unwrap();
  let bad_key = asked(reqwest::Method::GET, "/test-runs/first").await.unwrap();
  let unknown = asked(reqwest::Method::GET, "/test-runs?colour=red").await.unwrap();

  for (answer, status, code) in [
    (no_route, 404, "no-route"),
    (posted, 404, "no-route"),
    (bad_limit, 400, "bad-request"),
    (bad_key, 400, "bad-request"),
    (unknown, 400, "bad-request"),
  ] {
    assert_eq!(answer.status().as_u16(), status);

    let body: serde_json::Value = answer.json().await.unwrap();

    assert_eq!(body["code"], code);
    assert!(body["error"].as_str().is_some_and(|error| !error.is_empty()));
  }

  assert_eq!(
    code_of(served.client().test_run(7).await.unwrap_err()),
    (404, Some(ErrorCode::NotFound))
  );
  assert_eq!(
    code_of(served.client().deployment(7).await.unwrap_err()),
    (404, Some(ErrorCode::NotFound))
  );
  assert_eq!(
    code_of(served.client().log_lines(7, &PageQuery::default()).await.unwrap_err()),
    (404, Some(ErrorCode::NotFound))
  );
}

// "the test runs are the record's, newest first, narrowed by checkout"
#[tokio::test]
async fn the_test_runs_are_the_records() {
  let served = Served::start().await;
  let mut record = served.record();
  let first = record.begin_test_run(&test_run("/a")).unwrap();
  let second = record.begin_test_run(&test_run("/b")).unwrap();
  let client = served.client();
  let all = client.test_runs(&TestRunsQuery::default()).await.unwrap();
  let narrowed = client
    .test_runs(&TestRunsQuery {
      checkout: Some("/a".to_owned()),
      ..TestRunsQuery::default()
    })
    .await
    .unwrap();

  assert_eq!(all.items, vec![second.clone(), first.clone()]);
  assert_eq!(narrowed.items, vec![first.clone()]);
  assert_eq!(client.test_run(first.id).await.unwrap(), first);
}

// "the lanes answer as they stand, the wide one first, and a change of one is a queue.changed event"
#[tokio::test]
async fn the_lanes_answer_as_they_stand_and_a_change_is_an_event() {
  let served = Served::start().await.watching(Duration::from_millis(10));
  let client = served.client();
  let lanes = client.queues(&PageQuery::default()).await.unwrap();

  assert_eq!(
    lanes.items.iter().map(|lane| lane.name).collect::<Vec<_>>(),
    vec![LaneName::Wide, LaneName::Narrow]
  );
  assert!(lanes.items[0].holder.is_none() && lanes.items[0].width > 0);

  let lock = served.dir.path().join("wide.lock");
  let env = Env::from([("BMO_TESTBED_SCOPE".to_owned(), "scope-9".to_owned())]);
  let turn = take_turn(&TurnAsk {
    command: "a check".to_owned(),
    lane: Lane::Wide,
    say: &|_| {},
    path: Some(lock),
    env,
    checkout: "/checkout".to_owned(),
    poll: Some(Duration::from_millis(5)),
    notice_seconds: None,
  });

  assert!(turn.had());

  let holder = client.queues(&PageQuery::default()).await.unwrap().items[0].holder.clone().unwrap();

  assert_eq!((holder.scope.as_deref(), holder.command.as_str()), (Some("scope-9"), "a check"));

  let began = Instant::now();
  let changed = loop {
    let page = client
      .events(&EventsQuery {
        kinds: Some("queue.changed".to_owned()),
        after: Some(0),
        limit: None,
      })
      .await
      .unwrap();

    if let Some(event) = page.items.into_iter().next() {
      break event;
    }

    assert!(began.elapsed() < PATIENCE, "the watcher tells the change");
    tokio::time::sleep(Duration::from_millis(10)).await;
  };

  assert_eq!(changed.kind, EventKind::QueueChanged);
  assert_eq!(changed.data["holder"]["scope"], "scope-9");
  drop(turn);
}

// "the stream resumes after a number and carries each event written since, in its stream shape"
#[tokio::test]
async fn the_stream_carries_each_event_written_after_the_resume() {
  let served = Served::start().await;
  let mut record = served.record();
  record.begin_test_run(&test_run("/a")).unwrap();
  let answer = reqwest::Client::new()
    .get(format!("{}/events?kinds=test-run.created", served.base))
    .bearer_auth(TOKEN)
    .header("Last-Event-ID", record.last_event_id().unwrap().to_string())
    .send()
    .await
    .unwrap();

  assert_eq!(answer.status().as_u16(), 200);
  assert!(answer.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));

  record.begin_test_run(&test_run("/b")).unwrap();
  let mut answer = answer;
  let mut heard = String::new();
  let began = Instant::now();

  while !heard.contains("\n\n") {
    let chunk = tokio::time::timeout(PATIENCE, answer.chunk()).await.expect("the stream speaks in time");

    heard.push_str(&String::from_utf8_lossy(&chunk.unwrap().expect("the stream stays open")));
    assert!(began.elapsed() < PATIENCE);
  }

  assert!(heard.contains("event: test-run.created"), "{heard}");
  assert!(heard.contains("\"checkout\":\"/b\""), "{heard}");
  assert!(!heard.contains("\"checkout\":\"/a\""), "{heard}");

  let unknown = reqwest::Client::new()
    .get(format!("{}/events?kinds=queue.dropped", served.base))
    .bearer_auth(TOKEN)
    .send()
    .await
    .unwrap();

  assert_eq!(unknown.status().as_u16(), 400);
}
