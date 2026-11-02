//! The `check` verb (docs/boundary.md, 「The verbs」; docs/checks.md): its words, a quick check run alone only
//! where the declaration enables it, and the dashes of a markdown file shown and written only on asking.

use bmo_testbed_cli::check::{checked_part, dashes};
use bmo_testbed_cli::words::CheckPart;
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::turn::Env;

use crate::stand::{checkout, ran, testbed};

fn words(given: &[&str]) -> Vec<String> {
  given.iter().map(|one| (*one).to_owned()).collect()
}

// "reads files or a commit and a log, and refuses both together or a flag it does not read"
#[test]
fn files_and_a_commit_together_and_a_flag_it_does_not_read_are_refused() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");

  checkout(&repo, "testbed = 0\n");

  for given in [
    &["check", "--since", "abc", "a.ts"][..],
    &["check", "--fast"],
    &["check", "quick", "--all"],
    &["check", "korean", "--settle", "--kinds"],
    &["check", "dashes"],
  ] {
    let refused = ran(&mut testbed(&repo, dir.path(), given));

    assert_eq!(refused.code, 2, "{given:?}: {}", refused.err);
  }
}

// "a part the declaration does not enable is refused, and quick runs only what it enables"
#[test]
fn a_part_the_declaration_does_not_enable_is_refused() {
  let dir = tempfile::tempdir().unwrap();

  checkout(dir.path(), "testbed = 0\n\n[check]\nquick = [\"layout\"]\n");

  let repo = Repository::read(dir.path()).unwrap();
  let refused = checked_part(CheckPart::Dependencies, &repo, &Env::new(), dir.path());
  let quick = checked_part(CheckPart::Quick, &repo, &Env::new(), dir.path());

  assert_eq!(refused.code, 2);
  assert!(refused.err[0].contains("dependencies"), "{:?}", refused.err);
  assert!(quick.out[0].contains("layout"), "{:?}", quick.out);
  assert!(quick.out.iter().all(|line| !line.contains("dependencies")));
}

// "dashes shows the lines it would change and writes them only with --write"
#[test]
fn dashes_shows_the_lines_it_would_change_and_writes_them_only_with_write() {
  let dir = tempfile::tempdir().unwrap();
  let before = "- A rule \u{2014} its reason\n  wrapped \u{2014} over two lines.\n";

  std::fs::write(dir.path().join("doc.md"), before).unwrap();

  let shown = dashes(&words(&["doc.md"]), dir.path(), false);

  assert_eq!(shown.code, 1);
  assert!(shown.out.iter().any(|line| line.contains("1+ - A rule, its reason")), "{:?}", shown.out);
  assert_eq!(std::fs::read_to_string(dir.path().join("doc.md")).unwrap(), before);

  let written = dashes(&words(&["doc.md"]), dir.path(), true);

  assert_eq!(written.code, 0);
  assert_eq!(
    std::fs::read_to_string(dir.path().join("doc.md")).unwrap(),
    "- A rule, its reason\n  wrapped, over two lines.\n"
  );
  assert_eq!(dashes(&words(&["doc.md"]), dir.path(), false).out, ["no dash to change"]);
  assert_eq!(dashes(&words(&["gone.md"]), dir.path(), false).err, ["no such file: gone.md"]);
}

// "shared-versions --report says a line for each checkout of the registry and how many differ, and ends 1
// where any does"
#[test]
fn shared_versions_report_says_a_line_for_each_checkout_and_how_many_differ() {
  let dir = tempfile::tempdir().unwrap();
  let base = dir.path().join("base");
  let unit = dir.path().join("unit");

  checkout(&base, "testbed = 0\nname = \"shared-base\"\n");
  std::fs::write(base.join("shared-versions.toml"), "[dependencies]\nrt = \"1.4.0\"\n").unwrap();
  checkout(
    &unit,
    "testbed = 0\nname = \"unit\"\n\n[check]\nquick = [\"shared-versions\"]\n\n[check.shared_versions]\nrepository = \"shared-base\"\n",
  );
  std::fs::write(
    unit.join("Cargo.toml"),
    "[package]\nname = \"unit\"\n\n[dependencies]\nrt = \"1.3.0\"\n",
  )
  .unwrap();
  std::fs::create_dir_all(dir.path().join("data/testbed")).unwrap();
  std::fs::write(
    dir.path().join("data/testbed/checkouts.toml"),
    "checkouts = [\"../../base\", \"../../unit\"]\n",
  )
  .unwrap();

  let repo = Repository::read(&unit).unwrap();
  let env =
    Env::from([("BMO_DATA_DIR".to_owned(), dir.path().join("data").to_string_lossy().into_owned())]);
  let said = checked_part(CheckPart::SharedVersions { report: true }, &repo, &env, &unit);

  assert_eq!(said.code, 1, "{said:?}");
  assert_eq!(
    said.out,
    [
      "shared-base: holds to the shared versions (0 entries)",
      "unit: 1 to fix: lines rt",
      "1 of 2 checkouts differ from shared-versions.toml",
    ]
  );
}
