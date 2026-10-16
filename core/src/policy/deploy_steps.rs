//! The name each of the testbed's own heavy steps of a deployment and its gate is written under
//! (docs/deploy.md, 「What a deployment says of itself as it runs」). A repository's declared steps are
//! named by their own ids and are not among these.

use std::collections::BTreeMap;

use serde::Deserialize;

/// One of the testbed's own heavy steps, keyed as the file keys it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeployStep {
  Turn,
  Quick,
  CargoFormat,
  CargoLint,
  Tests,
  BackupSnapshot,
  BackupPack,
  BackupCheck,
  CargoRelease,
}

impl DeployStep {
  /// Every step, in the order a deployment can pass them.
  pub const ALL: [Self; 9] = [
    Self::Turn,
    Self::Quick,
    Self::CargoFormat,
    Self::CargoLint,
    Self::Tests,
    Self::BackupSnapshot,
    Self::BackupPack,
    Self::BackupCheck,
    Self::CargoRelease,
  ];

  /// The name the step is written under.
  pub fn name(self) -> &'static str {
    &DEPLOY_STEPS[&self].name
  }
}

/// One step's name.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Named {
  pub name: String,
}

super::part!(pub DEPLOY_STEPS: BTreeMap<DeployStep, Named> = "deploy-steps.toml");
