//! A deploy tree that is still a directory (docs/deploy.md, 「A deploy tree that is still a directory」):
//! adopted once into a release, a cut-off run finished by the next, the retired tree kept while something
//! stands in it, and the one-rename exchange.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_deploy::adopt::{Adopted, Adopting, adopt, exchange, prune_retired, stood_in};
use bmo_testbed_deploy::release::{
  DEPLOY_VARIABLE, Preparing, Standing, built_from, live_release, prepare, release_dir, standing,
};
use bmo_testbed_deploy::tree::short;

use crate::stand::{Stand, git, must, sleeper, stand};

// A repository and a deploy tree standing as a directory: a worktree of it at its commit, with binaries
// built from that commit.
fn directory() -> (Stand, String) {
  let at = stand();
  let commit = at.head();

  must(
    &["worktree", "add", "--quiet", "--detach", &at.deploy_dir.to_string_lossy(), &commit],
    &at.source,
  );
  std::fs::create_dir_all(at.deploy_dir.join("bin")).unwrap();
  std::fs::write(at.deploy_dir.join("bin/server"), "binary").unwrap();
  std::fs::write(at.deploy_dir.join("bin/built-from"), format!("{commit}\n")).unwrap();

  (at, commit)
}

fn adopted(at: &Stand, stamp: &str) -> Result<Option<Adopted>, String> {
  let preparing = |commit: &str| {
    prepare(
      &Preparing {
        source: &at.source,
        deploy_dir: &at.deploy_dir,
        commit,
        data_root: &at.data_root,
      },
      &git,
      &|_| Ok(()),
      &|_| {},
    )
    .map(|prepared| prepared.release)
  };

  adopt(
    &Adopting {
      source: &at.source,
      deploy_dir: &at.deploy_dir,
      data_root: &at.data_root,
      stamp,
    },
    &git,
    &preparing,
    &|_| {},
  )
}

fn inode(path: &Path) -> u64 {
  std::fs::metadata(path).unwrap().ino()
}

fn worktrees(source: &Path) -> String {
  must(&["worktree", "list"], source)
}

#[test]
fn a_directory_is_exchanged_for_a_link_onto_a_release_of_its_commit_and_retired_beside_it() {
  let (at, commit) = directory();
  let old_binary = inode(&at.deploy_dir.join("bin/server"));
  let adopted = adopted(&at, "20261014-051500").unwrap().unwrap();
  let release = release_dir(&at.deploy_dir, &commit);

  assert_eq!(adopted.release, release);
  assert_eq!(live_release(&at.deploy_dir), Some(release.clone()));
  assert_eq!(
    std::fs::read_link(&at.deploy_dir).unwrap(),
    Path::new("deploy-releases").join(short(&commit))
  );
  assert_eq!(adopted.retired, at.root.path().join("deploy.retired-20261014-051500"));

  // The binaries are the same files, not copies.
  assert_eq!(inode(&release.join("bin/server")), old_binary);
  assert_eq!(built_from(&release).as_deref(), Some(commit.as_str()));

  // The retired tree is the old directory, and the repository knows it there.
  assert_eq!(inode(&adopted.retired.join("bin/server")), old_binary);
  assert_eq!(must(&["rev-parse", "HEAD"], &adopted.retired), commit);
  assert!(worktrees(&at.source).contains(&adopted.retired.to_string_lossy().into_owned()));
}

#[test]
fn a_second_run_finds_the_link_and_does_nothing() {
  let (at, _) = directory();

  adopted(&at, "20261014-051500").unwrap().unwrap();

  let names = || std::fs::read_dir(at.root.path()).unwrap().flatten().count();
  let before = names();

  assert_eq!(adopted(&at, "20261014-051600").unwrap(), None);
  assert_eq!(names(), before);
}

#[test]
fn a_deploy_tree_not_there_is_left_to_the_deployment() {
  let at = stand();

  assert_eq!(adopted(&at, "20261014-051500").unwrap(), None);
  assert!(std::fs::symlink_metadata(&at.deploy_dir).is_err());
}

#[test]
fn a_directory_whose_binaries_are_not_its_commits_is_refused_and_left_in_place() {
  let (at, commit) = directory();

  std::fs::write(at.deploy_dir.join("bin/built-from"), "0123456789abcdef0123456789abcdef01234567\n")
    .unwrap();

  assert_eq!(
    adopted(&at, "20261014-051500").unwrap_err(),
    fill_shown(
      &DEPLOY.adopt.not_built_from,
      &[("dir", &at.deploy_dir.display()), ("commit", &short(&commit))]
    )
  );
  assert_eq!(standing(&at.deploy_dir), Standing::NotLink);
  assert!(!release_dir(&at.deploy_dir, &commit).exists());
}

#[test]
fn a_directory_that_is_no_worktree_or_a_file_in_its_place_is_refused() {
  let at = stand();
  // What git says of the directory follows the policy's line, so the line is matched up to it.
  let no_worktree =
    fill_shown(&DEPLOY.adopt.no_worktree, &[("dir", &at.deploy_dir.display()), ("said", &"")]);

  std::fs::create_dir_all(&at.deploy_dir).unwrap();
  assert!(adopted(&at, "20261014-051500").unwrap_err().starts_with(&no_worktree));

  std::fs::remove_dir(&at.deploy_dir).unwrap();
  std::fs::write(&at.deploy_dir, "a file").unwrap();
  assert_eq!(
    adopted(&at, "20261014-051500").unwrap_err(),
    fill_shown(&DEPLOY.adopt.a_file, &[("dir", &at.deploy_dir.display())])
  );
}

#[test]
fn a_directory_exchanged_by_a_cut_off_run_is_retired_by_the_next() {
  let (at, _) = directory();
  let first = adopted(&at, "20261014-051500").unwrap().unwrap();
  let left = at.root.path().join(".deploy.adopt-1");
  let staged = at.root.path().join(".deploy.adopt-2");

  // As a run cut off between the exchange and the retirement leaves it, and one cut off before the
  // exchange leaves its staged link.
  std::fs::rename(&first.retired, &left).unwrap();
  std::os::unix::fs::symlink("deploy-releases/none", &staged).unwrap();

  assert_eq!(adopted(&at, "20261014-051600").unwrap(), None);
  assert!(!left.exists());
  assert!(std::fs::symlink_metadata(&staged).is_err());

  let retired = at.root.path().join("deploy.retired-20261014-051600");

  assert!(retired.join("bin/server").exists());
  assert!(worktrees(&at.source).contains(&retired.to_string_lossy().into_owned()));
}

#[test]
fn a_retired_name_taken_already_gets_a_count() {
  let (at, _) = directory();

  std::fs::create_dir_all(at.root.path().join("deploy.retired-20261014-051500")).unwrap();

  let adopted = adopted(&at, "20261014-051500").unwrap().unwrap();

  assert_eq!(adopted.retired, at.root.path().join("deploy.retired-20261014-051500-2"));
}

#[test]
fn a_retired_tree_goes_once_nothing_stands_in_it_and_its_worktree_with_it() {
  let (at, _) = directory();
  let adopted = adopted(&at, "20261014-051500").unwrap().unwrap();
  let kept = prune_retired(&at.deploy_dir, &at.source, &at.data_root, &git, &|_| true);

  assert!(kept.removed.is_empty() && adopted.retired.exists());

  let pruned = prune_retired(&at.deploy_dir, &at.source, &at.data_root, &git, &|_| false);

  assert_eq!(pruned.removed, ["deploy.retired-20261014-051500"]);
  assert!(!adopted.retired.exists() && adopted.release.exists());
  assert!(!worktrees(&at.source).contains(&adopted.retired.to_string_lossy().into_owned()));
}

#[test]
fn the_exchange_swaps_a_directory_and_a_link_in_one_rename() {
  let dir = tempfile::tempdir().unwrap();
  let tree = dir.path().join("tree");
  let link = dir.path().join("link");

  std::fs::create_dir_all(tree.join("inside")).unwrap();
  std::os::unix::fs::symlink("elsewhere", &link).unwrap();
  exchange(&link, &tree).unwrap();

  assert_eq!(std::fs::read_link(&tree).unwrap(), Path::new("elsewhere"));
  assert!(link.join("inside").is_dir());
  assert!(exchange(&dir.path().join("missing"), &tree).is_err());
}

// A deep shape: the repository under `work/app` and the deploy tree under `srv/app/deploy`, a worktree of
// it whose submodule's `.git` file names its module relatively.
#[test]
fn an_adoption_in_a_deep_tree_keeps_the_submodule_whole_and_the_deploy_tree_never_absent() {
  use std::sync::Arc;
  use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

  let root = tempfile::tempdir().unwrap();
  let remote = root.path().join("remote/engine");
  let source = root.path().join("work/app");
  let deploy_dir = root.path().join("srv/app/deploy");
  let data_root = root.path().join("srv/app/data");

  std::fs::create_dir_all(&remote).unwrap();
  std::fs::write(remote.join("engine.rs"), "engine\n").unwrap();
  must(&["init", "--quiet", "-b", "main"], &remote);
  must(&["add", "."], &remote);
  must(&["commit", "--quiet", "-m", "engine"], &remote);

  let engine = must(&["rev-parse", "HEAD"], &remote);

  std::fs::create_dir_all(source.join("app")).unwrap();
  std::fs::write(source.join("app/main.txt"), "app\n").unwrap();
  must(&["init", "--quiet", "-b", "main"], &source);
  must(&["submodule", "add", "--quiet", &remote.to_string_lossy(), "engine"], &source);
  must(&["add", "."], &source);
  must(&["commit", "--quiet", "-m", "first"], &source);

  let commit = must(&["rev-parse", "HEAD"], &source);
  let bring = |tree: &Path| {
    let brought = git(&["submodule", "update", "--init", "--quiet"], tree);

    if brought.ok { Ok(()) } else { Err(brought.out) }
  };

  must(
    &["worktree", "add", "--quiet", "--detach", &deploy_dir.to_string_lossy(), &commit],
    &source,
  );
  bring(&deploy_dir).unwrap();

  let gitfile = std::fs::read_to_string(deploy_dir.join("engine/.git")).unwrap();

  assert!(
    gitfile.starts_with("gitdir: ../../../../work/app/.git/worktrees/deploy/modules/engine"),
    "{gitfile}"
  );

  std::fs::create_dir_all(deploy_dir.join("bin")).unwrap();
  std::fs::write(deploy_dir.join("bin/server"), "binary").unwrap();
  std::fs::write(deploy_dir.join("bin/built-from"), format!("{commit}\n")).unwrap();

  // A process started in the directory: its working directory and its environment in the old tree.
  let stand_in = sleeper(&deploy_dir.join("app"), Some(&deploy_dir));

  // Something looking at the deploy tree's name the whole while, as every reader does.
  let stop = Arc::new(AtomicBool::new(false));
  let (looks, absent) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
  let watcher = {
    let (stop, looks, absent, deploy_dir) =
      (stop.clone(), looks.clone(), absent.clone(), deploy_dir.clone());

    std::thread::spawn(move || {
      while !stop.load(Ordering::Relaxed) {
        looks.fetch_add(1, Ordering::Relaxed);

        if std::fs::symlink_metadata(&deploy_dir).is_err() {
          absent.fetch_add(1, Ordering::Relaxed);
        }
      }
    })
  };
  let adopting = Adopting {
    source: &source,
    deploy_dir: &deploy_dir,
    data_root: &data_root,
    stamp: "20261014-051500",
  };
  let preparing = |commit: &str| {
    prepare(
      &Preparing {
        source: &source,
        deploy_dir: &deploy_dir,
        commit,
        data_root: &data_root,
      },
      &git,
      &|release| bring(release),
      &|_| {},
    )
    .map(|prepared| prepared.release)
  };
  let adopted = adopt(&adopting, &git, &preparing, &|_| {}).unwrap().unwrap();

  stop.store(true, Ordering::Relaxed);
  watcher.join().unwrap();

  assert!(looks.load(Ordering::Relaxed) > 0);
  assert_eq!(absent.load(Ordering::Relaxed), 0, "the deploy tree was absent while it was adopted");

  let retired = root.path().join("srv/app/deploy.retired-20261014-051500");

  assert_eq!(adopted.retired, retired);
  assert_eq!(live_release(&deploy_dir), Some(adopted.release.clone()));

  // The stand-in moved with the directory, and the retention sees it standing there.
  let cwd = std::fs::read_link(format!("/proc/{}/cwd", stand_in.0.id())).unwrap();

  assert_eq!(cwd, std::fs::canonicalize(retired.join("app")).unwrap());
  assert!(stood_in(&retired));

  let environ = std::fs::read(format!("/proc/{}/environ", stand_in.0.id())).unwrap();
  let named = format!("{DEPLOY_VARIABLE}={}", deploy_dir.display());

  assert!(environ.split(|byte| *byte == 0).any(|pair| pair == named.as_bytes()));
  // What it names through the deploy tree's path is the same commit's.
  assert_eq!(std::fs::read_to_string(deploy_dir.join("app/main.txt")).unwrap(), "app\n");

  // The retired tree's submodule still finds its module and its own work tree, and the release's has its
  // own.
  let retired_module = retired.join("engine");
  let release_module = adopted.release.join("engine");

  assert_eq!(std::fs::read_to_string(retired_module.join(".git")).unwrap(), gitfile);
  assert_eq!(must(&["rev-parse", "HEAD"], &retired_module), engine);
  assert_eq!(
    PathBuf::from(must(&["rev-parse", "--show-toplevel"], &retired_module)),
    std::fs::canonicalize(&retired_module).unwrap()
  );
  assert!(git(&["status", "--short"], &retired_module).ok);
  assert!(git(&["status", "--short"], &release_module).ok);
  assert_eq!(must(&["rev-parse", "HEAD"], &release_module), engine);
  assert!(worktrees(&source).contains(&retired.to_string_lossy().into_owned()));

  // A second run does nothing, and the retired tree stays while the stand-in stands in it.
  assert_eq!(adopt(&adopting, &git, &preparing, &|_| {}).unwrap(), None);
  assert!(prune_retired(&deploy_dir, &source, &data_root, &git, &stood_in).removed.is_empty());

  drop(stand_in);

  let pruned = prune_retired(&deploy_dir, &source, &data_root, &git, &stood_in);

  assert_eq!(pruned.removed, ["deploy.retired-20261014-051500"]);
  assert!(git(&["status", "--short"], &release_module).ok);
  assert!(!worktrees(&source).contains(&retired.to_string_lossy().into_owned()));
}

#[test]
fn a_process_standing_in_a_tree_is_seen() {
  let here = std::env::current_dir().unwrap();
  let elsewhere = tempfile::tempdir().unwrap();

  assert!(stood_in(&here));
  assert!(!stood_in(elsewhere.path()));
}
