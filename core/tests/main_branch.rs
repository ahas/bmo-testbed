//! The head of a repository's main branch (docs/declaration.md, 「The main branch's head」): the branch as its
//! remote holds it, the local branch and the remote-tracking branch weighed against each other, and the remote
//! asked where the reader fetches.

use std::path::Path;
use std::process::{Command, Stdio};

use bmo_testbed_core::main_branch::MainBranch;

// Git run in a folder under a neutral identity, its output trimmed; the run must pass.
fn git(dir: &Path, args: &[&str]) -> String {
  let done = Command::new("git")
    .args(["-c", "user.name=tester", "-c", "user.email=tester@example.com"])
    .args(args)
    .current_dir(dir)
    .env_remove("GIT_DIR")
    .env_remove("GIT_WORK_TREE")
    .env_remove("GIT_INDEX_FILE")
    .stdin(Stdio::null())
    .output()
    .unwrap();

  assert!(done.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&done.stderr));
  String::from_utf8_lossy(&done.stdout).trim().to_owned()
}

// A commit on whatever the clone stands on: its id.
fn commit(clone: &Path, message: &str) -> String {
  git(clone, &["commit", "-q", "--allow-empty", "-m", message]);
  git(clone, &["rev-parse", "HEAD"])
}

// A bare remote under `root` and a clone of it whose main branch holds one commit, pushed.
fn remote_and_clone(root: &Path, clone: &str) -> std::path::PathBuf {
  let remote = root.join("remote.git");
  let at = root.join(clone);

  git(root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
  git(root, &["clone", "-q", &remote.to_string_lossy(), clone]);
  git(&at, &["checkout", "-q", "-b", "main"]);
  commit(&at, "first");
  git(&at, &["push", "-q", "-u", "origin", "main"]);
  at
}

// "a close pushed onto the remote's main from a job's branch is the head, and a local main that has parted from
// it is refused"
#[test]
fn the_remotes_main_is_the_head_where_the_local_branch_stands_behind_and_parted_is_refused() {
  let root = tempfile::tempdir().unwrap();
  let clone = remote_and_clone(root.path(), "checkout");

  git(&clone, &["checkout", "-q", "-b", "work"]);
  let closed = commit(&clone, "closed");
  git(&clone, &["push", "-q", "origin", "HEAD:main"]);

  assert_eq!(MainBranch::new(&clone, "main").head(), Ok(closed.clone()));

  git(&clone, &["checkout", "-q", "main"]);
  let ahead = commit(&clone, "local alone");
  let parted = MainBranch::new(&clone, "main").head().unwrap_err();

  assert!(
    parted.contains("parted") && parted.contains(&ahead) && parted.contains(&closed),
    "{parted}"
  );
}

// "a fetch brings the main another checkout pushed, and a local branch ahead of the remote's is the head"
#[test]
fn a_fetch_brings_the_main_another_checkout_pushed() {
  let root = tempfile::tempdir().unwrap();
  let clone = remote_and_clone(root.path(), "checkout");
  let other = root.path().join("other");

  git(
    root.path(),
    &["clone", "-q", &root.path().join("remote.git").to_string_lossy(), "other"],
  );
  let pushed = commit(&other, "from another checkout");
  git(&other, &["push", "-q", "origin", "main"]);

  let main = MainBranch::new(&clone, "main");

  assert_ne!(main.head(), Ok(pushed.clone()));
  main.fetch().unwrap();
  assert_eq!(main.head(), Ok(pushed.clone()));

  git(&clone, &["merge", "-q", "--ff-only", "origin/main"]);
  let ahead = commit(&clone, "not pushed yet");

  assert_eq!(main.head(), Ok(ahead));
}
