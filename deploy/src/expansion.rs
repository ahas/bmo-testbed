//! The build tree (docs/deploy.md, 「The releases」): `deploy-expand` beside the deploy tree, a detached
//! worktree moved to each commit deployed, where the release build runs, so its own target directory keeps
//! the build warm from one deployment to the next, until the prune takes it once the last deployment has
//! outlived its span.

use std::path::{Component, Path, PathBuf};

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_core::queue::process_alive;
use serde_json::Value;

use crate::deployment::DeployPlaces;
use crate::progress::read_deploy_progress;
use crate::release::{beside, put_link};
use crate::remove::remove_tree;
use crate::run::{Ran, command, words};
use crate::tree::{read_deploy_record, short};
use crate::unit::first_chars;

/// Runs one command to its end in a directory: whether it passed, and what it printed on both streams.
pub type CommandRun<'a> = &'a dyn Fn(&[String], &Path) -> Ran;

/// The build tree: beside the deploy tree, named after it with `-expand`.
pub fn expansion_tree(deploy_dir: &Path) -> PathBuf {
  beside(deploy_dir, "-expand")
}

/// The build tree's target directory, where the release build keeps its output between deployments.
pub fn build_target(deploy_dir: &Path) -> PathBuf {
  expansion_tree(deploy_dir).join("target")
}

/// Whether a deployment of the deploy root is running: its progress record holds a run with no ending whose
/// process is alive.
pub fn deployment_running(progress: &Path) -> bool {
  read_deploy_progress(std::fs::read_to_string(progress).ok().as_deref())
    .is_some_and(|run| run.ending.is_none() && process_alive(run.pid))
}

/// When the deploy root's last deployment was laid down, in milliseconds since the epoch, as its deployment
/// record says; none where no record reads.
pub fn last_deployed_ms(record: &Path) -> Option<i64> {
  let record = read_deploy_record(record)?;

  chrono::DateTime::parse_from_rfc3339(&record.at).ok().map(|at| at.timestamp_millis())
}

/// The build tree's target directory of a deployment where it has outlived its span (docs/deploy.md, 「The
/// releases」): the last deployment was laid down more than `span_ms` before `now`, and none is running. None
/// where it stays, or there is none.
pub fn idle_build_target(places: &DeployPlaces, now: i64, span_ms: i64) -> Option<PathBuf> {
  let target = build_target(&places.deploy_dir);

  (target.is_dir()
    && last_deployed_ms(&places.record).is_some_and(|at| at < now - span_ms)
    && !deployment_running(&places.progress))
  .then_some(target)
}

/// The build tree's target directory removed through the guarded removal, unless a deployment began since it
/// was found idle.
pub fn remove_build_target(places: &DeployPlaces, target: &Path) -> Result<(), String> {
  if deployment_running(&places.progress) {
    return Err(DEPLOY.expansion.target_in_use.clone());
  }

  remove_tree(target, &places.data_root)
}

/// The build tree put at a commit.
pub struct BuildTree<'a> {
  /// The repository the build tree is a worktree of.
  pub source: &'a Path,
  /// The build tree itself.
  pub tree: PathBuf,
  /// The commit it is put at.
  pub head: &'a str,
  /// The runtime data root, which nothing the build tree takes away may hold.
  pub data_root: &'a Path,
}

// Git run through the command runner, in a directory.
fn git(run: CommandRun<'_>, args: &[&str], cwd: &Path) -> Ran {
  let mut line = words(["git"]);

  line.extend(args.iter().map(|arg| (*arg).to_owned()));
  run(&line, cwd)
}

/// Puts the build tree at the commit, made where it is not there yet, with the submodules the commit pins
/// brought to their pins, and every file the commit moved made new to the clock a build reads.
pub fn put_build_tree(build: &BuildTree<'_>, run: CommandRun<'_>) -> Result<(), String> {
  let tree = &build.tree;
  let before = if tree.join(".git").exists() {
    let read = git(run, &["rev-parse", "HEAD"], tree);

    read.ok.then(|| read.out.trim().to_owned()).filter(|head| !head.is_empty())
  } else {
    None
  };
  let placed = if tree.join(".git").exists() {
    git(run, &["checkout", "--force", "--detach", build.head], tree)
  } else {
    // A worktree taken away by hand is still registered, and git refuses to make a second one under a
    // name it thinks it already has; the prune forgets only registrations whose directory is gone.
    git(run, &["worktree", "prune"], build.source);
    git(run, &["worktree", "add", "--detach", &tree.to_string_lossy(), build.head], build.source)
  };

  if !placed.ok {
    return Err(fill_shown(
      &DEPLOY.expansion.not_placed,
      &[
        ("tree", &tree.display()),
        ("commit", &short(build.head)),
        ("said", &first_chars(&placed.out, DEPLOY.numbers.quoted_chars)),
      ],
    ));
  }

  if let Some(before) = &before.filter(|before| before.as_str() != build.head) {
    refresh(tree, before, build.head, run)?;
  }

  bring_submodules(tree, build.data_root, run)
}

// Every path the commit moved in the tree, stamped with the moment the tree was put at it: a build reads a
// source by its modification time against what it built before, and a file the checkout leaves older than
// that build is one the build counts as built already. A path the commit removed, and a folder standing for
// a submodule, have nothing to stamp.
fn refresh(tree: &Path, from: &str, to: &str, run: CommandRun<'_>) -> Result<(), String> {
  let moved = git(run, &["diff", "--name-only", from, to], tree);

  if !moved.ok {
    return Err(fill_shown(
      &DEPLOY.expansion.failed,
      &[
        ("what", &"the files the commit moved"),
        ("said", &first_chars(&moved.out, DEPLOY.numbers.quoted_chars)),
      ],
    ));
  }

  let now = std::time::SystemTime::now();

  for name in moved.out.lines().map(str::trim).filter(|name| !name.is_empty()) {
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(tree.join(name)) {
      let _ = file.set_modified(now);
    }
  }

  Ok(())
}

/// Every submodule a tree's commit pins brought to its pin, whatever was changed in it discarded. A folder
/// the commit turned into a submodule, standing with the files it held before, is emptied first, since git
/// checks no submodule out over a folder that holds anything.
pub fn bring_submodules(tree: &Path, data_root: &Path, run: CommandRun<'_>) -> Result<(), String> {
  let declared = git(
    run,
    &["config", "--file", ".gitmodules", "--get-regexp", r"^submodule\..*\.path$"],
    tree,
  );

  // No `.gitmodules`, or one declaring nothing: git answers no, and there is nothing to bring.
  if !declared.ok {
    return Ok(());
  }

  // Each line git prints is the key, one space, then the value: the submodule's path.
  for path in declared.out.lines().filter_map(|line| line.split_once(' ').map(|(_, path)| path)) {
    let pinned = git(run, &["ls-files", "--stage", "--", path], tree).out.starts_with("160000 ");
    let at = tree.join(path);

    if pinned && !at.join(".git").exists() {
      empty_folder(&at, data_root).map_err(|why| {
        fill_shown(&DEPLOY.expansion.not_emptied, &[("at", &at.display()), ("why", &why)])
      })?;
    }
  }

  let fault = |ran: Ran, what: &str| {
    (!ran.ok).then(|| {
      fill_shown(
        &DEPLOY.expansion.failed,
        &[("what", &what), ("said", &first_chars(&ran.out, DEPLOY.numbers.quoted_chars))],
      )
    })
  };

  if let Some(fault) = fault(
    git(run, &["submodule", "sync", "--quiet", "--recursive"], tree),
    &DEPLOY.expansion.no_addresses,
  ) {
    return Err(fault);
  }

  match fault(
    git(run, &["submodule", "update", "--quiet", "--init", "--recursive", "--force"], tree),
    &fill_shown(&DEPLOY.expansion.not_pinned, &[("tree", &tree.display())]),
  ) {
    Some(fault) => Err(fault),
    None => Ok(()),
  }
}

// Whatever a folder holds taken away through the guarded removal, the folder itself left; a folder not
// there is empty already.
fn empty_folder(folder: &Path, data_root: &Path) -> Result<(), String> {
  let Ok(entries) = std::fs::read_dir(folder) else {
    return Ok(());
  };

  entries.flatten().try_for_each(|entry| remove_tree(&entry.path(), data_root))
}

/// The trees outside the repository that its manifests name by a path — the base libraries and the
/// sibling repositories' crates a release build reads — laid beside `tree` where those paths point, out
/// of the checkout at the same place. A worktree of the repository carries nothing beside it, so a
/// release built in one reads such a tree from a path only the workspace around the checkout makes.
///
/// A tree that is a member of another Cargo workspace is laid at that workspace's root, through which
/// the named path resolves and the member finds the root it inherits from; the trees named inside one
/// workspace share that one link. A standalone package is laid by itself.
///
/// The list comes from the checkout's own workspace: a release or a build tree reads no manifest until
/// the trees its manifests name stand, and the checkout is the one place they do.
pub fn lay_outside_trees(tree: &Path, checkout: &Path) -> Result<(), String> {
  let mut laid: Vec<PathBuf> = Vec::new();

  for named in outside_trees(tree, checkout)? {
    let target = std::fs::canonicalize(&named).map_err(|why| {
      fill_shown(
        &DEPLOY.expansion.tree_absent,
        &[("at", &named.display()), ("tree", &tree.display()), ("why", &why.to_string())],
      )
    })?;

    laid.push(workspace_root(&named, &target, checkout));
  }

  laid.sort();
  laid.dedup();

  // A tree inside another laid one resolves through that one's link and is laid no further.
  let mut kept: Vec<PathBuf> = Vec::new();

  for one in laid {
    if !kept.iter().any(|outer| one.starts_with(outer)) {
      kept.push(one);
    }
  }

  for source in kept {
    let Some(relative) = upward(checkout, &source) else {
      continue;
    };
    let place = normalize(&tree.join(relative));
    let target = std::fs::canonicalize(&source).map_err(|why| {
      fill_shown(
        &DEPLOY.expansion.tree_absent,
        &[("at", &source.display()), ("tree", &tree.display()), ("why", &why.to_string())],
      )
    })?;

    if let Some(parent) = place.parent() {
      std::fs::create_dir_all(parent).map_err(|why| {
        fill_shown(
          &DEPLOY.expansion.tree_unlinked,
          &[("at", &place.display()), ("tree", &tree.display()), ("why", &why.to_string())],
        )
      })?;
    }

    // A tree already standing there is left as it is, unless it is an earlier laying's folder of links:
    // what is not the deployment's own is nobody's to replace.
    if std::fs::symlink_metadata(&place).is_ok_and(|meta| !meta.file_type().is_symlink())
      && !links_only(&place)
    {
      continue;
    }

    put_link(&place, &target).map_err(|why| {
      fill_shown(
        &DEPLOY.expansion.tree_unlinked,
        &[("at", &place.display()), ("tree", &tree.display()), ("why", &why.to_string())],
      )
    })?;
  }

  Ok(())
}

// The tree to lay for one a manifest names: the root of the Cargo workspace it is a member of, on
// Cargo's own word asked from the checkout's copy of it, or the tree itself when it stands alone. A root
// that holds the checkout is no sibling's to lay, and the tree is laid by itself.
fn workspace_root(named: &Path, target: &Path, checkout: &Path) -> PathBuf {
  let read = command(
    &words(["cargo", "locate-project", "--workspace", "--message-format", "plain", "--offline"]),
    Some(target),
  )
  .output();
  let Some(read) = read.ok().filter(|read| read.status.success()) else {
    return named.to_path_buf();
  };
  let manifest = PathBuf::from(String::from_utf8_lossy(&read.stdout).trim());
  let Some(root) = manifest.parent().and_then(|root| std::fs::canonicalize(root).ok()) else {
    return named.to_path_buf();
  };
  let Ok(below) = target.strip_prefix(&root) else {
    return named.to_path_buf();
  };
  let mut at = named.to_path_buf();

  for _ in below.components() {
    at.pop();
  }

  if checkout.starts_with(&at) {
    return named.to_path_buf();
  }

  at
}

// Whether a folder holds nothing but folders and links, all the way down.
fn links_only(folder: &Path) -> bool {
  let Ok(mut entries) = std::fs::read_dir(folder) else {
    return false;
  };

  entries.all(|entry| {
    entry
      .and_then(|entry| entry.file_type().map(|kind| (entry.path(), kind)))
      .is_ok_and(|(path, kind)| kind.is_symlink() || (kind.is_dir() && links_only(&path)))
  })
}

// Every path dependency of the checkout's Cargo workspace that points outside it, read off Cargo's own
// word, as the path it names from the checkout's surroundings. The checkout is what is read: `tree`'s own
// manifests resolve nothing until the trees they name stand.
fn outside_trees(tree: &Path, checkout: &Path) -> Result<Vec<PathBuf>, String> {
  if !tree.join("Cargo.toml").exists() {
    return Ok(Vec::new());
  }

  let unread = |said: String| {
    fill_shown(&DEPLOY.expansion.trees_unread, &[("tree", &tree.display()), ("said", &said)])
  };
  let read = command(
    &words(["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"]),
    Some(checkout),
  )
  .output()
  .map_err(|why| unread(why.to_string()))?;

  if !read.status.success() {
    return Err(unread(String::from_utf8_lossy(&read.stderr).trim().to_owned()));
  }

  let metadata: Value = serde_json::from_slice(&read.stdout).map_err(|why| unread(why.to_string()))?;
  let packages = metadata["packages"].as_array().into_iter().flatten();
  let mut outside = Vec::new();

  for package in packages {
    let Some(manifest) = package["manifest_path"].as_str() else {
      continue;
    };
    let Some(dir) = Path::new(manifest).parent() else {
      continue;
    };
    if !dir.starts_with(checkout) {
      continue;
    }

    let dependencies = package["dependencies"].as_array().into_iter().flatten();

    for dependency in dependencies {
      let Some(path) = dependency["path"].as_str() else {
        continue;
      };
      let at = normalize(Path::new(path));

      if !at.starts_with(checkout) {
        outside.push(at);
      }
    }
  }

  Ok(outside)
}

// The path from one absolute directory to another, a step up for each folder between them.
fn upward(from: &Path, to: &Path) -> Option<PathBuf> {
  let from: Vec<Component<'_>> = from.components().collect();
  let to: Vec<Component<'_>> = to.components().collect();
  let shared = from.iter().zip(&to).take_while(|(one, other)| one == other).count();

  // Two paths of different roots share no step to count.
  if shared == 0 {
    return None;
  }

  let mut relative = PathBuf::new();

  for _ in shared..from.len() {
    relative.push("..");
  }

  for part in &to[shared..] {
    relative.push(part.as_os_str());
  }

  Some(relative)
}

// A path with its `.` and `..` taken out, an absolute path keeping its root.
fn normalize(path: &Path) -> PathBuf {
  let mut out = PathBuf::new();

  for part in path.components() {
    match part {
      Component::CurDir => {}
      Component::ParentDir => match out.components().next_back() {
        Some(Component::Normal(_)) => {
          out.pop();
        }
        _ => out.push(".."),
      },
      other => out.push(other.as_os_str()),
    }
  }

  out
}
