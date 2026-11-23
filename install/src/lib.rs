//! The install verb (docs/install.md): a preset of an index's presets file resolved into the repositories it
//! clones and installs, each repository's own install declaration read, and the steps that bring the machine to
//! them run in order, each printed as one JSON line.

pub mod declaration;
pub mod line;
pub mod machine;
pub mod presets;
pub mod run;
pub mod template;

pub use line::{Line, Status};
pub use machine::{Host, Machine, Ran};
pub use run::{Ask, Install, Mode, Services, exit_code};
