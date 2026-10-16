//! The dash check over people's text (docs/checks.md, 「The quick checks」): every literal in the files
//! `people_text` reaches, each read by the reader of its kind, and none of its comments, patterns or test
//! modules.

use bmo_testbed_suite::quick::dashes::{
  checkout_written_dashes, excerpt, find_written_dashes, is_people_text,
};

use crate::quick::checkout;

fn hits_in(path: &str, text: &str) -> Vec<(usize, String)> {
  find_written_dashes(&[path.to_owned()], &|_| text.to_owned())
    .into_iter()
    .map(|hit| (hit.line, hit.text))
    .collect()
}

fn globs(globs: &[&str]) -> Vec<String> {
  globs.iter().map(|glob| (*glob).to_owned()).collect()
}

// "a literal or a template holding a dash is refused, its comments and patterns are not"
#[test]
fn a_literal_or_a_template_holding_a_dash_is_refused_and_its_comments_and_patterns_are_not() {
  let source = [
    "// held \u{2014} a comment no screen shows",
    "const a = \"held \u{2014} reason\";",
    "const b = `${why} \u{2014} #${id} held`;",
    "const c = /^- (.+?) \u{2014} (.*)$/;",
    "const d = \"held: reason\";",
  ]
  .join("\n");

  assert_eq!(
    hits_in("app/sample.ts", &source),
    [(2, "held \u{2014} reason".to_owned()), (3, " \u{2014} #\u{2026} held".to_owned())]
  );
}

// "a template is read with what its holes hold, and a lone dash standing for no value passes"
#[test]
fn a_template_is_read_with_what_its_holes_hold_and_a_lone_dash_standing_for_no_value_passes() {
  let source = [
    "const a = `held: ${held ? ` \u{2014} ${reason}` : \"\"}`;",
    "const b = `held: ${value ?? \"\u{2014}\"}`;",
    "export const c = `${from} \u{2013} ${to}`;",
  ]
  .join("\n");

  assert_eq!(
    hits_in("app/sample.ts", &source),
    [(1, "held: \u{2026} \u{2014} ".to_owned()), (3, " \u{2013} ".to_owned())]
  );
}

// "a screen's template text, attributes and expressions are read with its script"
#[test]
fn a_screens_template_text_attributes_and_expressions_are_read_with_its_script() {
  let screen = [
    "<script setup lang=\"ts\">",
    "const label = \"held \u{2014} reason\";",
    "</script>",
    "<template>",
    "  <p title=\"held \u{2014} reason\">held \u{2014} reason</p>",
    "  <span>{{ `${count} held \u{2014} reason` }}</span>",
    "  <span>{{ value ?? \"\u{2014}\" }}</span>",
    "</template>",
  ]
  .join("\n");
  let mut lines: Vec<usize> = hits_in("app/components/Sample.vue", &screen)
    .into_iter()
    .map(|(line, _)| line)
    .collect();

  lines.sort();
  assert_eq!(lines, [2, 5, 5, 6]);
}

// "every value of a YAML file and every string of a TOML file is read, a TOML comment is not"
#[test]
fn every_value_of_a_yaml_file_and_every_string_of_a_toml_file_is_read_and_a_toml_comment_is_not() {
  assert_eq!(
    hits_in("app/locales/en.yaml", "todo:\n  held: held \u{2014} reason\n  empty: reason\n"),
    [(2, "held \u{2014} reason".to_owned())]
  );
  assert_eq!(
    hits_in(
      "policy/sample.toml",
      "# a note \u{2014} aside\nlabel = \"plain\"\n\n[lines]\nheld = \"held \u{2014} reason\"\n"
    ),
    [(5, "held \u{2014} reason".to_owned())]
  );
}

// "a Rust string, a macro's included, is read, and its comments and test module are not"
#[test]
fn a_rust_string_a_macros_included_is_read_and_its_comments_and_test_module_are_not() {
  let source = [
    "// held \u{2014} a comment",
    "fn say(fault: &str) -> String {",
    "  format!(\"held \u{2014} {fault}\")",
    "}",
    "#[cfg(test)]",
    "mod tests {",
    "  const SAID: &str = \"held? \u{2014} reason\";",
    "}",
  ]
  .join("\n");

  assert_eq!(
    hits_in("crates/sample/src/lib.rs", &source),
    [(3, "\"held \u{2014} {fault}\"".to_owned())]
  );
}

// "people's text is what the globs reach, a reader here reads, and no test file is"
#[test]
fn peoples_text_is_what_the_globs_reach_a_reader_here_reads_and_no_test_file_is() {
  let declared = globs(&["app/**", "policy/*.toml"]);

  assert!(is_people_text("app/sample.ts", &declared));
  assert!(is_people_text("policy/lines.toml", &declared));
  assert!(!is_people_text("app/sample.test.ts", &declared));
  assert!(!is_people_text("app/tests/flow.ts", &declared));
  assert!(!is_people_text("app/notes.md", &declared));
  assert!(!is_people_text("docs/sample.ts", &declared));
}

// "the check over a checkout reads people's text and counts the files it read"
#[test]
fn the_check_over_a_checkout_reads_peoples_text_and_counts_the_files_it_read() {
  let dir = checkout(&[
    ("app/a.ts", "const a = \"held \u{2014} reason\";\n"),
    ("app/b.ts", "const b = \"plain\";\n"),
    ("app/a.test.ts", "const a = \"held \u{2014} reason\";\n"),
    ("app/fixtures/page.ts", "const a = \"held \u{2014} reason\";\n"),
    ("docs/c.ts", "const c = \"held \u{2014} reason\";\n"),
  ]);
  let found = checkout_written_dashes(dir.path(), &globs(&["app/**"]));

  assert_eq!(found.paths, 2);
  assert_eq!(found.findings, ["app/a.ts:1: held \u{2014} reason"]);
}

// "an excerpt is the stretch around the first dash, on one line"
#[test]
fn an_excerpt_is_the_stretch_around_the_first_dash_on_one_line() {
  let long = format!("{}\n{} \u{2014} {}", "a".repeat(50), "b".repeat(10), "c".repeat(60));
  let said = excerpt(&long);

  assert!(said.starts_with('\u{2026}'));
  assert!(said.ends_with('\u{2026}'));
  assert!(!said.contains('\n'));
  assert_eq!(excerpt("held \u{2014} reason"), "held \u{2014} reason");
}
