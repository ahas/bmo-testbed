//! Writes the OpenAPI document `docs/openapi.json` keeps on standard output:
//! `cargo run -q -p bmo-testbed-cli --example openapi > docs/openapi.json`.

fn main() {
  print!("{}", bmo_testbed_cli::openapi_document());
}
