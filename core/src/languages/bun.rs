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
// The heading bun writes above what one test file said, and its count of errors outside any test.
static FILE_HEADING: LazyLock<Regex> =
  LazyLock::new(|| pattern(r"^(\S+[._](?:test|spec)\.[cm]?[jt]sx?):$"));
static ERRORS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*([\d,]+) errors?$"));

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

  // Each `(fail)` line given to the file heading above it. A file that failed to load, or an error thrown
  // between tests, counts among the failures or the errors with no `(fail)` line of its own, so where the
  // lines given a file fall short of bun's own counts no file is named.
  fn failed_files(&self, output: &str) -> Vec<String> {
    let mut under: Option<&str> = None;
    let mut given = 0u64;
    let mut files: Vec<String> = Vec::new();

    for line in output.split('\n').map(str::trim_end) {
      if let Some(heading) = FILE_HEADING.captures(line) {
        under = heading.get(1).map(|file| file.as_str());
      } else if FAILED.is_match(line)
        && let Some(file) = under
      {
        given += 1;

        if !files.iter().any(|one| one == file) {
          files.push(file.to_owned());
        }
      }
    }

    let counted = sum_of(output, &FAIL).unwrap_or(0);
    let errors = sum_of(output, &ERRORS).unwrap_or(0);

    if given == counted && errors == 0 { files } else { Vec::new() }
  }

  fn test_naming(&self) -> &'static [&'static str] {
    &["*.test.ts", "**/*.test.ts"]
  }

  fn configuration_file(&self) -> Option<&'static str> {
    Some("bunfig.toml")
  }

  // Several files run side by side in bun's own pool. Bun ends a test at a limit of its own unless told
  // otherwise, so it is handed `--timeout 0`, which lifts that limit, unless the words name a limit themselves.
  // bun reads a word as a file only where it is written as a path, and any other word as a filter it matches
  // within every test file's path, so a file handed by a bare name would run every file whose path holds it.
  fn test_command(&self, files: &[String], words: &[String]) -> Option<Vec<String>> {
    let mut command = vec!["bun".to_owned(), "test".to_owned()];

    if files.len() > 1 {
      command.push("--parallel".to_owned());
    }

    command.extend(words.iter().cloned());

    if !names_flag(words, "--timeout") {
      command.extend(["--timeout".to_owned(), "0".to_owned()]);
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

  // Bun takes its per-test limit in milliseconds.
  fn per_test_limit(&self, words: &[String], seconds: u64) -> Vec<String> {
    if names_flag(words, "--timeout") {
      return Vec::new();
    }

    vec![format!("--timeout={}", seconds.saturating_mul(1000))]
  }

  fn script_command(&self, script: &str) -> Option<Vec<String>> {
    Some(vec!["bun".to_owned(), "run".to_owned(), script.to_owned()])
  }

  fn home_tool_directory(&self) -> Option<&'static str> {
    Some(".bun/bin")
  }

  fn toolchain_command(&self) -> Option<&'static [&'static str]> {
    Some(&["bun", "--version"])
  }

  fn root_configuration(&self) -> &'static [&'static str] {
    &["package.json", "bun.lock", "bunfig.toml", "tsconfig.json"]
  }

  fn lockfile(&self) -> Option<&'static str> {
    Some("bun.lock")
  }
}
