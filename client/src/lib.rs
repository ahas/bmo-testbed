//! The testbed's published client (docs/boundary.md, 「The HTTP service」): every member, page, query and
//! error code the HTTP service answers in, as the types it serialises, and with the default `http` feature
//! the client calling each route.

#[cfg(feature = "http")]
mod client;
mod error;
mod members;
mod queries;

#[cfg(feature = "http")]
pub use client::{ClientFailure, TestbedClient};
pub use error::{ErrorCode, WireError};
pub use members::{
  CommandRun, Deployment, DeploymentStage, DeploymentState, DeploymentStep, Event, EventKind, Health,
  LaneName, LogLine, Page, Queue, QueueEntry, Restart, StageName, StepPart, StepState, StopCode, TestRun,
  TestRunKind, TestRunState,
};
pub use queries::{EventsQuery, PageQuery, RestartsQuery, TestRunsQuery};
