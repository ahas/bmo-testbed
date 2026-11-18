//! The `close-line` verb (docs/boundary.md, 「`close-line`」): what only the running binary shows, the line it
//! prints once every close line is held, the holder the status then names, and its letting go once its input ends.

use std::io::{BufRead, BufReader};
use std::process::Stdio;

use bmo_testbed_core::close_line::close_line_path;
use bmo_testbed_core::policy::close_line::CLOSE_LINE;
use bmo_testbed_core::policy::fill_marks;

use crate::stand::{checkout, finished, ran, testbed};

// The close line of the repository at `repo`, as the status there names it.
fn close_line_at(repo: &std::path::Path, data: &std::path::Path) -> serde_json::Value {
  let status: serde_json::Value =
    serde_json::from_str(&ran(&mut testbed(repo, data, &["status", "--json"])).out)
      .expect("the status reads");

  status["closeLine"].clone()
}

// "holds every named repository's line, in its lock files' order, until its input ends, and then lets them go"
#[test]
fn every_named_line_is_held_in_lock_order_until_the_input_ends() {
  let dir = tempfile::tempdir().unwrap();
  let (first, second) = (dir.path().join("first"), dir.path().join("second"));

  checkout(&first, "testbed = 0\n");
  checkout(&second, "testbed = 0\n");

  let mut named = [&second, &first];
  named.sort_by_key(|repo| close_line_path(dir.path(), repo));
  let in_order: Vec<String> = named.iter().map(|repo| repo.to_string_lossy().into_owned()).collect();

  let mut holder =
    testbed(dir.path(), dir.path(), &["close-line", "--checkout", "second", "--checkout", "first"])
      .stdin(Stdio::piped())
      .spawn()
      .expect("the verb starts");
  let mut said = String::new();

  BufReader::new(holder.stdout.as_mut().expect("its output"))
    .read_line(&mut said)
    .expect("a line");

  assert_eq!(
    said.trim_end(),
    fill_marks(
      &CLOSE_LINE.hold.held,
      &[("count", "2"), ("checkouts", &in_order.join(&CLOSE_LINE.hold.joiner))]
    )
  );
  for repo in [&first, &second] {
    assert_eq!(
      close_line_at(repo, dir.path())["holder"]["pid"],
      i64::from(holder.id()),
      "{}",
      repo.display()
    );
  }

  drop(holder.stdin.take());
  let ended = finished(holder);

  assert_eq!(ended.code, 0, "{}", ended.err);
  for repo in [&first, &second] {
    assert!(close_line_at(repo, dir.path())["holder"].is_null(), "{}", repo.display());
  }
}

// "a path naming no checkout of a git repository is refused with 2, the line naming it"
#[test]
fn a_path_naming_no_checkout_is_refused_naming_it() {
  let dir = tempfile::tempdir().unwrap();

  std::fs::create_dir(dir.path().join("plain")).unwrap();

  let refused = ran(&mut testbed(dir.path(), dir.path(), &["close-line", "--checkout", "plain"]));

  assert_eq!(refused.code, 2);
  assert_eq!(
    refused.err.trim_end(),
    fill_marks(&CLOSE_LINE.hold.not_a_checkout, &[("path", "plain")])
  );
  assert!(refused.out.is_empty());
}
