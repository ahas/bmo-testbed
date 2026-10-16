//! The wording check (docs/checks.md, 「The quick checks」): the wording document's sections read as
//! markdown, a banned term found a line at a time on lines of its own language, the uses the document
//! allows, and the files of a checkout the declaration's `wording_files` reach.

use std::collections::HashMap;

use bmo_testbed_suite::quick::Findings;
use bmo_testbed_suite::quick::wording::{
  AllowedUse, BannedTerm, check_wording, checkout_wording, find_banned, line_language, parse_wording,
};

use crate::quick::{checkout, write};

// Korean words, written by their escapes so this file holds none of its own: a bridge, a path, a ladder
// and the words around them.
const BRIDGE: &str = "\u{b2e4}\u{b9ac}";
const PATH: &str = "\u{acbd}\u{b85c}";
const UNDER: &str = "\u{bc11}";

const SAMPLE: &str = "# Wording\n\n## Banned terms\n\n- Each line is `<language code> \u{b7} <banned term> \u{2192} <what to write>`.\n- en \u{b7} tool strip \u{2192} toolbar\n- en \u{b7} fixing \u{2192} editing \u{2014} when it means editing\n- en \u{b7} slot \u{2192} position\n\n## Uses in the word's own sense\n\n- Each line is `<language code> \u{b7} <banned term> \u{b7} <a phrase holding the term>`.\n- en \u{b7} slot \u{b7} slot machine \u{2014} a machine\n- en \u{b7} slot \u{b7} coin slot \u{2014} an opening\n\n## Allowed uses\n\n- en \u{b7} fixing \u{b7} docs/a.md \u{b7} bike fixing \u{2014} a repair\n";

fn term(term: &str, instead: &str, when: Option<&str>) -> BannedTerm {
  BannedTerm {
    lang: "en".to_owned(),
    term: term.to_owned(),
    instead: instead.to_owned(),
    when: when.map(str::to_owned),
    keeps: vec![],
  }
}

// "the lists are read from their own sections, and the explanation bullets are not"
#[test]
fn the_lists_are_read_from_their_own_sections_and_the_explanation_bullets_are_not() {
  let glossary = parse_wording("words.md", SAMPLE).unwrap();
  let mut slot = term("slot", "position", None);

  slot.keeps = vec!["slot machine".to_owned(), "coin slot".to_owned()];

  assert_eq!(
    glossary.banned,
    [
      term("tool strip", "toolbar", None),
      term("fixing", "editing", Some("when it means editing")),
      slot
    ]
  );
  assert_eq!(
    glossary.allowed,
    [AllowedUse {
      lang: "en".to_owned(),
      term: "fixing".to_owned(),
      path: "docs/a.md".to_owned(),
      excerpt: "bike fixing".to_owned(),
    }]
  );
}

// "a list under a heading of another section or level is no entry"
#[test]
fn a_list_under_a_heading_of_another_section_or_level_is_no_entry() {
  let text = "## Notes\n\n- en \u{b7} a \u{2192} b\n\n## Banned terms\n\n### Banned terms\n\n- en \u{b7} c \u{2192} d\n\n## Other\n\n- en \u{b7} e \u{2192} f\n";
  let banned: Vec<String> = parse_wording("words.md", text)
    .unwrap()
    .banned
    .into_iter()
    .map(|one| one.term)
    .collect();

  assert_eq!(banned, ["c"]);
}

// "an entry naming an unknown language, or an own sense naming no banned term above it, is the
// document's fault, said naming it"
#[test]
fn an_unknown_language_or_an_own_sense_naming_no_banned_term_above_it_is_the_documents_fault() {
  assert_eq!(
    parse_wording("words.md", "## Banned terms\n\n- xx \u{b7} a \u{2192} b\n").unwrap_err(),
    "words.md: unknown language code \"xx\" in: xx \u{b7} a \u{2192} b"
  );
  assert_eq!(
    parse_wording(
      "words.md",
      "## Banned terms\n\n- en \u{b7} slot \u{2192} position\n\n## Uses in the word's own sense\n\n- en \u{b7} seat \u{b7} seat belt \u{2014} a belt\n"
    )
    .unwrap_err(),
    "words.md: a phrase of a word's own sense must hold a banned term listed above it: en \u{b7} seat \u{b7} seat belt \u{2014} a belt"
  );
}

// "a banned word is found where it stands alone, and not in its own sense or inside a longer word"
#[test]
fn a_banned_word_is_found_where_it_stands_alone_and_not_in_its_own_sense_or_inside_a_longer_word() {
  let document = format!(
    "## Banned terms\n\n- ko \u{b7} {BRIDGE} \u{2192} {PATH}\n- en \u{b7} utilise \u{2192} use \u{2014} always\n\n## Uses in the word's own sense\n\n- ko \u{b7} {BRIDGE} \u{b7} {BRIDGE} {UNDER} \u{2014} under a bridge\n"
  );
  let glossary = parse_wording("words.md", &document).unwrap();
  let found = |text: &str| {
    find_banned(text, &glossary.banned)
      .into_iter()
      .map(|hit| hit.term.term)
      .collect::<Vec<_>>()
  };

  // "this bridge, go by it"
  assert_eq!(found(&format!("\u{c774} {BRIDGE}\u{b85c} \u{ac00}")), [BRIDGE]);
  // "under the bridge"
  assert!(found(&format!("{BRIDGE} {UNDER}\u{c5d0}")).is_empty());
  // "climb the ladder", the term the tail of a longer word
  assert!(found(&format!("\u{c0ac}{BRIDGE}\u{b97c} \u{c62c}\u{b77c}")).is_empty());
  assert_eq!(found("we utilise it"), ["utilise"]);
  assert_eq!(glossary.banned[1].when.as_deref(), Some("always"));
}

// "a term is found a line at a time, on lines of its own language, and not in its own sense"
#[test]
fn a_term_is_found_a_line_at_a_time_on_lines_of_its_own_language_and_not_in_its_own_sense() {
  let banned = parse_wording("words.md", SAMPLE).unwrap().banned;
  // A Korean word, written by its escapes, makes its line a Korean one.
  let korean_line = "\u{c704}\u{c5d0} tool strip";
  let hits =
    find_banned(&format!("{korean_line}\nthe tool strip above\na slot machine\nan empty slot"), &banned);

  assert_eq!(line_language(korean_line), "ko");
  assert_eq!(line_language("\u{30c4}\u{30fc}\u{30eb}"), "ja");
  assert_eq!(line_language("plain"), "en");
  assert_eq!(
    hits.iter().map(|hit| (hit.line, hit.term.term.as_str())).collect::<Vec<_>>(),
    [(2, "tool strip"), (4, "slot")]
  );
  assert_eq!(hits[0].text, "the tool strip above");
}

// "an allowed use passes only in its own file and line, and a stale one is reported"
#[test]
fn an_allowed_use_passes_only_in_its_own_file_and_line_and_a_stale_one_is_reported() {
  let glossary = parse_wording("words.md", SAMPLE).unwrap();
  let files: HashMap<&str, &str> = HashMap::from([
    ("docs/a.md", "bike fixing\ndocument fixing\n"),
    ("docs/b.md", "bike fixing\nthe tool strip\n"),
  ]);
  let paths = ["docs/a.md".to_owned(), "docs/b.md".to_owned()];
  let report = check_wording(&paths, &|path| files[path].to_owned(), &glossary);
  let hits: Vec<String> = report
    .hits
    .iter()
    .map(|(path, hit)| format!("{path}:{}:{}", hit.line, hit.term.term))
    .collect();

  assert_eq!(hits, ["docs/a.md:2:fixing", "docs/b.md:1:fixing", "docs/b.md:2:tool strip"]);
  assert!(report.unused_allowances.is_empty());
  assert_eq!(check_wording(&[], &|_| String::new(), &glossary).unused_allowances.len(), 1);
}

// "a checkout is read over every file, a test file, a file that is not text and the document aside"
#[test]
fn a_checkout_is_read_over_every_file_a_test_file_a_file_that_is_not_text_and_the_document_aside() {
  let dir = checkout(&[
    ("meta/words.md", SAMPLE),
    ("docs/a.md", "bike fixing\n"),
    ("scripts/say.py", "print(\"the tool strip\")\n"),
    ("scripts/say.test.ts", "the tool strip\n"),
    ("tests/flow.rs", "the tool strip\n"),
  ]);
  let mut binary = b"the tool strip ".to_vec();

  binary.extend([0xff, 0xfe]);
  std::fs::write(dir.path().join("image.bin"), binary).unwrap();

  assert_eq!(
    checkout_wording(dir.path(), &dir.path().join("meta/words.md"), "meta/words.md", None).unwrap(),
    Findings {
      paths: 3,
      findings: vec!["scripts/say.py:1: \"tool strip\" \u{2192} toolbar".to_owned()],
    }
  );
}

// "the declared globs narrow the files, and an allowance in a file they leave out goes stale"
#[test]
fn the_declared_globs_narrow_the_files_and_an_allowance_in_a_file_they_leave_out_goes_stale() {
  let dir = checkout(&[
    ("words.md", SAMPLE),
    ("docs/a.md", "bike fixing\n"),
    ("scripts/say.py", "print(\"the tool strip\")\n"),
    ("notes.txt", "the tool strip\n"),
  ]);
  let globs = ["scripts/**".to_owned()];

  assert_eq!(
    checkout_wording(dir.path(), &dir.path().join("words.md"), "words.md", Some(&globs)).unwrap(),
    Findings {
      paths: 1,
      findings: vec![
        "scripts/say.py:1: \"tool strip\" \u{2192} toolbar".to_owned(),
        "docs/a.md: allowed \"fixing\" matches nothing".to_owned(),
      ],
    }
  );
}

// "a document that cannot be read, or does not read, is the answer's fault"
#[test]
fn a_document_that_cannot_be_read_or_does_not_read_is_the_answers_fault() {
  let dir = checkout(&[("words.md", "## Banned terms\n\n- xx \u{b7} a \u{2192} b\n")]);

  assert!(
    checkout_wording(dir.path(), &dir.path().join("absent.md"), "absent.md", None)
      .unwrap_err()
      .starts_with("absent.md: ")
  );
  assert!(
    checkout_wording(dir.path(), &dir.path().join("words.md"), "words.md", None)
      .unwrap_err()
      .contains("unknown language code")
  );
  write(dir.path(), "words.md", "## Banned terms\n\n- en \u{b7} a \u{2192} b\n");
  assert!(checkout_wording(dir.path(), &dir.path().join("words.md"), "words.md", None).is_ok());
}
