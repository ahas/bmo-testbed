//! The wording check (docs/checks.md, 「The quick checks」): every file of the checkout the declaration's
//! `wording_files` reach, held to the terms the repository's wording document bans, and every use the
//! document allows held to still cover something.
//!
//! The document is markdown, read by the markdown crate: its sections are its level-two headings and its
//! entries the items of their lists. An entry's own shape, its parts parted by a middle dot, an arrow and
//! a dash, is this check's own syntax, read here by its patterns. The Hangul and kana ranges and the dash
//! are written as escapes, so this source holds none of them.

use std::path::Path;
use std::sync::LazyLock;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use markdown::mdast::Node;
use markdown::{ParseOptions, to_mdast};
use regex::Regex;

pub use super::Findings;
use super::files::{checkout_files_where, is_test_file};
use crate::selection::glob_matches;

/// A word the document bans, with what to write instead, when the entry applies where its item says so,
/// and the phrases in which it keeps its own sense and passes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BannedTerm {
  pub lang: String,
  pub term: String,
  pub instead: String,
  pub when: Option<String>,
  pub keeps: Vec<String>,
}

/// A use of a banned term the document allows in one file, on a line holding the excerpt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowedUse {
  pub lang: String,
  pub term: String,
  pub path: String,
  pub excerpt: String,
}

/// The document's lists: the banned terms and the uses it allows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Glossary {
  pub banned: Vec<BannedTerm>,
  pub allowed: Vec<AllowedUse>,
}

/// One line of a text using a banned term: its number, counted from one, the line and the term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordingHit {
  pub line: usize,
  pub text: String,
  pub term: BannedTerm,
}

static BANNED_ITEM: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^([a-z]{2,3}) · (.+?) → (.+?)(?: \x{2014} (.*))?$").expect("a banned term's item")
});
static ALLOWED_ITEM: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^([a-z]{2,3}) · (.+?) · (\S+) · (.+) \x{2014} .+$").expect("an allowed use's item")
});
static OWN_SENSE_ITEM: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^([a-z]{2,3}) · (.+?) · (.+?) \x{2014} .+$").expect("an own sense's item"));

// The languages a line can be told to be in, by the script it is written in.
const KNOWN_LANGUAGES: [&str; 3] = ["ko", "ja", "en"];

fn hangul(c: char) -> bool {
  matches!(c, '\u{1100}'..='\u{11ff}' | '\u{3130}'..='\u{318f}' | '\u{ac00}'..='\u{d7af}')
}

fn kana(c: char) -> bool {
  matches!(c, '\u{3040}'..='\u{30ff}')
}

fn hangul_syllable(c: char) -> bool {
  matches!(c, '\u{ac00}'..='\u{d7a3}')
}

/// The language a line is written in, told by its script: Korean where it holds Hangul, Japanese where
/// it holds kana alone, English otherwise.
pub fn line_language(line: &str) -> &'static str {
  if line.chars().any(hangul) {
    "ko"
  } else if line.chars().any(kana) {
    "ja"
  } else {
    "en"
  }
}

// What a section of the document holds, by its heading.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
  Banned,
  OwnSense,
  Allowed,
  Other,
}

impl Section {
  fn of(heading: &str) -> Self {
    let words = &QUICK_CHECKS.wording;

    match heading.trim() {
      title if title == words.banned_section => Self::Banned,
      title if title == words.own_sense_section => Self::OwnSense,
      title if title == words.allowed_section => Self::Allowed,
      _ => Self::Other,
    }
  }
}

// The source of a node's children, from the first's start to the last's end: a list item without its
// marker.
fn inner_source<'t>(node: &Node, text: &'t str) -> Option<&'t str> {
  let children = node.children()?;
  let start = children.first()?.position()?.start.offset;
  let end = children.last()?.position()?.end.offset;

  text.get(start..end)
}

/// The document's lists out of its own sections, each entry an item of a list under its section's
/// level-two heading. An entry naming a language no line can be told in, or a phrase of a word's own
/// sense that names no banned term listed above it or does not hold it, is a fault in the document,
/// said naming `document`.
pub fn parse_wording(document: &str, text: &str) -> Result<Glossary, String> {
  let words = &QUICK_CHECKS.wording;
  let tree = to_mdast(text, &ParseOptions::default())
    .map_err(|why| fill_marks(&words.not_markdown, &[("document", document), ("why", &why.to_string())]))?;
  let mut glossary = Glossary::default();
  let mut section = Section::Other;
  let checked = |lang: &str, item: &str| -> Result<String, String> {
    if KNOWN_LANGUAGES.contains(&lang) {
      Ok(lang.to_owned())
    } else {
      Err(fill_marks(
        &words.unknown_language,
        &[("document", document), ("lang", lang), ("item", item)],
      ))
    }
  };

  for node in tree.children().into_iter().flatten() {
    match node {
      Node::Heading(heading) if heading.depth == 2 => section = Section::of(&node.to_string()),
      Node::List(_) if section != Section::Other => {
        for item in node.children().into_iter().flatten() {
          let Some(item) = inner_source(item, text) else {
            continue;
          };

          read_item(section, item, &checked, &mut glossary, document)?;
        }
      }
      _ => {}
    }
  }

  Ok(glossary)
}

// One entry of a section into the glossary; an item not in the section's shape explains the list and is
// passed over.
fn read_item(
  section: Section,
  item: &str,
  checked: &dyn Fn(&str, &str) -> Result<String, String>,
  glossary: &mut Glossary,
  document: &str,
) -> Result<(), String> {
  match section {
    Section::Banned => {
      if let Some(found) = BANNED_ITEM.captures(item) {
        glossary.banned.push(BannedTerm {
          lang: checked(&found[1], item)?,
          term: found[2].to_owned(),
          instead: found[3].to_owned(),
          when: found.get(4).map(|when| when.as_str().to_owned()),
          keeps: Vec::new(),
        });
      }
    }
    Section::Allowed => {
      if let Some(found) = ALLOWED_ITEM.captures(item) {
        glossary.allowed.push(AllowedUse {
          lang: checked(&found[1], item)?,
          term: found[2].to_owned(),
          path: found[3].to_owned(),
          excerpt: found[4].to_owned(),
        });
      }
    }
    Section::OwnSense => {
      if let Some(found) = OWN_SENSE_ITEM.captures(item) {
        let lang = checked(&found[1], item)?;
        let (term, phrase) = (&found[2], &found[3]);
        let entry = glossary.banned.iter_mut().find(|one| one.lang == lang && one.term == term);

        match entry {
          Some(entry) if phrase.contains(term) => entry.keeps.push(phrase.to_owned()),
          _ => {
            return Err(fill_marks(
              &QUICK_CHECKS.wording.own_sense,
              &[("document", document), ("item", item)],
            ));
          }
        }
      }
    }
    Section::Other => {}
  }

  Ok(())
}

// Whether the term stands on its own where it was found, rather than being the tail of a longer word: a
// Korean term another syllable runs into from the left belongs to that other word.
fn stands_alone(line: &str, at: usize, term: &BannedTerm) -> bool {
  if at == 0 || !term.term.chars().next().is_some_and(hangul_syllable) {
    return true;
  }

  !line[..at].chars().next_back().is_some_and(hangul_syllable)
}

// Whether the term, where it was found, is part of a phrase in which it keeps its own sense.
fn keeps_own_sense(line: &str, at: usize, term: &BannedTerm) -> bool {
  term.keeps.iter().any(|phrase| {
    phrase
      .find(&term.term)
      .and_then(|inside| at.checked_sub(inside))
      .is_some_and(|start| line.get(start..).is_some_and(|rest| rest.starts_with(phrase.as_str())))
  })
}

// Whether a line of the term's own language uses it anywhere it neither runs into a longer word nor
// keeps its own sense.
fn used_in(line: &str, term: &BannedTerm) -> bool {
  let mut from = 0;

  while let Some(found) = line[from..].find(&term.term) {
    let at = from + found;

    if stands_alone(line, at, term) && !keeps_own_sense(line, at, term) {
      return true;
    }
    from = at + line[at..].chars().next().map_or(1, char::len_utf8);
  }

  false
}

/// Every use of a banned term in a text, a line at a time and on the lines of the term's own language.
pub fn find_banned(text: &str, banned: &[BannedTerm]) -> Vec<WordingHit> {
  let mut hits = Vec::new();

  for (index, line) in text.split('\n').enumerate() {
    let lang = line_language(line);

    for term in banned.iter().filter(|term| term.lang == lang && used_in(line, term)) {
      hits.push(WordingHit {
        line: index + 1,
        text: line.to_owned(),
        term: term.clone(),
      });
    }
  }

  hits
}

/// What the check found over a set of files: the uses no allowance covers, each with its file, and the
/// allowances that no longer cover anything.
#[derive(Debug, Default)]
pub struct WordingReport {
  pub hits: Vec<(String, WordingHit)>,
  pub unused_allowances: Vec<AllowedUse>,
}

/// The given files, each read through `read`, held to the glossary.
pub fn check_wording(
  paths: &[String],
  read: &dyn Fn(&str) -> String,
  glossary: &Glossary,
) -> WordingReport {
  let mut used = vec![false; glossary.allowed.len()];
  let mut report = WordingReport::default();

  for path in paths {
    for hit in find_banned(&read(path), &glossary.banned) {
      let allowance = glossary.allowed.iter().position(|one| {
        one.lang == hit.term.lang
          && one.term == hit.term.term
          && one.path == *path
          && hit.text.contains(&one.excerpt)
      });

      match allowance {
        Some(at) => used[at] = true,
        None => report.hits.push((path.clone(), hit)),
      }
    }
  }

  report.unused_allowances = glossary
    .allowed
    .iter()
    .zip(used)
    .filter(|(_, used)| !used)
    .map(|(one, _)| one.clone())
    .collect();
  report
}

/// The check over the checkout at `root`, held to the document at `document`, named `named` in what it
/// says: every file it tracks, or holds new and not ignored, that `files` reach, every file where none
/// are declared, test files and the document itself aside. A file that is not UTF-8 text holds no words
/// to read. A line for each use no allowance covers, and one for each allowance that covers nothing; a
/// document that does not read is the answer's fault.
pub fn checkout_wording(
  root: &Path,
  document: &Path,
  named: &str,
  files: Option<&[String]>,
) -> Result<Findings, String> {
  let words = &QUICK_CHECKS.wording;
  let text = std::fs::read_to_string(document)
    .map_err(|why| fill_marks(&QUICK_CHECKS.unreadable, &[("path", named), ("why", &why.to_string())]))?;
  let glossary = parse_wording(named, &text)?;
  let own = document.strip_prefix(root).ok().map(|own| own.to_string_lossy().into_owned());
  let reached = |path: &str| files.is_none_or(|globs| globs.iter().any(|glob| glob_matches(glob, path)));
  let paths =
    checkout_files_where(root, |path| !is_test_file(path) && own.as_deref() != Some(path) && reached(path));
  let read = |path: &str| {
    std::fs::read(root.join(path))
      .ok()
      .and_then(|bytes| String::from_utf8(bytes).ok())
      .unwrap_or_default()
  };
  let report = check_wording(&paths, &read, &glossary);
  let hits = report.hits.iter().map(|(path, hit)| {
    fill_marks(
      &words.hit,
      &[
        ("path", path),
        ("line", &hit.line.to_string()),
        ("term", &hit.term.term),
        ("instead", &hit.term.instead),
      ],
    )
  });
  let stale = report
    .unused_allowances
    .iter()
    .map(|one| fill_marks(&words.stale, &[("path", &one.path), ("term", &one.term)]));

  Ok(Findings {
    paths: paths.len(),
    findings: hits.chain(stale).collect(),
  })
}
