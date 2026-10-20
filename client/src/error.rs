//! The error shape every route answers its failures in, and the codes it names.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

/// The codes the service answers with, each meaning one thing on every route.
#[derive(
  Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, VariantArray, IntoStaticStr,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ErrorCode {
  /// 400: the query is not the shape the route takes.
  BadRequest,
  /// 401: the token is missing or wrong.
  Unauthorized,
  /// 404: no route answers this method and path.
  NoRoute,
  /// 404: nothing stands at the key the path names.
  NotFound,
  /// 410: the events asked for are past the window.
  Gone,
  /// 500: the service failed.
  Failed,
}

impl ErrorCode {
  /// The HTTP status the code is answered with.
  pub fn status(self) -> u16 {
    match self {
      Self::BadRequest => 400,
      Self::Unauthorized => 401,
      Self::NoRoute | Self::NotFound => 404,
      Self::Gone => 410,
      Self::Failed => 500,
    }
  }

  /// The code as the error shape writes it.
  pub fn name(self) -> &'static str {
    self.into()
  }
}

/// A failure: its code, and an English sentence saying what is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireError {
  pub code: ErrorCode,
  pub error: String,
}
