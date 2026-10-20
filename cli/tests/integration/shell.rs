//! A shell line read the way the shell splits it (docs/testbed.md, 「A shell hook sends a harness's heavy
//! command to it」): its simple commands, their words apart from what their redirects name, the text its
//! substitutions run, and nothing of a heredoc's body.

use std::collections::HashMap;

use bmo_testbed_cli::shell::{Split, expanded, program_of, split, value};

fn words(text: &str) -> Vec<Vec<String>> {
  split(text).commands.into_iter().map(|command| command.words).collect()
}

#[test]
fn words_keep_their_quoting_and_operators_end_a_command() {
  assert_eq!(
    words(r#"git commit -m "x" && rm -rf "${S:?}"/*.md; A=1 cmd"#),
    [
      vec!["git", "commit", "-m", "\"x\""],
      vec!["rm", "-rf", "\"${S:?}\"/*.md"],
      vec!["A=1", "cmd"]
    ]
  );
}

#[test]
fn a_descriptor_belongs_to_its_redirect_and_a_duplicate_names_no_file() {
  let read = split("echo hi > out.txt 2>&1 </dev/null >> log");
  let command = &read.commands[0];

  assert_eq!(command.words, ["echo", "hi"]);
  assert_eq!(command.redirects, ["out.txt", "/dev/null", "log"]);
}

#[test]
fn a_heredoc_body_is_text_and_not_commands() {
  assert_eq!(
    words("cat > x <<'EOF' | wc -l\nls\npkill -f x\nEOF\necho after"),
    [vec!["cat"], vec!["wc", "-l"], vec!["echo", "after"]]
  );
}

#[test]
fn substitutions_come_out_beside_the_words_they_stand_in() {
  let read = split("echo $(echo one) \"a $(two) `three`\" <(ls four)");

  assert_eq!(
    read.inner.iter().map(|inner| inner.text.as_str()).collect::<Vec<_>>(),
    ["echo one", "two", "three", "ls four"]
  );
  assert_eq!(read.commands[0].words.last().unwrap(), "<(ls four)");
}

#[test]
fn a_line_the_tokenizer_cannot_read_reads_as_nothing() {
  assert_eq!(split("echo 'open"), Split::default());
}

#[test]
fn a_value_takes_the_quoting_off_and_leaves_expansions_as_written() {
  assert_eq!(value(r#""$HOME/.config"/x"#), "$HOME/.config/x");
  assert_eq!(value("'a b'c\\ d"), "a bc d");
  assert_eq!(value("$(echo tool)"), "$(echo tool)");
}

#[test]
fn a_variable_the_line_assigned_is_read_as_its_value() {
  let assigned = HashMap::from([("TOOL".to_owned(), "cargo test".to_owned())]);

  assert_eq!(expanded("$TOOL", &assigned), ["cargo", "test"]);
  assert_eq!(expanded("${TOOL}", &assigned), ["cargo", "test"]);
  assert_eq!(expanded("\"$TOOL\"", &assigned), ["cargo test"]);
  assert_eq!(expanded("$OTHER", &assigned), ["$OTHER"]);
  assert_eq!(expanded("x$TOOL", &assigned), ["x$TOOL"]);
}

#[test]
fn the_program_is_found_past_assignments_runners_and_grammar() {
  let run =
    program_of(&["FOO=1".into(), "timeout".into(), "30".into(), "tool".into(), "-p".into()]).unwrap();

  assert_eq!(
    (run.program.as_str(), run.args.as_slice(), run.runners.as_slice()),
    ("tool", &["-p".to_owned()][..], &["timeout".to_owned()][..])
  );
  assert_eq!(program_of(&["if".into(), "!".into()]), None);
}
