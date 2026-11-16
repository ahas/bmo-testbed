//! What Cargo's runner writes down of a test binary and how the record reads back (docs/suite.md, 「Heavy units
//! run in a lane of one」; docs/boundary.md, 「crate-tests」). Running the binaries is the suite verb's test.

use std::path::Path;

use bmo_testbed_suite::crate_tests::{TestBinary, read_record, runner_config, write_down};

fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
  pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
}

// "the variables Cargo hands it, `CARGO`, `CARGO_MANIFEST_DIR`, `CARGO_MANIFEST_PATH`, the package's `CARGO_PKG_`
// values and the library path": of the runner's whole environment, those alone are kept.
#[test]
fn a_binary_keeps_only_the_variables_cargo_sets() {
  let seen = TestBinary::seen(
    "/build/deps/sample-0abc".to_owned(),
    vec!["--exact".to_owned()],
    "/repo/sample".to_owned(),
    owned(&[
      ("CARGO", "/toolchain/cargo"),
      ("CARGO_MANIFEST_DIR", "/repo/sample"),
      ("CARGO_PKG_NAME", "sample"),
      ("LD_LIBRARY_PATH", "/build/deps"),
      ("CARGO_HOME", "/cargo-home"),
      ("HOME", "/test-home"),
      ("PATH", "/usr/bin"),
    ]),
  );

  assert_eq!(seen.package(), Some("sample"));
  assert_eq!(
    seen.env.keys().map(String::as_str).collect::<Vec<_>>(),
    ["CARGO", "CARGO_MANIFEST_DIR", "CARGO_PKG_NAME", "LD_LIBRARY_PATH"]
  );
}

// "writes down ... into a record of the run's own": the binaries read back whole, in the order written.
#[test]
fn the_record_reads_back_in_the_order_it_was_written() {
  let dir = tempfile::tempdir().unwrap();
  let record = dir.path().join("record.jsonl");
  let binary = |name: &str| {
    TestBinary::seen(
      format!("/build/deps/{name}"),
      Vec::new(),
      format!("/repo/{name}"),
      owned(&[("CARGO_PKG_NAME", name)]),
    )
  };
  let written = [binary("first"), binary("second")];

  for one in &written {
    write_down(&record, one).unwrap();
  }

  assert_eq!(read_record(&record).unwrap(), written);
  assert!(read_record(&dir.path().join("absent.jsonl")).is_err());
}

// "with a runner of the testbed's own (`crate-tests record`)": the words Cargo reads as its configuration name the
// runner for any target, the record its last word.
#[test]
fn the_runner_configuration_names_the_record_verb_for_any_target() {
  let config = runner_config(Path::new("/bin dir/bmo-testbed"), Path::new("/tmp/record.jsonl"));
  let read: toml::Table = toml::from_str(&config).unwrap();

  assert_eq!(
    read["target"]["cfg(all())"]["runner"],
    toml::Value::Array(
      ["/bin dir/bmo-testbed", "crate-tests", "record", "/tmp/record.jsonl"]
        .map(|word| toml::Value::String(word.to_owned()))
        .to_vec()
    )
  );
}
