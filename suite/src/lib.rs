//! The suite of a repository (docs/suite.md): which units a change reaches, read off the checkout's package
//! files, its Rust workspace and its TypeScript imports against its declaration; and a run of them, each unit
//! in a process of its own under a guard of its own, inside the machine's turn and a fence of its own.

pub mod cargo;
pub mod check;
pub mod count;
pub mod crates;
pub mod dependents;
pub mod fence;
pub mod files;
pub mod imports;
pub mod log;
pub mod moved_imports;
pub mod once_more;
pub mod own_binary;
pub mod prune;
pub mod quick;
pub mod run;
pub mod selection;
pub mod static_check;
pub mod superseded;
pub mod sweep;
pub mod template;
pub mod test_server;
pub mod test_targets;
pub mod titles;
pub mod tool_checks;
pub mod touched;
pub mod unit;
pub mod workspace;
