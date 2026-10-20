//! Letters that are not English in the code (docs/checks.md, 「The quick checks」). The code is written in
//! English, and what it still holds of another language is a debt recorded file by file in the repository's
//! ledger: the check refuses a file holding more such lines than the ledger allows it, a file the ledger
//! does not name being allowed none, and the ledger is only ever lowered, so the debt only shrinks. Whether
//! a line holds a letter that is not English, written as itself or named by an escape, is bmo-text's
//! verdict; sweeping the files, matching them to the ledger and telling what holds each line are the
//! testbed's.

use std::collections::BTreeMap;
use std::path::Path;

use bmo_testbed_core::policy::answer::ANSWER;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use bmo_text::english::{CodeStretches, holds_non_english, non_english_letters};

use super::files::{checkout_files_where, is_test_file, read_text};
use super::source::{Language, Reading, within};
use crate::selection::glob_matches;

/// Whether a path of the checkout is code, by its extension. A document, a data file and a settings file
/// are where another language may stand, and none of them is code.
pub fn is_code(path: &str) -> bool {
  path
    .rsplit_once('.')
    .is_some_and(|(_, extension)| QUICK_CHECKS.language.code.iter().any(|code| code == extension))
}

// Where a text holds a letter that is not English, by byte offset: each such letter, or the backslash of
// the escape naming one. bmo-text places them in UTF-16 units, which are turned into the text's bytes.
fn foreign_offsets(text: &str) -> Vec<usize> {
  let mut starts = non_english_letters(text, &CodeStretches::default())
    .into_iter()
    .map(|one| one.start)
    .peekable();
  let mut offsets = Vec::new();
  let mut units = 0;

  for (at, character) in text.char_indices() {
    while starts.peek().is_some_and(|start| *start <= units) {
      starts.next();
      offsets.push(at);
    }
    units += character.len_utf16();
  }

  offsets
}

/// How many lines of a text hold a letter that is not English, which is what the ledger counts.
pub fn foreign_line_count(text: &str) -> u64 {
  text.split('\n').filter(|line| holds_non_english(line)).count() as u64
}

/// How many lines holding letters that are not English each file may hold, by its path from the checkout's
/// root.
pub type Ledger = BTreeMap<String, u64>;

/// The ledger at `ledger`, named `named` in what it says; an empty one where none is written yet.
pub fn read_ledger(ledger: &Path, named: &str) -> Result<Ledger, String> {
  let fault = |why: String| fill_marks(&QUICK_CHECKS.unreadable, &[("path", named), ("why", &why)]);

  match std::fs::read_to_string(ledger) {
    Ok(text) => serde_json::from_str(&text).map_err(|why| {
      fill_marks(&QUICK_CHECKS.language.not_ledger, &[("ledger", named), ("why", &why.to_string())])
    }),
    Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::new()),
    Err(why) => Err(fault(why.to_string())),
  }
}

/// The ledger as it is written: one file to a line, in path order.
pub fn ledger_text(ledger: &Ledger) -> String {
  format!("{}\n", serde_json::to_string_pretty(ledger).expect("a ledger is written"))
}

/// How many lines holding letters that are not English each code file of the checkout holds, the files
/// holding none left out.
pub fn checkout_foreign_counts(root: &Path) -> Ledger {
  checkout_files_where(root, is_code)
    .into_iter()
    .filter_map(|path| {
      let count = foreign_line_count(&read_text(root, &path));

      (count > 0).then_some((path, count))
    })
    .collect()
}

/// What the check came to: how many code files hold letters that are not English, a line for each holding more than the
/// ledger allows it, and how many hold fewer, which settling the ledger would lower.
#[derive(Debug, PartialEq, Eq)]
pub struct LanguageInCode {
  pub files: usize,
  pub findings: Vec<String>,
  pub below: usize,
}

/// The check over the checkout at `root`, held to the ledger at `ledger` named `named`.
pub fn checkout_language_in_code(
  root: &Path,
  ledger: &Path,
  named: &str,
) -> Result<LanguageInCode, String> {
  let ledger = read_ledger(ledger, named)?;
  let counts = checkout_foreign_counts(root);
  let allowed = |path: &str| ledger.get(path).copied().unwrap_or(0);
  let findings = counts
    .iter()
    .filter(|(path, count)| **count > allowed(path))
    .map(|(path, count)| {
      fill_marks(
        &QUICK_CHECKS.language.over,
        &[("path", path), ("count", &count.to_string()), ("allowed", &allowed(path).to_string())],
      )
    })
    .collect();
  let below = ledger
    .iter()
    .filter(|(path, allowed)| counts.get(*path).copied().unwrap_or(0) < **allowed)
    .count();

  Ok(LanguageInCode {
    files: counts.len(),
    findings,
    below,
  })
}

/// What settling the ledger did: how many files it lowered and how many still hold such lines.
#[derive(Debug, PartialEq, Eq)]
pub struct Settled {
  pub lowered: usize,
  pub kept: usize,
}

/// The ledger at `ledger` lowered to what each file of the checkout at `root` holds now and written back.
/// It is never raised: a file holding more than the ledger allows is the check's fault to report, and a
/// file the ledger does not name is allowed none.
pub fn settle(root: &Path, ledger: &Path, named: &str) -> Result<Settled, String> {
  let held = read_ledger(ledger, named)?;
  let counts = checkout_foreign_counts(root);
  let mut settled = Ledger::new();
  let mut lowered = 0;

  for (path, allowed) in &held {
    let now = (*allowed).min(counts.get(path).copied().unwrap_or(0));

    if now < *allowed {
      lowered += 1;
    }

    if now > 0 {
      settled.insert(path.clone(), now);
    }
  }

  std::fs::write(ledger, ledger_text(&settled))
    .map_err(|why| fill_marks(&QUICK_CHECKS.unreadable, &[("path", named), ("why", &why.to_string())]))?;

  Ok(Settled {
    lowered,
    kept: settled.len(),
  })
}

/// What holds a line that is not English: a comment; test input; people's text, a literal in a file the declaration
/// names as such; another literal, a fixed sentence or a pattern; or a line of a language no parser here
/// reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
  Comment,
  Test,
  People,
  Literal,
  Unread,
}

impl LineKind {
  /// Every kind, in the order a report names them.
  pub const ALL: [Self; 5] = [Self::Comment, Self::Test, Self::People, Self::Literal, Self::Unread];

  pub fn name(self) -> &'static str {
    match self {
      Self::Comment => "comment",
      Self::Test => "test",
      Self::People => "people",
      Self::Literal => "literal",
      Self::Unread => "unread",
    }
  }

  // The kind a file's code is, by the file: a test file's test input, people's text where the globs reach
  // it, a literal otherwise.
  fn of_file(path: &str, people: &[String]) -> Self {
    if is_test_file(path) {
      Self::Test
    } else if people.iter().any(|glob| glob_matches(glob, path)) {
      Self::People
    } else {
      Self::Literal
    }
  }
}

// What holds a line, by where its letters stand: a line whose letters are all in comments is a comment's,
// one whose letters in code are all in a test module is test input, one whose letters in code are all in
// patterns is a literal, and the rest is the file's own kind.
fn line_kind(offsets: &[usize], reading: &Reading, own: LineKind) -> LineKind {
  let code: Vec<usize> = offsets.iter().copied().filter(|at| !within(&reading.comments, *at)).collect();

  if code.is_empty() {
    LineKind::Comment
  } else if own == LineKind::Test || code.iter().all(|at| within(&reading.tests, *at)) {
    LineKind::Test
  } else if code.iter().all(|at| within(&reading.patterns, *at)) {
    LineKind::Literal
  } else {
    own
  }
}

/// Every line of a code file that holds a letter that is not English, counted from one, with what holds it;
/// `people` are the globs naming people's text.
pub fn foreign_lines(path: &str, text: &str, people: &[String]) -> Vec<(usize, LineKind)> {
  if !holds_non_english(text) {
    return Vec::new();
  }

  let reading = Language::of(path).map(|language| language.read(text));
  let own = LineKind::of_file(path, people);
  let mut lines = Vec::new();
  let mut start = 0;

  for (index, line) in text.split('\n').enumerate() {
    let offsets: Vec<usize> = foreign_offsets(line).into_iter().map(|at| start + at).collect();

    start += line.len() + 1;

    if offsets.is_empty() {
      continue;
    }

    let kind = reading.as_ref().map_or(LineKind::Unread, |reading| line_kind(&offsets, reading, own));

    lines.push((index + 1, kind));
  }

  lines
}

/// The lines that are not English the code of the checkout at `root` still holds, by what holds them: without a prefix a row
/// to an area, the first two folders of a path, and with one a row to each file under it; the largest
/// rows first, cut under the answer's ceiling with the cut said.
pub fn language_report(root: &Path, prefix: Option<&str>, people: &[String]) -> Vec<String> {
  type Tally = [usize; LineKind::ALL.len()];

  let words = &QUICK_CHECKS.language;
  // The rows in the order their first file is listed, which the sort keeps among rows of one size.
  let mut rows: Vec<(String, Tally)> = Vec::new();
  let mut total: Tally = [0; LineKind::ALL.len()];
  let row_of = |path: &str| match prefix {
    Some(_) => path.to_owned(),
    None => {
      let folders: Vec<&str> = path.split('/').collect();
      let area = folders[..folders.len() - 1].iter().take(2).copied().collect::<Vec<_>>().join("/");

      if area.is_empty() { ".".to_owned() } else { area }
    }
  };

  for path in
    checkout_files_where(root, |path| is_code(path) && prefix.is_none_or(|prefix| path.starts_with(prefix)))
  {
    for (_, kind) in foreign_lines(&path, &read_text(root, &path), people) {
      let at = LineKind::ALL.iter().position(|one| *one == kind).unwrap_or_default();
      let row = row_of(&path);
      let index = rows.iter().position(|(name, _)| *name == row).unwrap_or_else(|| {
        rows.push((row, [0; LineKind::ALL.len()]));
        rows.len() - 1
      });

      rows[index].1[at] += 1;
      total[at] += 1;
    }
  }

  let say = |name: &str, tally: &Tally| {
    let kinds: Vec<String> = LineKind::ALL
      .iter()
      .zip(tally)
      .map(|(kind, count)| fill_marks(&words.kind, &[("kind", kind.name()), ("count", &count.to_string())]))
      .collect();

    fill_marks(
      &words.row,
      &[
        ("name", name),
        ("total", &tally.iter().sum::<usize>().to_string()),
        ("kinds", &kinds.join(", ")),
      ],
    )
  };
  rows.sort_by_key(|(_, tally)| std::cmp::Reverse(tally.iter().sum::<usize>()));

  let counted = if prefix.is_none() { &words.areas } else { &words.files };
  let head = say(&fill_marks(counted, &[("count", &rows.len().to_string())]), &total);
  let mut length = head.chars().count();
  let mut out = vec![head];

  for (name, tally) in &rows {
    let line = say(name, tally);

    if length + line.chars().count() + 1 > ANSWER.ceiling_chars {
      break;
    }

    length += line.chars().count() + 1;
    out.push(line);
  }

  if out.len() - 1 < rows.len() {
    out.push(fill_marks(
      &words.cut,
      &[("shown", &(out.len() - 1).to_string()), ("rows", &rows.len().to_string())],
    ));
  }

  out
}
