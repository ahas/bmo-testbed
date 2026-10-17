//! A markdown file's dashes between words rewritten (docs/checks.md, 「The quick checks」): the way a person
//! types the same sentence in its prose, and every dash of what is not prose left as it stands.

use bmo_testbed_suite::quick::rewrite::without_dashes;

#[test]
fn a_name_and_its_explanation_get_a_colon() {
  assert_eq!(without_dashes("The tool \u{2014} it reads files"), "The tool: it reads files");
}

#[test]
fn an_aside_gets_commas_and_a_range_a_tilde() {
  assert_eq!(
    without_dashes("The run \u{2014} the second one \u{2014} failed."),
    "The run, the second one, failed."
  );
  assert_eq!(without_dashes("pages 3\u{2014}5"), "pages 3~5");
}

#[test]
fn an_aside_holding_commas_of_its_own_gets_brackets() {
  assert_eq!(
    without_dashes("The run \u{2014} the first, the second \u{2014} failed."),
    "The run (the first, the second) failed."
  );
}

#[test]
fn what_is_not_prose_keeps_its_dashes() {
  for text in [
    "`a \u{2014} b`",
    "> a \u{2014} b",
    "a range 3\u{2013}5",
    "```\na \u{2014} b\n```",
    "see [a \u{2014} b](https://example.com/a)",
    "she said \"a \u{2014} b\"",
    "| a | \u{2014} |\n|---|---|\n| b | \u{2014} |",
  ] {
    assert_eq!(without_dashes(text), text, "{text}");
  }
}

// "a document's paragraph wrapped over lines is one piece, its list marks and the rest of the file kept"
#[test]
fn a_wrapped_paragraph_is_one_piece_and_the_rest_of_the_file_stands() {
  let text = "# Title\n\n- A rule \u{2014} its reason\n  wrapped \u{2014} over two lines.\n\n```\nkept \u{2014} here\n```\n";

  assert_eq!(
    without_dashes(text),
    "# Title\n\n- A rule, its reason\n  wrapped, over two lines.\n\n```\nkept \u{2014} here\n```\n"
  );
}
