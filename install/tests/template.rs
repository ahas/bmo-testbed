//! A template's marks (docs/install.md, 「Values」).

use std::path::PathBuf;

use bmo_testbed_install::template::{Absent, Source, Unfilled, Values};

fn values(root: &std::path::Path) -> Values {
  let mut values = Values {
    data_root: root.to_path_buf(),
    home: root.join("home"),
    ..Values::default()
  };

  values.named.insert("port.a".to_owned(), Source::Text("4100".to_owned()));
  values
    .named
    .insert("value.a.address".to_owned(), Source::Template("http://127.0.0.1:{{port.a}}".to_owned()));
  values.named.insert(
    "token.a".to_owned(),
    Source::File {
      path: root.join("env/a.token"),
      absent: Absent::Missing,
    },
  );
  values.named.insert(
    "token.planned".to_owned(),
    Source::File {
      path: root.join("env/none"),
      absent: Absent::Planned("planned".to_owned()),
    },
  );
  values.named.insert(
    "input.default".to_owned(),
    Source::File {
      path: root.join("env/none"),
      absent: Absent::Text("en".to_owned()),
    },
  );
  values
}

#[test]
fn every_mark_is_filled_and_a_files_last_line_ending_is_taken_off() {
  let dir = tempfile::tempdir().unwrap();

  std::fs::create_dir_all(dir.path().join("env")).unwrap();
  std::fs::write(dir.path().join("env/a.token"), "abc\n").unwrap();

  let filled = values(dir.path()).fill("[s]\naddress = \"{{ value.a.address }}\"\ntoken = \"{{token.a}}\"\nlanguage = \"{{input.default}}\"\n").unwrap();

  assert_eq!(filled, "[s]\naddress = \"http://127.0.0.1:4100\"\ntoken = \"abc\"\nlanguage = \"en\"\n");
}

#[test]
fn a_toml_inline_table_is_no_mark() {
  let dir = tempfile::tempdir().unwrap();

  assert_eq!(values(dir.path()).fill("a = { b = 1 }").unwrap(), "a = { b = 1 }");
}

#[test]
fn a_name_nothing_declares_is_unknown() {
  let dir = tempfile::tempdir().unwrap();

  assert_eq!(
    values(dir.path()).fill("{{token.ghost}}"),
    Err(Unfilled::Unknown("token.ghost".to_owned()))
  );
  assert_eq!(values(dir.path()).fill("{{token.a}}"), Err(Unfilled::Unknown("token.a".to_owned())));
}

#[test]
fn a_dry_runs_token_stands_as_its_name() {
  let dir = tempfile::tempdir().unwrap();

  assert_eq!(values(dir.path()).fill("{{token.planned}}").unwrap(), "<planned>");
}

#[test]
fn a_file_that_does_not_stand_waits() {
  let dir = tempfile::tempdir().unwrap();
  let waiting = values(dir.path()).fill("{{file.tool-host/console-token}}");

  assert_eq!(
    waiting,
    Err(Unfilled::Waiting(dir.path().join("tool-host/console-token").display().to_string()))
  );
}

#[test]
fn a_path_under_home_is_read_there() {
  let dir = tempfile::tempdir().unwrap();

  assert_eq!(values(dir.path()).path("~/.config/x"), dir.path().join("home/.config/x"));
  assert_eq!(values(dir.path()).path("env/x"), dir.path().join("env/x"));
  assert_eq!(values(dir.path()).path("/etc/x"), PathBuf::from("/etc/x"));
}

#[test]
fn a_mark_left_open_is_refused() {
  let dir = tempfile::tempdir().unwrap();

  assert_eq!(values(dir.path()).fill("x {{port.a"), Err(Unfilled::Open));
}
