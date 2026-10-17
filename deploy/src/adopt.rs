//! A deploy tree that is still a real directory taken once into the release layout (docs/deploy.md, 「A
//! deploy tree that is still a directory」): a release prepared at the commit it stands on with its
//! binaries, the directory and a link onto that release exchanged in one rename, and the directory kept
//! beside the link as a retired tree at the depth it stood at, until nothing stands in it.

use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;

use crate::release::{
  GitRun, Pruned, RETIRED, Standing, beside, built_from, hard_link_tree, live_processes, release_dir,
  same_place, standing,
};
use crate::remove::remove_tree;
use crate::tree::short;
use crate::unit::first_chars;

/// What an adoption is made out of.
pub struct Adopting<'a> {
  /// The repository the deploy tree and its releases are worktrees of.
  pub source: &'a Path,
  pub deploy_dir: &'a Path,
  /// The runtime data root, which no removal reaches.
  pub data_root: &'a Path,
  /// The name a retired tree is given after its suffix: the time of the adoption.
  pub stamp: &'a str,
}

/// What an adoption came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopted {
  /// The release the deploy tree's link now names.
  pub release: PathBuf,
  /// Where the directory that stood there went.
  pub retired: PathBuf,
}

/// The deploy tree adopted where it is still a directory, and a directory an earlier adoption exchanged
/// but did not retire retired now. Hands back the adoption made, or none where the deploy tree was a link
/// or absent already. `prepare` makes or takes the release of a commit, the way every release is made.
pub fn adopt(
  adopting: &Adopting<'_>,
  git: GitRun<'_>,
  prepare: &dyn Fn(&str) -> Result<PathBuf, String>,
  say: &dyn Fn(&str),
) -> Result<Option<Adopted>, String> {
  for left in exchanged_leftovers(adopting.deploy_dir) {
    let retired = retire(adopting, &left, git)?;

    say(&fill_shown(&DEPLOY.adopt.retired_earlier, &[("dir", &retired.display())]));
  }

  if standing(adopting.deploy_dir) != Standing::NotLink {
    return Ok(None);
  }

  let deploy_dir = adopting.deploy_dir;

  if !std::fs::symlink_metadata(deploy_dir).is_ok_and(|meta| meta.is_dir()) {
    return Err(fill_shown(&DEPLOY.adopt.a_file, &[("dir", &deploy_dir.display())]));
  }

  let head = git(&["rev-parse", "HEAD"], deploy_dir);
  let commit = head.out.trim().to_owned();

  if !head.ok || commit.len() != 40 {
    return Err(fill_shown(
      &DEPLOY.adopt.no_worktree,
      &[
        ("dir", &deploy_dir.display()),
        ("said", &first_chars(&head.out, DEPLOY.numbers.quoted_chars)),
      ],
    ));
  }

  // The binaries are taken over rather than built, so they must be the commit's own: a release whose
  // `built-from` names another commit would be made again over the live tree by the next run.
  if built_from(deploy_dir).as_deref() != Some(commit.as_str()) {
    return Err(fill_shown(
      &DEPLOY.adopt.not_built_from,
      &[("dir", &deploy_dir.display()), ("commit", &short(&commit))],
    ));
  }

  say(&fill_shown(
    &DEPLOY.adopt.adopting,
    &[("dir", &deploy_dir.display()), ("commit", &short(&commit))],
  ));

  let release = prepare(&commit)?;

  if release != release_dir(deploy_dir, &commit) {
    return Err(fill_shown(
      &DEPLOY.adopt.elsewhere,
      &[("commit", &short(&commit)), ("release", &release.display())],
    ));
  }

  take_binaries(deploy_dir, &release, adopting.data_root)?;

  let staged = exchanged_name(deploy_dir);
  let parent = deploy_dir.parent().unwrap_or(Path::new("/"));
  let target = release.strip_prefix(parent).map_or_else(|_| release.clone(), Path::to_path_buf);
  let _ = std::fs::remove_file(&staged);

  std::os::unix::fs::symlink(&target, &staged).map_err(|why| why.to_string())?;

  if let Err(why) = exchange(&staged, deploy_dir) {
    let _ = std::fs::remove_file(&staged);

    return Err(fill_shown(
      &DEPLOY.adopt.not_exchanged,
      &[("release", &release.display()), ("why", &why)],
    ));
  }

  say(&fill_shown(&DEPLOY.adopt.linked, &[("release", &release.display())]));

  let retired = retire(adopting, &staged, git)?;

  say(&fill_shown(&DEPLOY.adopt.retired, &[("dir", &retired.display())]));

  Ok(Some(Adopted { release, retired }))
}

// The name the directory stands under between the exchange and its retirement: the staged link's.
fn exchanged_name(deploy_dir: &Path) -> PathBuf {
  let parent = deploy_dir.parent().unwrap_or(Path::new("/"));

  parent.join(format!(".{}.adopt-{}", file_name(deploy_dir), std::process::id()))
}

fn file_name(path: &Path) -> String {
  path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

// Directories an adoption exchanged out of the deploy tree's place and was cut off before retiring; a
// staged link it never exchanged is taken away.
fn exchanged_leftovers(deploy_dir: &Path) -> Vec<PathBuf> {
  let parent = deploy_dir.parent().unwrap_or(Path::new("/"));
  let prefix = format!(".{}.adopt-", file_name(deploy_dir));
  let Ok(entries) = std::fs::read_dir(parent) else {
    return vec![];
  };
  let mut found = vec![];

  for entry in entries.flatten() {
    if !entry.file_name().to_string_lossy().starts_with(&prefix) {
      continue;
    }

    match entry.file_type() {
      Ok(kind) if kind.is_dir() => found.push(entry.path()),
      Ok(kind) if kind.is_symlink() => {
        let _ = std::fs::remove_file(entry.path());
      }
      _ => {}
    }
  }

  found.sort();
  found
}

/// Two names swapped in one rename, both standing before and after (`renameat2` with `RENAME_EXCHANGE`).
pub fn exchange(one: &Path, other: &Path) -> std::io::Result<()> {
  use std::os::unix::ffi::OsStrExt;

  let one = std::ffi::CString::new(one.as_os_str().as_bytes())?;
  let other = std::ffi::CString::new(other.as_os_str().as_bytes())?;

  // SAFETY: both paths are NUL-terminated strings that outlive the call, read against the working
  // directory, and renameat2 reads nothing else.
  let done = unsafe {
    libc::renameat2(libc::AT_FDCWD, one.as_ptr(), libc::AT_FDCWD, other.as_ptr(), libc::RENAME_EXCHANGE)
  };

  if done == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

// The directory's `bin/` hard-linked into the release, `built-from` with it, where the release does not
// stand built already: into a staged folder renamed into place, so a cut-off copy is never taken as whole.
fn take_binaries(deploy_dir: &Path, release: &Path, data_root: &Path) -> Result<(), String> {
  let bin = release.join("bin");

  if built_from(release) == built_from(deploy_dir) {
    return Ok(());
  }

  let staged = release.join(".bin.adopt");

  for stale in [&staged, &bin] {
    if std::fs::symlink_metadata(stale).is_ok() {
      remove_tree(stale, data_root)?;
    }
  }

  hard_link_tree(&deploy_dir.join("bin"), &staged)
    .and_then(|()| std::fs::rename(&staged, &bin))
    .map_err(|why| {
      fill_shown(&DEPLOY.adopt.bin_not_taken, &[("release", &release.display()), ("why", &why)])
    })
}

// A directory exchanged out of the deploy tree's place renamed to a retired tree beside the link, at the
// depth it stood at, and the repository's records of it and of its submodules pointed at it there.
fn retire(adopting: &Adopting<'_>, exchanged: &Path, git: GitRun<'_>) -> Result<PathBuf, String> {
  let base = beside(adopting.deploy_dir, &format!("{RETIRED}{}", adopting.stamp));
  let mut retired = base.clone();
  let mut count = 1;

  while std::fs::symlink_metadata(&retired).is_ok() {
    count += 1;
    retired = PathBuf::from(format!("{}-{count}", base.display()));
  }

  std::fs::rename(exchanged, &retired).map_err(|why| {
    fill_shown(&DEPLOY.adopt.not_retired, &[("dir", &exchanged.display()), ("why", &why)])
  })?;

  let repaired = git(&["worktree", "repair", &retired.to_string_lossy()], adopting.source);

  if !repaired.ok {
    return Err(fill_shown(
      &DEPLOY.adopt.not_repaired,
      &[
        ("dir", &retired.display()),
        ("said", &first_chars(&repaired.out, DEPLOY.numbers.quoted_chars)),
      ],
    ));
  }

  // The submodules' `.git` files are relative and still resolve at this depth, but each module's own
  // record of its work tree names the deploy tree's path, which now runs through the link into the
  // release.
  let modules =
    git(&["config", "--file", ".gitmodules", "--get-regexp", r"^submodule\..*\.path$"], &retired);

  // Each line git prints is the key, one space, then the value: the submodule's path.
  for path in modules
    .out
    .lines()
    .filter(|_| modules.ok)
    .filter_map(|line| line.split_once(' ').map(|(_, path)| path))
  {
    let tree = retired.join(path);

    if !tree.join(".git").is_file() {
      continue;
    }

    let admin = git(&["rev-parse", "--absolute-git-dir"], &tree);
    let pointed = admin.ok
      && git(
        &[
          "config",
          "--file",
          &Path::new(admin.out.trim()).join("config").to_string_lossy(),
          "core.worktree",
          &tree.to_string_lossy(),
        ],
        &retired,
      )
      .ok;

    if !pointed {
      return Err(fill_shown(
        &DEPLOY.adopt.submodule_not_pointed,
        &[
          ("path", &path),
          ("dir", &tree.display()),
          ("said", &first_chars(&admin.out, DEPLOY.numbers.quoted_chars)),
        ],
      ));
    }
  }

  Ok(retired)
}

/// Whether a live process of this account stands in a tree, as the release module reads it; every tree
/// counts as stood in where `/proc` could not be listed.
pub fn stood_in(tree: &Path) -> bool {
  live_processes(Path::new("/proc"))
    .is_none_or(|processes| processes.iter().any(|process| process.stands_in(tree)))
}

/// Every retired tree beside the deploy tree that nothing stands in taken away, through the guarded
/// removal, and the repository told to forget its worktree. One still stood in is left for a later run;
/// one that will not go is a warning.
pub fn prune_retired(
  deploy_dir: &Path,
  source: &Path,
  data_root: &Path,
  git: GitRun<'_>,
  in_use: &dyn Fn(&Path) -> bool,
) -> Pruned {
  let parent = deploy_dir.parent().unwrap_or(Path::new("/"));
  let prefix = format!("{}{RETIRED}", file_name(deploy_dir));
  let Ok(entries) = std::fs::read_dir(parent) else {
    return Pruned::default();
  };
  let mut retired: Vec<PathBuf> = entries
    .flatten()
    .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    .map(|entry| entry.path())
    .collect();
  let mut pruned = Pruned::default();

  retired.sort();

  for tree in retired {
    if in_use(&tree) || same_place(&tree, deploy_dir) {
      continue;
    }

    match remove_tree(&tree, data_root) {
      Ok(()) => pruned.removed.push(file_name(&tree)),
      Err(why) => pruned
        .warnings
        .push(fill_shown(&DEPLOY.adopt.not_removed, &[("dir", &tree.display()), ("why", &why)])),
    }
  }

  if !pruned.removed.is_empty() {
    git(&["worktree", "prune"], source);
  }

  pruned
}
