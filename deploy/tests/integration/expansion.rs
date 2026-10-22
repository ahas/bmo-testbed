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
