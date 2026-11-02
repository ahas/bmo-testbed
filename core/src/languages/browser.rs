//! Tests that drive a browser: Playwright's runner, and a `bun test` over a file under a `browser`
//! directory, which is how a repository's browser tests declare themselves. Every browser takes several
//! hundred megabytes, so Playwright is handed a quarter of the width.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::bun::{Bun, browser_path, test_paths};
use super::{Language, Tally, Widened, names_option, pattern, sum_of};

pub struct Browser;

// Playwright's closing lines: a count, then what it counts, indented.
static PASS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s+(\d+) passed \("));
static FAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s+(\d+) failed$"));
static SKIP: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s+(\d+) skipped$"));
static FAILED: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s+\d+\) (\[\w+\] › .+)$"));

fn playwright_test(program: &str, args: &[String]) -> bool {
  let word = |at: usize| args.get(at).map(String::as_str);

  match program {
    "playwright" => word(0) == Some("test"),
    "npx" | "bunx" | "pnpx" => word(0) == Some("playwright") && word(1) == Some("test"),
    "pnpm" | "yarn" => word(0) == Some("exec") && word(1) == Some("playwright") && word(2) == Some("test"),
    _ => false,
  }
}

impl Language for Browser {
  fn name(&self) -> &'static str {
    "browser"
  }

  fn claims(&self, program: &str, args: &[String]) -> bool {
    playwright_test(program, args)
      || (program == "bun"
        && args.first().map(String::as_str) == Some("test")
        && test_paths(&args[1..]).iter().any(|path| browser_path(path)))
  }

  fn widen(&self, command: &[String], width: usize) -> Widened {
    let playwright = command.iter().any(|word| word.rsplit('/').next() == Some("playwright"));
    let mut widened = command.to_vec();

    if playwright && !names_option(command, &["--workers", "-j"]) {
      widened.push(format!("--workers={}", (width / 4).max(1)));
    }

    Widened {
      command: widened,
      env: BTreeMap::new(),
    }
  }

  fn tally(&self, log: &str) -> Option<Tally> {
    let (pass, fail) = (sum_of(log, &PASS), sum_of(log, &FAIL));

    (pass.is_some() || fail.is_some()).then(|| Tally {
      pass: pass.unwrap_or(0),
      fail: fail.unwrap_or(0),
      skip: sum_of(log, &SKIP).unwrap_or(0),
    })
  }

  fn failures(&self, log: &str) -> Vec<String> {
    log
      .split('\n')
      .filter_map(|line| FAILED.captures(line).map(|found| found[1].to_owned()))
      .collect()
  }

  // A browser test file is a `bun test` file, run as bun runs it, from where bun's configuration stands.
  fn configuration_file(&self) -> Option<&'static str> {
    Bun.configuration_file()
  }

  fn test_command(&self, files: &[String], words: &[String]) -> Option<Vec<String>> {
    Bun.test_command(files, words)
  }
}
