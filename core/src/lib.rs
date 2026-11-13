//! The testbed's core: what every verb of the testbed stands on (docs/testbed.md). The ticket-queue lock
//! every lane is held through and the lanes' turns taken on it, the repository's close line, the
//! languages a command is run and read by, a Cargo lock's path packages brought to their manifests, one
//! run and its result, where a run is placed, the memory a run is given again for, the watch that ends a
//! held command fallen silent, the witness a run tells its record through, the verb over them, and the
//! policy every number and word of theirs is read from.

pub mod answer;
pub mod cargo_lock;
pub mod close_line;
pub mod declaration;
pub mod events;
pub mod languages;
pub mod memory;
pub mod placement;
pub mod policy;
pub mod queue;
pub mod run;
pub mod silence;
pub mod turn;
pub mod verb;
pub mod witness;
