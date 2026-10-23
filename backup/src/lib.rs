//! The data backup (docs/backup.md): a machine's databases dumped and its data folders packed every hour,
//! its media every week, each sent to the destinations its settings name and rotated there, and a drill
//! that restores the newest copy and counts it. What is backed up is the settings file's alone.

pub mod destination;
pub mod drill;
pub mod dump;
pub mod manifest;
pub mod media;
pub mod outcome;
pub mod pack;
pub mod policy;
pub mod rotation;
pub mod settings;
pub mod snapshot;
