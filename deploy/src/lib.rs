//! A repository's releases on the machine it runs on (docs/deploy.md): the commands a deployment runs, the
//! guarded removal, the machine's deployment line and a deployment asked again, the deploy tree's record and its unit links, the releases and the link flip, a deploy
//! tree still a directory adopted, the build tree, the transient unit a deploy run stands in, the wait for
//! the service to come up and the rollback where it does not, the probe of a console face, the run's progress record, and the backups
//! with their upload.

pub mod adopt;
pub mod again;
pub mod artifact_deploy;
pub mod backup;
pub mod boot;
pub mod console;
pub mod deployment;
pub mod expansion;
pub mod line;
pub mod pack;
pub mod progress;
pub mod release;
pub mod remove;
pub mod rollback;
pub mod run;
pub mod tree;
pub mod unit;
pub mod upload;
