//! The hosted deploy tool's executable: one call handed on standard input, answered on standard output
//! (bmo-pocket's `docs/mcp-host.md`, 「A call」).

use std::process::ExitCode;

use deploy_tool::{Ground, answer, read_handed};

fn main() -> ExitCode {
  let said = read_handed(std::io::stdin().lock()).and_then(|handed| answer(&handed, &Ground::of_process()));
  let written = said
    .and_then(|said| serde_json::to_writer(std::io::stdout().lock(), &said).map_err(|why| why.to_string()));

  match written {
    Ok(()) => ExitCode::SUCCESS,
    Err(why) => {
      eprintln!("{why}");
      ExitCode::FAILURE
    }
  }
}
