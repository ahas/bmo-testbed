//! What the tests over a repository stand on: git run in a temporary directory, a repository with a
//! deploy tree's place beside it, and a sleeping process standing somewhere.

use std::path::{Path, PathBuf};
use std::process::Command;

use bmo_testbed_deploy::release::{DEPLOY_VARIABLE, Prepared, Preparing, prepare};
use bmo_testbed_deploy::run::Ran;

/// Git run in a directory with an identity of its own and no signing.
pub fn git(args: &[&str], cwd: &Path) -> Ran {
  let ran = Command::new("git")
    .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
    .args(["-c", "protocol.file.allow=always"])
    .args(args)
    .current_dir(cwd)
    .output()
    .unwrap();

  Ran {
    ok: ran.status.success(),
    out: format!("{}{}", String::from_utf8_lossy(&ran.stdout), String::from_utf8_lossy(&ran.stderr))
      .trim()
      .to_owned(),
  }
}

/// Git that must pass, answering what it printed.
pub fn must(args: &[&str], cwd: &Path) -> String {
  let ran = git(args, cwd);

  assert!(ran.ok, "git {}: {}", args.join(" "), ran.out);
  ran.out
}

/// A repository with one commit, and a deploy tree's place and a data root beside it.
pub struct Stand {
  pub root: tempfile::TempDir,
  pub source: PathBuf,
  pub deploy_dir: PathBuf,
  pub data_root: PathBuf,
}

/// A repository with a folder of sources, and a deploy tree's place beside it.
pub fn stand() -> Stand {
  let root = tempfile::tempdir().unwrap();
  let source = root.path().join("source");

  std::fs::create_dir_all(source.join("app")).unwrap();
  std::fs::write(source.join("app/main.txt"), "app\n").unwrap();
  must(&["init", "--quiet", "-b", "main"], &source);
  must(&["add", "."], &source);
  must(&["commit", "--quiet", "-m", "first"], &source);

  Stand {
    deploy_dir: root.path().join("deploy"),
    data_root: root.path().join("data"),
    source,
    root,
  }
}

impl Stand {
  pub fn head(&self) -> String {
    must(&["rev-parse", "HEAD"], &self.source)
  }

  /// A commit adding a file named and holding its name.
  pub fn commit(&self, file: &str) -> String {
    if let Some(dir) = self.source.join(file).parent() {
      std::fs::create_dir_all(dir).unwrap();
    }

    std::fs::write(self.source.join(file), file).unwrap();
    must(&["add", "."], &self.source);
    must(&["commit", "--quiet", "-m", file], &self.source);
    self.head()
  }

  /// The release of a commit prepared, each setup counted.
  pub fn prepare(&self, commit: &str, setups: &std::cell::Cell<u32>) -> Prepared {
    let preparing = Preparing {
      source: &self.source,
      deploy_dir: &self.deploy_dir,
      commit,
      data_root: &self.data_root,
    };
    let setup = |_: &Path| {
      setups.set(setups.get() + 1);
      Ok(())
    };

    prepare(&preparing, &git, &setup, &|_| {}).unwrap()
  }
}

/// A release's `bin/` marked as built from a commit.
pub fn mark_built(release: &Path, commit: &str) {
  std::fs::create_dir_all(release.join("bin")).unwrap();
  std::fs::write(release.join("bin/built-from"), format!("{commit}\n")).unwrap();
}

/// A sleeping child of the test, killed when dropped.
pub struct Sleeper(pub std::process::Child);

impl Drop for Sleeper {
  fn drop(&mut self) {
    let _ = self.0.kill();
    let _ = self.0.wait();
  }
}

/// A sleeper standing in a directory, its `BMO_DEPLOY_DIR` naming a tree or nothing.
pub fn sleeper(cwd: &Path, named: Option<&Path>) -> Sleeper {
  let mut command = Command::new("sleep");

  command.arg("60").current_dir(cwd).env_remove(DEPLOY_VARIABLE);

  if let Some(named) = named {
    command.env(DEPLOY_VARIABLE, named);
  }

  Sleeper(command.spawn().unwrap())
}
