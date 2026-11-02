//! A source file as the quick checks read it (docs/checks.md, 「The quick checks」): its comments, its
//! patterns, its test modules and the pieces of text its code writes, each language by its own parser.

use bmo_testbed_suite::quick::source::{Language, holds_dash, is_placeholder};

fn pieces(language: Language, text: &str) -> Vec<(usize, String)> {
  language.read(text).pieces.into_iter().map(|piece| (piece.line, piece.text)).collect()
}

// "a path is read by the parser of its language, and one no parser here reads is not"
#[test]
fn a_path_is_read_by_the_parser_of_its_language_and_one_no_parser_here_reads_is_not() {
  assert_eq!(Language::of("a/b.mts"), Some(Language::Script { tsx: false }));
  assert_eq!(Language::of("a/b.tsx"), Some(Language::Script { tsx: true }));
  assert_eq!(Language::of("a/B.vue"), Some(Language::Screen));
  assert_eq!(Language::of("a/b.rs"), Some(Language::Rust));
  assert_eq!(Language::of("a/b.py"), None);
}

// "a script gives its comments, its patterns and its literals, a template whole"
#[test]
fn a_script_gives_its_comments_its_patterns_and_its_literals_a_template_whole() {
  let text = "// note\nconst a = \"one\";\nconst b = /x/;\nconst c = `head ${d ? `in` : \"or\"} tail`;\n";
  let reading = Language::Script { tsx: false }.read(text);

  assert_eq!(reading.comments.len(), 1);
  assert_eq!(&text[reading.comments[0].clone()], "// note");
  assert_eq!(&text[reading.patterns[0].clone()], "/x/");
  assert_eq!(
    pieces(Language::Script { tsx: false }, text),
    [(2, "one".to_owned()), (4, "head \u{2026}in\u{2026}or\u{2026} tail".to_owned())]
  );
}

// "a Rust file gives its comments, its test modules and its strings outside them"
#[test]
fn a_rust_file_gives_its_comments_its_test_modules_and_its_strings_outside_them() {
  let text = "/// doc\nfn f() -> String { format!(\"said {x}\") }\n#[cfg(test)]\nmod tests {\n  const A: &str = \"t\";\n}\n";
  let reading = Language::Rust.read(text);

  assert_eq!(&text[reading.comments[0].clone()], "/// doc\n");
  assert!(text[reading.tests[0].clone()].starts_with("mod tests"));
  assert_eq!(pieces(Language::Rust, text), [(2, "\"said {x}\"".to_owned())]);
}

// "a screen gives its script, its template's words, attributes and expressions, and its styles as
// comments"
#[test]
fn a_screen_gives_its_script_its_template_words_attributes_and_expressions_and_its_styles_as_comments() {
  let text = [
    "<script setup lang=\"ts\">",
    "// note",
    "const label = \"one\";",
    "</script>",
    "<template>",
    "  <!-- aside -->",
    "  <p title=\"two\" :class=\"'three'\" v-else>four {{ `five ${n}` }} six</p>",
    "</template>",
    "<style>",
    "/* styled */",
    "</style>",
  ]
  .join("\n");
  let reading = Language::Screen.read(&text);
  let comments: Vec<&str> = reading.comments.iter().map(|span| &text[span.clone()]).collect();

  assert_eq!(comments, ["<!-- aside -->", "// note", "<style>\n/* styled */\n</style>"]);
  assert_eq!(
    pieces(Language::Screen, &text),
    [
      (3, "one".to_owned()),
      (7, "two".to_owned()),
      (7, "three".to_owned()),
      (7, "four".to_owned()),
      (7, "five ".to_owned()),
      (7, "six".to_owned()),
    ]
  );
}

// "a dash between words is an em dash, a horizontal bar or a spaced en dash, and a lone one is a
// placeholder"
#[test]
fn a_dash_between_words_is_an_em_dash_a_bar_or_a_spaced_en_dash_and_a_lone_one_is_a_placeholder() {
  assert!(holds_dash("a \u{2014} b") && holds_dash("a\u{2015}b") && holds_dash("a \u{2013} b"));
  assert!(!holds_dash("1\u{2013}3") && !holds_dash("a - b"));
  assert!(is_placeholder("\u{2014}") && !is_placeholder("\u{2014} a"));
}
