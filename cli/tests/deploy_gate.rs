//! The tree a deployment's gate reads (docs/deploy.md, 「What the deploy run does, in order」): the
//! declared setup runs there before the static check and the suite, so a fresh checkout whose check needs
//! installed packages passes, and a check that fails for its own reason still blocks the deployment; and
//! the suite a passed run of the whole suite on the commit already stands for is left out. A release carrying
//! the testbed binary, once live, brings the placed machine's testbed to it over that machine's `ssh`, and a
//! machine out of reach leaves the deployment live with a warning naming it and what it answered.

use std::path::{Path, PathBuf};

use bmo_testbed_client::{TestRun, TestRunKind, TestRunState, TestRunsQuery};
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::queue::now_ms;
use bmo_testbed_record::{Record, TestRunEnd};

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
    self.run_with(&["deploy"], &[])
  }

  // The same, with the words and the variables given.
  fn run_with(&self, args: &[&str], variables: &[(&str, &str)]) -> Ran {
    let bin = self.dir.path().join("bin");

    std::fs::create_dir_all(&bin).unwrap();
    install("#!/bin/sh\nexit 0\n", &bin.join("systemctl"));

    let mut command = testbed(&self.repo, &self.data, args);

    command
      .env("BMO_DEPLOY_UNIT", "bmo-deploy-test")
      .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
      .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()))
      .envs(variables.iter().copied());

    ran(&mut command)
  }

  fn deployed(&self) -> bool {
    std::fs::canonicalize(self.dir.path().join("live/deploy")).is_ok()
  }

  // A passed run of the repository's whole suite on its HEAD written into the record, from a clean tree or
  // not; its id.
  fn passed_before(&self, clean: bool) -> i64 {
    let mut record = Record::open(&self.data).unwrap();
    let started_at = now_ms() - 60_000;
    let begun = record
      .begin_test_run(&TestRun {
        id: 0,
        kind: TestRunKind::Suite,
        checkout: "/a/close/checkout".to_owned(),
        repository_name: Some("sample".to_owned()),
        commit: Some(git(&self.repo, &["rev-parse", "HEAD"])),
        whole: Some(true),
        clean: Some(clean),
        command: "bmo-testbed suite".to_owned(),
        scope: None,
        unit_name: None,
        pid: i64::from(std::process::id()),
        state: TestRunState::Running,
        passed_count: 0,
        failed_count: 0,
        skipped_count: 0,
        log_path: "/logs/close-suite.log".to_owned(),
        started_at,
        ended_at: None,
      })
      .unwrap();
    let end = TestRunEnd {
      state: TestRunState::Passed,
      passed_count: 5,
      failed_count: 0,
      skipped_count: 0,
      ended_at: started_at + 1000,
    };

    record.end_test_run(begun.id, &end).unwrap();
    begun.id
  }

  // How many runs of the suite the record holds.
  fn suite_runs(&self) -> usize {
    let query = TestRunsQuery::default();

    Record::open(&self.data)
      .unwrap()
      .test_runs(&query)
      .unwrap()
      .items
      .iter()
      .filter(|run| run.kind == TestRunKind::Suite)
      .count()
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
  assert!(done.err.contains(", exit 1"), "{}", done.err);
  assert!(!scene.deployed(), "{}", done.err);
}

// "a passed run of the whole suite on the commit from a clean tree leaves the gate's suite out: the static
// check runs, the line names the run, and no suite runs"
#[test]
fn a_passed_whole_suite_on_the_commit_leaves_the_gate_suite_out() {
  let scene = Scene::new(PASSING);
  let id = scene.passed_before(true);
  let done = scene.run();

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(done.err.contains("needs-packages"), "the static check runs: {}", done.err);
  // The deploy run's own lines stand on standard output.
  assert!(
    done
      .out
      .contains(&format!("the suite is left out: test run {id} passed the whole suite")),
    "{}",
    done.out
  );
  assert!(done.out.contains("its log is /logs/close-suite.log"), "{}", done.out);
  assert_eq!(scene.suite_runs(), 1, "{}", done.err);
  assert!(scene.deployed(), "{}", done.err);
}

// "a passed run from a tree with uncommitted changes to tracked files leaves the gate's suite in"
#[test]
fn a_passed_whole_suite_from_a_dirty_tree_leaves_the_gate_suite_in() {
  let scene = Scene::new(PASSING);

  scene.passed_before(false);

  let done = scene.run();

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(!done.out.contains("the suite is left out"), "{}", done.out);
  assert_eq!(scene.suite_runs(), 2, "the gate ran its own suite: {}", done.err);
}

// The release carries the testbed binary: committed as `bin/bmo-testbed`, so the release's `bin/` holds it beside
// the `built-from` the deployment writes. The placement settings name a machine whose `ssh` is the stand-in the
// scene's directory gives, the testbed there at its default place under that machine's home.
fn carrying_the_testbed(ssh: impl Fn(&Path) -> String) -> Scene {
  let scene = Scene::new("");
  let far_ssh = scene.dir.path().join("far-ssh");

  std::fs::create_dir_all(scene.repo.join("bin")).unwrap();
  std::fs::write(scene.repo.join("bin/bmo-testbed"), "#!/bin/sh\necho the released testbed\n").unwrap();
  git(&scene.repo, &["add", "."]);
  git(&scene.repo, &["commit", "-q", "-m", "carry the testbed"]);
  git(&scene.repo, &["push", "-q", "origin", "HEAD:main"]);
  install(&ssh(scene.dir.path()), &far_ssh);
  std::fs::create_dir_all(scene.data.join("env")).unwrap();
  std::fs::write(
    scene.data.join("env/placement.toml"),
    format!(
      "machine = \"machine-2\"\ndestination = \"stand\"\nssh = [\"{ssh}\"]\nbench = \"{bench}\"\n\
       deploy_bench = \"{bench}\"\ncargo_target = \"{bench}\"\n",
      ssh = far_ssh.display(),
      bench = scene.dir.path().join("bench").display(),
    ),
  )
  .unwrap();

  scene
}

// A stand-in `ssh` whose far side is a home of its own under the directory given: past the destination, the line
// runs there.
fn far_home_ssh(dir: &Path) -> String {
  format!(
    "#!/bin/sh\nshift\nexport HOME='{}'\nexec sh -c \"$*\"\n",
    dir.join("far-home").display()
  )
}

// Where the testbed binary stands on the stand-in machine.
fn far_bin(scene: &Scene) -> PathBuf {
  scene.dir.path().join("far-home/.local/share/bmo-testbed/deploy/bin")
}

// "once it is live ... copies the live release's binary of that name onto the settings' `testbed` and the
// release's `bin/built-from` beside it ... then reads that `built-from` back ... The deployment's line names the
// machine, the files written there and the commit read back"
#[test]
fn a_live_release_carrying_the_testbed_brings_the_placed_machine_s_testbed_to_its_commit() {
  let scene = carrying_the_testbed(far_home_ssh);
  let done = scene.run_with(&["deploy", "--no-check"], &[]);
  let head = git(&scene.repo, &["rev-parse", "HEAD"]);
  let brought = fill_marks(
    &DEPLOY.run.testbed_brought,
    &[
      ("machine", "machine-2"),
      (
        "files",
        "~/.local/share/bmo-testbed/deploy/bin/bmo-testbed, ~/.local/share/bmo-testbed/deploy/bin/built-from",
      ),
      ("commit", &head[..12]),
    ],
  );

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(scene.deployed(), "{}", done.err);
  assert_eq!(
    std::fs::read(far_bin(&scene).join("bmo-testbed")).unwrap(),
    std::fs::read(scene.dir.path().join("live/deploy/bin/bmo-testbed")).unwrap()
  );
  assert_eq!(std::fs::read_to_string(far_bin(&scene).join("built-from")).unwrap().trim(), head);
  assert!(done.out.contains(&brought), "{}\n{}", done.out, done.err);
}

// "A machine that cannot be reached ... is a warning on a line naming the machine and what it answered: the
// deployment here stays live"
#[test]
fn a_placed_machine_out_of_reach_leaves_the_deployment_live_with_a_line_naming_it() {
  let answered = "ssh: connect to host stand port 22: No route to host";
  let scene = carrying_the_testbed(|_| format!("#!/bin/sh\necho '{answered}' >&2\nexit 255\n"));
  let done = scene.run_with(&["deploy", "--no-check"], &[]);
  let head = git(&scene.repo, &["rev-parse", "HEAD"]);
  let unbrought = fill_marks(
    &DEPLOY.run.testbed_unbrought,
    &[
      ("machine", "machine-2"),
      ("commit", &head[..12]),
      ("said", &fill_marks(&LANES.placed.testbed_unreached, &[("said", answered)])),
    ],
  );

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(scene.deployed(), "{}", done.err);
  assert!(done.out.contains(&unbrought), "{}\n{}", done.out, done.err);
}

// "Nothing is copied where ... the deployment's caller sets `BMO_TESTBED_PLACEMENT` empty"
#[test]
fn a_placement_set_empty_leaves_the_placed_machine_s_testbed_as_it_stands() {
  let scene = carrying_the_testbed(far_home_ssh);
  let done = scene.run_with(&["deploy", "--no-check"], &[("BMO_TESTBED_PLACEMENT", "")]);

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(scene.deployed(), "{}", done.err);
  assert!(!far_bin(&scene).exists(), "{}", done.out);
}
