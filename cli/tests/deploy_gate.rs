//! The tree a deployment's gate reads (docs/deploy.md, 「What the deploy run does, in order」): the
//! declared setup runs there before the static check and the suite, so a fresh checkout whose check needs
//! installed packages passes, and a check that fails for its own reason still blocks the deployment.

use std::path::PathBuf;

use crate::stand::{Ran, git, install, ran, testbed};

// A declaration whose one check runs the package its setup installs, ending 0.
const PASSING: &str = r#"[test]
setup = ["mkdir -p node_modules/.bin && ln -sf /bin/true node_modules/.bin/dep-check"]

[[check.step]]
id = "needs-packages"
command = "node_modules/.bin/dep-check"
"#;

// The same, its installed package ending 1: a check that fails of its own account.
const FAILING: &str = r#"[test]
setup = ["mkdir -p node_modules/.bin && ln -sf /bin/false node_modules/.bin/dep-check"]

[[check.step]]
id = "needs-packages"
command = "node_modules/.bin/dep-check"
"#;

// A repository with a remote, its declaration deploying into the scene's own places.
struct Scene {
  dir: tempfile::TempDir,
  repo: PathBuf,
  data: PathBuf,
}

impl Scene {
  fn new(body: &str) -> Self {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let repo = dir.path().join("repo");
    let data = dir.path().join("testbed-data");

    git(dir.path(), &["init", "-q", "--bare", "-b", "main", &origin.to_string_lossy()]);
    git(dir.path(), &["clone", "-q", &origin.to_string_lossy(), &repo.to_string_lossy()]);
    std::fs::create_dir_all(dir.path().join("runtime")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
      repo.join("testbed.toml"),
      format!(
        r#"testbed = 0
name = "sample"

{body}

[deploy]
root = "{live}"
data_root = "{runtime}"
"#,
        live = dir.path().join("live").display(),
        runtime = dir.path().join("runtime").display(),
      ),
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    git(&repo, &["push", "-q", "origin", "HEAD:main"]);

    Self { dir, repo, data }
  }

  // The deployment run inside a unit of its own, with a service manager that says every command passed.
  fn run(&self) -> Ran {
    let bin = self.dir.path().join("bin");

    std::fs::create_dir_all(&bin).unwrap();
    install("#!/bin/sh\nexit 0\n", &bin.join("systemctl"));

    let mut command = testbed(&self.repo, &self.data, &["deploy"]);

    command
      .env("BMO_DEPLOY_UNIT", "bmo-deploy-test")
      .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
      .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));

    ran(&mut command)
  }

  fn deployed(&self) -> bool {
    std::fs::canonicalize(self.dir.path().join("live/deploy")).is_ok()
  }
}

// "a fresh checkout is set up before the gate: a check that needs its installed packages passes and the
// commit goes live"
#[test]
fn the_gate_reads_a_tree_its_declared_setup_has_set_up() {
  let scene = Scene::new(PASSING);

  assert!(!scene.repo.join("node_modules").exists());

  let done = scene.run();

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(scene.repo.join("node_modules/.bin/dep-check").exists(), "{}", done.err);
  assert!(scene.deployed(), "{}", done.err);
}

// "a check that genuinely fails still blocks the deployment: nothing goes live"
#[test]
fn a_check_that_fails_still_blocks_the_deployment() {
  let scene = Scene::new(FAILING);
  let done = scene.run();

  assert_eq!(done.code, 1, "{}\n{}", done.out, done.err);
  assert!(done.err.contains("needs-packages"), "{}", done.err);
  assert!(done.err.contains("exited 1"), "{}", done.err);
  assert!(!scene.deployed(), "{}", done.err);
}
