//! The deploy run a declaration drives (docs/deploy.md, 「What the deploy run does, in order」), over a
//! repository with a remote of its own in temporary directories: the gate and the service manager stand-ins,
//! every other command real. A commit goes live with its `bot` service restarted and watched; what may not
//! be deployed is refused with nothing changed; a gate that fails puts nothing live; and a service that does
//! not come up is rolled back onto the release before.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_client::{DeploymentState, PageQuery, RestartsQuery};
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::Hook;
use bmo_testbed_core::turn::Env;
use bmo_testbed_deploy::deployment::{
  DeployAsk, DeployHands, DeployPlaces, Gate, GateAsk, GateOutcome, Machine, StepAsk, StepRan, deploy,
  release_build, workspace_binaries,
};
use bmo_testbed_deploy::progress::{Outcome, read_deploy_progress};
use bmo_testbed_deploy::release::{LiveProcess, release_dir};
use bmo_testbed_deploy::run::{Ran, run_quiet};
use bmo_testbed_deploy::tree::read_deploy_record;
use bmo_testbed_record::Record;
use serde_json::Value;

use crate::stand::must;

// The service manager's stand-in, which says every unit runs, and the machine's own hands for the rest.
#[derive(Default)]
struct Hands {
  systemctl: RefCell<Vec<String>>,
  said: RefCell<Vec<String>>,
  /// Whether a restart of the `bot` service is refused.
  refuse_restart: bool,
}

impl DeployHands for Hands {
  fn run(&self, line: &[String], cwd: &Path) -> Ran {
    match line[0].as_str() {
      "systemctl" => {
        let asked = line[2..].join(" ");
        let refused = self.refuse_restart && asked.starts_with("restart ");

        self.systemctl.borrow_mut().push(asked.clone());

        Ran {
          ok: !refused,
          out: if asked.starts_with("show ") {
            "ActiveState=active\nSubState=running\nMainPID=42\nNRestarts=0\nExecMainStatus=0".to_owned()
          } else {
            String::new()
          },
        }
      }
      "journalctl" | "systemd-run" => {
        self.systemctl.borrow_mut().push(line.join(" "));
        Ran {
          ok: true,
          out: String::new(),
        }
      }
      _ => run_quiet(line, Some(cwd)),
    }
  }

  fn step(&self, ask: &StepAsk) -> StepRan {
    Machine.step(ask)
  }

  fn say(&self, line: &str) {
    self.said.borrow_mut().push(line.to_owned());
  }

  fn now_ms(&self) -> i64 {
    Machine.now_ms()
  }

  fn live_processes(&self) -> Option<Vec<LiveProcess>> {
    Some(Vec::new())
  }
}

// A gate that passes or fails as it is told.
struct Told(bool);

impl Gate for Told {
  fn pass(&self, _: &GateAsk<'_>) -> GateOutcome {
    GateOutcome {
      passed: self.0,
      reason: if self.0 { String::new() } else { "the suite failed: a.test.ts".to_owned() },
      flaky: if self.0 { vec![("a test".to_owned(), "a.test.ts".to_owned())] } else { Vec::new() },
      steps: Vec::new(),
    }
  }
}

// A repository with a remote, its declaration deploying a binary its build step makes and a `bot` service,
// with every place under one temporary directory.
struct Scene {
  dir: tempfile::TempDir,
  source: PathBuf,
}

impl Scene {
  fn new() -> Self {
    let scene = Self::empty();

    scene.write(
      "units/app.service",
      "[Service]\nExecStart=%h/live/deploy/bin/app\n\n[Install]\nX-BMO-Deploy=bot\n",
    );
    scene.declare("v1", "true");
    scene.push("first");
    scene
  }

  // A repository with a remote and a data root, nothing in it yet.
  fn empty() -> Self {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let source = dir.path().join("source");

    must(&["init", "--quiet", "--bare", "-b", "main", &origin.to_string_lossy()], dir.path());
    must(&["clone", "--quiet", &origin.to_string_lossy(), &source.to_string_lossy()], dir.path());
    std::fs::create_dir_all(dir.path().join("data")).unwrap();

    Self { dir, source }
  }

  fn at(&self, path: &str) -> PathBuf {
    self.dir.path().join(path)
  }

  fn write(&self, path: &str, text: &str) {
    let at = self.source.join(path);

    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, text).unwrap();
  }

  // The declaration, its build making a binary that says `version`, its health the command given.
  fn declare(&self, version: &str, health: &str) {
    self.write(
      "testbed.toml",
      &format!(
        r#"testbed = 0
name = "sample"

[build]
binaries = ["app"]

[[build.step]]
id = "make-app"
command = "mkdir -p target/release && printf 'echo {version}\n' > target/release/app"

[deploy]
root = "{live}"
data_root = "{data}"
health = "{health}"
health_wait_seconds = 0
stay_up_seconds = 0

[[deploy.step]]
id = "before"
command = "test -f bin/app"
phase = "before-flip"

[[deploy.step]]
id = "after"
command = "touch {after}"
phase = "after-health"

[deploy.backup]
"#,
        live = self.at("live").display(),
        data = self.at("data").display(),
        after = self.at("after-ran").display(),
      ),
    );
  }

  // A repository whose one Cargo workspace names a base library outside it, the checkout carrying the
  // library and nothing under the deploy root standing where either tree will read it.
  fn over_an_outside_tree() -> Self {
    let scene = Self::empty();

    scene.write(
      "Cargo.toml",
      "[workspace]\nresolver = \"2\"\nmembers = [\"app\"]\n\n[workspace.dependencies]\nbase-lib = { path = \"../base/text\" }\n",
    );
    scene.write(
      "app/Cargo.toml",
      "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nbase-lib = { workspace = true }\n",
    );
    scene.write("app/src/main.rs", "fn main() {}\n");
    scene.write(
      "units/app.service",
      "[Service]\nExecStart=%h/live/deploy/bin/app\n\n[Install]\nX-BMO-Deploy=bot\n",
    );
    scene.write(
      "testbed.toml",
      &format!(
        r#"testbed = 0
name = "sample"

[deploy]
root = "{live}"
data_root = "{data}"
health = "true"
health_wait_seconds = 0
stay_up_seconds = 0

[deploy.backup]
"#,
        live = scene.at("live").display(),
        data = scene.at("data").display(),
      ),
    );

    let base = scene.at("base/text");

    std::fs::create_dir_all(base.join("src")).unwrap();
    std::fs::write(
      base.join("Cargo.toml"),
      "[package]\nname = \"base-lib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(base.join("src/lib.rs"), "").unwrap();

    let lock = std::process::Command::new("cargo")
      .args(["generate-lockfile", "--offline"])
      .current_dir(&scene.source)
      .output()
      .unwrap();

    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));

    scene.push("first");
    scene
  }

  fn push(&self, message: &str) -> String {
    let git = |args: &[&str]| must(args, &self.source);

    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", message]);
    git(&["push", "--quiet", "origin", "HEAD:main"]);
    git(&["rev-parse", "HEAD"])
  }

  fn env(&self) -> Env {
    Env::from([
      ("HOME".to_owned(), self.at("home").to_string_lossy().into_owned()),
      ("XDG_CONFIG_HOME".to_owned(), self.at("config").to_string_lossy().into_owned()),
    ])
  }

  fn hook(&self) -> Hook {
    let events = self.at("events");

    Hook {
      command: Some(format!("f() {{ cat >> {0}; echo >> {0}; }}; f", events.display())),
      timeout: Duration::from_secs(30),
      repository: Some("sample".to_owned()),
      root: Some(self.source.clone()),
    }
  }

  // The events the hook was handed, in order.
  fn events(&self) -> Vec<Value> {
    std::fs::read_to_string(self.at("events"))
      .unwrap_or_default()
      .lines()
      .filter(|line| !line.trim().is_empty())
      .map(|line| serde_json::from_str(line).unwrap())
      .collect()
  }

  fn names(&self) -> Vec<String> {
    self
      .events()
      .iter()
      .map(|event| event["event"].as_str().unwrap_or_default().to_owned())
      .collect()
  }

  fn run_with(&self, ask: &DeployAsk, gate: &dyn Gate, hands: &Hands, env: &Env) -> u8 {
    let repo = Repository::read(&self.source).unwrap();
    let places = DeployPlaces::of(&repo, env).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    runtime.block_on(deploy(&repo, &places, ask, env, gate, hands, &self.hook()))
  }

  fn run(&self, hands: &Hands) -> u8 {
    self.run_with(&DeployAsk::default(), &Told(true), hands, &self.env())
  }

  fn deploy_dir(&self) -> PathBuf {
    self.at("live/deploy")
  }

  fn unit_link(&self) -> PathBuf {
    self.at("config/systemd/user/app.service")
  }
}

fn said(hands: &Hands) -> String {
  hands.said.borrow().join("\n")
}

fn symlink(at: &Path) -> Option<PathBuf> {
  std::fs::symlink_metadata(at)
    .ok()
    .filter(|meta| meta.file_type().is_symlink())
    .and_then(|_| std::fs::read_link(at).ok())
}

// "a commit goes live: built into a release of its own, flipped onto, its bot restarted and watched"
#[test]
fn a_commit_goes_live_with_its_bot_restarted_and_watched() {
  let scene = Scene::new();
  let hands = Hands::default();
  let head = must(&["rev-parse", "HEAD"], &scene.source);
  let recorded = DeployAsk {
    record: Some(scene.at("testbed-data")),
    ..DeployAsk::default()
  };
  let code = scene.run_with(&recorded, &Told(true), &hands, &scene.env());
  let release = release_dir(&scene.deploy_dir(), &head);

  assert_eq!(code, 0, "{}", said(&hands));
  assert_eq!(
    std::fs::canonicalize(scene.deploy_dir()).unwrap(),
    std::fs::canonicalize(&release).unwrap()
  );
  assert_eq!(std::fs::read_to_string(release.join("bin/app")).unwrap(), "echo v1\n");
  assert_eq!(std::fs::read_to_string(release.join("bin/built-from")).unwrap().trim(), head);
  let written = read_deploy_record(&scene.at("live/deployed.json")).unwrap();

  assert_eq!(written.commit, head);
  // No reason was named, so the record's message is the commit's subject, and the commit carries no
  // VERSION file.
  assert_eq!(written.message, "first");
  assert_eq!(written.version, "");
  assert_eq!(
    std::fs::read_link(scene.unit_link()).unwrap(),
    scene.deploy_dir().join("units/app.service")
  );
  assert!(scene.at("after-ran").exists(), "{}", said(&hands));

  let systemctl = hands.systemctl.borrow();

  for asked in ["daemon-reload", "enable app", "reset-failed app", "restart app"] {
    assert!(systemctl.iter().any(|one| one == asked), "{asked}: {systemctl:?}");
  }

  let backups: Vec<_> = std::fs::read_dir(scene.at("live/backups")).unwrap().flatten().collect();

  assert_eq!(backups.len(), 1);
  assert!(backups[0].file_name().to_string_lossy().starts_with("sample-"));

  let events = scene.events();

  assert_eq!(scene.names(), ["deployed"]);
  assert_eq!(events[0]["detail"]["message"], "first");
  assert_eq!(
    events[0]["detail"]["flaky"],
    serde_json::json!([{ "test": "a test", "unit": "a.test.ts" }])
  );

  let progress =
    read_deploy_progress(std::fs::read_to_string(scene.at("live/deploy-progress.json")).ok().as_deref());

  assert_eq!(progress.unwrap().ending.unwrap().outcome, Outcome::Live);

  // The testbed's record holds the deployment as it went live, the lines it said, and why it restarted its
  // bot service.
  let record = Record::open(&scene.at("testbed-data")).unwrap();
  let deployment = record.deployments(&PageQuery::default()).unwrap().items.remove(0);
  let restarts = record.restarts(&RestartsQuery::default()).unwrap().items;

  assert_eq!(
    (deployment.state, deployment.commit.as_deref()),
    (DeploymentState::Live, Some(head.as_str()))
  );
  assert!(
    !record
      .log_lines(deployment.id, &PageQuery::default())
      .unwrap()
      .unwrap()
      .items
      .is_empty()
  );
  assert!(
    restarts.iter().any(|one| one.unit_name == "app" && one.deployment_id == deployment.id),
    "{restarts:?}"
  );
}

// "the reason a deploy is made and the release's version are written into the record and the event"
#[test]
fn the_message_overrides_the_subject_and_the_version_file_is_recorded() {
  let scene = Scene::new();
  let hands = Hands::default();

  scene.write("VERSION", "3.4.0\n");
  let head = scene.push("a later commit subject");

  let ask = DeployAsk {
    record: Some(scene.at("testbed-data")),
    message: Some("hotfix for the placement bug".to_owned()),
    ..DeployAsk::default()
  };
  let code = scene.run_with(&ask, &Told(true), &hands, &scene.env());

  assert_eq!(code, 0, "{}", said(&hands));

  let written = read_deploy_record(&scene.at("live/deployed.json")).unwrap();

  assert_eq!(written.commit, head);
  assert_eq!(written.subject, "a later commit subject");
  assert_eq!(written.message, "hotfix for the placement bug");
  assert_eq!(written.version, "3.4.0");

  let events = scene.events();

  assert_eq!(events.last().unwrap()["detail"]["message"], "hotfix for the placement bug");
}

// "a first deployment lays the base-library tree beside the release and the build tree, from the checkout"
#[test]
fn a_first_deployment_lays_the_trees_outside_the_workspace_from_the_checkout() {
  let scene = Scene::over_an_outside_tree();
  let hands = Hands::default();
  let code = scene.run(&hands);
  let library = std::fs::canonicalize(scene.at("base/text")).unwrap();

  assert_eq!(code, 0, "{}", said(&hands));
  assert_eq!(
    symlink(&scene.at("live/deploy-releases/base/text")),
    Some(library.clone()),
    "the tree the release names outside it stands as a link from the checkout: {}",
    said(&hands)
  );
  assert_eq!(
    symlink(&scene.at("live/base/text")),
    Some(library),
    "the tree the build tree names outside it stands as a link from the checkout: {}",
    said(&hands)
  );
  assert!(scene.at("live/deploy/bin/app").exists(), "{}", said(&hands));
}

// "the records stand under the deploy root, and a data root another deployment records into is left alone"
#[test]
fn the_records_stand_under_the_deploy_root_and_leave_a_shared_data_root_alone() {
  let scene = Scene::new();
  let elsewhere = "{\n  \"commit\": \"another\",\n  \"subject\": \"another deployment\"\n}\n";
  let elsewhere_log = "2026-10-20T00:00:00.000Z another backup another deployment\n";

  std::fs::write(scene.at("data/deployed.json"), elsewhere).unwrap();
  std::fs::write(scene.at("data/deployed.log"), elsewhere_log).unwrap();

  let hands = Hands::default();

  assert_eq!(scene.run(&hands), 0, "{}", said(&hands));

  assert_eq!(std::fs::read_to_string(scene.at("data/deployed.json")).unwrap(), elsewhere);
  assert_eq!(std::fs::read_to_string(scene.at("data/deployed.log")).unwrap(), elsewhere_log);
  assert!(!scene.at("data/deploy-progress.json").exists());

  let head = must(&["rev-parse", "HEAD"], &scene.source);

  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().commit, head);
  assert!(scene.at("live/deployed.log").exists());
  assert!(scene.at("live/deploy-progress.json").exists());
  assert!(scene.at("live/backups").is_dir());
}

// "a second commit goes live beside the first, the binary that changed placed and the first release kept"
#[test]
fn a_second_commit_goes_live_beside_the_first() {
  let scene = Scene::new();
  let first = must(&["rev-parse", "HEAD"], &scene.source);

  assert_eq!(scene.run(&Hands::default()), 0);

  scene.declare("v2", "true");

  let second = scene.push("second");
  let hands = Hands::default();

  assert_eq!(scene.run(&hands), 0, "{}", said(&hands));
  assert_eq!(std::fs::read_to_string(scene.deploy_dir().join("bin/app")).unwrap(), "echo v2\n");
  assert!(release_dir(&scene.deploy_dir(), &first).join("bin/app").exists());
  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().commit, second);
}

// "what may not be deployed is refused before anything changes"
#[test]
fn an_open_change_an_unpushed_commit_and_a_backup_inside_the_data_root_are_refused() {
  let scene = Scene::new();

  scene.write("units/app.service", "changed\n");

  let open = Hands::default();

  assert_eq!(scene.run(&open), 2);
  assert!(said(&open).contains("uncommitted"), "{}", said(&open));

  must(&["commit", "--quiet", "-am", "local"], &scene.source);

  let unpushed = Hands::default();

  assert_eq!(scene.run(&unpushed), 2);
  assert!(said(&unpushed).contains("not on"), "{}", said(&unpushed));

  must(&["push", "--quiet", "origin", "HEAD:main"], &scene.source);

  let mut env = scene.env();
  let inside = Hands::default();

  env.insert("BMO_BACKUP_DIR".to_owned(), scene.at("data/backups").to_string_lossy().into_owned());
  assert_eq!(scene.run_with(&DeployAsk::default(), &Told(true), &inside, &env), 2);
  assert!(said(&inside).contains("inside the data root"), "{}", said(&inside));

  let job = Hands::default();

  env = scene.env();
  env.insert("CLAUDE_PID".to_owned(), "4242".to_owned());
  assert_eq!(scene.run_with(&DeployAsk::default(), &Told(true), &job, &env), 2);

  assert!(!scene.deploy_dir().exists());
  assert!(scene.events().is_empty());
  assert!(read_deploy_record(&scene.at("live/deployed.json")).is_none());
}

// "the deploy tree is the declaration's alone, whatever release the caller stands in"
#[test]
fn the_deploy_tree_is_the_declarations_whatever_release_the_caller_stands_in() {
  let scene = Scene::new();
  let repo = Repository::read(&scene.source).unwrap();
  let mut env = scene.env();

  env.insert("BMO_DEPLOY_DIR".to_owned(), "/another/release".to_owned());

  assert_eq!(DeployPlaces::of(&repo, &env).unwrap().deploy_dir, scene.deploy_dir());
}

// "a dry run says what it would do and changes nothing"
#[test]
fn a_dry_run_changes_nothing() {
  let scene = Scene::new();
  let hands = Hands::default();
  let ask = DeployAsk {
    dry_run: true,
    ..DeployAsk::default()
  };

  assert_eq!(scene.run_with(&ask, &Told(true), &hands, &scene.env()), 0);
  assert!(said(&hands).contains("--dry-run"), "{}", said(&hands));
  assert!(!scene.deploy_dir().exists());
  assert!(!scene.at("live/backups").exists());
}

// "a gate that fails puts nothing live and says so to the hook"
#[test]
fn a_gate_that_fails_puts_nothing_live() {
  let scene = Scene::new();
  let hands = Hands::default();

  assert_eq!(scene.run_with(&DeployAsk::default(), &Told(false), &hands, &scene.env()), 1);
  assert!(!scene.deploy_dir().exists());
  assert!(hands.systemctl.borrow().is_empty());

  let events = scene.events();

  assert_eq!(scene.names(), ["deploy-failed"]);
  assert_eq!(events[0]["detail"]["stage"], "check");
  assert!(events[0]["detail"]["reason"].as_str().unwrap().contains("a.test.ts"));
}

// "a service that does not come up is rolled back onto the release that was live before"
#[test]
fn a_service_that_does_not_come_up_is_rolled_back() {
  let scene = Scene::new();
  let first = must(&["rev-parse", "HEAD"], &scene.source);

  assert_eq!(scene.run(&Hands::default()), 0);

  scene.declare("v2", "false");
  scene.push("second");

  let hands = Hands::default();

  assert_eq!(scene.run(&hands), 1, "{}", said(&hands));
  assert_eq!(
    std::fs::canonicalize(scene.deploy_dir()).unwrap(),
    std::fs::canonicalize(release_dir(&scene.deploy_dir(), &first)).unwrap()
  );
  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().commit, first);
  assert_eq!(scene.names()[1..], ["deploy-rolled-back", "deploy-failed"]);

  let failed = &scene.events()[2];

  assert_eq!(failed["detail"]["rolled_back"], true);
  assert_eq!(failed["detail"]["stage"], "restart");

  let progress =
    read_deploy_progress(std::fs::read_to_string(scene.at("live/deploy-progress.json")).ok().as_deref());

  let progress = progress.unwrap();

  assert_eq!(progress.ending.unwrap().outcome, Outcome::RolledBack);
  assert_eq!(
    progress.stages.last().map(|at| at.stage),
    Some(bmo_testbed_deploy::progress::Stage::Rollback)
  );
}

// "a build that fails stops the deployment with every service untouched"
#[test]
fn a_build_that_fails_flips_nothing_and_touches_no_service() {
  let scene = Scene::new();
  let declared = std::fs::read_to_string(scene.source.join("testbed.toml")).unwrap();

  scene.write("testbed.toml", &declared.replace("mkdir -p target/release &&", "exit 3 &&"));
  scene.push("broken build");

  let hands = Hands::default();

  assert_eq!(scene.run(&hands), 1, "{}", said(&hands));
  assert!(!scene.deploy_dir().exists());
  assert!(hands.systemctl.borrow().is_empty());
  assert!(read_deploy_record(&scene.at("live/deployed.json")).is_none());
  assert_eq!(scene.events()[0]["detail"]["stage"], "build");
}

// "a release carries every binary target of the Cargo workspace where the declaration names none"
#[test]
fn the_binaries_of_a_workspace_are_its_bin_targets() {
  let metadata = r#"{"packages":[
    {"targets":[{"name":"tool","kind":["bin"]},{"name":"tool","kind":["lib"]}]},
    {"targets":[{"name":"agent","kind":["bin"]},{"name":"tests","kind":["test"]},{"name":"tool","kind":["bin"]}]}
  ]}"#;

  assert_eq!(workspace_binaries(metadata).unwrap(), ["agent", "tool"]);
  assert!(workspace_binaries("warning: not json").is_err());
}

// "The Cargo release build covers every package in the workspace, not the root package alone"
#[test]
fn the_release_build_covers_every_package_of_the_workspace() {
  assert_eq!(
    release_build("release", Path::new("/build/target")),
    [
      "cargo",
      "build",
      "--workspace",
      "--locked",
      "--profile",
      "release",
      "--target-dir",
      "/build/target"
    ]
  );
}

// "a first deployment whose service will not restart is not rolled back, and fails"
#[test]
fn a_first_deployment_whose_service_will_not_restart_fails_unrolled() {
  let scene = Scene::new();
  let hands = Hands {
    refuse_restart: true,
    ..Hands::default()
  };

  assert_eq!(scene.run(&hands), 1, "{}", said(&hands));
  assert_eq!(scene.names(), ["deploy-failed"]);
  assert_eq!(scene.events()[0]["detail"]["rolled_back"], false);
}
