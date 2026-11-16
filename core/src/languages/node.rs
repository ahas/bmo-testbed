//! Node.js and the package managers: a package script that tests, checks or builds, under npm, pnpm, yarn
//! or bun; `node --test`; Vitest and Jest, run directly or through a package manager's runner. A package
//! script's own runners are read by every language at once, so this reads only Node's, Vitest's and
//! Jest's summaries.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::{Language, Tally, Widened, names_option, pattern, sum_of};

pub struct Node;

const MANAGERS: [&str; 4] = ["npm", "pnpm", "yarn", "bun"];
const LAUNCHERS: [&str; 3] = ["npx", "bunx", "pnpx"];
const RUNNERS: [&str; 2] = ["vitest", "jest"];

// A package script that is a run of tests, a check or a build, by its name: `test`, `check:hub`, `build`,
// `test-e2e`.
static HEAVY_SCRIPT: LazyLock<Regex> = LazyLock::new(|| pattern(r"^(test|check|build)([:\-_].*)?$"));

// node --test's closing comments, Vitest's and Jest's tests line, each a count.
static NODE_PASS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^# pass (\d+)$"));
static NODE_FAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"^# fail (\d+)$"));
static NODE_SKIP: LazyLock<Regex> = LazyLock::new(|| pattern(r"^# skipped (\d+)$"));
static VITEST_PASS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*Tests\s+.*?(\d+) passed"));
static VITEST_FAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*Tests\s+(\d+) failed"));
static VITEST_SKIP: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*Tests\s+.*?(\d+) skipped"));
static JEST_PASS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^Tests:\s+.*?(\d+) passed"));
static JEST_FAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"^Tests:\s+(\d+) failed"));
static JEST_SKIP: LazyLock<Regex> = LazyLock::new(|| pattern(r"^Tests:\s+.*?(\d+) skipped"));
static SAID: LazyLock<Regex> = LazyLock::new(|| {
  pattern(r"(?m)^(# (pass|fail) \d+|\s*Tests\s+.*\d+ (passed|failed)|Tests:\s+.*\d+ (passed|failed))")
});

// What failed: Vitest's, Jest's and TAP's lines.
static VITEST_FAILED: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*(?:FAIL|×)\s+(.+)$"));
static JEST_FAILED: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*● (.+)$"));
static TAP_FAILED: LazyLock<Regex> = LazyLock::new(|| pattern(r"^not ok \d+ - (.+)$"));

// The words left once a manager's options are passed: `-C dir`, `--prefix dir`, `--filter pkg`.
fn past_options(args: &[String]) -> Vec<&str> {
  const VALUED: [&str; 8] = ["-C", "--dir", "--prefix", "--filter", "-F", "--cwd", "-w", "--workspace"];
  let mut rest = vec![];
  let mut at = 0;

  while at < args.len() {
    if VALUED.contains(&args[at].as_str()) {
      at += 2;
      continue;
    }

    if !args[at].starts_with('-') {
      rest.push(args[at].as_str());
    }

    at += 1;
  }

  rest
}

// Whether a launcher or `pnpm exec` starts Vitest or Jest.
fn launches_runner(program: &str, words: &[&str]) -> bool {
  let runner = |word: Option<&&str>| word.is_some_and(|word| RUNNERS.contains(word));

  if LAUNCHERS.contains(&program) {
    return runner(words.first());
  }

  if (program == "pnpm" || program == "yarn") && matches!(words.first(), Some(&"exec" | &"dlx")) {
    return runner(words.get(1));
  }

  program == "yarn" && runner(words.first())
}

fn summed(log: &str, patterns: [&Regex; 3]) -> u64 {
  patterns.iter().map(|one| sum_of(log, one).unwrap_or(0)).sum()
}

impl Language for Node {
  fn name(&self) -> &'static str {
    "node"
  }

  fn claims(&self, program: &str, args: &[String]) -> bool {
    if program == "node" {
      return args.iter().any(|arg| arg == "--test");
    }

    if RUNNERS.contains(&program) {
      return true;
    }

    let words = past_options(args);

    if launches_runner(program, &words) {
      return true;
    }

    if !MANAGERS.contains(&program) {
      return false;
    }

    match words.first() {
      Some(&"test" | &"t") => program != "bun",
      Some(&"run" | &"run-script") => HEAVY_SCRIPT.is_match(words.get(1).unwrap_or(&"")),
      // yarn and pnpm run a script named as the first word.
      first => (program == "yarn" || program == "pnpm") && HEAVY_SCRIPT.is_match(first.unwrap_or(&"")),
    }
  }

  fn widen(&self, command: &[String], width: usize) -> Widened {
    let env = BTreeMap::from([
      ("BMO_TEST_PARALLEL".to_owned(), width.to_string()),
      ("CARGO_BUILD_JOBS".to_owned(), width.to_string()),
    ]);
    let names: Vec<&str> = command.iter().map(|word| word.rsplit('/').next().unwrap_or(word)).collect();
    let mut widened = command.to_vec();

    if names.contains(&"node")
      && command.iter().any(|word| word == "--test")
      && !names_option(command, &["--test-concurrency"])
    {
      widened.push(format!("--test-concurrency={width}"));
    } else if names.iter().any(|name| RUNNERS.contains(name))
      && !names_option(command, &["--maxWorkers", "--max-workers"])
    {
      widened.push(format!("--maxWorkers={width}"));
    }

    Widened {
      command: widened,
      env,
    }
  }

  fn tally(&self, log: &str) -> Option<Tally> {
    SAID.is_match(log).then(|| Tally {
      pass: summed(log, [&NODE_PASS, &VITEST_PASS, &JEST_PASS]),
      fail: summed(log, [&NODE_FAIL, &VITEST_FAIL, &JEST_FAIL]),
      skip: summed(log, [&NODE_SKIP, &VITEST_SKIP, &JEST_SKIP]),
    })
  }

  fn failures(&self, log: &str) -> Vec<String> {
    log
      .split('\n')
      .filter_map(|line| {
        [&VITEST_FAILED, &JEST_FAILED, &TAP_FAILED]
          .iter()
          .find_map(|one| one.captures(line))
          .map(|found| found[1].trim().to_owned())
      })
      .collect()
  }

  fn test_naming(&self) -> &'static [&'static str] {
    &[
      "*.test.js",
      "**/*.test.js",
      "*.test.mjs",
      "**/*.test.mjs",
      "*.test.cjs",
      "**/*.test.cjs",
    ]
  }

  // `node --test` sets no per-test limit of its own, so a test runs until it ends.
  fn test_command(&self, files: &[String], words: &[String]) -> Option<Vec<String>> {
    let mut command = vec!["node".to_owned(), "--test".to_owned()];

    command.extend(words.iter().cloned());
    command.extend(files.iter().cloned());
    Some(command)
  }

  fn script_command(&self, script: &str) -> Option<Vec<String>> {
    Some(vec!["npm".to_owned(), "run".to_owned(), script.to_owned()])
  }

  fn toolchain_command(&self) -> Option<&'static [&'static str]> {
    Some(&["node", "--version"])
  }

  fn root_configuration(&self) -> &'static [&'static str] {
    &["package.json", "package-lock.json", "tsconfig.json"]
  }

  fn lockfile(&self) -> Option<&'static str> {
    Some("package-lock.json")
  }
}
