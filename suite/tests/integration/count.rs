//! The test count (docs/checks.md, 「The test count」), over a range held in memory, and the titles
//! read off a test file's source by oxc.

use std::collections::BTreeMap;

use bmo_testbed_suite::count::{
  GitRange, NamedTest, RangeCount, RangeEnd, RangeReading, count_range, tree_count,
};
use bmo_testbed_suite::titles::test_titles;

fn source(titles: &[&str]) -> String {
  titles
    .iter()
    .map(|title| format!("test({}, () => {{}});", serde_json::to_string(title).unwrap()))
    .collect::<Vec<_>>()
    .join("\n")
}

// A file's titles at the range's start and at its head, none where it stands at neither.
type Ends = (Option<Vec<&'static str>>, Option<Vec<&'static str>>);

// One changed file and its ends, as a test lays it out.
type Row = (&'static str, Option<Vec<&'static str>>, Option<Vec<&'static str>>);

// A range held in memory: each changed file's titles at either end, the commits' words and the Rust sources.
struct Held {
  files: BTreeMap<&'static str, Ends>,
  messages: &'static str,
  rust: Vec<&'static str>,
}

impl RangeReading for Held {
  fn changed_tests(&self) -> Result<Vec<String>, String> {
    Ok(self.files.keys().map(|file| (*file).to_owned()).collect())
  }

  fn file_at(&self, end: RangeEnd, file: &str) -> Option<String> {
    let (start, head) = self.files.get(file)?;

    match end {
      RangeEnd::Start => start.as_deref().map(source),
      RangeEnd::Head => head.as_deref().map(source),
    }
  }

  fn messages(&self) -> Result<String, String> {
    Ok(self.messages.to_owned())
  }

  fn rust_sources(&self) -> Result<Vec<String>, String> {
    Ok(self.rust.iter().map(|one| (*one).to_owned()).collect())
  }
}

fn held(files: &[Row]) -> Held {
  Held {
    files: files.iter().map(|(file, start, head)| (*file, (start.clone(), head.clone()))).collect(),
    messages: "",
    rust: Vec::new(),
  }
}

fn named(file: &str, title: &str) -> NamedTest {
  NamedTest {
    file: file.to_owned(),
    title: title.to_owned(),
  }
}

// "a test's title is read off its call, typed source, skips and conditions and all"
#[test]
fn a_test_s_title_is_read_off_its_call() {
  let typed = r#"const port: number = 1;
describe("GET /health", () => {
  test("answers", async (): Promise<void> => {});
  test.skip(`waits`, () => {});
  test.if(port > 0)("listens", () => {});
  it("is old style", () => {});
  test(title, () => {});
});"#;

  assert_eq!(test_titles(typed).unwrap(), ["answers", "waits", "listens", "is old style"]);
}

// "a test moved to another file, or rewritten in place, is no removal anyone owes"
#[test]
fn a_test_moved_or_rewritten_in_place_is_no_removal_anyone_owes() {
  let range = held(&[
    ("a.test.ts", Some(vec!["kept", "moved", "old words"]), Some(vec!["kept", "new words"])),
    ("b.test.ts", None, Some(vec!["moved"])),
  ]);

  assert_eq!(
    count_range(&range).unwrap(),
    RangeCount {
      before: 3,
      after: 3,
      removed: vec![named("a.test.ts", "old words")],
      unaccounted: Vec::new(),
    }
  );
}

// "a removal stands accounted for by a Rust test naming it, or a retirement with its reason"
#[test]
fn a_removal_stands_accounted_for_by_a_rust_test_or_a_retirement() {
  let range = Held {
    messages: "moved the reading path\n\nRetired-test: retired — the SQLite lock retires with its code\nRetired-test: dropped —\n",
    rust: vec!["/// Replaces scripts/a.test.ts: replaced\n#[test]\nfn one() {}"],
    ..held(&[("a.test.ts", Some(vec!["replaced", "retired", "dropped"]), Some(vec![]))])
  };
  let count = count_range(&range).unwrap();

  assert_eq!(count.removed.len(), 3);
  assert_eq!(count.unaccounted, [named("a.test.ts", "dropped")]);
}

// "a title holding a dash of its own is retired by the line's last dash"
#[test]
fn a_title_holding_a_dash_of_its_own_is_retired_by_the_lines_last_dash() {
  let range = Held {
    messages: "Retired-test: pins a key to it — and never carries a credential — its route retired with its code\n",
    ..held(&[(
      "a.test.ts",
      Some(vec!["pins a key to it — and never carries a credential"]),
      Some(vec![]),
    )])
  };

  assert_eq!(count_range(&range).unwrap().unaccounted, []);
}

fn git(cwd: &std::path::Path, args: &[&str]) -> String {
  let done = std::process::Command::new("git")
    .args([
      "-c",
      "user.name=tester",
      "-c",
      "user.email=tester@example.com",
      "-c",
      "protocol.file.allow=always",
    ])
    .args(args)
    .current_dir(cwd)
    .output()
    .unwrap();

  assert!(done.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&done.stderr));
  String::from_utf8_lossy(&done.stdout).trim().to_owned()
}

fn committed(cwd: &std::path::Path, file: &str, titles: &[&str]) -> String {
  std::fs::create_dir_all(cwd.join(file).parent().unwrap()).unwrap();
  std::fs::write(cwd.join(file), source(titles)).unwrap();
  git(cwd, &["add", "-A"]);
  git(cwd, &["commit", "-qm", file]);
  git(cwd, &["rev-parse", "HEAD"])
}

// "a folder that became a submodule keeps its tests at the pin, and a pin that moved past a test owes it"
#[test]
fn a_folder_become_a_submodule_keeps_its_tests_and_a_moved_pin_owes_what_it_dropped() {
  let dir = tempfile::tempdir().unwrap();
  let (top, sub) = (dir.path().join("top"), dir.path().join("sub"));

  for one in [&top, &sub] {
    std::fs::create_dir_all(one).unwrap();
    git(one, &["init", "-q", "-b", "main"]);
  }

  let folder = committed(&top, "project/a.test.ts", &["one", "two"]);

  committed(&sub, "a.test.ts", &["one", "two"]);
  git(&top, &["rm", "-rq", "project"]);
  git(&top, &["submodule", "add", "-q", &sub.to_string_lossy(), "project"]);
  git(&top, &["commit", "-qm", "the folder is a submodule"]);

  let range = |root: &std::path::Path, since: &str| count_range(&GitRange { root, since }).unwrap();

  assert_eq!(range(&top, &folder).removed, []);

  let pinned = git(&top, &["rev-parse", "HEAD"]);

  committed(&top.join("project"), "a.test.ts", &["one"]);
  git(&top, &["add", "project"]);
  git(&top, &["commit", "-qm", "the pin moves past a test"]);

  assert_eq!(range(&top, &pinned).unaccounted, [named("project/a.test.ts", "two")]);
}

// "the Rust tests are counted wherever the repository tracks a Rust source, a tool's folder included"
#[test]
fn the_rust_tests_are_counted_in_the_role_folders_and_the_tool_folders_alike() {
  let dir = tempfile::tempdir().unwrap();
  let test = "#[test]\nfn one() {}\n";

  git(dir.path(), &["init", "-q", "-b", "main"]);

  for file in ["policy/alpha/src/lib.rs", "tools/graph/src/tests.rs", "song/bench.rs"] {
    std::fs::create_dir_all(dir.path().join(file).parent().unwrap()).unwrap();
    std::fs::write(dir.path().join(file), test).unwrap();
  }

  git(dir.path(), &["add", "-A"]);
  git(dir.path(), &["commit", "-qm", "three crates' sources"]);

  assert_eq!(tree_count(dir.path()).unwrap(), (0, 3));
}

// "a whole file deleted owes every test it held"
#[test]
fn a_whole_file_deleted_owes_every_test_it_held() {
  let gone = [named("gone.test.ts", "one"), named("gone.test.ts", "two")];

  assert_eq!(
    count_range(&held(&[("gone.test.ts", Some(vec!["one", "two"]), None)])).unwrap(),
    RangeCount {
      before: 2,
      after: 0,
      removed: gone.to_vec(),
      unaccounted: gone.to_vec(),
    }
  );
}
