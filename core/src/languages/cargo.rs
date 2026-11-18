//! Rust's Cargo: every command that compiles the project. Formatting, the metadata and the other reading
//! verbs compile nothing and run where they stand. Where the target directory goes is the caller's: Cargo
//! is run with the environment it was given.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::{Language, Tally, TestProgress, Widened, names_option, pattern, sum_of};
use crate::policy::lanes::LANES;

pub struct Cargo;

const COMPILING: [&str; 13] = [
  "build", "b", "check", "c", "clippy", "test", "t", "bench", "doc", "d", "nextest", "llvm-cov", "miri",
];

// The subcommands whose whole product is what they print and their exit code: run on another machine, they
// leave nothing here that a later command reads.
const PLACEABLE: [&str; 6] = ["check", "c", "clippy", "test", "t", "nextest"];

// A Cargo command's subcommand, read past a `+toolchain` and the options before it.
fn subcommand(args: &[String]) -> Option<&str> {
  args
    .iter()
    .find(|word| !word.starts_with('-') && !word.starts_with('+'))
    .map(String::as_str)
}

/// Whether a Cargo command compiles, its subcommand read past a `+toolchain` and the options before it.
pub fn compiles(args: &[String]) -> bool {
  subcommand(args).is_some_and(|sub| COMPILING.contains(&sub))
}

/// Whether a Cargo command may run on another machine (docs/testbed.md, 「A run on another machine」): it
/// tests, checks or lints, builds no test binaries to run here afterwards, and names no absolute path, which
/// would point elsewhere there.
pub fn placeable(args: &[String]) -> bool {
  subcommand(args).is_some_and(|sub| PLACEABLE.contains(&sub))
    && !args.iter().any(|word| word == "--no-run")
    && !args.iter().any(|word| {
      word.starts_with('/') || word.split_once('=').is_some_and(|(_, value)| value.starts_with('/'))
    })
}

static PASS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^test result: \w+\. (\d+) passed;"));
static FAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"^test result: \w+\. \d+ passed; (\d+) failed;"));
static SKIP: LazyLock<Regex> =
  LazyLock::new(|| pattern(r"^test result: \w+\. \d+ passed; \d+ failed; (\d+) ignored;"));
static FAILED_TEST: LazyLock<Regex> = LazyLock::new(|| pattern(r"^test (\S+) \.\.\. FAILED$"));
// Rust's test harness on a test it has waited on for its warning's span, and on a test's result.
static RUNNING_LONG: LazyLock<Regex> =
  LazyLock::new(|| pattern(r"^test (.+) has been running for over (\d+) seconds$"));
static RESULT: LazyLock<Regex> = LazyLock::new(|| pattern(r"^test (.+) \.\.\. (?:ok|FAILED|ignored)"));
static PLACE: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s+--> (.+)$"));
static CODED_ERROR: LazyLock<Regex> = LazyLock::new(|| pattern(r"^error\[\w+\]: "));

// The lines the harness kept of a failed test, read from its own `---- <test> stdout ----` section below its
// result line: from the panic, or the section's first line where nothing panicked, such as a test returning an
// error, to the first blank line or note, as many as the policy names. None where the harness printed no
// section for it.
fn panic_of<'a>(below: &[&'a str], test: &str) -> Vec<&'a str> {
  let heading = format!("---- {test} stdout ----");
  let Some(at) = below.iter().position(|line| line.trim_end() == heading) else {
    return Vec::new();
  };
  let section: Vec<&str> = below[at + 1..]
    .iter()
    .map(|line| line.trim_end())
    .take_while(|line| {
      !line.starts_with("---- ") && *line != "failures:" && !line.starts_with("test result: ")
    })
    .collect();
  let start = section
    .iter()
    .position(|line| line.contains(" panicked at "))
    .or_else(|| section.iter().position(|line| !line.is_empty()))
    .unwrap_or(section.len());

  section[start..]
    .iter()
    .take_while(|line| !line.is_empty() && !line.starts_with("note: "))
    .take(LANES.numbers.panic_lines)
    .copied()
    .collect()
}

impl Language for Cargo {
  fn name(&self) -> &'static str {
    "cargo"
  }

  fn claims(&self, program: &str, args: &[String]) -> bool {
    program == "cargo" && compiles(args)
  }

  fn widen(&self, command: &[String], width: usize) -> Widened {
    Widened {
      command: command.to_vec(),
      env: if names_option(command, &["-j", "--jobs"]) {
        BTreeMap::new()
      } else {
        BTreeMap::from([("CARGO_BUILD_JOBS".to_owned(), width.to_string())])
      },
    }
  }

  fn tally(&self, log: &str) -> Option<Tally> {
    sum_of(log, &PASS).map(|pass| Tally {
      pass,
      fail: sum_of(log, &FAIL).unwrap_or(0),
      skip: sum_of(log, &SKIP).unwrap_or(0),
    })
  }

  fn toolchain_command(&self) -> Option<&'static [&'static str]> {
    Some(&["rustc", "-vV"])
  }

  fn test_progress(&self, line: &str) -> Option<TestProgress> {
    let line = line.trim_end();

    if let Some(found) = RUNNING_LONG.captures(line) {
      return Some(TestProgress::Running {
        test: found[1].to_owned(),
        seconds: found[2].parse().unwrap_or(0),
      });
    }

    RESULT.captures(line).map(|found| TestProgress::Ended {
      test: found[1].to_owned(),
    })
  }

  // A test binary ends its output on the harness's `test result:` line; the tests it failed are the ones
  // whose own result line said so above it.
  fn failed_tests(&self, output: &str) -> Option<Vec<String>> {
    sum_of(output, &PASS)?;

    let mut failed: Vec<String> = Vec::new();

    for test in output.split('\n').filter_map(|line| FAILED_TEST.captures(line.trim_end())) {
      if !failed.iter().any(|one| one == &test[1]) {
        failed.push(test[1].to_owned());
      }
    }

    Some(failed)
  }

  // The harness takes every word that is no option as a filter, and `--exact` makes each match a test's
  // whole name only.
  fn exact_tests(&self, tests: &[String]) -> Option<Vec<String>> {
    Some(std::iter::once("--exact".to_owned()).chain(tests.iter().cloned()).collect())
  }

  // A failed test by its name, the lines its panic left standing under it.
  fn failures(&self, log: &str) -> Vec<String> {
    let lines: Vec<&str> = log.split('\n').collect();

    lines
      .iter()
      .enumerate()
      .filter_map(|(at, line)| {
        if let Some(test) = FAILED_TEST.captures(line) {
          return Some(
            std::iter::once(test[1].to_owned())
              .chain(panic_of(&lines[at + 1..], &test[1]).iter().map(|said| format!("    {said}")))
              .collect::<Vec<_>>()
              .join("\n"),
          );
        }

        // A compiler error, with the place it names on the line under it: one with a code, or one that
        // names a place, since other runners print `error:` lines of their own.
        let place = lines
          .get(at + 1)
          .and_then(|under| PLACE.captures(under))
          .map(|found| found[1].to_owned());

        (CODED_ERROR.is_match(line) || (line.starts_with("error: ") && place.is_some()))
          .then(|| place.map_or_else(|| (*line).to_owned(), |place| format!("{line} ({place})")))
      })
      .collect()
  }
}
