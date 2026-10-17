//! The languages a command is run and read by (docs/testbed.md, 「Languages」): what each claims, how it
//! widens a command, what it reads out of a log, and how it names, runs and checks what the suite hands it.

use std::collections::BTreeMap;
use std::path::Path;

use bmo_testbed_core::languages::{
  Browser, Bun, Cargo, LANGUAGES, Language, Node, PLAIN, Tally, failures_of_log, language_for,
  program_words, run_folder, tally_of_log,
};

fn words(line: &[&str]) -> Vec<String> {
  line.iter().map(|word| (*word).to_owned()).collect()
}

fn claims(language: &dyn Language, program: &str, args: &[&str]) -> bool {
  language.claims(program, &words(args))
}

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
  pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
}

fn lines(text: &[&str]) -> String {
  text.join("\n")
}

fn tally(pass: u64, fail: u64, skip: u64) -> Option<Tally> {
  Some(Tally { pass, fail, skip })
}

// "a language is named once in the list of members beside it": the one list every reader asks.
#[test]
fn every_language_is_one_member_of_the_one_list_in_the_order_of_their_names() {
  assert_eq!(LANGUAGES.map(|one| one.name()), ["browser", "bun", "cargo", "node"]);
}

// "run a command by the first that claims it, the one named, or the plain one"
#[test]
fn a_command_runs_by_the_first_language_that_claims_it_the_one_named_or_the_plain_one() {
  let name = |command: &[&str], named: Option<&str>| {
    language_for(&LANGUAGES, &words(command), named).map(|one| one.name())
  };

  assert_eq!(name(&["env", "RUST_LOG=1", "cargo", "test"], None), Ok("cargo"));
  assert_eq!(name(&["make", "all"], None), Ok(PLAIN.name()));
  assert_eq!(name(&["make", "check"], Some("cargo")), Ok("cargo"));
  assert_eq!(name(&["make"], Some("cobol")), Err(vec!["browser", "bun", "cargo", "node", "plain"]));
}

// "read a command's program past assignments and env"
#[test]
fn a_command_s_program_is_read_past_assignments_and_env() {
  assert_eq!(
    program_words(&words(&["FOO=1", "/usr/bin/env", "/usr/local/bin/cargo", "build"])),
    Some(("cargo".to_owned(), words(&["build"])))
  );
  assert_eq!(program_words(&words(&["FOO=1"])), None);
}

// A package script prints whatever its runners print, Cargo's results beside bun's.
// "read one log together, every language's summary summed"
#[test]
fn one_log_is_read_together_every_language_s_summary_summed() {
  let log = lines(&[
    " 3 pass",
    " 1 fail",
    "(fail) adds",
    "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out",
  ]);

  assert_eq!(tally_of_log(&LANGUAGES, &log), tally(5, 1, 0));
  assert_eq!(failures_of_log(&LANGUAGES, &log), ["adds"]);
  assert_eq!(tally_of_log(&LANGUAGES, "built"), None);
}

// "claims Playwright's runner and a bun test over a browser file"
#[test]
fn browser_claims_playwright_s_runner_and_a_bun_test_over_a_browser_file() {
  assert!(claims(&Browser, "npx", &["playwright", "test"]));
  assert!(claims(&Browser, "pnpm", &["exec", "playwright", "test", "e2e/"]));
  assert!(claims(&Browser, "bun", &["test", "client/browser/live.test.ts"]));
  assert!(!claims(&Browser, "npx", &["playwright", "install"]));
  assert!(!claims(&Browser, "bun", &["test", "server/a.test.ts"]));
}

// "hands Playwright a quarter of the width, at least one, unless the command names its own"
#[test]
fn browser_hands_playwright_a_quarter_of_the_width_unless_the_command_names_its_own() {
  let widened = |command: &[&str], width: usize| Browser.widen(&words(command), width).command;

  assert_eq!(
    widened(&["npx", "playwright", "test"], 24),
    words(&["npx", "playwright", "test", "--workers=6"])
  );
  assert_eq!(
    widened(&["npx", "playwright", "test"], 2),
    words(&["npx", "playwright", "test", "--workers=1"])
  );
  assert_eq!(
    widened(&["npx", "playwright", "test", "--workers=2"], 24),
    words(&["npx", "playwright", "test", "--workers=2"])
  );
  assert_eq!(
    widened(&["bun", "test", "client/browser/a.test.ts"], 24),
    words(&["bun", "test", "client/browser/a.test.ts"])
  );
}

// "reads Playwright's closing counts and its failed tests"
#[test]
fn browser_reads_playwright_s_closing_counts_and_its_failed_tests() {
  let log = lines(&[
    "  1) [chromium] › login.spec.ts:3:1 › signs in",
    "",
    "  1 failed",
    "  2 skipped",
    "  9 passed (12.3s)",
  ]);

  assert_eq!(Browser.tally(&log), tally(9, 1, 2));
  assert_eq!(Browser.failures(&log), ["[chromium] › login.spec.ts:3:1 › signs in"]);
}

// "claims a run over several files, a directory or a name, never one test file or a browser file"
#[test]
fn bun_claims_a_run_over_several_files_never_one_test_file_or_a_browser_file() {
  assert!(claims(&Bun, "bun", &["test"]));
  assert!(claims(&Bun, "bun", &["test", "--parallel", "a.test.ts", "b.test.ts"]));
  assert!(!claims(&Bun, "bun", &["test", "--timeout", "9000", "a.test.ts"]));
  assert!(claims(&Bun, "bun", &["test", "testbed/testbed/"]));
  assert!(claims(&Bun, "bun", &["test", "suite-run"]));
  assert!(!claims(&Bun, "bun", &["test", "client/browser/x.test.ts", "a.test.ts"]));
  assert!(!claims(&Bun, "bun", &["run", "test"]));
  assert!(!claims(&Bun, "node", &["test"]));
}

// "hands its parallel workers the width, leaving a number already named"
#[test]
fn bun_hands_its_parallel_workers_the_width_leaving_a_number_already_named() {
  let widened = Bun.widen(&words(&["bun", "test", "--parallel", "a", "b"]), 24);

  assert_eq!(widened.command, words(&["bun", "test", "--parallel=24", "a", "b"]));
  assert_eq!(widened.env, env(&[("BMO_TEST_PARALLEL", "24")]));
  assert_eq!(
    Bun.widen(&words(&["bun", "test", "--parallel=4"]), 24).command,
    words(&["bun", "test", "--parallel=4"])
  );
}

// "sums every summary block and names the failed tests"
#[test]
fn bun_sums_every_summary_block_and_names_the_failed_tests() {
  let log = lines(&[
    "(fail) one > two [1.20ms]",
    " 3 pass",
    " 1 skip",
    " 1 fail",
    "",
    " 1,002 pass",
    " 0 fail",
  ]);

  assert_eq!(Bun.tally(&log), tally(1005, 1, 1));
  assert_eq!(Bun.failures(&log), ["one > two"]);
  assert_eq!(Bun.tally("nothing here"), None);
}

// "claims what compiles, past a toolchain and the options before it, and nothing that only reads"
#[test]
fn cargo_claims_what_compiles_and_nothing_that_only_reads() {
  assert!(claims(&Cargo, "cargo", &["test", "--workspace"]));
  assert!(claims(&Cargo, "cargo", &["+nightly", "--locked", "clippy"]));
  assert!(claims(&Cargo, "cargo", &["b"]));
  assert!(!claims(&Cargo, "cargo", &["fmt", "--check"]));
  assert!(!claims(&Cargo, "cargo", &["metadata"]));
  assert!(!claims(&Cargo, "cargo", &["run"]));
}

// "hands Cargo the width as its jobs, unless the command names its own"
#[test]
fn cargo_is_handed_the_width_as_its_jobs_unless_the_command_names_its_own() {
  let widened = Cargo.widen(&words(&["cargo", "build"]), 24);

  assert_eq!(widened.command, words(&["cargo", "build"]));
  assert_eq!(widened.env, env(&[("CARGO_BUILD_JOBS", "24")]));
  assert!(Cargo.widen(&words(&["cargo", "build", "-j", "2"]), 24).env.is_empty());
  assert!(Cargo.widen(&words(&["cargo", "build", "--jobs=2"]), 24).env.is_empty());
}

// "sums every test binary's result, and names failed tests and compiler errors with their place"
#[test]
fn cargo_sums_every_test_binary_and_names_failed_tests_and_compiler_errors_with_their_place() {
  let log = lines(&[
    "test result: ok. 4 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out",
    "test parse::reads_lines ... FAILED",
    "test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out",
    "error[E0308]: mismatched types",
    "  --> crates/a/src/lib.rs:3:5",
    "error: could not compile `a` due to 1 previous error",
    "error: expect(received).toBe(expected)",
  ]);

  assert_eq!(Cargo.tally(&log), tally(6, 1, 1));
  assert_eq!(
    Cargo.failures(&log),
    ["parse::reads_lines", "error[E0308]: mismatched types (crates/a/src/lib.rs:3:5)"]
  );
  assert_eq!(Cargo.tally("   Compiling a v0.1.0"), None);
}

// "claims package scripts that test, check or build, node --test, Vitest and Jest"
#[test]
fn node_claims_package_scripts_that_test_check_or_build_node_test_vitest_and_jest() {
  assert!(claims(&Node, "npm", &["test"]));
  assert!(claims(&Node, "npm", &["--prefix", "web", "run", "build"]));
  assert!(claims(&Node, "pnpm", &["-C", "client", "test:unit"]));
  assert!(claims(&Node, "yarn", &["check"]));
  assert!(claims(&Node, "bun", &["run", "check:hub"]));
  assert!(claims(&Node, "node", &["--test", "test/"]));
  assert!(claims(&Node, "npx", &["vitest", "run"]));
  assert!(claims(&Node, "pnpm", &["exec", "jest"]));
  assert!(claims(&Node, "jest", &[]));
}

// "leaves light scripts and bun's own test runner alone"
#[test]
fn node_leaves_light_scripts_and_bun_s_own_test_runner_alone() {
  assert!(!claims(&Node, "npm", &["run", "lint"]));
  assert!(!claims(&Node, "npm", &["install"]));
  assert!(!claims(&Node, "bun", &["run", "typecheck"]));
  assert!(!claims(&Node, "bun", &["test"]));
  assert!(!claims(&Node, "node", &["server.js"]));
  assert!(!claims(&Node, "npx", &["tsc"]));
}

// "hands each runner the width its own way, leaving a number already named"
#[test]
fn node_hands_each_runner_the_width_its_own_way_leaving_a_number_already_named() {
  let handed = env(&[("BMO_TEST_PARALLEL", "24"), ("CARGO_BUILD_JOBS", "24")]);
  let node = Node.widen(&words(&["node", "--test"]), 24);
  let npm = Node.widen(&words(&["npm", "test"]), 24);

  assert_eq!(node.command, words(&["node", "--test", "--test-concurrency=24"]));
  assert_eq!(node.env, handed);
  assert_eq!(
    Node.widen(&words(&["npx", "vitest", "run"]), 24).command,
    words(&["npx", "vitest", "run", "--maxWorkers=24"])
  );
  assert_eq!(
    Node.widen(&words(&["npx", "jest", "--maxWorkers=2"]), 24).command,
    words(&["npx", "jest", "--maxWorkers=2"])
  );
  assert_eq!(npm.command, words(&["npm", "test"]));
  assert_eq!(npm.env, handed);
}

// "reads node's, Vitest's and Jest's summaries and failures"
#[test]
fn node_reads_node_s_vitest_s_and_jest_s_summaries_and_failures() {
  assert_eq!(
    Node.tally(&lines(&["# pass 5", "# fail 1", "# skipped 2", "not ok 3 - reads a line"])),
    tally(5, 1, 2)
  );
  assert_eq!(Node.tally("      Tests  2 failed | 7 passed (9)"), tally(7, 2, 0));
  assert_eq!(Node.tally("Tests:       1 failed, 5 passed, 6 total"), tally(5, 1, 0));
  assert_eq!(
    Node.failures(&lines(&["not ok 3 - reads a line", " FAIL  src/a.test.ts > adds", "  ● sums > two"])),
    ["reads a line", "src/a.test.ts > adds", "sums > two"]
  );
  assert_eq!(Node.tally("built in 3s"), None);
}

// "a file language names its test files, runs them handed the per-test limit its own way unless the caller
// names one, and runs a project's script"
#[test]
fn a_file_language_names_runs_and_checks_its_own() {
  let one = words(&["a.test.ts"]);
  let two = words(&["a.test.ts", "b.test.ts"]);

  assert_eq!(Bun.test_naming(), ["*.test.ts", "**/*.test.ts"]);
  assert_eq!(
    Bun.test_command(&one, &words(&["--bail"]), Some(30)),
    Some(words(&["bun", "test", "--bail", "--timeout", "30000", "./a.test.ts"]))
  );
  assert_eq!(
    Bun.test_command(&one, &words(&["--timeout=5000"]), Some(30)),
    Some(words(&["bun", "test", "--timeout=5000", "./a.test.ts"]))
  );
  assert_eq!(
    Bun.test_command(&two, &[], None),
    Some(words(&["bun", "test", "--parallel", "./a.test.ts", "./b.test.ts"]))
  );
  assert_eq!(Bun.script_command("check"), Some(words(&["bun", "run", "check"])));

  let js = words(&["a.test.mjs"]);

  assert!(Node.test_naming().contains(&"**/*.test.mjs"));
  assert_eq!(
    Node.test_command(&js, &[], Some(30)),
    Some(words(&["node", "--test", "--test-timeout=30000", "a.test.mjs"]))
  );
  assert_eq!(
    Node.test_command(&js, &words(&["--test-timeout", "100"]), Some(30)),
    Some(words(&["node", "--test", "--test-timeout", "100", "a.test.mjs"]))
  );
  assert_eq!(Node.script_command("check"), Some(words(&["npm", "run", "check"])));

  // A browser file is a bun file, its files declared rather than named; Cargo and the plain language run none.
  assert_eq!(Browser.test_command(&one, &[], Some(30)), Bun.test_command(&one, &[], Some(30)));
  assert!(Browser.test_naming().is_empty() && Browser.script_command("check").is_none());

  for language in [&Cargo as &dyn Language, PLAIN] {
    assert!(language.test_naming().is_empty(), "{}", language.name());
    assert_eq!(language.test_command(&one, &[], None), None, "{}", language.name());
    assert_eq!(language.script_command("check"), None, "{}", language.name());
  }
}

// "bun hands each file as a path, since any other word is a filter it matches within every file's path"
#[test]
fn bun_hands_each_file_as_a_path_and_leaves_one_already_written_as_one() {
  assert_eq!(
    Bun.test_command(
      &words(&["unit/a.test.ts", "./b.test.ts", "../c.test.ts", "/abs/d.test.ts"]),
      &[],
      None
    ),
    Some(words(&[
      "bun",
      "test",
      "--parallel",
      "./unit/a.test.ts",
      "./b.test.ts",
      "../c.test.ts",
      "/abs/d.test.ts"
    ]))
  );
}

// "bun reads bunfig.toml, a browser file reads bun's, and the others read none"
#[test]
fn bun_and_the_browser_name_bun_s_configuration_and_the_others_none() {
  assert_eq!(Bun.configuration_file(), Some("bunfig.toml"));
  assert_eq!(Browser.configuration_file(), Some("bunfig.toml"));

  for language in [&Cargo as &dyn Language, &Node, PLAIN] {
    assert_eq!(language.configuration_file(), None, "{}", language.name());
  }
}

// "a language of a package workspace names its root configuration and its lockfile; the others name none"
#[test]
fn a_package_workspace_s_language_names_its_root_configuration_and_its_lockfile() {
  assert_eq!(Bun.root_configuration(), ["package.json", "bun.lock", "bunfig.toml", "tsconfig.json"]);
  assert_eq!(Bun.lockfile(), Some("bun.lock"));
  assert_eq!(Node.root_configuration(), ["package.json", "package-lock.json", "tsconfig.json"]);
  assert_eq!(Node.lockfile(), Some("package-lock.json"));

  for language in [&Browser as &dyn Language, &Cargo, PLAIN] {
    assert!(language.root_configuration().is_empty(), "{}", language.name());
    assert_eq!(language.lockfile(), None, "{}", language.name());
  }
}

// "a test file runs from the nearest folder at or above it inside the repository holding its language's
// configuration, else from the root"
#[test]
fn a_test_file_runs_from_the_nearest_folder_holding_its_configuration_else_the_root() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path().join("repo");

  for folder in ["tests/ts", "tests/ts/deep/own", "tests/rs"] {
    std::fs::create_dir_all(root.join(folder)).unwrap();
  }

  std::fs::write(root.join("tests/ts/bunfig.toml"), "").unwrap();
  std::fs::write(root.join("tests/ts/deep/own/bunfig.toml"), "").unwrap();
  // Above the repository, which no file of it runs from.
  std::fs::write(dir.path().join("bunfig.toml"), "").unwrap();

  let folder = |language: &dyn Language, file: &str| run_folder(language, &root, file);

  assert_eq!(folder(&Bun, "tests/ts/a.test.ts"), Path::new("tests/ts"));
  assert_eq!(folder(&Bun, "tests/ts/deep/b.test.ts"), Path::new("tests/ts"));
  assert_eq!(folder(&Bun, "tests/ts/deep/own/c.test.ts"), Path::new("tests/ts/deep/own"));
  assert_eq!(folder(&Browser, "tests/ts/browser/d.test.ts"), Path::new("tests/ts"));
  assert_eq!(folder(&Bun, "tests/rs/e.test.ts"), Path::new(""));
  assert_eq!(folder(&Bun, "../f.test.ts"), Path::new(""));
  assert_eq!(folder(&Node, "tests/ts/g.test.mjs"), Path::new(""));

  // The root's own configuration is the root.
  std::fs::write(root.join("bunfig.toml"), "").unwrap();
  assert_eq!(folder(&Bun, "tests/rs/e.test.ts"), Path::new(""));
}
