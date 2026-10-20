//! The testbed's HTTP service (docs/boundary.md, 「The HTTP service」): the record and the lanes served
//! read-only behind one bearer token, in the conventions' wire format, with one event stream. It runs
//! nothing: what the verbs did is read from the record, and the lanes are read as they stand. Its one
//! writing is its own `queue.changed` event, posted where a reading of the lanes differs from the last.

mod events;
mod queues;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use bmo_testbed_client::{ErrorCode, Health, PageQuery, RestartsQuery, TestRunsQuery, WireError};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::service::SERVICE;
use bmo_testbed_core::turn::Env;
use bmo_testbed_record::Record;
use serde::Serialize;

pub use queues::{queue_page, watch_queues};

/// What the service stands on: the data root whose record it reads, its token, the environment the lanes'
/// locks are found in, and how often an open stream asks the record for what was written since.
pub struct Ground {
  pub data_root: PathBuf,
  pub token: String,
  pub env: Env,
  pub event_poll: Duration,
}

/// A failure answered in the error shape, with the status its code names.
pub struct Failure {
  code: ErrorCode,
  error: String,
}

impl Failure {
  pub fn new(code: ErrorCode, error: impl Into<String>) -> Self {
    Self {
      code,
      error: error.into(),
    }
  }

  fn failed(why: String) -> Self {
    Self::new(ErrorCode::Failed, why)
  }

  fn not_found(what: &str, id: i64) -> Self {
    Self::new(
      ErrorCode::NotFound,
      fill_marks(&SERVICE.errors.not_found, &[("what", what), ("id", &id.to_string())]),
    )
  }

  fn bad_query(why: &str) -> Self {
    Self::new(ErrorCode::BadRequest, fill_marks(&SERVICE.errors.bad_query, &[("why", why)]))
  }
}

impl IntoResponse for Failure {
  fn into_response(self) -> Response {
    let status = StatusCode::from_u16(self.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    (
      status,
      Json(WireError {
        code: self.code,
        error: self.error,
      }),
    )
      .into_response()
  }
}

type Answer<T> = Result<Json<T>, Failure>;

/// The record read on a thread of its own, since a reading blocks on the file.
async fn reading<T: Send + 'static>(
  ground: &Arc<Ground>,
  read: impl FnOnce(&Record) -> Result<T, String> + Send + 'static,
) -> Result<T, Failure> {
  let data_root = ground.data_root.clone();

  tokio::task::spawn_blocking(move || Record::open(&data_root).and_then(|record| read(&record)))
    .await
    .map_err(|why| Failure::failed(why.to_string()))?
    .map_err(Failure::failed)
}

fn query<T>(asked: Result<Query<T>, QueryRejection>) -> Result<T, Failure> {
  asked
    .map(|Query(query)| query)
    .map_err(|rejection| Failure::bad_query(&rejection.body_text()))
}

fn key(asked: Result<Path<i64>, PathRejection>) -> Result<i64, Failure> {
  asked
    .map(|Path(key)| key)
    .map_err(|rejection| Failure::bad_query(&rejection.body_text()))
}

/// The service's routes over what it stands on.
pub fn router(ground: Arc<Ground>) -> Router {
  Router::new()
    .route("/health", get(health))
    .route("/events", get(events::events))
    .route("/queues", get(queues::queues))
    .route("/test-runs", get(test_runs))
    .route("/test-runs/{testRunId}", get(test_run))
    .route("/deployments", get(deployments))
    .route("/deployments/{deploymentId}", get(deployment))
    .route("/deployments/{deploymentId}/log-lines", get(log_lines))
    .route("/restarts", get(restarts))
    .fallback(no_route)
    .method_not_allowed_fallback(no_route)
    .layer(middleware::from_fn_with_state(Arc::clone(&ground), bearer))
    .with_state(ground)
}

// Every route takes the token, the health and a path no route answers among them.
async fn bearer(
  State(ground): State<Arc<Ground>>,
  headers: HeaderMap,
  request: Request,
  next: Next,
) -> Response {
  let given = headers
    .get(header::AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("Bearer "));

  if given != Some(ground.token.as_str()) {
    return Failure::new(ErrorCode::Unauthorized, SERVICE.errors.unauthorized.clone()).into_response();
  }

  next.run(request).await
}

async fn no_route(request: Request) -> Failure {
  Failure::new(
    ErrorCode::NoRoute,
    fill_marks(
      &SERVICE.errors.no_route,
      &[("method", request.method().as_str()), ("path", request.uri().path())],
    ),
  )
}

async fn health(State(ground): State<Arc<Ground>>) -> Json<Health> {
  let opened = reading(&ground, |_| Ok(())).await;

  Json(Health {
    ready: opened.is_ok(),
    reason: opened
      .err()
      .map(|failure| fill_marks(&SERVICE.errors.not_ready, &[("why", &failure.error)])),
    version: env!("CARGO_PKG_VERSION").to_owned(),
  })
}

async fn test_runs(
  State(ground): State<Arc<Ground>>,
  asked: Result<Query<TestRunsQuery>, QueryRejection>,
) -> Answer<impl Serialize> {
  let asked = query(asked)?;

  reading(&ground, move |record| record.test_runs(&asked)).await.map(Json)
}

async fn test_run(
  State(ground): State<Arc<Ground>>,
  asked: Result<Path<i64>, PathRejection>,
) -> Answer<impl Serialize> {
  let id = key(asked)?;

  reading(&ground, move |record| record.test_run(id))
    .await?
    .map(Json)
    .ok_or_else(|| Failure::not_found("test run", id))
}

async fn deployments(
  State(ground): State<Arc<Ground>>,
  asked: Result<Query<PageQuery>, QueryRejection>,
) -> Answer<impl Serialize> {
  let asked = query(asked)?;

  reading(&ground, move |record| record.deployments(&asked)).await.map(Json)
}

async fn deployment(
  State(ground): State<Arc<Ground>>,
  asked: Result<Path<i64>, PathRejection>,
) -> Answer<impl Serialize> {
  let id = key(asked)?;

  reading(&ground, move |record| record.deployment(id))
    .await?
    .map(Json)
    .ok_or_else(|| Failure::not_found("deployment", id))
}

async fn log_lines(
  State(ground): State<Arc<Ground>>,
  id: Result<Path<i64>, PathRejection>,
  asked: Result<Query<PageQuery>, QueryRejection>,
) -> Answer<impl Serialize> {
  let id = key(id)?;
  let asked = query(asked)?;

  reading(&ground, move |record| record.log_lines(id, &asked))
    .await?
    .map(Json)
    .ok_or_else(|| Failure::not_found("deployment", id))
}

async fn restarts(
  State(ground): State<Arc<Ground>>,
  asked: Result<Query<RestartsQuery>, QueryRejection>,
) -> Answer<impl Serialize> {
  let asked = query(asked)?;

  reading(&ground, move |record| record.restarts(&asked)).await.map(Json)
}
