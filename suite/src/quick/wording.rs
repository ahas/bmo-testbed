//! The wording check (docs/checks.md, 「The quick checks」): every file of the checkout the declaration's
//! `wording_files` reach, held to the terms the repository's wording document bans, and every use the
//! document allows held to still cover something. Reading the document, finding a banned term and covering
//! a use are bmo-text's wording; which files of the checkout are read is the testbed's own.

use std::convert::Infallible;
use std::path::Path;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use bmo_text::wording::{check_wording, parse_wording};

use super::Findings;
use super::files::{checkout_files_where, is_test_file};
use crate::selection::glob_matches;

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
  let unreadable = |why: &str| fill_marks(&QUICK_CHECKS.unreadable, &[("path", named), ("why", why)]);
  let text = std::fs::read_to_string(document).map_err(|why| unreadable(&why.to_string()))?;
  let glossary = parse_wording(&text).map_err(|why| unreadable(&why))?;
  let own = document.strip_prefix(root).ok().map(|own| own.to_string_lossy().into_owned());
  let reached = |path: &str| files.is_none_or(|globs| globs.iter().any(|glob| glob_matches(glob, path)));
  let paths =
    checkout_files_where(root, |path| !is_test_file(path) && own.as_deref() != Some(path) && reached(path));
  let read = |path: &str| {
    Ok::<_, Infallible>(
      std::fs::read(root.join(path))
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default(),
    )
  };
  let named_paths: Vec<&str> = paths.iter().map(String::as_str).collect();
  let Ok(report) = check_wording(&named_paths, read, &glossary);
  let hits = report.hits.iter().map(|one| {
    fill_marks(
      &words.hit,
      &[
        ("path", &one.path),
        ("line", &one.hit.line.to_string()),
        ("term", &one.hit.term.term),
        ("instead", &one.hit.term.instead),
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
