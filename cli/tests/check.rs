//! The `check` verb (docs/boundary.md, 「The verbs」; docs/checks.md): its words, a quick check run alone only
//! where the declaration enables it, and the dashes of a markdown file shown and written only on asking.

use bmo_testbed_cli::check::{checked_part, dashes};
use bmo_testbed_cli::words::CheckPart;
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::turn::Env;

use crate::stand::{checkout, git, ran, testbed};

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
  git(&base, &["add", "."]);
  git(&base, &["commit", "-q", "-m", "shared versions"]);
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

// A checkout of its own declaration, committed on main, then a branch whose commit changes `src/change.txt`;
// the commit main stood on is answered.
fn set_checkout(dir: &std::path::Path, declaration: &str) -> String {
  checkout(dir, declaration);
  std::fs::create_dir_all(dir.join("src")).unwrap();
  std::fs::write(dir.join("src/change.txt"), "before\n").unwrap();
  git(dir, &["add", "."]);
  git(dir, &["commit", "-q", "-m", "base"]);

  let base = git(dir, &["rev-parse", "HEAD"]);

  git(dir, &["switch", "-q", "-c", "work/1"]);
  std::fs::write(dir.join("src/change.txt"), "after\n").unwrap();
  std::fs::write(dir.join("src/more.txt"), "more\n").unwrap();
  git(dir, &["add", "."]);
  git(dir, &["commit", "-q", "-m", "the branch's own"]);
  base
}

// "each checkout is planned by its own declaration and its own since, every line names its repository, the last
// lines name each failed repository and its failed steps, and each repository is one test run of kind check"
#[test]
fn a_set_plans_each_checkout_by_its_own_declaration_and_since_and_names_what_failed() {
  use bmo_testbed_client::{TestRunKind, TestRunState, TestRunsQuery};

  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let alpha = dir.join("work/alpha");
  let beta = dir.join("base/beta");
  let data = dir.join("data");
  let alpha_base = set_checkout(
    &alpha,
    "testbed = 0\nname = \"alpha\"\n\n[[check.step]]\nid = \"alpha-step\"\ncommand = \"sh -c 'echo alpha ran'\"\n",
  );
  let beta_base = set_checkout(
    &beta,
    "testbed = 0\nname = \"beta\"\n\n[[check.step]]\nid = \"beta-step\"\ncommand = \"sh -c 'exit 4'\"\n",
  );
  // Beta's change is read since its branch's first commit, so only the file a later commit changed is changed,
  // where its merge base with main would read two.
  let beta_since = git(&beta, &["rev-parse", "HEAD"]);

  std::fs::write(beta.join("src/change.txt"), "later\n").unwrap();
  git(&beta, &["commit", "-q", "-am", "a later one"]);

  let since_word = format!("../../base/beta@{beta_since}");
  let done = ran(
    testbed(&alpha, &data, &["check", "--checkout", ".", "--checkout", &since_word])
      .env("BMO_TESTBED_PLACED", "1"),
  );
  let lines: Vec<&str> = done.out.lines().collect();

  assert_eq!(done.code, 1, "{}\n{}", done.out, done.err);
  assert!(lines.contains(&&*format!("alpha: check: 2 path(s) changed (since {alpha_base}); 0 test file(s), 0 crate(s) tested, 0 checked, 0 project check(s), 0 tool check(s)")), "{}", done.out);
  assert!(lines.contains(&&*format!("beta: check: 1 path(s) changed (since {beta_since}); 0 test file(s), 0 crate(s) tested, 0 checked, 0 project check(s), 0 tool check(s)")), "{}", done.out);
  assert_ne!(beta_since, beta_base);
  assert!(lines.iter().any(|line| line.starts_with("alpha: ok alpha-step")), "{}", done.out);
  assert!(
    lines.iter().any(|line| line.starts_with("beta: FAILED beta-step · exit 4")),
    "{}",
    done.out
  );
  assert!(lines.contains(&"check FAILED: beta: beta-step"), "{}", done.out);
  assert!(!lines.iter().any(|line| line.starts_with("check FAILED: alpha")), "{}", done.out);
  assert!(lines.last().unwrap().starts_with("log: "), "{}", done.out);

  let runs = bmo_testbed_record::Record::open(&data)
    .unwrap()
    .test_runs(&TestRunsQuery::default())
    .unwrap()
    .items;
  let of = |name: &str| runs.iter().find(|run| run.repository_name.as_deref() == Some(name)).unwrap();

  assert_eq!(runs.len(), 2, "{runs:?}");
  assert!(
    runs.iter().all(|run| run.kind == TestRunKind::Check && run.whole == Some(false)),
    "{runs:?}"
  );
  assert_eq!(of("alpha").checkout, alpha.to_string_lossy());
  assert_eq!(of("alpha").commit.as_deref(), Some(&*git(&alpha, &["rev-parse", "HEAD"])));
  assert_eq!(of("alpha").state, TestRunState::Passed);
  assert_eq!(of("beta").checkout, beta.to_string_lossy());
  assert_eq!(of("beta").commit.as_deref(), Some(&*git(&beta, &["rev-parse", "HEAD"])));
  assert_eq!(of("beta").state, TestRunState::Failed);
}

// "a path naming no checkout, two words naming one checkout and a since naming no commit end the call 2 before
// anything runs, and --checkout is never given with --since or files"
#[test]
fn a_checkout_that_does_not_read_refuses_the_set_before_anything_runs() {
  let dir = tempfile::tempdir().unwrap();
  let dir = std::fs::canonicalize(dir.path()).unwrap();
  let repo = dir.join("repo");

  checkout(&repo, "testbed = 0\n");
  std::fs::create_dir_all(dir.join("plain")).unwrap();

  for (given, said) in [
    (
      &["check", "--checkout", ".", "--checkout", "../plain"][..],
      "../plain names no checkout",
    ),
    (
      &["check", "--checkout", ".", "--checkout", "../repo"],
      "../repo names a checkout another --checkout already names",
    ),
    (
      &["check", "--checkout", ".@no-such-commit"],
      ".@no-such-commit: no-such-commit names no commit",
    ),
    (&["check", "--checkout", ".", "--since", "HEAD"], ""),
    (&["check", "--checkout", ".", "a.ts"], ""),
  ] {
    let refused = ran(testbed(&repo, &dir.join("data"), given).env("BMO_TESTBED_PLACED", "1"));

    assert_eq!(refused.code, 2, "{given:?}: {}", refused.err);
    assert!(refused.err.contains(said), "{given:?}: {}", refused.err);
    assert!(!refused.out.contains("check:"), "{given:?}: {}", refused.out);
  }
}
