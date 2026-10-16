//! The testbed's core: what every verb of the testbed stands on (docs/testbed.md). The ticket-queue lock
//! every lane is held through and the lanes' turns taken on it, the repository's close line, the
//! languages a command is run and read by, one run and its result, the memory a run is given again for,
//! the verb over them, and the policy every number and word of theirs is read from.

pub mod close_line;
pub mod declaration;
pub mod languages;
pub mod memory;
pub mod policy;
pub mod queue;
pub mod run;
pub mod turn;
pub mod verb;
