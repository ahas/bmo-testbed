//! A run of the suite read back out of its log (docs/suite.md, 「Reading the output」): the verdict it ended
//! on, the tests and the files that failed, each failing test with the unit it stands in, and the files it
//! never started — the lines the run writes in the policy's words, read by their shape — and the warnings
//! the tools it ran printed.

use std::sync::LazyLock;

use bmo_testbed_core::policy::suite::SUITE;
use regex::Regex;

fn pattern(text: &str) -> Regex {
  Regex::new(text).expect("a reading of the suite's own lines compiles")
}

static FAILED_TEST: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\(fail\) (.+?)(?: \[[\d.]+m?s\])?$"));
static NAMED_FILES: LazyLock<Regex> =
  LazyLock::new(|| pattern(r"^(?:failed|did not end within \d+s): (.+)$"));
static FILE_HEADER: LazyLock<Regex> = LazyLock::new(|| pattern(r"^(\S+\.test\.ts):$"));
static MARKED_FILE: LazyLock<Regex> =
  LazyLock::new(|| pattern(r"^(?:fail|CUT)\s+[\d.]+s\s+(\S+\.test\.ts)$"));
static UNSTARTED: LazyLock<Regex> = LazyLock::new(|| pattern(r"^never started within \d+s: (.+)$"));
// The line the sweep writes before each unit's output, whatever its mark, and Cargo's line for a failed test.
static UNIT_HEADING: LazyLock<Regex> = LazyLock::new(|| pattern(r"^(?:ok|fail|CUT)\s+[\d.]+s\s+(\S+)$"));
static CARGO_FAILED: LazyLock<Regex> = LazyLock::new(|| pattern(r"^test (\S+) \.\.\. FAILED$"));

// A warning as each tool the check runs prints one. A log holds the output of many tools one after another,
// and no reader of any one tool's output reads that mix, so each tool's line is read by its own shape: the
// console build's badge, Python's warnings module behind whatever dots unittest printed before it, Node's
// process warning, and Cargo's, less its count of what a crate generated.
static BADGE_WARNING: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*WARN\s+(.*)$"));
static PYTHON_WARNING: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\.*(\S+:\d+: \w*Warning: .*)$"));
static NODE_WARNING: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\(node:\d+\) (\w*Warning: .*)$"));
static CARGO_WARNING: LazyLock<Regex> = LazyLock::new(|| pattern(r"^warning: (.*)$"));
static CARGO_COUNT: LazyLock<Regex> = LazyLock::new(|| pattern(r"generated \d+ warnings?\b"));
// A badge whose words are only the bracketed names of where it came from says what it warns of on the line
// below.
static ONLY_SOURCES: LazyLock<Regex> = LazyLock::new(|| pattern(r"^(?:\[[^\]]*\]\s*)*$"));

fn split_names(list: &str) -> impl Iterator<Item = String> + '_ {
  list.split(", ").map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned)
}

fn once(all: impl IntoIterator<Item = String>) -> Vec<String> {
  let mut seen: Vec<String> = Vec::new();

  for one in all {
    if !seen.contains(&one) {
      seen.push(one);
    }
  }

  seen
}

// A crate's test binary ends on Cargo's own one-line result, whose ignored tests are what bun calls skipped; bun's
// counts stand alone on their lines, grouped in thousands with commas.
static CARGO_RESULT: LazyLock<Regex> =
  LazyLock::new(|| pattern(r"^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;"));
static BUN_COUNT: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*([\d,]+) (pass|fail|skip|todo)$"));
static RAN_FILES: LazyLock<Regex> =
  LazyLock::new(|| pattern(r"^Ran [\d,]+ tests? across ([\d,]+) files?\."));

/// What the summaries of a run add up to.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
  pub pass: u64,
  pub fail: u64,
  pub skip: u64,
  pub todo: u64,
  /// How many test files the run reached, where the log says.
  pub files: u64,
}

fn count(digits: &str) -> u64 {
  digits.replace(',', "").parse().unwrap_or(0)
}

/// The run's totals: the sum of every summary block in the log, since a whole-suite run writes one per test file
/// and a crate's run one per test binary, so the last block alone is never the run's.
pub fn tally_of(log: &str) -> Tally {
  let mut tally = Tally::default();

  for line in log.lines() {
    if let Some(cargo) = CARGO_RESULT.captures(line) {
      tally.pass += count(&cargo[1]);
      tally.fail += count(&cargo[2]);
      tally.skip += count(&cargo[3]);
    } else if let Some(one) = BUN_COUNT.captures(line) {
      let at = match &one[2] {
        "pass" => &mut tally.pass,
        "fail" => &mut tally.fail,
        "skip" => &mut tally.skip,
        _ => &mut tally.todo,
      };

      *at += count(&one[1]);
    } else if let Some(ran) = RAN_FILES.captures(line) {
      tally.files += count(&ran[1]);
    }
  }

  tally
}

/// The verdict the run ended on: the last line opening with the mark and saying how the run came out, so
/// neither the run's other lines about itself nor bun's own output is mistaken for it; none in a log from a
/// run that never ended.
pub fn verdict_of(log: &str) -> Option<String> {
  let ok = format!("{} {} in ", SUITE.lines.mark, SUITE.ending.ok);
  let failed = format!("{} {} in ", SUITE.lines.mark, SUITE.ending.failed);

  log
    .lines()
    .rfind(|line| line.starts_with(&ok) || line.starts_with(&failed))
    .map(str::to_owned)
}

/// What failed: each test bun names on a line of its own, and the files the sweep's closing lines name, in
/// the order they were written, each once.
pub fn failures_of(log: &str) -> Vec<String> {
  once(log.lines().flat_map(|line| {
    if let Some(test) = FAILED_TEST.captures(line) {
      return vec![test[1].to_owned()];
    }

    NAMED_FILES
      .captures(line)
      .map(|files| split_names(&files[1]).collect())
      .unwrap_or_default()
  }))
}

/// Which test files failed: each failing test given to the file printed above it, and the files the sweep
/// names outright, cut and failed alike; nothing inferred from a test's name.
pub fn failed_files_of(log: &str) -> Vec<String> {
  let mut under: Option<String> = None;
  let mut files = Vec::new();

  for line in log.lines() {
    if let Some(header) = FILE_HEADER.captures(line) {
      under = Some(header[1].to_owned());
      continue;
    }

    if line.starts_with("(fail) ")
      && let Some(file) = &under
    {
      files.push(file.clone());
      continue;
    }

    if let Some(one) = MARKED_FILE.captures(line) {
      files.push(one[1].to_owned());
      continue;
    }

    if let Some(many) = NAMED_FILES.captures(line) {
      files.extend(split_names(&many[1]));
    }
  }

  once(files)
}

/// The files the sweep never started, which no reader of failures counts.
pub fn unstarted_of(log: &str) -> Vec<String> {
  once(
    log
      .lines()
      .filter_map(|line| UNSTARTED.captures(line))
      .flat_map(|many| split_names(&many[1]).collect::<Vec<_>>()),
  )
}

/// A test that failed, by its name, with the file or unit it stands in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FailedTest {
  pub file: String,
  pub test: String,
}

impl FailedTest {
  /// The test as a line names it: its name, then where it stands.
  pub fn named(&self) -> String {
    format!("{} ({})", self.test, self.file)
  }
}

/// Each test that failed, bun's and Cargo's alike, given to the unit whose output it stands in, each once;
/// a log the sweep wrote no heading in, a single file's `bun test`, gives each to bun's own file header, and
/// a failure under neither is not given a place it cannot be read to have.
pub fn failed_tests_of(log: &str) -> Vec<FailedTest> {
  let mut swept = false;
  let mut under: Option<String> = None;
  let mut tests: Vec<FailedTest> = Vec::new();

  for line in log.lines() {
    if let Some(unit) = UNIT_HEADING.captures(line) {
      swept = true;
      under = Some(unit[1].to_owned());
      continue;
    }

    if !swept && let Some(header) = FILE_HEADER.captures(line) {
      under = Some(header[1].to_owned());
      continue;
    }

    let test = FAILED_TEST.captures(line).or_else(|| CARGO_FAILED.captures(line));

    if let (Some(test), Some(file)) = (test, &under) {
      let one = FailedTest {
        file: file.clone(),
        test: test[1].to_owned(),
      };

      if !tests.contains(&one) {
        tests.push(one);
      }
    }
  }

  tests
}

/// The warnings the tools the run started printed, each on a line of its own with the terminal's colours
/// taken off, in the order they were written; a badge naming only its source takes the next line that says
/// something as its words.
pub fn warnings_of(log: &str) -> Vec<String> {
  let plain = console::strip_ansi_codes(log);
  let lines: Vec<&str> = plain.lines().collect();
  let mut warnings = Vec::new();

  for (at, line) in lines.iter().enumerate() {
    if let Some(badge) = BADGE_WARNING.captures(line) {
      let said = badge[1].trim();
      let below = ONLY_SOURCES
        .is_match(said)
        .then(|| lines[at + 1..].iter().map(|next| next.trim()).find(|next| !next.is_empty()))
        .flatten();

      warnings.push(match below {
        Some(words) => format!("WARN {said} {words}"),
        None => format!("WARN {said}"),
      });
    } else if let Some(python) = PYTHON_WARNING.captures(line) {
      warnings.push(python[1].trim_end().to_owned());
    } else if let Some(node) = NODE_WARNING.captures(line) {
      warnings.push(node[1].trim_end().to_owned());
    } else if let Some(cargo) = CARGO_WARNING.captures(line)
      && !CARGO_COUNT.is_match(&cargo[1])
    {
      warnings.push(line.trim_end().to_owned());
    }
  }

  warnings
}
