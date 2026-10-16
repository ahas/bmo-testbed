//! Rust's Cargo: every command that compiles the project. Formatting, the metadata and the other reading
//! verbs compile nothing and run where they stand. Where the target directory goes is the caller's: Cargo
//! is run with the environment it was given.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::{Language, Tally, Widened, names_option, pattern, sum_of};

pub struct Cargo;

const COMPILING: [&str; 13] = [
  "build", "b", "check", "c", "clippy", "test", "t", "bench", "doc", "d", "nextest", "llvm-cov", "miri",
];

/// Whether a Cargo command compiles, its subcommand read past a `+toolchain` and the options before it.
pub fn compiles(args: &[String]) -> bool {
  let sub = args.iter().find(|word| !word.starts_with('-') && !word.starts_with('+'));

  sub.is_some_and(|sub| COMPILING.contains(&sub.as_str()))
}

static PASS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^test result: \w+\. (\d+) passed;"));
static FAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"^test result: \w+\. \d+ passed; (\d+) failed;"));
static SKIP: LazyLock<Regex> =
  LazyLock::new(|| pattern(r"^test result: \w+\. \d+ passed; \d+ failed; (\d+) ignored;"));
static FAILED_TEST: LazyLock<Regex> = LazyLock::new(|| pattern(r"^test (\S+) \.\.\. FAILED$"));
static PLACE: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s+--> (.+)$"));
static CODED_ERROR: LazyLock<Regex> = LazyLock::new(|| pattern(r"^error\[\w+\]: "));

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

  fn failures(&self, log: &str) -> Vec<String> {
    let lines: Vec<&str> = log.split('\n').collect();

    lines
      .iter()
      .enumerate()
      .filter_map(|(at, line)| {
        if let Some(test) = FAILED_TEST.captures(line) {
          return Some(test[1].to_owned());
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
