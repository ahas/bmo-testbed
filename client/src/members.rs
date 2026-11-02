//! The members the HTTP service answers (docs/boundary.md, 「Members」), each with one shape wherever it is
//! answered: alone, in a page, inside an event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum::{EnumString, IntoStaticStr, VariantArray};

/// A page of a list: the items, and whether anything stands past the last of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "{T}Page")]
pub struct Page<T> {
  pub items: Vec<T>,
  pub more: bool,
}

/// Whether the service can answer, and its version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Health {
  pub ready: bool,
  /// Why it is not ready; null when it is.
  pub reason: Option<String>,
  pub version: String,
}

/// The kinds of event the stream carries, each a resource and its change.
#[derive(
  Debug,
  Clone,
  Copy,
  PartialEq,
  Eq,
  Hash,
  Serialize,
  Deserialize,
  JsonSchema,
  VariantArray,
  IntoStaticStr,
  EnumString,
)]
pub enum EventKind {
  #[serde(rename = "queue.changed")]
  #[strum(serialize = "queue.changed")]
  QueueChanged,
  #[serde(rename = "test-run.created")]
  #[strum(serialize = "test-run.created")]
  TestRunCreated,
  #[serde(rename = "test-run.ended")]
  #[strum(serialize = "test-run.ended")]
  TestRunEnded,
  #[serde(rename = "deployment.created")]
  #[strum(serialize = "deployment.created")]
  DeploymentCreated,
  #[serde(rename = "deployment.changed")]
  #[strum(serialize = "deployment.changed")]
  DeploymentChanged,
  #[serde(rename = "restart.created")]
  #[strum(serialize = "restart.created")]
  RestartCreated,
}

impl EventKind {
  /// The kind as the stream names it.
  pub fn name(self) -> &'static str {
    self.into()
  }
}

/// One event of the stream: its number, its kind, when, and the member as a `GET` answered it then.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
  pub id: i64,
  pub kind: EventKind,
  /// Epoch milliseconds.
  pub at: i64,
  #[schemars(with = "serde_json::Map<String, Value>")]
  pub data: Value,
}

/// The two lanes, by their own names.
#[derive(
  Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, VariantArray, IntoStaticStr,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum LaneName {
  Wide,
  Narrow,
}

/// One lane's line: who holds its turn and who waits behind, in the order they will be served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
  pub name: LaneName,
  /// How many threads its holder runs on.
  pub width: i64,
  pub holder: Option<QueueEntry>,
  pub waiting: Vec<QueueEntry>,
}

/// Who stands in a lane's line, or holds its turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
  /// The caller's opaque scope, which the testbed never reads.
  pub scope: Option<String>,
  /// The checkout it was started in.
  pub checkout: String,
  /// The command, as one line.
  pub command: String,
  pub pid: i64,
  /// When it asked for the turn, epoch milliseconds.
  pub asked_at: i64,
  /// When the turn came; null while it waits.
  pub held_at: Option<i64>,
  /// Always false: a waiter waits its turn out, and no place is kept for one that left.
  pub kept: bool,
  /// Standing at the front.
  pub front: bool,
  /// Standing next after the front.
  pub next: bool,
}

/// What a test run is a run of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, VariantArray)]
#[serde(rename_all = "kebab-case")]
pub enum TestRunKind {
  /// A run of the `suite` verb.
  Suite,
  /// One heavy command of the `run` verb.
  Command,
}

/// Where a test run stands in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, VariantArray)]
#[serde(rename_all = "kebab-case")]
pub enum TestRunState {
  Running,
  Passed,
  Failed,
  NotRun,
  /// Its process went without ending it.
  Gone,
}

/// One test run and its counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TestRun {
  pub id: i64,
  pub kind: TestRunKind,
  pub checkout: String,
  pub repository_name: Option<String>,
  pub commit: Option<String>,
  /// The words run, as one line.
  pub command: String,
  pub scope: Option<String>,
  /// The unit it runs in, where it runs in one.
  pub unit_name: Option<String>,
  pub pid: i64,
  pub state: TestRunState,
  pub passed_count: i64,
  pub failed_count: i64,
  pub skipped_count: i64,
  pub log_path: String,
  pub started_at: i64,
  pub ended_at: Option<i64>,
}

/// The stages a deployment passes, in the order it passes them; a rollback is never planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, VariantArray)]
#[serde(rename_all = "kebab-case")]
pub enum StageName {
  Check,
  Backup,
  Tree,
  Build,
  Before,
  Restart,
  StayUp,
  After,
  Rollback,
}

/// The parts of the backup stage written as steps of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, VariantArray)]
#[serde(rename_all = "kebab-case")]
pub enum StepPart {
  Snapshot,
  Pack,
  Check,
}

/// Where a deployment stands in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, VariantArray)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentState {
  Running,
  Live,
  RolledBack,
  Failed,
  Stopped,
  /// Its process went without ending it.
  Gone,
}

/// Why a deployment did not go live: the stage it stopped in, or a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, VariantArray)]
#[serde(rename_all = "kebab-case")]
pub enum StopCode {
  Check,
  Backup,
  Tree,
  Build,
  Before,
  Restart,
  StayUp,
  Refused,
}

/// One stage a deployment reached, with when.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentStage {
  pub stage: StageName,
  pub started_at: i64,
}

/// Where a deployment step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, VariantArray)]
#[serde(rename_all = "kebab-case")]
pub enum StepState {
  Ahead,
  Running,
  Ended,
}

/// One heavy step of a deployment: a check, the suite, a build, a part of the backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentStep {
  /// Its declared English id.
  pub name: String,
  pub stage: Option<StageName>,
  pub part: Option<StepPart>,
  pub state: StepState,
  pub exit_code: Option<i64>,
  pub waited_ms: Option<i64>,
  pub spent_ms: Option<i64>,
  pub started_at: Option<i64>,
  pub ended_at: Option<i64>,
}

/// One deployment: its stages, its steps and why it stopped where it did not go live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
  pub id: i64,
  pub repository_name: Option<String>,
  /// The deployment's unit, whose journal is its log.
  pub unit_name: Option<String>,
  pub pid: i64,
  pub commit: Option<String>,
  pub state: DeploymentState,
  /// The stages it means to pass, in order.
  pub plan: Vec<StageName>,
  pub stages: Vec<DeploymentStage>,
  pub steps: Vec<DeploymentStep>,
  pub rolled_back: bool,
  pub stopped_for: Option<StopCode>,
  /// The sentence beside its outcome.
  pub reason: Option<String>,
  pub started_at: i64,
  pub ended_at: Option<i64>,
}

/// One line a deployment said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogLine {
  pub id: i64,
  /// Epoch milliseconds.
  pub at: i64,
  pub text: String,
}

/// Why a deployment stopped a unit, for the unit to read as it starts again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Restart {
  pub id: i64,
  pub unit_name: String,
  pub deployment_id: i64,
  pub reason: String,
  /// Epoch milliseconds.
  pub at: i64,
}

/// What a run, a check or a suite came to, as its verb prints it with `--json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandRun {
  pub exit_code: i64,
  pub log_path: String,
  pub waited_ms: i64,
  pub spent_ms: i64,
  /// The test run it wrote; null for a check, and for a run placed elsewhere.
  pub test_run_id: Option<i64>,
}
