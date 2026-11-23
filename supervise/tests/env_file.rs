//! An environment file read as systemd reads one.

use bmo_testbed_supervise::env_file::assignments;

fn pairs(text: &str) -> Vec<(String, String)> {
  assignments(text)
}

// "assignments are read whole, a quoted value as it stands and no dollar or hash read inside one"
#[test]
fn assignments_are_read_as_systemd_reads_them() {
  let text = "# a comment\n; another\n\nPLAIN=one two  \nTOKEN=a$b#c\nSINGLE='it is $HOME'\nDOUBLE=\"say \\\"hi\\\" \\\\ there\"\n not a line\nCONTINUED=one\\\ntwo\n";

  assert_eq!(
    pairs(text),
    vec![
      ("PLAIN".to_owned(), "one two".to_owned()),
      ("TOKEN".to_owned(), "a$b#c".to_owned()),
      ("SINGLE".to_owned(), "it is $HOME".to_owned()),
      ("DOUBLE".to_owned(), "say \"hi\" \\ there".to_owned()),
      ("CONTINUED".to_owned(), "onetwo".to_owned()),
    ]
  );
}
