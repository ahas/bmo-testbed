//! Korean in the code (docs/checks.md, 「The quick checks」). The code is written in English, and the Korean
//! it still holds is a debt recorded file by file in the repository's ledger: the check refuses a file
//! holding more lines with Korean in them than the ledger allows it, a file the ledger does not name being
//! allowed none, and the ledger is only ever lowered, so the debt only shrinks. Korean written as an escape
//! is Korean all the same, so a line naming a Hangul character by its escape counts as one holding it.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use bmo_testbed_core::policy::answer::ANSWER;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use regex::Regex;

use super::files::{checkout_files_where, is_test_file, read_text};
use super::source::{Language, Reading, within};
use super::wording::hangul;
use crate::selection::glob_matches;

// A Unicode escape in any of the shapes the code languages share, its code point in hex: a braced `\u`, a
// four-digit `\u`, an eight-digit `\U`, or a pattern's braced `\x`. Every code file is read the same way,
// a comment as much as a literal and a language no parser here reads among them, so an escape is found by
// its shape: the parsers here decode escapes only inside the literals of the languages they read.
static ESCAPE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"\\(?:u\{([0-9A-Fa-f]{1,8})\}|x\{([0-9A-Fa-f]{1,8})\}|u([0-9A-Fa-f]{4})|U([0-9A-Fa-f]{8}))")
    .expect("a Unicode escape")
});

/// Whether a path of the checkout is code, by its extension. A document, a data file and a settings file
/// are where Korean may stand, and none of them is code.
pub fn is_code(path: &str) -> bool {
  path
    .rsplit_once('.')
    .is_some_and(|(_, extension)| QUICK_CHECKS.korean.code.iter().any(|code| code == extension))
}

// Where a text holds Korean, by byte offset: each Hangul character, as the wording check tells a Korean line,
// and each escape naming one, at its backslash.
fn korean_offsets(text: &str) -> impl Iterator<Item = usize> + '_ {
  let characters = text.char_indices().filter(|(_, c)| hangul(*c)).map(|(at, _)| at);
  let escapes = ESCAPE.captures_iter(text).filter_map(|escape| {
    let digits = escape.iter().skip(1).flatten().next()?;
    let named = u32::from_str_radix(digits.as_str(), 16).ok().and_then(char::from_u32)?;

    hangul(named).then(|| escape.get_match().start())
  });

  characters.chain(escapes)
}

/// How many lines of a text hold Korean, which is what the ledger counts.
pub fn korean_line_count(text: &str) -> u64 {
  text.split('\n').filter(|line| korean_offsets(line).next().is_some()).count() as u64
}

/// How many Korean lines each file may hold, by its path from the checkout's root.
pub type Ledger = BTreeMap<String, u64>;

/// The ledger at `ledger`, named `named` in what it says; an empty one where none is written yet.
pub fn read_ledger(ledger: &Path, named: &str) -> Result<Ledger, String> {
  let fault = |why: String| fill_marks(&QUICK_CHECKS.unreadable, &[("path", named), ("why", &why)]);

  match std::fs::read_to_string(ledger) {
    Ok(text) => serde_json::from_str(&text).map_err(|why| {
      fill_marks(&QUICK_CHECKS.korean.not_ledger, &[("ledger", named), ("why", &why.to_string())])
    }),
    Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::new()),
    Err(why) => Err(fault(why.to_string())),
  }
}

/// The ledger as it is written: one file to a line, in path order.
pub fn ledger_text(ledger: &Ledger) -> String {
  format!("{}\n", serde_json::to_string_pretty(ledger).expect("a ledger is written"))
}

/// How many Korean lines each code file of the checkout holds, the files holding none left out.
pub fn checkout_korean_counts(root: &Path) -> Ledger {
  checkout_files_where(root, is_code)
    .into_iter()
    .filter_map(|path| {
      let count = korean_line_count(&read_text(root, &path));

      (count > 0).then_some((path, count))
    })
    .collect()
}

/// What the check came to: how many code files hold Korean, a line for each holding more than the
/// ledger allows it, and how many hold fewer, which settling the ledger would lower.
#[derive(Debug, PartialEq, Eq)]
pub struct KoreanInCode {
  pub files: usize,
  pub findings: Vec<String>,
  pub below: usize,
}

/// The check over the checkout at `root`, held to the ledger at `ledger` named `named`.
pub fn checkout_korean_in_code(root: &Path, ledger: &Path, named: &str) -> Result<KoreanInCode, String> {
  let ledger = read_ledger(ledger, named)?;
  let counts = checkout_korean_counts(root);
  let allowed = |path: &str| ledger.get(path).copied().unwrap_or(0);
  let findings = counts
    .iter()
    .filter(|(path, count)| **count > allowed(path))
    .map(|(path, count)| {
      fill_marks(
        &QUICK_CHECKS.korean.over,
        &[("path", path), ("count", &count.to_string()), ("allowed", &allowed(path).to_string())],
      )
    })
    .collect();
  let below = ledger
    .iter()
    .filter(|(path, allowed)| counts.get(*path).copied().unwrap_or(0) < **allowed)
    .count();

  Ok(KoreanInCode {
    files: counts.len(),
    findings,
    below,
  })
}

/// What settling the ledger did: how many files it lowered and how many still hold Korean.
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
  let counts = checkout_korean_counts(root);
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

/// What holds a Korean line: a comment; test input; people's text, a literal in a file the declaration
/// names as such; another literal, a fixed sentence or a pattern; or a line of a language no parser here
/// reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KoreanKind {
  Comment,
  Test,
  People,
  Literal,
  Unread,
}

impl KoreanKind {
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

// What holds a line, by where its Korean stands: a line whose Korean is all in comments is a comment's,
// one whose code Korean is all in a test module is test input, one whose code Korean is all in patterns
// is a literal, and the rest is the file's own kind.
fn line_kind(offsets: &[usize], reading: &Reading, own: KoreanKind) -> KoreanKind {
  let code: Vec<usize> = offsets.iter().copied().filter(|at| !within(&reading.comments, *at)).collect();

  if code.is_empty() {
    KoreanKind::Comment
  } else if own == KoreanKind::Test || code.iter().all(|at| within(&reading.tests, *at)) {
    KoreanKind::Test
  } else if code.iter().all(|at| within(&reading.patterns, *at)) {
    KoreanKind::Literal
  } else {
    own
  }
}

/// Every line of a code file that holds Korean, counted from one, with what holds it; `people` are the
/// globs naming people's text.
pub fn korean_lines(path: &str, text: &str, people: &[String]) -> Vec<(usize, KoreanKind)> {
  if korean_offsets(text).next().is_none() {
    return Vec::new();
  }

  let reading = Language::of(path).map(|language| language.read(text));
  let own = KoreanKind::of_file(path, people);
  let mut lines = Vec::new();
  let mut start = 0;

  for (index, line) in text.split('\n').enumerate() {
    let offsets: Vec<usize> = korean_offsets(line).map(|at| start + at).collect();

    start += line.len() + 1;

    if offsets.is_empty() {
      continue;
    }

    let kind = reading.as_ref().map_or(KoreanKind::Unread, |reading| line_kind(&offsets, reading, own));

    lines.push((index + 1, kind));
  }

  lines
}

/// The Korean the code of the checkout at `root` still holds, by what holds it: without a prefix a row
/// to an area, the first two folders of a path, and with one a row to each file under it; the largest
/// rows first, cut under the answer's ceiling with the cut said.
pub fn korean_report(root: &Path, prefix: Option<&str>, people: &[String]) -> Vec<String> {
  type Tally = [usize; KoreanKind::ALL.len()];

  let words = &QUICK_CHECKS.korean;
  // The rows in the order their first file is listed, which the sort keeps among rows of one size.
  let mut rows: Vec<(String, Tally)> = Vec::new();
  let mut total: Tally = [0; KoreanKind::ALL.len()];
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
    for (_, kind) in korean_lines(&path, &read_text(root, &path), people) {
      let at = KoreanKind::ALL.iter().position(|one| *one == kind).unwrap_or_default();
      let row = row_of(&path);
      let index = rows.iter().position(|(name, _)| *name == row).unwrap_or_else(|| {
        rows.push((row, [0; KoreanKind::ALL.len()]));
        rows.len() - 1
      });

      rows[index].1[at] += 1;
      total[at] += 1;
    }
  }

  let say = |name: &str, tally: &Tally| {
    let kinds: Vec<String> = KoreanKind::ALL
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
