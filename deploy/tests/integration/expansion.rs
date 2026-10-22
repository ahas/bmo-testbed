//! The build tree (docs/deploy.md, 「The releases」): the trees a workspace names by a path outside the
//! repository, laid beside a release and the build tree from the checkout that carries them.

use std::path::Path;

use bmo_testbed_deploy::expansion::lay_outside_trees;

// A package standing in a folder, its library empty.
fn package(dir: &Path, name: &str) {
  std::fs::create_dir_all(dir.join("src")).unwrap();
  std::fs::write(
    dir.join("Cargo.toml"),
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
  )
  .unwrap();
  std::fs::write(dir.join("src/lib.rs"), "").unwrap();
}

// A package whose one dependency is a tree at the path the manifest names.
fn app(dir: &Path, dependency: &str) {
  std::fs::create_dir_all(dir.join("src")).unwrap();
  std::fs::write(
    dir.join("Cargo.toml"),
    format!(
      "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nbmo-text = {{ path = \"{dependency}\" }}\n"
    ),
  )
  .unwrap();
  std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
}

// A workspace root whose one member depends on a tree the workspace names by a path.
fn workspace_with(dir: &Path, dependency: &str) {
  std::fs::create_dir_all(dir.join("app/src")).unwrap();
  std::fs::write(
    dir.join("Cargo.toml"),
    format!(
      "[workspace]\nresolver = \"2\"\nmembers = [\"app\"]\n\n[workspace.dependencies]\nbmo-text = {{ path = \"{dependency}\" }}\n"
    ),
  )
  .unwrap();
  std::fs::write(
    dir.join("app/Cargo.toml"),
    "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nbmo-text = { workspace = true }\n",
  )
  .unwrap();
  std::fs::write(dir.join("app/src/main.rs"), "fn main() {}\n").unwrap();
}

fn symlink(at: &Path) -> Option<std::path::PathBuf> {
  std::fs::symlink_metadata(at)
    .ok()
    .filter(|meta| meta.file_type().is_symlink())
    .and_then(|_| std::fs::read_link(at).ok())
}

// "the trees a workspace names outside it are laid beside the tree from the checkout"
#[test]
fn a_tree_named_outside_the_workspace_is_laid_where_its_path_points() {
  let root = tempfile::tempdir().unwrap();
  let base = root.path().join("workspace/base/text");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");

  package(&base, "bmo-text");
  app(&checkout, "../../base/text");
  app(&tree, "../../base/text");

  lay_outside_trees(&tree, &checkout).unwrap();

  let link = root.path().join("base/text");

  assert_eq!(
    symlink(&link).as_deref(),
    Some(std::fs::canonicalize(&base).unwrap().as_path()),
    "the tree named beside the build tree stands as a link from the checkout"
  );
  assert_eq!(std::fs::canonicalize(&link).unwrap(), std::fs::canonicalize(&base).unwrap());
}

// "the trees outside a workspace are read from the checkout, since the tree reads none until they stand"
#[test]
fn a_workspace_reads_its_outside_trees_from_the_checkout_before_they_stand() {
  let root = tempfile::tempdir().unwrap();
  let base = root.path().join("workspace/base/text");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");

  package(&base, "bmo-text");
  workspace_with(&checkout, "../../base/text");
  workspace_with(&tree, "../../base/text");

  // The build tree resolves nothing until the tree it names stands where its path points.
  let read = std::process::Command::new("cargo")
    .args(["metadata", "--no-deps", "--offline", "--format-version", "1"])
    .current_dir(&tree)
    .output()
    .unwrap();

  assert!(!read.status.success(), "no manifest of the build tree reads before its tree stands");

  lay_outside_trees(&tree, &checkout).unwrap();

  let link = root.path().join("base/text");

  assert_eq!(
    symlink(&link).as_deref(),
    Some(std::fs::canonicalize(&base).unwrap().as_path()),
    "the tree the checkout carries stands beside the build tree, though it stood nowhere there before"
  );
}

// "a tree the checkout does not carry refuses the laying, naming it"
#[test]
fn a_tree_the_checkout_does_not_carry_refuses_the_laying() {
  let root = tempfile::tempdir().unwrap();
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");

  app(&checkout, "../../base/text");
  app(&tree, "../../base/text");

  let why = lay_outside_trees(&tree, &checkout).unwrap_err();

  assert!(why.contains("base/text"), "{why}");
  assert!(!root.path().join("base/text").exists());
}

// "a dependency inside the tree is no tree outside it, and no workspace at all is left alone"
#[test]
fn a_dependency_inside_the_tree_and_a_tree_with_no_workspace_are_left_alone() {
  let root = tempfile::tempdir().unwrap();
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");

  package(&checkout.join("support"), "support");
  package(&tree.join("support"), "support");
  app(&checkout, "support");
  app(&tree, "support");

  lay_outside_trees(&tree, &checkout).unwrap();

  assert!(symlink(&tree.join("support")).is_none());

  let plain = root.path().join("plain");

  std::fs::create_dir_all(&plain).unwrap();
  lay_outside_trees(&plain, &checkout).unwrap();
}

// A command run in a folder, its output both streams, as the deployment runs one.
fn runner() -> impl Fn(&[String], &Path) -> bmo_testbed_deploy::run::Ran {
  move |line: &[String], cwd: &Path| {
    let out = std::process::Command::new(&line[0])
      .args(&line[1..])
      .current_dir(cwd)
      .env("GIT_AUTHOR_NAME", "test")
      .env("GIT_AUTHOR_EMAIL", "test@example.com")
      .env("GIT_COMMITTER_NAME", "test")
      .env("GIT_COMMITTER_EMAIL", "test@example.com")
      .output();

    match out {
      Ok(ran) => bmo_testbed_deploy::run::Ran {
        ok: ran.status.success(),
        out: String::from_utf8_lossy(&ran.stdout).into_owned() + &String::from_utf8_lossy(&ran.stderr),
      },
      Err(why) => bmo_testbed_deploy::run::Ran {
        ok: false,
        out: why.to_string(),
      },
    }
  }
}

fn commit(dir: &Path, text: &str) -> String {
  std::fs::write(dir.join("src/lib.rs"), text).unwrap();
  let run = runner();

  assert!(run(&["git".to_owned(), "add".to_owned(), "-A".to_owned()], dir).ok);
  assert!(run(&["git".to_owned(), "commit".to_owned(), "-m".to_owned(), text.to_owned()], dir).ok);

  run(&["git".to_owned(), "rev-parse".to_owned(), "HEAD".to_owned()], dir)
    .out
    .trim()
    .to_owned()
}

fn tree_at<'a>(
  source: &'a Path,
  tree: &Path,
  data_root: &'a Path,
  head: &'a str,
) -> bmo_testbed_deploy::expansion::BuildTree<'a> {
  bmo_testbed_deploy::expansion::BuildTree {
    source,
    tree: tree.to_path_buf(),
    head,
    data_root,
  }
}

// "the build tree is stamped where its commit moved"
#[test]
fn a_commit_moved_in_the_build_tree_stamps_the_paths_it_changed() {
  use bmo_testbed_deploy::expansion::put_build_tree;
  use std::time::{Duration, SystemTime};

  let root = tempfile::tempdir().unwrap();
  let source = root.path().join("source");
  let tree = root.path().join("deploy-expand");
  let run = runner();

  std::fs::create_dir_all(source.join("src")).unwrap();
  std::fs::write(
    source.join("Cargo.toml"),
    "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
  )
  .unwrap();
  assert!(run(&["git".to_owned(), "init".to_owned()], &source).ok);

  let one = commit(&source, "one\n");
  let two = commit(&source, "two\n");
  put_build_tree(&tree_at(&source, &tree, root.path(), &one), &run).unwrap();

  // A build from the commit before is what a warm tree holds: the file the next commit moves stands older
  // than it, and the move leaves it newer than that build.
  let file = tree.join("src/lib.rs");
  let old = SystemTime::now() - Duration::from_secs(3600);

  std::fs::File::open(&file).unwrap().set_modified(old).unwrap();
  put_build_tree(&tree_at(&source, &tree, root.path(), &two), &run).unwrap();

  let stamped = std::fs::metadata(&file).unwrap().modified().unwrap();

  assert!(stamped > old, "the file the commit moved stands newer than the build before it");
}
