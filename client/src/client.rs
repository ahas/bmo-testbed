//! The client calling each route of the HTTP service, behind its bearer token, and reading each answer as
//! its member or as the error shape.

use reqwest::StatusCode;
use reqwest::header::ACCEPT;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{
  Deployment, Event, EventsQuery, Health, LogLine, Page, PageQuery, Queue, Restart, RestartsQuery, TestRun,
  TestRunsQuery, WireError,
};

/// What a call came to short of the route's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientFailure {
  /// Nothing answered at the address.
  Unreached(String),
  /// The service answered a failure: its status, and its error shape where the body read as one.
  Answered {
    status: u16,
    answer: Option<WireError>,
    body: String,
  },
  /// The service answered success with a body that is not the route's member.
  Unread(String),
}

impl std::fmt::Display for ClientFailure {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Unreached(why) => write!(f, "the testbed service could not be reached: {why}"),
      Self::Answered {
        status,
        answer: Some(answer),
        ..
      } => {
        write!(f, "the testbed service answered {status} {}: {}", answer.code.name(), answer.error)
      }
      Self::Answered { status, body, .. } => write!(f, "the testbed service answered {status}: {body}"),
      Self::Unread(why) => write!(f, "the testbed service's answer did not read: {why}"),
    }
  }
}

impl std::error::Error for ClientFailure {}

/// The client of one testbed service: its base address and its token.
#[derive(Debug, Clone)]
pub struct TestbedClient {
  base: String,
  token: String,
  http: reqwest::Client,
}

impl TestbedClient {
  pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
    Self {
      base: base.into().trim_end_matches('/').to_owned(),
      token: token.into(),
      http: reqwest::Client::new(),
    }
  }

  async fn get<Q: Serialize + ?Sized, T: DeserializeOwned>(
    &self,
    path: &str,
    query: &Q,
  ) -> Result<T, ClientFailure> {
    let answer = self
      .http
      .get(format!("{}{path}", self.base))
      .bearer_auth(&self.token)
      .header(ACCEPT, "application/json")
      .query(query)
      .send()
      .await
      .map_err(|why| ClientFailure::Unreached(why.to_string()))?;
    let status = answer.status();
    let body = answer.text().await.map_err(|why| ClientFailure::Unreached(why.to_string()))?;

    if status != StatusCode::OK {
      return Err(ClientFailure::Answered {
        status: status.as_u16(),
        answer: serde_json::from_str(&body).ok(),
        body,
      });
    }

    serde_json::from_str(&body).map_err(|why| ClientFailure::Unread(why.to_string()))
  }

  /// `GET /health`.
  pub async fn health(&self) -> Result<Health, ClientFailure> {
    self.get("/health", &()).await
  }

  /// `GET /events` as a page.
  pub async fn events(&self, query: &EventsQuery) -> Result<Page<Event>, ClientFailure> {
    self.get("/events", query).await
  }

  /// `GET /queues`.
  pub async fn queues(&self, query: &PageQuery) -> Result<Page<Queue>, ClientFailure> {
    self.get("/queues", query).await
  }

  /// `GET /test-runs`.
  pub async fn test_runs(&self, query: &TestRunsQuery) -> Result<Page<TestRun>, ClientFailure> {
    self.get("/test-runs", query).await
  }

  /// `GET /test-runs/{testRunId}`.
  pub async fn test_run(&self, test_run_id: i64) -> Result<TestRun, ClientFailure> {
    self.get(&format!("/test-runs/{test_run_id}"), &()).await
  }

  /// `GET /deployments`.
  pub async fn deployments(&self, query: &PageQuery) -> Result<Page<Deployment>, ClientFailure> {
    self.get("/deployments", query).await
  }

  /// `GET /deployments/{deploymentId}`.
  pub async fn deployment(&self, deployment_id: i64) -> Result<Deployment, ClientFailure> {
    self.get(&format!("/deployments/{deployment_id}"), &()).await
  }

  /// `GET /deployments/{deploymentId}/log-lines`.
  pub async fn log_lines(
    &self,
    deployment_id: i64,
    query: &PageQuery,
  ) -> Result<Page<LogLine>, ClientFailure> {
    self.get(&format!("/deployments/{deployment_id}/log-lines"), query).await
  }

  /// `GET /restarts`.
  pub async fn restarts(&self, query: &RestartsQuery) -> Result<Page<Restart>, ClientFailure> {
    self.get("/restarts", query).await
  }
}
