//! `bmo-testbed`: the testbed's one binary (docs/boundary.md, 「The binary」).

use std::process::ExitCode;

fn main() -> ExitCode {
  ExitCode::from(bmo_testbed_cli::main(std::env::args_os()))
}
