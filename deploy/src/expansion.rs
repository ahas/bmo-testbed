//! The build tree (docs/deploy.md, 「The releases」): `deploy-expand` beside the deploy tree, a detached
//! worktree moved to each commit deployed, where the release build runs, so its own target directory keeps
//! the build warm from one deployment to the next.

use std::path::{Component, Path, PathBuf};

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use serde_json::Value;

use crate::release::{beside, put_link};
use crate::remove::remove_tree;
use crate::run::{Ran, command, words};
use crate::tree::short;
use crate::unit::first_chars;

/// Runs one command to its end in a directory: whether it passed, and what it printed on both streams.
pub type CommandRun<'a> = &'a dyn Fn(&[String], &Path) -> Ran;

/// The build tree: beside the deploy tree, named after it with `-expand`.
pub fn expansion_tree(deploy_dir: &Path) -> PathBuf {
  beside(deploy_dir, "-expand")
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
/// brought to their pins.
pub fn put_build_tree(build: &BuildTree<'_>, run: CommandRun<'_>) -> Result<(), String> {
  let tree = &build.tree;
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

  bring_submodules(tree, build.data_root, run)
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

/// A tree a workspace's manifests name by a path outside it.
struct Outside {
  /// The folder the package that names it stands in, from the workspace's root.
  package: PathBuf,
  /// The path from that package to the tree it names, as the manifest gives it.
  relative: PathBuf,
}

/// The trees outside the repository that its manifests name by a path — the base libraries a release
/// build reads — laid beside `tree` where those paths point, out of the checkout at the same place. A
/// worktree of the repository carries nothing beside it, so a release built in one reads a base library
/// from a path only the workspace around the checkout makes.
///
/// The list comes from the checkout's own workspace: a release or a build tree reads no manifest until
/// the trees its manifests name stand, and the checkout is the one place they do.
pub fn lay_outside_trees(tree: &Path, checkout: &Path) -> Result<(), String> {
  for one in outside_trees(tree, checkout)? {
    let source = normalize(&checkout.join(&one.package).join(&one.relative));
    let place = normalize(&tree.join(&one.package).join(&one.relative));
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

    // A tree already standing there is left as it is: what is not a link of the deployment's is nobody's
    // to replace.
    if std::fs::symlink_metadata(&place).is_ok_and(|meta| !meta.file_type().is_symlink()) {
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

// Every path dependency of the checkout's Cargo workspace that points outside it, read off Cargo's own
// word, with the package that names it and the path it names. The checkout is what is read: `tree`'s own
// manifests resolve nothing until the trees they name stand.
fn outside_trees(tree: &Path, checkout: &Path) -> Result<Vec<Outside>, String> {
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
    let Ok(from_root) = dir.strip_prefix(checkout) else {
      continue;
    };
    let dependencies = package["dependencies"].as_array().into_iter().flatten();

    for dependency in dependencies {
      let Some(path) = dependency["path"].as_str() else {
        continue;
      };
      let at = normalize(Path::new(path));

      if at.starts_with(checkout) {
        continue;
      }

      let Some(relative) = upward(dir, &at).filter(|path| !path.as_os_str().is_empty()) else {
        continue;
      };

      outside.push(Outside {
        package: from_root.to_path_buf(),
        relative,
      });
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
