//! The head of a repository's main branch (docs/declaration.md, 「The main branch's head」): the branch as the
//! repository's remote holds it, as far as this machine knows it, since a job's close pushes onto the remote's
//! main and leaves the local branch of that name where it stood.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::policy::fill_marks;
use crate::policy::main_branch::MAIN_BRANCH;

// The variables a git hook hands its commands, which would point git at another repository than the one named.
const GIT_PLACE_VARIABLES: [&str; 3] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"];

/// One repository's main branch, by the name its declaration gives it.
pub struct MainBranch<'a> {
  repository: &'a Path,
  name: &'a str,
}

impl<'a> MainBranch<'a> {
  pub fn new(repository: &'a Path, name: &'a str) -> Self {
    Self { repository, name }
  }

  /// The remote's main branch fetched into its remote-tracking branch, with no prompt for credentials and a
  /// transfer that stalls ended. A remote that cannot be asked leaves the remote-tracking branch as it was,
  /// which the head is then read from; what git said where it could not.
  pub fn fetch(&self) -> Result<(), String> {
    let Some(remote) = self.remote() else {
      return Ok(());
    };
    let stall = MAIN_BRANCH.stall_seconds.to_string();
    let refspec = format!("+refs/heads/{}:{}", self.name, self.tracking(&remote));
    let fetched = self
      .git(&[
        "-c",
        "http.lowSpeedLimit=1",
        "-c",
        &format!("http.lowSpeedTime={stall}"),
        "fetch",
        "--quiet",
        "--no-tags",
        "--no-write-fetch-head",
        &remote,
        &refspec,
      ])
      .env("GIT_TERMINAL_PROMPT", "0")
      .output()
      .map_err(|why| why.to_string())?;

    if fetched.status.success() {
      Ok(())
    } else {
      Err(String::from_utf8_lossy(&fetched.stderr).trim().to_owned())
    }
  }

  /// The head: of the local branch and the remote-tracking branch, the one that holds the other, or the one
  /// that stands. The refusal saying why there is none, where neither stands or the two have parted.
  pub fn head(&self) -> Result<String, String> {
    let local = self.commit(&format!("refs/heads/{}", self.name));
    let tracking = self.remote().map(|remote| self.tracking(&remote));
    let remote = tracking.as_deref().and_then(|tracking| self.commit(tracking));

    match (local, remote) {
      (None, None) => Err(fill_marks(&MAIN_BRANCH.absent, &[("main", self.name)])),
      (Some(head), None) | (None, Some(head)) => Ok(head),
      (Some(local), Some(remote)) if self.holds(&remote, &local) => Ok(remote),
      (Some(local), Some(remote)) if self.holds(&local, &remote) => Ok(local),
      (Some(local), Some(remote)) => Err(fill_marks(
        &MAIN_BRANCH.parted,
        &[
          ("main", self.name),
          ("tracking", tracking.as_deref().unwrap_or_default()),
          ("local", &local),
          ("remote", &remote),
        ],
      )),
    }
  }

  // The remote the branch follows: the one its `remote` setting names, else the policy's, where the repository
  // has a remote of that name; none where it has not.
  fn remote(&self) -> Option<String> {
    let named = self.read(&["config", "--get", &format!("branch.{}.remote", self.name)]);
    let remote = named.filter(|remote| remote != ".").unwrap_or_else(|| MAIN_BRANCH.remote.clone());

    self.read(&["remote", "get-url", &remote]).map(|_| remote)
  }

  // The remote-tracking branch of the main branch under `remote`.
  fn tracking(&self, remote: &str) -> String {
    format!("refs/remotes/{remote}/{}", self.name)
  }

  // The commit a ref names, none where it names none.
  fn commit(&self, reference: &str) -> Option<String> {
    self.read(&["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")])
  }

  // Whether `later` holds `earlier`: the same commit, or one `earlier` is an ancestor of.
  fn holds(&self, later: &str, earlier: &str) -> bool {
    later == earlier
      || self
        .git(&["merge-base", "--is-ancestor", earlier, later])
        .output()
        .is_ok_and(|done| done.status.success())
  }

  // What a git command prints, trimmed; none where it fails or prints nothing.
  fn read(&self, args: &[&str]) -> Option<String> {
    let done = self.git(args).output().ok().filter(|done| done.status.success())?;
    let out = String::from_utf8_lossy(&done.stdout).trim().to_owned();

    (!out.is_empty()).then_some(out)
  }

  // Git run in the repository, reading nothing from standard input.
  fn git(&self, args: &[&str]) -> Command {
    let mut command = Command::new("git");

    for variable in GIT_PLACE_VARIABLES {
      command.env_remove(variable);
    }

    command.args(args).current_dir(self.repository).stdin(Stdio::null());
    command
  }
}
