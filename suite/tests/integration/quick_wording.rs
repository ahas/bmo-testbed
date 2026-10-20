//! The wording check over a checkout (docs/checks.md, 「The quick checks」): the files the declaration's
//! `wording_files` reach, test files, files that are not text and the document itself aside, held to the
//! document bmo-text's wording reads.

use bmo_testbed_suite::quick::Findings;
use bmo_testbed_suite::quick::wording::checkout_wording;

use crate::quick::{checkout, write};

const SAMPLE: &str = "# Wording\n\n## Banned terms\n\n- Each line is `<language code> \u{b7} <banned term> \u{2192} <what to write>`.\n- en \u{b7} tool strip \u{2192} toolbar\n- en \u{b7} fixing \u{2192} editing \u{2014} when it means editing\n- en \u{b7} slot \u{2192} position\n\n## Uses in the word's own sense\n\n- Each line is `<language code> \u{b7} <banned term> \u{b7} <a phrase holding the term>`.\n- en \u{b7} slot \u{b7} slot machine \u{2014} a machine\n- en \u{b7} slot \u{b7} coin slot \u{2014} an opening\n\n## Allowed uses\n\n- en \u{b7} fixing \u{b7} docs/a.md \u{b7} bike fixing \u{2014} a repair\n";

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
      .starts_with("words.md: ")
  );
  write(dir.path(), "words.md", "## Banned terms\n\n- en \u{b7} a \u{2192} b\n");
  assert!(checkout_wording(dir.path(), &dir.path().join("words.md"), "words.md", None).is_ok());
}
