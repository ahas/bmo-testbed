//! Korean in the code (docs/checks.md, 「The quick checks」): which files are code, what holds each Korean
//! line, the report by kind, and the ledger the repository declares, which only ever comes down.

use bmo_testbed_suite::quick::korean::{
  KoreanInCode, Ledger, Settled, checkout_korean_in_code, is_code, korean_line_count, korean_lines,
  korean_report, ledger_text, read_ledger, settle,
};
use bmo_testbed_suite::quick::korean_check;

use crate::quick::{checkout, env, repository, write};

// A Korean word, written by its escapes so this file holds none of its own.
const WORD: &str = "\u{bcf4}\u{b958}";

fn kinds(path: &str, text: &str, people: &[&str]) -> Vec<(usize, &'static str)> {
  let people: Vec<String> = people.iter().map(|glob| (*glob).to_owned()).collect();

  korean_lines(path, text, &people)
    .into_iter()
    .map(|(line, kind)| (line, kind.name()))
    .collect()
}

fn ledger(counts: &[(&str, u64)]) -> String {
  let ledger: Ledger = counts.iter().map(|(path, count)| ((*path).to_owned(), *count)).collect();

  ledger_text(&ledger)
}

// "a language pack, a policy file, a document, a prompt and a data file are not code"
#[test]
fn a_language_pack_a_policy_file_a_document_a_prompt_and_a_data_file_are_not_code() {
  for path in [
    "app/locales/en.yaml",
    "policy/lines.toml",
    "docs/sample.md",
    "prompts/sample.md",
    "tests/fixtures/sample.json",
    "app/__snapshots__/sample.test.ts.snap",
  ] {
    assert!(!is_code(path), "{path}");
  }

  for path in ["app/sample.ts", "app/pages/sample.vue", "crates/sample/src/lib.rs", "gate/sample.py"] {
    assert!(is_code(path), "{path}");
  }
}

// "in a module a comment is told from a literal, and a line holding both is the literal's"
#[test]
fn in_a_module_a_comment_is_told_from_a_literal_and_a_line_holding_both_is_the_literals() {
  let source = [
    format!("// {WORD} in a comment"),
    format!("const a = \"{WORD}\";"),
    "const b = 'english';".to_owned(),
    format!("/* {WORD}"),
    format!("   {WORD} */ const c = 1;"),
    format!("const d = `${{a}} {WORD}`; // {WORD}"),
  ]
  .join("\n");

  assert_eq!(
    kinds("app/sample.ts", &source, &[]),
    [(1, "comment"), (2, "literal"), (4, "comment"), (5, "comment"), (6, "literal")]
  );
}

// "a test file's code is test input, and a literal in people's text is people's"
#[test]
fn a_test_files_code_is_test_input_and_a_literal_in_peoples_text_is_peoples() {
  assert_eq!(
    kinds("app/sample.test.ts", &format!("const a = \"{WORD}\"; // {WORD}"), &[]),
    [(1, "test")]
  );
  assert_eq!(
    kinds("app/domain/sample.ts", &format!("export const a = \"{WORD}\";"), &["app/domain/**"]),
    [(1, "people")]
  );
  assert_eq!(
    kinds("app/other/sample.ts", &format!("export const a = \"{WORD}\";"), &["app/domain/**"]),
    [(1, "literal")]
  );
}

// "a screen's template text is its file's kind, and its template comment and style are comments"
#[test]
fn a_screens_template_text_is_its_files_kind_and_its_template_comment_and_style_are_comments() {
  let screen = [
    "<script setup lang=\"ts\">".to_owned(),
    format!("// {WORD}"),
    format!("const label = \"{WORD}\";"),
    "</script>".to_owned(),
    "<template>".to_owned(),
    format!("  <!-- {WORD} -->"),
    format!("  <p>{WORD}</p>"),
    "</template>".to_owned(),
    "<style>".to_owned(),
    format!("/* {WORD} */"),
    "</style>".to_owned(),
  ]
  .join("\n");

  assert_eq!(
    kinds("app/components/Sample.vue", &screen, &["app/**"]),
    [(2, "comment"), (3, "people"), (6, "comment"), (7, "people"), (10, "comment")]
  );
}

// "a Rust file's comments, literals and test module are told apart"
#[test]
fn a_rust_files_comments_literals_and_test_module_are_told_apart() {
  let source = [
    format!("/// {WORD}"),
    format!("fn say() -> String {{ format!(\"{WORD}\") }}"),
    "#[cfg(test)]".to_owned(),
    "mod tests {".to_owned(),
    format!("  const SAID: &str = \"{WORD}\";"),
    "}".to_owned(),
  ]
  .join("\n");

  assert_eq!(
    kinds("crates/sample/src/lib.rs", &source, &[]),
    [(1, "comment"), (2, "literal"), (5, "test")]
  );
}

// "a language no parser here reads is counted unread"
#[test]
fn a_language_no_parser_here_reads_is_counted_unread() {
  assert_eq!(
    kinds("gate/sample.py", &format!("# {WORD}\nprint(\"{WORD}\")"), &[]),
    [(1, "unread"), (2, "unread")]
  );
}

// "the report gives a row to each area, or to each file under a prefix, the largest first"
#[test]
fn the_report_gives_a_row_to_each_area_or_to_each_file_under_a_prefix_the_largest_first() {
  let dir = checkout(&[
    ("app/bot/one.ts", &format!("// {WORD}\nconst a = \"{WORD}\";\n")),
    ("app/bot/two.ts", &format!("const a = \"{WORD}\";\n")),
    ("crates/sample/src/lib.rs", &format!("/// {WORD}\n")),
    ("top.py", &format!("# {WORD}\n")),
  ]);
  let people = ["app/bot/two.ts".to_owned()];

  assert_eq!(
    korean_report(dir.path(), None, &people),
    [
      "3 areas  5  comment 2, test 0, people 1, literal 1, unread 1",
      "app/bot  3  comment 1, test 0, people 1, literal 1, unread 0",
      "crates/sample  1  comment 1, test 0, people 0, literal 0, unread 0",
      ".  1  comment 0, test 0, people 0, literal 0, unread 1",
    ]
  );
  assert_eq!(
    korean_report(dir.path(), Some("app/"), &people),
    [
      "2 files  3  comment 1, test 0, people 1, literal 1, unread 0",
      "app/bot/one.ts  2  comment 1, test 0, people 0, literal 1, unread 0",
      "app/bot/two.ts  1  comment 0, test 0, people 1, literal 0, unread 0",
    ]
  );
}

// "a line is counted once, however much Korean it holds"
#[test]
fn a_line_is_counted_once_however_much_korean_it_holds() {
  assert_eq!(korean_line_count(&format!("// {WORD}\nlet a = \"{WORD} {WORD}\";\nlet b = 1;\n")), 2);
  assert_eq!(korean_line_count("\u{3131}\n\u{1100}\nplain"), 2);
}

// "a ledger not yet written allows none, and one that does not read is said to"
#[test]
fn a_ledger_not_yet_written_allows_none_and_one_that_does_not_read_is_said_to() {
  let dir = tempfile::tempdir().unwrap();

  assert_eq!(read_ledger(&dir.path().join("absent.json"), "absent.json").unwrap(), Ledger::new());

  write(dir.path(), "bad.json", "[1, 2]");
  assert!(
    read_ledger(&dir.path().join("bad.json"), "bad.json")
      .unwrap_err()
      .starts_with("bad.json does not read as a ledger: ")
  );
}

// "refuses a file holding more Korean lines than the ledger allows, a file it does not name included"
#[test]
fn refuses_a_file_holding_more_korean_lines_than_the_ledger_allows_a_file_it_does_not_name_included() {
  let dir = checkout(&[
    ("app/grown.ts", &format!("const a = \"{WORD}\";\nconst b = \"{WORD}\";\n")),
    ("app/new.ts", &format!("// {WORD}\n")),
    ("app/kept.ts", &format!("const a = \"{WORD}\";\n")),
    ("docs/note.md", &format!("{WORD}\n")),
    ("ledger.json", &ledger(&[("app/grown.ts", 1), ("app/kept.ts", 1)])),
  ]);
  let over = "lines hold Korean where the ledger allows";
  let fix = "write it in English, or move it out of the code";

  assert_eq!(
    checkout_korean_in_code(dir.path(), &dir.path().join("ledger.json"), "ledger.json").unwrap(),
    KoreanInCode {
      files: 3,
      findings: vec![format!("app/grown.ts: 2 {over} 1: {fix}"), format!("app/new.ts: 1 {over} 0: {fix}"),],
      below: 0,
    }
  );
}

// "passes a file holding fewer, counts it below, and settling lowers it and never raises one"
#[test]
fn passes_a_file_holding_fewer_counts_it_below_and_settling_lowers_it_and_never_raises_one() {
  let dir = checkout(&[
    ("app/shrunk.ts", &format!("const a = \"{WORD}\";\n")),
    ("app/clean.ts", "const a = 1;\n"),
    ("app/grown.ts", &format!("// {WORD}\n// {WORD}\n")),
    (
      "ledger.json",
      &ledger(&[("app/shrunk.ts", 3), ("app/clean.ts", 2), ("app/grown.ts", 1)]),
    ),
  ]);
  let at = dir.path().join("ledger.json");

  assert_eq!(
    checkout_korean_in_code(dir.path(), &at, "ledger.json").unwrap(),
    KoreanInCode {
      files: 2,
      findings: vec![
        "app/grown.ts: 2 lines hold Korean where the ledger allows 1: write it in English, or move it out of the code"
          .to_owned()
      ],
      below: 2,
    }
  );
  assert_eq!(
    settle(dir.path(), &at, "ledger.json").unwrap(),
    Settled {
      lowered: 2,
      kept: 2
    }
  );
  assert_eq!(
    std::fs::read_to_string(&at).unwrap(),
    "{\n  \"app/grown.ts\": 1,\n  \"app/shrunk.ts\": 1\n}\n"
  );
}

// "the check reads the ledger at the path the repository declares and says its count below it"
#[test]
fn the_check_reads_the_ledger_at_the_path_the_repository_declares_and_says_its_count_below_it() {
  let dir = checkout(&[
    (
      "testbed.toml",
      "testbed = 0\n[check]\nquick = [\"korean\"]\nkorean_ledger = \"meta/korean.json\"\n",
    ),
    ("app/a.ts", &format!("const a = \"{WORD}\";\n")),
    ("meta/korean.json", &ledger(&[("app/a.ts", 2)])),
  ]);
  let checked = korean_check(&repository(&dir), &env());

  assert!(!checked.faulted);
  assert_eq!(
    checked.lines,
    [
      "quick check: korean (meta/korean.json): 1 files within the ledger, 1 below it: bmo-testbed check korean --settle lowers it"
    ]
  );
}
