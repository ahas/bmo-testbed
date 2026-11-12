//! The deploy run a declaration drives (docs/deploy.md, 「What the deploy run does, in order」), over a
//! repository with a remote of its own in temporary directories: the gate and the service manager stand-ins,
//! every other command real. A commit goes live with its `bot` service restarted and watched; what may not
//! be deployed is refused with nothing changed; a gate that fails puts nothing live; and a service that does
//! not come up is rolled back onto the release before.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use bmo_testbed_client::{DeploymentState, PageQuery, RestartsQuery, TestRun, TestRunKind, TestRunState};
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::Hook;
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::turn::Env;
use bmo_testbed_deploy::deployment::{
  BuildAsk, DeployAsk, DeployHands, DeployPlaces, Gate, GateAsk, GateOutcome, Machine, StepAsk, StepRan,
  SuitePassed, deploy, major_change, release_build, version_major, workspace_binaries,
};
use bmo_testbed_deploy::progress::{Outcome, read_deploy_progress};
use bmo_testbed_deploy::release::{LiveProcess, release_dir};
use bmo_testbed_deploy::run::{Ran, run_quiet};
use bmo_testbed_deploy::tree::read_deploy_record;
use bmo_testbed_record::{Record, TestRunEnd};
use serde_json::Value;

use crate::stand::must;

// The service manager's stand-in, which says every unit runs, and the machine's own hands for the rest.
#[derive(Default)]
struct Hands {
  systemctl: RefCell<Vec<String>>,
  said: RefCell<Vec<String>>,
  /// Whether a restart of the `bot` service is refused.
  refuse_restart: bool,
  /// Whether systemd holds the `bot` service as one that gave up starting.
  gives_up: bool,
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
          code: Some(i32::from(refused)),
          out: if asked.starts_with("show ") && self.gives_up {
            "ActiveState=failed\nSubState=failed\nMainPID=0\nNRestarts=0\nExecMainStatus=1".to_owned()
          } else if asked.starts_with("show ") {
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
          code: Some(0),
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

// A gate that counts the dependent runs it is asked for.
#[derive(Default)]
struct Counted(RefCell<usize>);

impl Gate for Counted {
  fn pass(&self, _: &GateAsk<'_>) -> GateOutcome {
    GateOutcome {
      passed: true,
      ..GateOutcome::default()
    }
  }

  fn dependents(&self, _: &GateAsk<'_>) -> GateOutcome {
    *self.0.borrow_mut() += 1;
    GateOutcome {
      passed: true,
      ..GateOutcome::default()
    }
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
      suite_passed: None,
    }
  }
}

// A gate that passes, keeping whether each ask ran its suite, and answering the passed run it is handed as
// one its own machine's record held.
#[derive(Default)]
struct Asked {
  suites: RefCell<Vec<bool>>,
  passed_there: Option<SuitePassed>,
}

impl Gate for Asked {
  fn pass(&self, ask: &GateAsk<'_>) -> GateOutcome {
    self.suites.borrow_mut().push(ask.suite);
    GateOutcome {
      passed: true,
      suite_passed: self.passed_there.clone(),
      ..GateOutcome::default()
    }
  }
}

// A gate whose machine builds elsewhere: it passes, and the Cargo release build it is asked puts a binary
// of its own into the directory asked, keeping what it was asked.
#[derive(Default)]
struct BuiltElsewhere(RefCell<Vec<(String, String, Vec<String>)>>);

impl Gate for BuiltElsewhere {
  fn pass(&self, _: &GateAsk<'_>) -> GateOutcome {
    GateOutcome {
      passed: true,
      ..GateOutcome::default()
    }
  }

  fn build(&self, ask: &BuildAsk<'_>) -> Option<Result<(), String>> {
    self
      .0
      .borrow_mut()
      .push((ask.commit.to_owned(), ask.profile.to_owned(), ask.binaries.to_vec()));
    std::fs::create_dir_all(ask.built).unwrap();
    std::fs::write(ask.built.join("app"), "built elsewhere").unwrap();
    Some(Ok(()))
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

  // The hook writes down each event it is handed, and whether the progress record held the deployment's
  // ending as it ran.
  fn hook(&self) -> Hook {
    let events = self.at("events");
    let endings = self.at("endings");
    let progress = self.at("live/deploy-progress.json");

    Hook {
      command: Some(format!(
        "f() {{ cat >> {0}; echo >> {0}; if grep -q '\"outcome\"' {2}; then echo \"$1 ended\"; else echo \"$1 running\"; fi >> {1}; }}; f",
        events.display(),
        endings.display(),
        progress.display()
      )),
      time: std::time::Duration::from_secs(60),
      repository: Some("sample".to_owned()),
      root: Some(self.source.clone()),
    }
  }

  // For each event the hook was handed, in order, its name and whether the deployment had ended by then.
  fn endings(&self) -> Vec<String> {
    std::fs::read_to_string(self.at("endings"))
      .unwrap_or_default()
      .lines()
      .map(str::to_owned)
      .collect()
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

  // A deployment on a clock that moves only while it waits, so the wait for quiet costs no time.
  fn run_paused(&self, ask: &DeployAsk, hands: &Hands) -> u8 {
    let repo = Repository::read(&self.source).unwrap();
    let places = DeployPlaces::of(&repo, &self.env()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .start_paused(true)
      .build()
      .unwrap();

    runtime.block_on(deploy(&repo, &places, ask, &self.env(), &Told(true), hands, &self.hook()))
  }

  // The declaration's `quiet`, a script that answers each ask with the next of the codes given, one a line, and 0
  // once they run out: 1 naming a turn under way, any other code saying it cannot read its store. The asks are
  // counted under `quiet-asks`.
  fn declare_quiet(&self, codes: &[i32]) {
    let script = self.at("quiet.sh");
    let codes_at = self.at("quiet-codes");
    let asks = self.at("quiet-asks");
    // A declaration that already names a quiet has it named afresh.
    let declared = std::fs::read_to_string(self.source.join("testbed.toml"))
      .unwrap()
      .lines()
      .filter(|line| !line.starts_with("quiet = "))
      .map(|line| format!("{line}\n"))
      .collect::<String>();

    std::fs::write(&codes_at, codes.iter().map(|code| format!("{code}\n")).collect::<String>()).unwrap();
    std::fs::write(
      &script,
      format!(
        "n=$(( $(cat {asks} 2>/dev/null || echo 0) + 1 ))\necho $n > {asks}\ncode=$(sed -n \"${{n}}p\" {codes})\ncase \"$code\" in\n  1) echo 'a turn under way' ;;\n  ''|0) ;;\n  *) echo 'relation \"slots\" does not exist' >&2 ;;\nesac\nexit \"${{code:-0}}\"\n",
        asks = asks.display(),
        codes = codes_at.display(),
      ),
    )
    .unwrap();
    self.write(
      "testbed.toml",
      &declared.replace(
        "\n[[deploy.step]]\nid = \"before\"",
        &format!("quiet = \"sh {}\"\n\n[[deploy.step]]\nid = \"before\"", script.display()),
      ),
    );
  }

  // How many times the declared `quiet` was asked.
  fn quiet_asks(&self) -> usize {
    std::fs::read_to_string(self.at("quiet-asks")).map_or(0, |asks| asks.trim().parse().unwrap())
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
  // The hook heard of the deployment only once its ending was written.
  assert_eq!(scene.endings(), ["deployed ended"]);

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

// "the Cargo release build goes to the gate's machine where it builds elsewhere, and the binaries it brings
// back are placed"
#[test]
fn a_release_build_the_gate_makes_elsewhere_is_placed_from_what_it_brought_back() {
  let scene = Scene::over_an_outside_tree();
  let hands = Hands::default();
  let gate = BuiltElsewhere::default();
  let ask = DeployAsk {
    record: Some(scene.at("data")),
    ..DeployAsk::default()
  };
  let code = scene.run_with(&ask, &gate, &hands, &scene.env());
  let head = must(&["rev-parse", "HEAD"], &scene.source);

  assert_eq!(code, 0, "{}", said(&hands));
  assert_eq!(
    std::fs::read_to_string(scene.at("live/deploy/bin/app")).unwrap(),
    "built elsewhere",
    "{}",
    said(&hands)
  );
  assert_eq!(
    gate.0.borrow().as_slice(),
    [(head, "release".to_owned(), vec!["app".to_owned()])],
    "the gate is asked the commit, the profile and the workspace's binaries"
  );
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
  assert_eq!(scene.endings(), ["deploy-failed ended"]);
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

  // Its health never answers, and systemd holds the service as one that gave up starting.
  let hands = Hands {
    gives_up: true,
    ..Hands::default()
  };

  assert_eq!(scene.run(&hands), 1, "{}", said(&hands));
  assert_eq!(
    std::fs::canonicalize(scene.deploy_dir()).unwrap(),
    std::fs::canonicalize(release_dir(&scene.deploy_dir(), &first)).unwrap()
  );
  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().commit, first);
  assert_eq!(scene.names()[1..], ["deploy-rolled-back", "deploy-failed"]);
  assert_eq!(scene.endings()[1..], ["deploy-rolled-back ended", "deploy-failed ended"]);

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

// "the deployment waits for quiet just before the record, however long the work takes, unless it was asked to go
// on past it at once"
#[test]
fn work_under_way_is_waited_out_before_the_flip_unless_asked_to_go_on_at_once() {
  let scene = Scene::new();
  let bound = usize::try_from(DEPLOY.numbers.quiet_unanswered_asks).unwrap();

  scene.declare_quiet(&[1, 9]);
  scene.push("a quiet check");

  let forced = Hands::default();
  let asked = DeployAsk {
    force_live: true,
    ..DeployAsk::default()
  };

  assert_eq!(scene.run_paused(&asked, &forced), 0, "{}", said(&forced));
  assert!(said(&forced).contains("--force-live goes on"), "{}", said(&forced));
  assert!(scene.deploy_dir().exists(), "it went live past the work");
  assert_eq!(scene.quiet_asks(), 1, "a forced deployment asks once");

  // Work under way for more asks than a quiet that could not tell is given, with such answers short of the bound
  // on either side of it: each answer of work starts the count again, and the deployment waits it all out.
  let mut codes = vec![9; bound - 1];

  codes.extend(vec![1; bound + 1]);
  codes.extend(vec![9; bound - 1]);
  std::fs::remove_file(scene.at("quiet-asks")).unwrap();
  scene.declare_quiet(&codes);
  scene.write("note", "second");

  let head = scene.push("a second quiet check");
  let waited = Hands::default();

  assert_eq!(scene.run_paused(&DeployAsk::default(), &waited), 0, "{}", said(&waited));
  assert_eq!(scene.quiet_asks(), codes.len() + 1, "asked until it answered 0");
  assert_eq!(
    said(&waited).matches("waiting: a turn under way").count(),
    bound + 1,
    "each line names the work: {}",
    said(&waited)
  );
  assert!(said(&waited).contains("could not tell"), "{}", said(&waited));
  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().commit, head);

  std::fs::remove_file(scene.at("quiet-asks")).unwrap();
  scene.declare_quiet(&[]);
  scene.write("note", "third");
  scene.push("a third quiet check");

  let quiet = Hands::default();

  assert_eq!(scene.run_paused(&DeployAsk::default(), &quiet), 0, "{}", said(&quiet));
  assert!(!said(&quiet).contains("waiting:"), "a quiet answer waits for nothing: {}", said(&quiet));
}

// "any other answer is a quiet that could not tell ... Once it has answered so a number of asks running, the
// deployment fails with nothing changed, its reason the quiet's last code and its own words"
#[test]
fn a_quiet_that_cannot_tell_fails_the_deployment_with_its_words_and_nothing_goes_live() {
  let scene = Scene::new();

  scene.write("note", "live");

  let live = scene.push("live");

  assert_eq!(scene.run(&Hands::default()), 0);

  let release = std::fs::read_link(scene.deploy_dir()).unwrap();
  let bound = usize::try_from(DEPLOY.numbers.quiet_unanswered_asks).unwrap();

  scene.declare_quiet(&vec![2; bound + 5]);
  scene.push("a quiet that cannot read its store");

  let hands = Hands::default();

  assert_eq!(scene.run_paused(&DeployAsk::default(), &hands), 1, "{}", said(&hands));
  assert_eq!(scene.quiet_asks(), bound, "it stops asking at the bound");
  assert_eq!(scene.names(), ["deployed", "deploy-failed"]);

  let reason = scene.events()[1]["detail"]["reason"].as_str().unwrap().to_owned();

  assert!(reason.contains("relation \"slots\" does not exist"), "the quiet's own words: {reason}");
  assert!(reason.contains("exit 2"), "its code: {reason}");
  assert_eq!(
    std::fs::read_link(scene.deploy_dir()).unwrap(),
    release,
    "the deploy tree was not flipped"
  );
  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().commit, live);
  assert!(
    !hands.systemctl.borrow().iter().any(|asked| asked.starts_with("restart ")),
    "no service restarted: {:?}",
    hands.systemctl.borrow()
  );

  let progress =
    read_deploy_progress(std::fs::read_to_string(scene.at("live/deploy-progress.json")).ok().as_deref());

  assert_eq!(progress.unwrap().ending.unwrap().outcome, Outcome::Failed);
}

// "a change is major where the version's major differs from the release before it"
#[test]
fn a_change_is_major_only_where_both_versions_name_a_major_and_they_differ() {
  assert_eq!(version_major("0.1.0-rev.7"), Some(0));
  assert_eq!(version_major("v12.4.1"), Some(12));
  assert_eq!(version_major("not a version"), None);

  assert!(major_change("2.0.0", "1.9.9"));
  assert!(!major_change("1.6.0", "1.9.9"));
  assert!(!major_change("2.0.0", ""));
  assert!(!major_change("", "1.9.9"));
}

// "a major change runs its consumers' suites, and a change of the same major does not"
#[test]
fn a_major_change_runs_the_dependents_and_a_minor_one_does_not() {
  let scene = Scene::new();
  let hands = Hands::default();

  scene.write("VERSION", "1.0.0\n");
  scene.push("the first release");
  assert_eq!(scene.run_with(&DeployAsk::default(), &Told(true), &hands, &scene.env()), 0);

  scene.write("VERSION", "1.1.0\n");
  scene.push("the same major");
  let counted = Counted::default();
  assert_eq!(scene.run_with(&DeployAsk::default(), &counted, &hands, &scene.env()), 0);
  assert_eq!(*counted.0.borrow(), 0);

  scene.write("VERSION", "2.0.0\n");
  scene.push("the next major");
  let counted = Counted::default();
  assert_eq!(scene.run_with(&DeployAsk::default(), &counted, &hands, &scene.env()), 0);
  assert_eq!(*counted.0.borrow(), 1);
}

// What a run of the suite written into the scene's record stood on, and how it ended.
struct Seeded<'a> {
  commit: &'a str,
  whole: bool,
  clean: bool,
  state: TestRunState,
  hours_ago: i64,
}

impl Scene {
  // A test run of the repository's suite written into the record under the scene's data root; its id.
  fn seed(&self, seeded: &Seeded<'_>) -> i64 {
    let mut record = Record::open(&self.at("testbed-data")).unwrap();
    let started_at = Machine.now_ms() - seeded.hours_ago * 60 * 60 * 1000;
    let begun = record
      .begin_test_run(&TestRun {
        id: 0,
        kind: TestRunKind::Suite,
        checkout: "/elsewhere/checkout".to_owned(),
        repository_name: Some(Repository::read(&self.source).unwrap().name()),
        commit: Some(seeded.commit.to_owned()),
        whole: Some(seeded.whole),
        clean: Some(seeded.clean),
        command: "bmo-testbed suite".to_owned(),
        scope: None,
        unit_name: None,
        pid: i64::from(std::process::id()),
        state: TestRunState::Running,
        passed_count: 0,
        failed_count: 0,
        skipped_count: 0,
        log_path: "/logs/suite.log".to_owned(),
        started_at,
        ended_at: None,
      })
      .unwrap();
    let end = TestRunEnd {
      state: seeded.state,
      passed_count: 3,
      failed_count: 0,
      skipped_count: 0,
      ended_at: started_at + 1000,
    };

    record.end_test_run(begun.id, &end).unwrap();
    begun.id
  }

  // The deployment run against the scene's record with a gate keeping whether it was asked for its suite.
  fn run_asked(&self, gate: &Asked, hands: &Hands) -> u8 {
    let ask = DeployAsk {
      record: Some(self.at("testbed-data")),
      ..DeployAsk::default()
    };

    self.run_with(&ask, gate, hands, &self.env())
  }
}

// "a passed run of the whole suite on the commit, from a clean tree, leaves the gate's suite out and the
// deployment says so, naming the run"
#[test]
fn a_passed_whole_suite_on_the_commit_from_a_clean_tree_leaves_the_gate_suite_out() {
  let scene = Scene::new();
  let hands = Hands::default();
  let head = must(&["rev-parse", "HEAD"], &scene.source);
  let id = scene.seed(&Seeded {
    commit: &head,
    whole: true,
    clean: true,
    state: TestRunState::Passed,
    hours_ago: 1,
  });
  let gate = Asked::default();
  let code = scene.run_asked(&gate, &hands);

  assert_eq!(code, 0, "{}", said(&hands));
  assert_eq!(*gate.suites.borrow(), [false], "{}", said(&hands));
  assert!(
    said(&hands).contains(&format!(
      "the suite is left out: test run {id} passed the whole suite on {}",
      &head[..12]
    )),
    "{}",
    said(&hands)
  );
  assert!(
    said(&hands).contains("in /elsewhere/checkout; its log is /logs/suite.log"),
    "{}",
    said(&hands)
  );
}

// "anything less certain runs the suite: a dirty tree, a failed run, another commit, an old run, a run of
// part of the suite"
#[test]
fn a_dirty_failed_other_old_or_partial_run_leaves_the_gate_suite_in() {
  const OTHER: &str = "0123456789abcdef0123456789abcdef01234567";
  // Each case: the commit the run stood on, the scene's own where none is named, whether it was whole and
  // clean, how it ended, and how many hours ago it began.
  let cases = [
    ("a dirty tree", None, true, false, TestRunState::Passed, 1),
    ("a failed run", None, true, true, TestRunState::Failed, 1),
    ("another commit", Some(OTHER), true, true, TestRunState::Passed, 1),
    ("an old run", None, true, true, TestRunState::Passed, 25),
    ("part of the suite", None, false, true, TestRunState::Passed, 1),
  ];

  for (case, commit, whole, clean, state, hours_ago) in cases {
    let scene = Scene::new();
    let hands = Hands::default();
    let head = must(&["rev-parse", "HEAD"], &scene.source);

    scene.seed(&Seeded {
      commit: commit.unwrap_or(&head),
      whole,
      clean,
      state,
      hours_ago,
    });

    let gate = Asked::default();
    let code = scene.run_asked(&gate, &hands);

    assert_eq!(code, 0, "{case}: {}", said(&hands));
    assert_eq!(*gate.suites.borrow(), [true], "{case}: {}", said(&hands));
    assert!(!said(&hands).contains("the suite is left out"), "{case}: {}", said(&hands));
  }
}

// "a newer run of the whole suite on the commit that failed outweighs an older one that passed"
#[test]
fn a_newer_failed_whole_suite_on_the_commit_leaves_the_gate_suite_in() {
  let scene = Scene::new();
  let hands = Hands::default();
  let head = must(&["rev-parse", "HEAD"], &scene.source);

  for state in [TestRunState::Passed, TestRunState::Failed] {
    scene.seed(&Seeded {
      commit: &head,
      whole: true,
      clean: true,
      state,
      hours_ago: 1,
    });
  }

  let gate = Asked::default();

  assert_eq!(scene.run_asked(&gate, &hands), 0, "{}", said(&hands));
  assert_eq!(*gate.suites.borrow(), [true], "{}", said(&hands));
}

// "a passed run the gate's own machine's record holds is named with that machine"
#[test]
fn a_passed_run_the_gate_found_on_its_machine_is_named_with_the_machine() {
  let scene = Scene::new();
  let hands = Hands::default();
  let head = must(&["rev-parse", "HEAD"], &scene.source);
  let found = TestRun {
    id: 41,
    kind: TestRunKind::Suite,
    checkout: "/bench/checkout".to_owned(),
    repository_name: Some("sample".to_owned()),
    commit: Some(head.clone()),
    whole: Some(true),
    clean: Some(true),
    command: "bmo-testbed suite".to_owned(),
    scope: None,
    unit_name: None,
    pid: 1,
    state: TestRunState::Passed,
    passed_count: 3,
    failed_count: 0,
    skipped_count: 0,
    log_path: "/far/suite.log".to_owned(),
    started_at: 0,
    ended_at: Some(1),
  };
  let gate = Asked {
    passed_there: Some(SuitePassed {
      run: found,
      machine: Some("the-placed-machine".to_owned()),
    }),
    ..Asked::default()
  };

  assert_eq!(scene.run_asked(&gate, &hands), 0, "{}", said(&hands));
  assert_eq!(*gate.suites.borrow(), [true], "{}", said(&hands));
  assert!(said(&hands).contains("test run 41 passed the whole suite"), "{}", said(&hands));
  assert!(
    said(&hands).contains("in /bench/checkout on the-placed-machine; its log is /far/suite.log"),
    "{}",
    said(&hands)
  );
}
