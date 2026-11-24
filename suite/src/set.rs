//! The set of repositories a release reads (docs/suite.md, 「The nightly round」): the repository whose root is
//! the set's folder, every repository whose root stands directly in a folder directly inside it, and the
//! checkouts kept outside it, each once per remote. A linked worktree is passed over; a repository two clones
//! reach by one `origin` remote is read once, from the clone whose folder's name ends the remote's repository
//! name; a repository with no `origin` remote is passed over.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::nightly::NIGHTLY;

/// The set as its folders hold it: the checkouts read, each with its `origin` remote, and the folders passed
/// over with why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetRead {
  pub members: Vec<(PathBuf, String)>,
  pub passed_over: Vec<(PathBuf, String)>,
}

// The folders directly inside one, by name, hidden folders left out.
fn folders_in(dir: &Path) -> Vec<PathBuf> {
  let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
    .map(|entry| entry.path())
    .collect();

  found.sort();
  found
}

/// The name a remote's address ends with, its `.git` left off.
pub fn remote_name(remote: &str) -> String {
  let last = remote.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or(remote);

  last.strip_suffix(".git").unwrap_or(last).to_owned()
}

/// The set standing in `folder` and in the checkouts `with` names, each read once per remote; `remote_of`
/// answers a checkout's `origin` remote, none where it has none. A checkout whose `.git` is a file, a linked
/// worktree, is no candidate, and neither is a folder holding no `.git` at all.
pub fn read_set(folder: &Path, with: &[PathBuf], remote_of: &dyn Fn(&Path) -> Option<String>) -> SetRead {
  let said = &NIGHTLY.set;
  let candidates = std::iter::once(folder.to_path_buf())
    .chain(folders_in(folder).into_iter().flat_map(|group| folders_in(&group)))
    .chain(with.iter().cloned())
    .filter(|dir| dir.join(".git").is_dir());
  let mut read = SetRead::default();
  let mut by_remote: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
  let mut seen: Vec<PathBuf> = Vec::new();

  for checkout in candidates {
    if seen.contains(&checkout) {
      continue;
    }

    seen.push(checkout.clone());

    let Some(remote) = remote_of(&checkout) else {
      read.passed_over.push((checkout, said.no_remote.clone()));
      continue;
    };

    by_remote.entry(remote).or_default().push(checkout);
  }

  for (remote, mut clones) in by_remote {
    let name = remote_name(&remote);
    let own = clones
      .iter()
      .position(|clone| {
        clone
          .file_name()
          .is_some_and(|folder| name.ends_with(folder.to_string_lossy().as_ref()))
      })
      .unwrap_or(0);
    let member = clones.remove(own);

    for other in clones {
      read.passed_over.push((other, fill_marks(&said.second_clone, &[("remote", &remote)])));
    }

    read.members.push((member, remote));
  }

  read.members.sort();
  read.passed_over.sort();
  read
}
