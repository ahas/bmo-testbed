//! What Cargo's runner writes down of a test binary and how the record reads back (docs/suite.md, 「Heavy units
//! run in a lane of one」; docs/boundary.md, 「crate-tests」). Running the binaries is the suite verb's test.

use std::path::Path;

use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_suite::crate_tests::{
  BinaryOnceMore, TestBinary, binaries_once_more, read_record, runner_config, write_down,
};

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

fn binary(executable: &str, package: &str) -> TestBinary {
  TestBinary::seen(
    executable.to_owned(),
    Vec::new(),
    "/repo/sample".to_owned(),
    owned(&[("CARGO_PKG_NAME", package)]),
  )
}

fn running(executable: &str) -> String {
  fill_shown(&SUITE.crate_tests.running, &[("executable", &executable)])
}

// "each test binary that failed ... A binary whose own result names the tests that failed runs only those ...
// one that ended with no result of its own ... runs whole, as does one that never started ... A binary that
// passed does not run again."
#[test]
fn a_crate_runs_once_more_only_its_failed_binaries_narrowed_to_their_failed_tests() {
  let binaries = [
    binary("/build/deps/passes-0a", "sample"),
    binary("/build/deps/fails-0b", "sample"),
    binary("/build/deps/cut-0c", "sample"),
    binary("/build/deps/never-0d", "sample"),
    binary("/build/deps/other-0e", "other"),
  ];
  let said = [
    running("/build/deps/passes-0a"),
    "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out".to_owned(),
    running("/build/deps/fails-0b"),
    "test store::waits ... FAILED".to_owned(),
    "test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out".to_owned(),
    running("/build/deps/cut-0c"),
    "test store::hangs has been running for over 60 seconds".to_owned(),
  ]
  .join("\n");
  let again = binaries_once_more(&binaries, "sample", &said);
  let mut narrowed = binaries[1].clone();

  narrowed.args.extend(["--exact".to_owned(), "store::waits".to_owned()]);

  assert_eq!(
    again,
    [
      BinaryOnceMore {
        binary: narrowed,
        tests: vec!["store::waits".to_owned()],
      },
      BinaryOnceMore {
        binary: binaries[2].clone(),
        tests: Vec::new(),
      },
      BinaryOnceMore {
        binary: binaries[3].clone(),
        tests: Vec::new(),
      },
    ]
  );
}

// "None where that comes to no fewer than the whole unit ... or to nothing": a crate whose output names nothing
// narrower runs whole.
#[test]
fn a_crate_whose_output_names_nothing_narrower_runs_whole() {
  let binaries = [binary("/build/deps/one-0a", "sample"), binary("/build/deps/two-0b", "sample")];
  let all_passed = [
    running("/build/deps/one-0a"),
    "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out".to_owned(),
    running("/build/deps/two-0b"),
    "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out".to_owned(),
  ]
  .join("\n");

  assert!(binaries_once_more(&binaries, "sample", "the record does not read").is_empty());
  assert!(binaries_once_more(&binaries, "sample", &all_passed).is_empty());
}
