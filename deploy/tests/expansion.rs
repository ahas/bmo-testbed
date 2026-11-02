//! The build tree (docs/deploy.md, 「The releases」): the trees a workspace names by a path outside the
//! repository, laid beside a release and the build tree from the checkout that carries them; and its target
//! directory, which outlives the last deployment by its span unless a deployment is running.

use std::path::{Path, PathBuf};

use bmo_testbed_deploy::deployment::DeployPlaces;
use bmo_testbed_deploy::expansion::{
  build_target, idle_build_target, last_deployed_ms, lay_outside_trees, remove_build_target,
};

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

// A package whose dependencies are the trees at the paths the manifest names, one name to a path.
fn app_on(dir: &Path, dependencies: &[(&str, &str)]) {
  let named: String = dependencies
    .iter()
    .map(|(name, path)| format!("{name} = {{ path = \"{path}\" }}\n"))
    .collect();

  std::fs::create_dir_all(dir.join("src")).unwrap();
  std::fs::write(
    dir.join("Cargo.toml"),
    format!(
      "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{named}"
    ),
  )
  .unwrap();
  std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
}

// A sibling repository's workspace whose member crates inherit their version and edition from its root.
fn sibling_workspace(dir: &Path, members: &[&str]) {
  let listed: Vec<String> = members.iter().map(|member| format!("\"crates/{member}\"")).collect();

  std::fs::create_dir_all(dir).unwrap();
  std::fs::write(
    dir.join("Cargo.toml"),
    format!(
      "[workspace]\nresolver = \"2\"\nmembers = [{}]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2021\"\n",
      listed.join(", ")
    ),
  )
  .unwrap();

  for member in members {
    let crate_dir = dir.join("crates").join(member);

    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::write(
      crate_dir.join("Cargo.toml"),
      format!("[package]\nname = \"{member}\"\nversion.workspace = true\nedition.workspace = true\n"),
    )
    .unwrap();
    std::fs::write(crate_dir.join("src/lib.rs"), "").unwrap();
  }
}

// Whether a tree's manifests all read, the trees they name included.
fn reads(tree: &Path) -> bool {
  std::process::Command::new("cargo")
    .args(["metadata", "--no-deps", "--offline", "--format-version", "1"])
    .current_dir(tree)
    .output()
    .unwrap()
    .status
    .success()
}

// The names standing in a folder, in order.
fn names(folder: &Path) -> Vec<String> {
  let mut names: Vec<String> = std::fs::read_dir(folder)
    .unwrap()
    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
    .collect();

  names.sort();
  names
}

// "a tree that is a member of another Cargo workspace is laid at that workspace's root"
#[test]
fn a_member_crate_of_a_sibling_workspace_lays_the_siblings_root() {
  let root = tempfile::tempdir().unwrap();
  let sibling = root.path().join("workspace/services/sibling");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");

  sibling_workspace(&sibling, &["client"]);
  app_on(&checkout, &[("client", "../sibling/crates/client")]);
  app_on(&tree, &[("client", "../sibling/crates/client")]);

  lay_outside_trees(&tree, &checkout).unwrap();

  assert_eq!(
    symlink(&root.path().join("run/sibling")).as_deref(),
    Some(std::fs::canonicalize(&sibling).unwrap().as_path()),
    "the sibling's workspace root stands as one link"
  );
  assert!(reads(&tree), "the member finds the root it inherits from through the link");
}

// "every tree named inside one workspace shares that one link"
#[test]
fn two_members_of_one_sibling_workspace_lay_one_link() {
  let root = tempfile::tempdir().unwrap();
  let sibling = root.path().join("workspace/services/sibling");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let named = [("client", "../sibling/crates/client"), ("plugin", "../sibling/crates/plugin")];

  sibling_workspace(&sibling, &["client", "plugin"]);
  app_on(&checkout, &named);
  app_on(&tree, &named);

  lay_outside_trees(&tree, &checkout).unwrap();

  assert_eq!(names(&root.path().join("run")), ["deploy-expand", "sibling"]);
  assert!(symlink(&root.path().join("run/sibling")).is_some());
  assert!(reads(&tree));
}

// "a standalone package is linked by itself"
#[test]
fn a_standalone_package_beside_a_sibling_member_is_laid_by_itself() {
  let root = tempfile::tempdir().unwrap();
  let base = root.path().join("workspace/base/text");
  let sibling = root.path().join("workspace/services/sibling");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let named = [("bmo-text", "../../base/text"), ("client", "../sibling/crates/client")];

  package(&base, "bmo-text");
  sibling_workspace(&sibling, &["client"]);
  app_on(&checkout, &named);
  app_on(&tree, &named);

  lay_outside_trees(&tree, &checkout).unwrap();

  assert_eq!(
    symlink(&root.path().join("base/text")).as_deref(),
    Some(std::fs::canonicalize(&base).unwrap().as_path()),
    "the standalone package stands as a link of its own, its folder not linked"
  );
  assert!(symlink(&root.path().join("base")).is_none());
  assert!(symlink(&root.path().join("run/sibling")).is_some());
  assert!(reads(&tree));
}

// "a link already on the checkout's tree is left as it is"
#[test]
fn a_link_already_on_the_checkouts_tree_is_left_alone() {
  use std::os::unix::fs::MetadataExt;

  let root = tempfile::tempdir().unwrap();
  let sibling = root.path().join("workspace/services/sibling");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let link = root.path().join("run/sibling");

  sibling_workspace(&sibling, &["client"]);
  app_on(&checkout, &[("client", "../sibling/crates/client")]);
  app_on(&tree, &[("client", "../sibling/crates/client")]);

  lay_outside_trees(&tree, &checkout).unwrap();

  let before = std::fs::symlink_metadata(&link).unwrap().ino();

  lay_outside_trees(&tree, &checkout).unwrap();

  assert_eq!(std::fs::symlink_metadata(&link).unwrap().ino(), before, "the same link, not a new one");
  assert_eq!(symlink(&link).as_deref(), Some(std::fs::canonicalize(&sibling).unwrap().as_path()));
}

// "a folder holding nothing but folders and links, as an earlier laying leaves, is replaced; any other
// tree standing there is left as it is"
#[test]
fn an_earlier_layings_folder_of_links_is_replaced_and_any_other_tree_is_kept() {
  let root = tempfile::tempdir().unwrap();
  let sibling = root.path().join("workspace/services/sibling");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let link = root.path().join("run/sibling");

  sibling_workspace(&sibling, &["client"]);
  app_on(&checkout, &[("client", "../sibling/crates/client")]);
  app_on(&tree, &[("client", "../sibling/crates/client")]);
  std::fs::create_dir_all(link.join("crates")).unwrap();
  std::os::unix::fs::symlink(sibling.join("crates/client"), link.join("crates/client")).unwrap();

  lay_outside_trees(&tree, &checkout).unwrap();

  assert_eq!(symlink(&link).as_deref(), Some(std::fs::canonicalize(&sibling).unwrap().as_path()));

  std::fs::remove_file(&link).unwrap();
  std::fs::create_dir_all(link.join("crates")).unwrap();
  std::fs::write(link.join("Cargo.toml"), "").unwrap();

  lay_outside_trees(&tree, &checkout).unwrap();

  assert!(symlink(&link).is_none(), "a tree holding a file of its own is nobody's to replace");
  assert!(link.join("Cargo.toml").exists());
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

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

// The places of a deploy root under `root`, as a declaration naming it would lay them.
fn places(root: &Path) -> DeployPlaces {
  let deploy_root = root.join("deploy-root");

  DeployPlaces {
    source: root.join("checkout"),
    deploy_dir: deploy_root.join("deploy"),
    data_root: root.join("data"),
    backup_dir: None,
    unit_dir: root.join("units"),
    record: deploy_root.join("deployed.json"),
    log: deploy_root.join("deployed.log"),
    progress: deploy_root.join("deploy-progress.json"),
    bot_unit: None,
    deploy_root,
  }
}

fn deployed_at(places: &DeployPlaces, at: &str) {
  std::fs::write(
    &places.record,
    format!(r#"{{ "commit": "c0ffee", "subject": "a change", "at": "{at}", "backup": "" }}"#),
  )
  .unwrap();
}

// A progress record of a run by `pid`, ended or not.
fn progress(places: &DeployPlaces, pid: u32, ended: bool) {
  let ending = if ended { r#", "ending": { "outcome": "live", "at": 2, "why": null }"# } else { "" };

  std::fs::write(&places.progress, format!(r#"{{ "unit": null, "pid": {pid}, "startedAt": 1{ending} }}"#))
    .unwrap();
}

fn built(places: &DeployPlaces) -> PathBuf {
  let target = build_target(&places.deploy_dir);

  std::fs::create_dir_all(target.join("release")).unwrap();
  std::fs::write(target.join("release/app"), "built").unwrap();
  target
}

// "The prune ... removes it once the deployment record was written more than three days ago and no deployment of
// the deploy root is running ... A deploy root holding no record that reads keeps its build tree's target
// directory, and the build tree itself always stays."
#[test]
fn a_build_tree_s_target_goes_once_the_last_deployment_outlived_its_span_and_none_runs() {
  let dir = tempfile::tempdir().unwrap();
  let places = places(dir.path());
  let at = last_deployed_ms(&places.record);

  assert_eq!(at, None);

  let target = built(&places);
  let now = chrono::DateTime::parse_from_rfc3339("2026-01-10T00:00:00Z").unwrap().timestamp_millis();
  let span = 3 * DAY_MS;

  // No record: it stays.
  assert_eq!(idle_build_target(&places, now, span), None);

  // Laid down two days ago: it stays; four days ago: it goes.
  deployed_at(&places, "2026-01-08T00:00:00.000Z");
  assert_eq!(idle_build_target(&places, now, span), None);
  deployed_at(&places, "2026-01-06T00:00:00.000Z");
  assert_eq!(last_deployed_ms(&places.record), Some(now - 4 * DAY_MS));
  assert_eq!(idle_build_target(&places, now, span), Some(target.clone()));

  // A run without an ending whose process lives holds it; one ended, or whose process is gone, does not.
  progress(&places, std::process::id(), false);
  assert_eq!(idle_build_target(&places, now, span), None);
  assert!(remove_build_target(&places, &target).is_err());
  assert!(target.is_dir());

  progress(&places, std::process::id(), true);
  assert_eq!(idle_build_target(&places, now, span), Some(target.clone()));
  progress(&places, 999_999_999, false);
  assert_eq!(idle_build_target(&places, now, span), Some(target.clone()));

  remove_build_target(&places, &target).unwrap();
  assert!(!target.exists());
  assert!(target.parent().unwrap().is_dir());
  assert_eq!(idle_build_target(&places, now, span), None);
}
