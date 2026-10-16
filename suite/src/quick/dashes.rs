//! The dash check over the text the code writes for a person (docs/checks.md, 「The quick checks」): a
//! dash between words in it is refused where it is written. People's text is every literal in the files
//! the declaration's `people_text` reach, read off the source rather than off a list of functions.
//!
//! Each source language is read by its own parser (`source`), a YAML file by the YAML reader and a TOML
//! file by the TOML reader. The dashes here are written as escapes, so this source holds none.

use std::path::Path;
use std::sync::LazyLock;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use regex::Regex;
use serde_json::Value;

use super::Findings;
use super::files::{checkout_files_where, is_test_file, read_text};
use super::source::{Language, Piece, holds_dash, is_placeholder};
use crate::selection::glob_matches;

static DASH: LazyLock<Regex> =
  LazyLock::new(|| Regex::new("[\u{2014}\u{2015}]| \u{2013} ").expect("a dash"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("a run of spaces"));

/// How a file of people's text is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reader {
  Yaml,
  Toml,
  Source(Language),
}

impl Reader {
  // How a path is read, by its extension, or none for a file no reader here reads.
  fn of(path: &str) -> Option<Self> {
    match path.rsplit_once('.').map(|(_, extension)| extension)? {
      "yaml" | "yml" => Some(Self::Yaml),
      "toml" => Some(Self::Toml),
      _ => Language::of(path).map(Self::Source),
    }
  }

  fn pieces(self, text: &str) -> Vec<Piece> {
    match self {
      Self::Yaml => data_pieces(text, serde_saphyr::from_str::<Value>(text).ok()),
      Self::Toml => data_pieces(text, toml::from_str::<Value>(text).ok()),
      Self::Source(language) => language.read(text).pieces,
    }
  }
}

// Every string a data file holds, read by its own parser, at the line its value opens: the first line
// holding the value's first line. A file that does not read gives none, its own check refusing it.
fn data_pieces(text: &str, value: Option<Value>) -> Vec<Piece> {
  fn strings(value: &Value, found: &mut Vec<String>) {
    match value {
      Value::String(text) => found.push(text.clone()),
      Value::Array(items) => items.iter().for_each(|item| strings(item, found)),
      Value::Object(fields) => fields.values().for_each(|item| strings(item, found)),
      _ => {}
    }
  }

  let mut found = Vec::new();

  if let Some(value) = value {
    strings(&value, &mut found);
  }

  let lines: Vec<&str> = text.split('\n').collect();
  let mut pieces: Vec<Piece> = found
    .into_iter()
    .map(|value| {
      let first = value.split('\n').next().unwrap_or_default();

      Piece {
        line: lines.iter().position(|line| line.contains(first)).map_or(0, |at| at + 1),
        text: value,
      }
    })
    .collect();

  pieces.sort_by_key(|piece| piece.line);
  pieces
}

/// A dash in people's text: the file, the line its piece opens on and the piece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashHit {
  pub path: String,
  pub line: usize,
  pub text: String,
}

/// Whether the check reads a path as people's text: one of the globs reaches it, a reader here reads its
/// kind, and it is no test file.
pub fn is_people_text(path: &str, globs: &[String]) -> bool {
  !is_test_file(path) && Reader::of(path).is_some() && globs.iter().any(|glob| glob_matches(glob, path))
}

/// Every dash in people's text among the given files, each read by the reader of its kind.
pub fn find_written_dashes(paths: &[String], read: &dyn Fn(&str) -> String) -> Vec<DashHit> {
  let mut hits = Vec::new();

  for path in paths {
    let Some(reader) = Reader::of(path) else {
      continue;
    };
    let text = read(path);

    if !holds_dash(&text) {
      continue;
    }

    hits.extend(
      reader
        .pieces(&text)
        .into_iter()
        .filter(|piece| holds_dash(&piece.text) && !is_placeholder(&piece.text))
        .map(|piece| DashHit {
          path: path.clone(),
          line: piece.line,
          text: piece.text,
        }),
    );
  }

  hits
}

/// The stretch of a piece around its first dash, on one line, an ellipsis where it was cut.
pub fn excerpt(text: &str) -> String {
  let reach = QUICK_CHECKS.dashes.excerpt_reach;
  let flat: Vec<char> = SPACES.replace_all(text, " ").chars().collect();
  let joined: String = flat.iter().collect();
  let at = DASH.find(&joined).map_or(0, |found| joined[..found.start()].chars().count());
  let from = at.saturating_sub(reach);
  let to = (at + reach).min(flat.len());
  let stretch: String = flat[from..to].iter().collect();

  format!(
    "{}{stretch}{}",
    if from > 0 { "\u{2026}" } else { "" },
    if at + reach < flat.len() { "\u{2026}" } else { "" }
  )
}

/// The check over the checkout's tracked and new files that `globs` name as people's text.
pub fn checkout_written_dashes(root: &Path, globs: &[String]) -> Findings {
  let paths = checkout_files_where(root, |path| is_people_text(path, globs));
  let hits = find_written_dashes(&paths, &|path| read_text(root, path));

  Findings {
    paths: paths.len(),
    findings: hits
      .iter()
      .map(|hit| {
        fill_marks(
          &QUICK_CHECKS.dashes.hit,
          &[("path", &hit.path), ("line", &hit.line.to_string()), ("excerpt", &excerpt(&hit.text))],
        )
      })
      .collect(),
  }
}
