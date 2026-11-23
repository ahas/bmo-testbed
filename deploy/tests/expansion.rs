//! The build tree (docs/deploy.md, 「The releases」): the trees a workspace names by a path outside the
//! repository, laid beside a release and the build tree as links into their repositories' committed trees,
//! never into a checkout's working tree; and its target directory, which outlives the last deployment by its
//! span unless a deployment is running.

use std::path::{Path, PathBuf};

use bmo_testbed_deploy::deployment::DeployPlaces;
use bmo_testbed_deploy::expansion::{
  TreeStore, build_target, committed_tree, idle_build_target, last_deployed_ms, lay_outside_trees,
  remove_build_target,
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

// Git run in a folder as the deployment runs it, its output trimmed; the run must pass.
fn git_in(dir: &Path, args: &[&str]) -> String {
  let line: Vec<String> = std::iter::once("git").chain(args.iter().copied()).map(str::to_owned).collect();
  let ran = runner()(&line, dir);

  assert!(ran.ok, "git {args:?} in {}: {}", dir.display(), ran.out);
  ran.out.trim().to_owned()
}

// The folder made a repository whose main branch holds everything in it: the commit.
fn committed(dir: &Path) -> String {
  if !dir.join(".git").exists() {
    git_in(dir, &["init", "-q", "-b", "main"]);
  }

  git_in(dir, &["add", "-A"]);
  git_in(dir, &["commit", "-q", "--allow-empty", "-m", "commit"]);
  git_in(dir, &["rev-parse", "HEAD"])
}

// The places a test lays its trees with, under the folder given: the committed trees' folder and the data root.
struct Laid {
  store: PathBuf,
  data_root: PathBuf,
}

impl Laid {
  fn under(root: &Path) -> Self {
    Self {
      store: root.join("run/deploy-trees"),
      data_root: root.join("data"),
    }
  }

  fn lay(&self, tree: &Path, checkout: &Path) -> Result<Vec<String>, String> {
    lay_outside_trees(
      tree,
      checkout,
      &TreeStore {
        store: &self.store,
        data_root: &self.data_root,
      },
      &runner(),
    )
  }

  // The committed tree `named` is read from: its repository's worktree in the store, at its path there.
  fn committed_for(&self, named: &Path, checkout: &Path) -> PathBuf {
    committed_tree(named, checkout).unwrap().within(&self.store)
  }
}

// "the trees a workspace names outside it are laid beside the build tree as links into their committed trees"
#[test]
fn a_tree_named_outside_the_workspace_is_laid_where_its_path_points() {
  let root = tempfile::tempdir().unwrap();
  let base = root.path().join("workspace/base/text");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let laid = Laid::under(root.path());

  package(&base, "bmo-text");
  let head = committed(&base);
  app(&checkout, "../../base/text");
  app(&tree, "../../base/text");

  laid.lay(&tree, &checkout).unwrap();

  let link = root.path().join("base/text");
  let target = laid.committed_for(&base, &checkout);

  assert_eq!(
    symlink(&link),
    Some(target.clone()),
    "the tree named beside the build tree is a link into its committed tree"
  );
  assert!(target.starts_with(&laid.store));
  assert_eq!(git_in(&target, &["rev-parse", "HEAD"]), head);
  assert!(reads(&tree));
}

// "a tree whose checkout works on a job's branch is laid at the main its close pushed, not at the local main
// left behind"
#[test]
fn a_tree_is_laid_at_the_main_its_remote_holds_where_the_local_branch_stands_behind() {
  let root = tempfile::tempdir().unwrap();
  let base = root.path().join("workspace/base/text");
  let remote = root.path().join("remote.git");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let laid = Laid::under(root.path());

  package(&base, "bmo-text");
  let behind = committed(&base);
  git_in(root.path(), &["init", "-q", "--bare", "-b", "main", "remote.git"]);
  git_in(&base, &["remote", "add", "origin", &remote.to_string_lossy()]);
  git_in(&base, &["push", "-q", "-u", "origin", "main"]);
  git_in(&base, &["checkout", "-q", "-b", "work"]);
  let closed = committed(&base);
  git_in(&base, &["push", "-q", "origin", "HEAD:main"]);
  app(&checkout, "../../base/text");
  app(&tree, "../../base/text");

  laid.lay(&tree, &checkout).unwrap();

  let target = laid.committed_for(&base, &checkout);

  assert_ne!(behind, closed);
  assert_eq!(git_in(&base, &["rev-parse", "main"]), behind, "the local main stays behind");
  assert_eq!(git_in(&target, &["rev-parse", "HEAD"]), closed);
}

// "the trees outside a workspace are read from the checkout, since the tree reads none until they stand"
#[test]
fn a_workspace_reads_its_outside_trees_from_the_checkout_before_they_stand() {
  let root = tempfile::tempdir().unwrap();
  let base = root.path().join("workspace/base/text");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let laid = Laid::under(root.path());

  package(&base, "bmo-text");
  committed(&base);
  workspace_with(&checkout, "../../base/text");
  workspace_with(&tree, "../../base/text");

  assert!(!reads(&tree), "no manifest of the build tree reads before its tree stands");

  laid.lay(&tree, &checkout).unwrap();

  assert_eq!(
    symlink(&root.path().join("base/text")),
    Some(laid.committed_for(&base, &checkout)),
    "the tree the checkout's workspace names stands beside the build tree, though it stood nowhere there before"
  );
  assert!(reads(&tree));
}

// "a tree the checkout does not carry refuses the laying, naming it"
#[test]
fn a_tree_the_checkout_does_not_carry_refuses_the_laying() {
  let root = tempfile::tempdir().unwrap();
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");

  app(&checkout, "../../base/text");
  app(&tree, "../../base/text");

  let why = Laid::under(root.path()).lay(&tree, &checkout).unwrap_err();

  assert!(why.contains("base/text"), "{why}");
  assert!(!root.path().join("base/text").exists());
}

// "a tree no git repository holds, and one whose repository's main branch does not stand, refuse the
// deployment, the line naming the tree and why"
#[test]
fn a_tree_with_no_committed_tree_refuses_the_laying() {
  let root = tempfile::tempdir().unwrap();
  let base = root.path().join("workspace/base/text");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let laid = Laid::under(root.path());

  package(&base, "bmo-text");
  app(&checkout, "../../base/text");
  app(&tree, "../../base/text");

  let unheld = laid.lay(&tree, &checkout).unwrap_err();

  assert!(
    unheld.contains("base/text") && unheld.contains("no git repository holds it"),
    "{unheld}"
  );

  git_in(&base, &["init", "-q", "-b", "trunk"]);
  git_in(&base, &["add", "-A"]);
  git_in(&base, &["commit", "-q", "-m", "on another branch"]);

  let no_main = laid.lay(&tree, &checkout).unwrap_err();

  assert!(no_main.contains("base/text") && no_main.contains("no branch main"), "{no_main}");
  assert!(symlink(&root.path().join("base/text")).is_none());
}

// "a dependency inside the tree is no tree outside it, and no workspace at all is left alone"
#[test]
fn a_dependency_inside_the_tree_and_a_tree_with_no_workspace_are_left_alone() {
  let root = tempfile::tempdir().unwrap();
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let laid = Laid::under(root.path());

  package(&checkout.join("support"), "support");
  package(&tree.join("support"), "support");
  app(&checkout, "support");
  app(&tree, "support");

  laid.lay(&tree, &checkout).unwrap();

  assert!(symlink(&tree.join("support")).is_none());
  assert!(!laid.store.exists(), "nothing outside the tree, nothing committed laid");

  let plain = root.path().join("plain");

  std::fs::create_dir_all(&plain).unwrap();
  laid.lay(&plain, &checkout).unwrap();
}

// "The list of trees is read off Cargo's word from the workspace at the commit deployed, never from a checkout's
// working tree ... taken away once it is read": the primary checkout left behind the commit a build tree stands
// on, which adds a path dependency, and that dependency is laid all the same.
#[test]
fn a_dependency_the_commit_adds_is_laid_where_the_checkout_stands_behind_it() {
  let root = tempfile::tempdir().unwrap();
  let text = root.path().join("workspace/base/text");
  let other = root.path().join("workspace/base/other");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let laid = Laid::under(root.path());

  package(&text, "bmo-text");
  committed(&text);
  package(&other, "other");
  committed(&other);
  app_on(&checkout, &[("bmo-text", "../../base/text")]);
  let behind = committed(&checkout);
  app_on(&checkout, &[("bmo-text", "../../base/text"), ("other", "../../base/other")]);
  let ahead = committed(&checkout);

  git_in(&checkout, &["checkout", "-q", "--detach", &behind]);
  git_in(&checkout, &["worktree", "add", "-q", "--detach", &tree.to_string_lossy(), &ahead]);
  laid.lay(&tree, &checkout).unwrap();

  assert_eq!(
    symlink(&root.path().join("base/other")),
    Some(laid.committed_for(&other, &checkout)),
    "the tree the commit names stands beside the build tree, though the checkout's working tree names none"
  );
  assert!(reads(&tree));
  assert_eq!(names(&root.path().join("workspace/services")), ["app"], "the reading is taken away");
  assert!(!git_in(&checkout, &["worktree", "list"]).contains(".app.at-"));
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

// A sibling repository's workspace whose member crates inherit their version and edition from its root,
// committed on its main branch.
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

  committed(dir);
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
  let laid = Laid::under(root.path());

  sibling_workspace(&sibling, &["client"]);
  app_on(&checkout, &[("client", "../sibling/crates/client")]);
  app_on(&tree, &[("client", "../sibling/crates/client")]);

  laid.lay(&tree, &checkout).unwrap();

  assert_eq!(
    symlink(&root.path().join("run/sibling")),
    Some(laid.committed_for(&sibling, &checkout)),
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
  let laid = Laid::under(root.path());

  sibling_workspace(&sibling, &["client", "plugin"]);
  app_on(&checkout, &named);
  app_on(&tree, &named);

  laid.lay(&tree, &checkout).unwrap();

  assert_eq!(names(&root.path().join("run")), ["deploy-expand", "deploy-trees", "sibling"]);
  assert_eq!(names(&laid.store).len(), 1, "one repository, one committed tree");
  assert!(symlink(&root.path().join("run/sibling")).is_some());
  assert!(reads(&tree));
}

// "a standalone package is linked by itself"; "a tree inside the repository reached at its own path within
// the worktree"
#[test]
fn a_standalone_package_beside_a_sibling_member_is_laid_by_itself() {
  let root = tempfile::tempdir().unwrap();
  let base = root.path().join("workspace/base/text");
  let sibling = root.path().join("workspace/services/sibling");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let named = [("bmo-text", "../../base/text"), ("client", "../sibling/crates/client")];
  let laid = Laid::under(root.path());

  // The standalone package stands inside a repository of its own folder's parent.
  package(&base, "bmo-text");
  committed(&root.path().join("workspace/base"));
  sibling_workspace(&sibling, &["client"]);
  app_on(&checkout, &named);
  app_on(&tree, &named);

  laid.lay(&tree, &checkout).unwrap();

  let target = laid.committed_for(&base, &checkout);

  assert_eq!(
    symlink(&root.path().join("base/text")),
    Some(target.clone()),
    "the standalone package stands as a link of its own, its folder not linked"
  );
  assert!(target.ends_with("text") && target.parent().unwrap().starts_with(&laid.store));
  assert!(symlink(&root.path().join("base")).is_none());
  assert!(symlink(&root.path().join("run/sibling")).is_some());
  assert!(reads(&tree));
}

// A standalone package, committed in a repository of its own, that names one tree by a path among its
// dependencies and another among its dev-dependencies.
fn package_naming(dir: &Path, name: &str, dependency: (&str, &str), dev_dependency: (&str, &str)) {
  std::fs::create_dir_all(dir.join("src")).unwrap();
  std::fs::write(
    dir.join("Cargo.toml"),
    format!(
      "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{} = {{ path = \"{}\" }}\n\n[dev-dependencies]\n{} = {{ path = \"{}\" }}\n",
      dependency.0, dependency.1, dev_dependency.0, dev_dependency.1
    ),
  )
  .unwrap();
  std::fs::write(dir.join("src/lib.rs"), "").unwrap();
  committed(dir);
}

// Whether a tree's whole dependency graph resolves, every tree reached by a path included.
fn resolves(tree: &Path) -> bool {
  std::process::Command::new("cargo")
    .args(["metadata", "--offline", "--format-version", "1"])
    .current_dir(tree)
    .output()
    .unwrap()
    .status
    .success()
}

// "every tree those trees name in turn" is laid; "its dev-dependencies left out"
#[test]
fn a_tree_a_named_sibling_names_by_a_path_is_laid_as_well() {
  let root = tempfile::tempdir().unwrap();
  let middle = root.path().join("workspace/services/middle");
  let far = root.path().join("workspace/services/far");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let named = [("middle", "../middle")];
  let laid = Laid::under(root.path());

  sibling_workspace(&far, &["client"]);
  package_naming(&middle, "middle", ("client", "../far/crates/client"), ("probe", "../absent"));
  app_on(&checkout, &named);
  app_on(&tree, &named);

  let heads = laid.lay(&tree, &checkout).unwrap();

  assert_eq!(heads.len(), 2, "the named sibling and the tree it names are both laid");
  assert_eq!(symlink(&root.path().join("run/middle")), Some(laid.committed_for(&middle, &checkout)));
  assert_eq!(
    symlink(&root.path().join("run/far")),
    Some(laid.committed_for(&far, &checkout)),
    "the tree the sibling names is laid at its workspace root"
  );
  assert!(
    !root.path().join("run/absent").exists(),
    "a dependency's dev-dependency is not followed"
  );
  assert!(resolves(&tree), "the build tree's whole graph resolves through the links");
}

// "a link already on its committed tree is left as it is"
#[test]
fn a_link_already_on_the_committed_tree_is_left_alone() {
  use std::os::unix::fs::MetadataExt;

  let root = tempfile::tempdir().unwrap();
  let sibling = root.path().join("workspace/services/sibling");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let link = root.path().join("run/sibling");
  let laid = Laid::under(root.path());

  sibling_workspace(&sibling, &["client"]);
  app_on(&checkout, &[("client", "../sibling/crates/client")]);
  app_on(&tree, &[("client", "../sibling/crates/client")]);

  laid.lay(&tree, &checkout).unwrap();

  let before = std::fs::symlink_metadata(&link).unwrap().ino();

  laid.lay(&tree, &checkout).unwrap();

  assert_eq!(std::fs::symlink_metadata(&link).unwrap().ino(), before, "the same link, not a new one");
  assert_eq!(symlink(&link), Some(laid.committed_for(&sibling, &checkout)));
}

// "a link pointing elsewhere, onto a checkout's working tree among them, is moved onto it, and a folder
// holding nothing but folders and links, as an earlier laying leaves, is replaced; any other tree standing
// there is left as it is"
#[test]
fn a_link_onto_the_working_tree_or_a_folder_of_links_is_replaced_and_any_other_tree_is_kept() {
  let root = tempfile::tempdir().unwrap();
  let sibling = root.path().join("workspace/services/sibling");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let link = root.path().join("run/sibling");
  let laid = Laid::under(root.path());

  sibling_workspace(&sibling, &["client"]);
  app_on(&checkout, &[("client", "../sibling/crates/client")]);
  app_on(&tree, &[("client", "../sibling/crates/client")]);
  std::os::unix::fs::symlink(&sibling, &link).unwrap();

  laid.lay(&tree, &checkout).unwrap();

  assert_eq!(
    symlink(&link),
    Some(laid.committed_for(&sibling, &checkout)),
    "a link onto the checkout's working tree is moved"
  );

  std::fs::remove_file(&link).unwrap();
  std::fs::create_dir_all(link.join("crates")).unwrap();
  std::os::unix::fs::symlink(sibling.join("crates/client"), link.join("crates/client")).unwrap();

  laid.lay(&tree, &checkout).unwrap();

  assert_eq!(symlink(&link), Some(laid.committed_for(&sibling, &checkout)));

  std::fs::remove_file(&link).unwrap();
  std::fs::create_dir_all(link.join("crates")).unwrap();
  std::fs::write(link.join("Cargo.toml"), "").unwrap();

  laid.lay(&tree, &checkout).unwrap();

  assert!(symlink(&link).is_none(), "a tree holding a file of its own is nobody's to replace");
  assert!(link.join("Cargo.toml").exists());
}

// "A deployment builds committed trees only": a dependency's checkout mid-work, on a job's branch with a
// commit main does not hold and an uncommitted half-written edit over it, changes nothing the build tree
// compiles; its main moving on is what moves the committed tree, every path it moved stamped.
#[test]
fn a_dependencys_uncommitted_edit_changes_nothing_the_build_tree_compiles() {
  let root = tempfile::tempdir().unwrap();
  let base = root.path().join("workspace/base/text");
  let checkout = root.path().join("workspace/services/app");
  let tree = root.path().join("run/deploy-expand");
  let laid = Laid::under(root.path());
  let lib = |text: &str| std::fs::write(base.join("src/lib.rs"), text).unwrap();

  package(&base, "bmo-text");
  lib("pub fn greeting() -> &'static str {\n  \"committed\"\n}\n");
  let first = committed(&base);
  app(&checkout, "../../base/text");
  app(&tree, "../../base/text");
  std::fs::write(tree.join("src/main.rs"), "fn main() {\n  println!(\"{}\", bmo_text::greeting());\n}\n")
    .unwrap();

  // Another job works in the dependency's checkout: a commit on its own branch, and an edit half written.
  git_in(&base, &["checkout", "-q", "-b", "job"]);
  lib("pub fn greeting() -> &'static str {\n  \"on the job's branch\"\n}\n");
  committed(&base);
  lib("pub fn greeting( -> \n");

  laid.lay(&tree, &checkout).unwrap();

  let target = laid.committed_for(&base, &checkout);
  let built = |dir: &Path| {
    std::process::Command::new("cargo")
      .args(["run", "--quiet", "--offline"])
      .env("CARGO_TARGET_DIR", root.path().join("target"))
      .current_dir(dir)
      .output()
      .unwrap()
  };
  let ran = built(&tree);

  assert_eq!(git_in(&target, &["rev-parse", "HEAD"]), first);
  assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
  assert_eq!(
    String::from_utf8_lossy(&ran.stdout),
    "committed\n",
    "the build reads main's committed tree alone"
  );
  assert_eq!(git_in(&target, &["status", "--porcelain"]), "");

  // The job lands on main; the next laying builds its committed head, still none of the working tree.
  git_in(&base, &["checkout", "-q", "--", "src/lib.rs"]);
  git_in(&base, &["checkout", "-q", "main"]);
  git_in(&base, &["merge", "-q", "--ff-only", "job"]);
  lib("an edit no commit holds\n");

  laid.lay(&tree, &checkout).unwrap();

  let ran = built(&tree);

  assert_eq!(git_in(&target, &["rev-parse", "HEAD"]), git_in(&base, &["rev-parse", "main"]));
  assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
  assert_eq!(
    String::from_utf8_lossy(&ran.stdout),
    "on the job's branch\n",
    "the moved file was stamped and built again"
  );
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
        code: ran.status.code(),
        out: String::from_utf8_lossy(&ran.stdout).into_owned() + &String::from_utf8_lossy(&ran.stderr),
      },
      Err(why) => bmo_testbed_deploy::run::Ran {
        ok: false,
        code: None,
        out: why.to_string(),
      },
    }
  }
}

fn init_repo(dir: &Path) {
  std::fs::create_dir_all(dir.join("src")).unwrap();
  std::fs::write(
    dir.join("Cargo.toml"),
    "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
  )
  .unwrap();
  assert!(runner()(&["git".to_owned(), "init".to_owned(), "--quiet".to_owned()], dir).ok);
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
    fresh: false,
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

// "the build tree holds the commit it is moved to and nothing the commit before left ... all but its target
// directory"; "put again at the commit it already stands on, the tree keeps what that commit's own setup laid"
#[test]
fn a_build_tree_moved_to_another_commit_keeps_nothing_the_commit_before_left_but_its_target() {
  use bmo_testbed_deploy::expansion::put_build_tree;

  let root = tempfile::tempdir().unwrap();
  let source = root.path().join("source");
  let tree = root.path().join("deploy-expand");
  let run = runner();

  init_repo(&source);
  std::fs::write(source.join(".gitignore"), "/installed/\n/target/\n").unwrap();
  let one = commit(&source, "one\n");
  let two = commit(&source, "two\n");
  let leftovers = ["vendored/package.json", "installed/package/index.js", "target/release/app"];
  let lay = |paths: &[&str]| {
    for path in paths {
      let file = tree.join(path);

      std::fs::create_dir_all(file.parent().unwrap()).unwrap();
      std::fs::write(&file, "left\n").unwrap();
    }
  };

  put_build_tree(&tree_at(&source, &tree, root.path(), &one), &run).unwrap();
  lay(&leftovers);
  put_build_tree(&tree_at(&source, &tree, root.path(), &one), &run).unwrap();

  for path in leftovers {
    assert!(tree.join(path).exists(), "the same commit keeps what its own setup laid: {path}");
  }

  put_build_tree(&tree_at(&source, &tree, root.path(), &two), &run).unwrap();

  assert!(!tree.join("vendored").exists(), "an untracked file of the commit before is taken away");
  assert!(!tree.join("installed").exists(), "an ignored file of the commit before is taken away");
  assert!(tree.join("target/release/app").exists(), "the target directory stays warm");
  assert_eq!(std::fs::read_to_string(tree.join("src/lib.rs")).unwrap(), "two\n");
}

// "a run that starts on the tree finds nothing an earlier run left, even at the same commit"
#[test]
fn a_run_starting_on_the_build_tree_at_its_own_commit_finds_nothing_an_earlier_run_left() {
  use bmo_testbed_deploy::expansion::put_build_tree;

  let root = tempfile::tempdir().unwrap();
  let source = root.path().join("source");
  let tree = root.path().join("deploy-expand");
  let run = runner();

  init_repo(&source);
  std::fs::write(source.join(".gitignore"), "/target/\n").unwrap();
  let one = commit(&source, "one\n");

  put_build_tree(&tree_at(&source, &tree, root.path(), &one), &run).unwrap();
  std::fs::create_dir_all(tree.join("vendored")).unwrap();
  std::fs::write(tree.join("vendored/package.json"), "left\n").unwrap();
  std::fs::create_dir_all(tree.join("target/release")).unwrap();
  std::fs::write(tree.join("target/release/app"), "warm\n").unwrap();

  let fresh = bmo_testbed_deploy::expansion::BuildTree {
    fresh: true,
    ..tree_at(&source, &tree, root.path(), &one)
  };
  put_build_tree(&fresh, &run).unwrap();

  assert!(
    !tree.join("vendored").exists(),
    "what a failed run at the same commit wrote is taken away"
  );
  assert!(tree.join("target/release/app").exists(), "the target directory stays warm");
}

// "whether a tree is a worktree of the source: the common git directory each names"
#[test]
fn a_tree_belongs_to_the_source_whose_common_git_directory_it_names() {
  use bmo_testbed_deploy::expansion::worktree_of;

  let root = tempfile::tempdir().unwrap();
  let source = root.path().join("source");
  let other = root.path().join("other");
  let tree = root.path().join("deploy-expand");
  let run = runner();

  init_repo(&source);
  init_repo(&other);
  let one = commit(&source, "one\n");
  let added = run(
    &[
      "git".to_owned(),
      "worktree".to_owned(),
      "add".to_owned(),
      "--detach".to_owned(),
      tree.to_string_lossy().into_owned(),
      one,
    ],
    &source,
  );

  assert!(added.ok, "{}", added.out);
  assert!(worktree_of(&tree, &source, &run));
  assert!(!worktree_of(&tree, &other, &run), "another checkout is another repository");
  assert!(
    !worktree_of(&root.path().join("missing"), &source, &run),
    "a tree whose git directory cannot be read belongs to no source"
  );
}

// "a build tree made from another checkout of the repository is discarded and made again"
#[test]
fn a_build_tree_made_from_another_checkout_is_made_again_from_the_source() {
  use bmo_testbed_deploy::expansion::{put_build_tree, worktree_of};

  let root = tempfile::tempdir().unwrap();
  let first = root.path().join("first");
  let second = root.path().join("second");
  let tree = root.path().join("deploy-expand");
  let run = runner();

  init_repo(&first);
  let one = commit(&first, "one\n");
  let cloned = run(
    &[
      "git".to_owned(),
      "clone".to_owned(),
      "--quiet".to_owned(),
      first.to_string_lossy().into_owned(),
      second.to_string_lossy().into_owned(),
    ],
    root.path(),
  );

  assert!(cloned.ok, "{}", cloned.out);

  let two = commit(&second, "two\n");

  put_build_tree(&tree_at(&first, &tree, root.path(), &one), &run).unwrap();
  assert_eq!(std::fs::read_to_string(tree.join("src/lib.rs")).unwrap(), "one\n");

  // The first checkout never fetched the second's commit, so a tree kept from it cannot be put there.
  put_build_tree(&tree_at(&second, &tree, root.path(), &two), &run).unwrap();

  let head = run(&["git".to_owned(), "rev-parse".to_owned(), "HEAD".to_owned()], &tree)
    .out
    .trim()
    .to_owned();

  assert_eq!(head, two);
  assert_eq!(std::fs::read_to_string(tree.join("src/lib.rs")).unwrap(), "two\n");
  assert!(worktree_of(&tree, &second, &run), "the tree now belongs to the source");
  assert!(!worktree_of(&tree, &first, &run), "the earlier checkout no longer owns it");
}

// A source that cannot be read says nothing of whose the build tree is, so the warm tree is never taken away for
// it.
#[test]
fn a_build_tree_is_kept_where_the_checkout_it_was_put_from_is_gone() {
  use bmo_testbed_deploy::expansion::{put_build_tree, worktree_of};

  let root = tempfile::tempdir().unwrap();
  let primary = root.path().join("primary");
  let job = root.path().join("job");
  let tree = root.path().join("deploy-expand");
  let run = runner();
  let git = |args: &[&str], cwd: &Path| {
    let line: Vec<String> = std::iter::once("git").chain(args.iter().copied()).map(str::to_owned).collect();
    let ran = run(&line, cwd);

    assert!(ran.ok, "git {}: {}", args.join(" "), ran.out);
  };

  init_repo(&primary);
  let one = commit(&primary, "one\n");

  git(&["worktree", "add", "--quiet", "--detach", &job.to_string_lossy(), &one], &primary);
  put_build_tree(&tree_at(&job, &tree, root.path(), &one), &run).unwrap();
  git(&["worktree", "remove", "--force", &job.to_string_lossy()], &primary);

  let two = commit(&primary, "two\n");
  let _ = put_build_tree(&tree_at(&job, &tree, root.path(), &two), &run);

  assert!(worktree_of(&tree, &primary, &run), "the warm build tree stays the repository's");
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
