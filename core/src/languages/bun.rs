//! bun's own test runner. One test file is a touched test a harness runs while it writes, light enough to run
//! where it stands; a run over several files, a directory or a name that may match many is a suite. A
//! browser file is the browser language's.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::{Language, Tally, Widened, names_flag, pattern, sum_of};

pub struct Bun;

// The options of `bun test` that take a value as the next word.
const VALUED: [&str; 11] = [
  "-t",
  "--test-name-pattern",
  "--timeout",
  "--preload",
  "--rerun-each",
  "--seed",
  "--reporter",
  "--reporter-outfile",
  "--coverage-dir",
  "--max-concurrency",
  "--shard",
];

/// The paths a `bun test` names, past its options and their values.
pub(crate) fn test_paths(args: &[String]) -> Vec<&String> {
  let mut paths = vec![];
  let mut at = 0;

  while at < args.len() {
    if VALUED.contains(&args[at].as_str()) {
      at += 2;
      continue;
    }

    if !args[at].starts_with('-') {
      paths.push(&args[at]);
    }

    at += 1;
  }

  paths
}

static TEST_FILE: LazyLock<Regex> = LazyLock::new(|| pattern(r"[._](test|spec)\.[cm]?[jt]sx?$"));
static BROWSER_PATH: LazyLock<Regex> = LazyLock::new(|| pattern(r"(^|/)browser/"));
static PASS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*([\d,]+) pass$"));
static FAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*([\d,]+) fail$"));
static SKIP: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*([\d,]+) (?:skip|todo)$"));
static FAILED: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\(fail\) (.+?)(?: \[[\d.]+m?s\])?$"));

/// Whether a path is a browser test's, which drives a browser and is the browser language's.
pub(crate) fn browser_path(path: &str) -> bool {
  BROWSER_PATH.is_match(path)
}

impl Language for Bun {
  fn name(&self) -> &'static str {
    "bun"
  }

  fn claims(&self, program: &str, args: &[String]) -> bool {
    if program != "bun" || args.first().map(String::as_str) != Some("test") {
      return false;
    }

    let paths = test_paths(&args[1..]);

    !(paths.len() == 1 && TEST_FILE.is_match(paths[0])) && !paths.iter().any(|path| browser_path(path))
  }

  fn widen(&self, command: &[String], width: usize) -> Widened {
    // `--parallel` given with no number takes one; a number already named stands.
    Widened {
      command: command
        .iter()
        .map(|word| if word == "--parallel" { format!("--parallel={width}") } else { word.clone() })
        .collect(),
      env: BTreeMap::from([("BMO_TEST_PARALLEL".to_owned(), width.to_string())]),
    }
  }

  fn tally(&self, log: &str) -> Option<Tally> {
    let (pass, fail, skip) = (sum_of(log, &PASS), sum_of(log, &FAIL), sum_of(log, &SKIP));

    (pass.is_some() || fail.is_some() || skip.is_some()).then(|| Tally {
      pass: pass.unwrap_or(0),
      fail: fail.unwrap_or(0),
      skip: skip.unwrap_or(0),
    })
  }

  fn failures(&self, log: &str) -> Vec<String> {
    log
      .split('\n')
      .filter_map(|line| FAILED.captures(line).map(|found| found[1].to_owned()))
      .collect()
  }

  fn test_naming(&self) -> &'static [&'static str] {
    &["*.test.ts", "**/*.test.ts"]
  }

  fn configuration_file(&self) -> Option<&'static str> {
    Some("bunfig.toml")
  }

  // Several files run side by side in bun's own pool; the per-test limit is bun's `--timeout`, in milliseconds.
  // bun reads a word as a file only where it is written as a path, and any other word as a filter it matches
  // within every test file's path, so a file handed by a bare name would run every file whose path holds it.
  fn test_command(
    &self,
    files: &[String],
    words: &[String],
    timeout_seconds: Option<u64>,
  ) -> Option<Vec<String>> {
    let mut command = vec!["bun".to_owned(), "test".to_owned()];

    if files.len() > 1 {
      command.push("--parallel".to_owned());
    }

    command.extend(words.iter().cloned());

    if let Some(seconds) = timeout_seconds
      && !names_flag(words, "--timeout")
    {
      command.extend(["--timeout".to_owned(), (seconds * 1000).to_string()]);
    }

    command.extend(files.iter().map(|file| {
      if file.starts_with('/') || file.starts_with("./") || file.starts_with("../") {
        file.clone()
      } else {
        format!("./{file}")
      }
    }));
    Some(command)
  }

  fn script_command(&self, script: &str) -> Option<Vec<String>> {
    Some(vec!["bun".to_owned(), "run".to_owned(), script.to_owned()])
  }

  fn root_configuration(&self) -> &'static [&'static str] {
    &["package.json", "bun.lock", "bunfig.toml", "tsconfig.json"]
  }

  fn lockfile(&self) -> Option<&'static str> {
    Some("bun.lock")
  }
}
