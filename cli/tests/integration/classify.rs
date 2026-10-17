//! The `classify` verb (docs/boundary.md, 「The verbs」; docs/testbed.md, 「A shell hook sends a job's heavy
//! command to it」): a heavy command found wherever the shell puts it and answered with the line that runs
//! it in the testbed, and a light one, a run of the testbed and a command taking the turn itself let
//! through.

use bmo_testbed_cli::classify::{Heavy, TakesTurn, heavy_command, instead};
use bmo_testbed_core::languages::LANGUAGES;

use crate::stand::{ran, testbed};

fn heavy(line: &str) -> Option<Heavy> {
  heavy_command(line, &LANGUAGES, &[TakesTurn::read("tool cargo").unwrap()])
}

fn words_of(line: &str) -> Option<String> {
  heavy(line).map(|heavy| heavy.command.join(" "))
}

// "is refused with the line that runs it there, wherever the shell puts it"
#[test]
fn a_heavy_command_is_found_wherever_the_shell_puts_it() {
  assert_eq!(words_of("cd hub && cargo test --offline").as_deref(), Some("cargo test --offline"));
  assert_eq!(
    words_of("RUST_LOG=1 timeout 500 cargo clippy --workspace").as_deref(),
    Some("cargo clippy --workspace")
  );
  assert_eq!(words_of("bash -c 'npm test'").as_deref(), Some("npm test"));
  assert_eq!(words_of("echo $(bun run build)").as_deref(), Some("bun run build"));
  assert_eq!(words_of("eval 'cargo build'").as_deref(), Some("cargo build"));
  assert_eq!(words_of("T='cargo test'; $T -p core").as_deref(), Some("cargo test -p core"));
  assert_eq!(words_of("export C=cargo; \"$C\" check").as_deref(), Some("cargo check"));
  assert_eq!(heavy("cargo nextest run").map(|heavy| heavy.language), Some("cargo"));
}

// "is let through once run through the testbed, and a light command is never refused"
#[test]
fn a_light_command_a_run_of_the_testbed_and_one_taking_the_turn_itself_are_let_through() {
  for line in [
    "bmo-testbed run -- cargo test",
    "/deploy/bin/bmo-testbed run --narrow -- cargo test",
    "bun test --parallel server/a.test.ts",
    "bun test --parallel server/sample.test.ts > /tmp/sample.log 2>&1",
    "cargo fmt --all --check && bunx tsc --noEmit",
    "bun scripts/cargo.ts clippy --workspace",
    "tool cargo test -p core",
    "/deploy/bin/tool --quiet cargo test",
    "cat <<'EOF'\ncargo test\nEOF",
    "echo 'open",
  ] {
    assert_eq!(heavy(line), None, "{line}");
  }

  // A command opening some other way is read as any other, and what it hands a shell with it.
  assert!(heavy("tool other; sh -c 'cargo test'").is_some());
}

// "the line to run instead is the testbed's own run, the command's words quoted as the shell reads them"
#[test]
fn the_line_to_run_instead_is_the_testbeds_own_run_of_the_same_words() {
  let line = instead(&Heavy {
    language: "cargo",
    command: ["cargo", "test", "--", "a b"].map(str::to_owned).to_vec(),
  });

  assert!(line.ends_with(" run -- cargo test -- 'a b'"), "{line}");
  assert_eq!(shlex::split(&line).unwrap()[1..], ["run", "--", "cargo", "test", "--", "a b"]);
}

// "answers light with 0, heavy with 1 and the line instead, and the same as JSON"
#[test]
fn the_binary_answers_thin_or_as_json_with_its_code() {
  let dir = tempfile::tempdir().unwrap();
  let classify =
    |words: &[&str]| ran(&mut testbed(dir.path(), dir.path(), &[&["classify"][..], words].concat()));
  let light = classify(&["ls -la"]);
  let thin = classify(&["cd x && cargo test"]);
  let json = classify(&["--json", "cargo test"]);
  let read: serde_json::Value = serde_json::from_str(&json.out).unwrap();
  let taken = classify(&["--takes-turn", "tool cargo", "tool cargo test"]);

  assert_eq!((light.code, light.out.as_str()), (0, "light\n"));
  assert_eq!(thin.code, 1);
  assert!(
    thin.out.starts_with("heavy cargo\n") && thin.out.trim_end().ends_with(" run -- cargo test"),
    "{}",
    thin.out
  );
  assert_eq!(json.code, 1);
  assert_eq!(read["heavy"], true);
  assert_eq!(read["language"], "cargo");
  assert_eq!(read["command"], serde_json::json!(["cargo", "test"]));
  assert!(read["instead"].as_str().unwrap().ends_with(" run -- cargo test"));
  assert_eq!(taken.code, 0);
  assert_eq!(classify(&["--takes-turn", "'open", "ls"]).code, 2);
  assert_eq!(classify(&[]).code, 2);
}
