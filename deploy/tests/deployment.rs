//! The deploy run a declaration drives (docs/deploy.md, 「What the deploy run does, in order」), over a
//! repository with a remote of its own in temporary directories: the gate and the service manager stand-ins,
//! every other command real. A commit goes live with its `bot` service restarted and watched; what may not
//! be deployed is refused with nothing changed; a gate that fails puts nothing live; and a service that does
//! not come up is rolled back onto the release before. Once live, the testbed the placed machine runs is
//! brought to the release, and a machine that does not take it leaves the deployment live with a warning.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bmo_testbed_client::{
  Deployment, DeploymentState, PageQuery, RestartsQuery, StopCode, TestRun, TestRunKind, TestRunState,
};
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::events::Hook;
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::release_target::RELEASE_TARGET_MARK;
use bmo_testbed_core::turn::Env;
use bmo_testbed_core::version::platform_day;
use bmo_testbed_deploy::console::{ConsoleHands, MachineConsole};
use bmo_testbed_deploy::deployment::{
  BuildAsk, DeployAsk, DeployHands, DeployPlaces, Gate, GateAsk, GateFault, GateFooting, GateOutcome,
  LaidUnits, Machine, StepAsk, StepRan, SuitePassed, deploy, gate_ground, major_change, passed_before,
  release_build, version_major, workspace_binaries,
};
use bmo_testbed_deploy::expansion::committed_tree;
use bmo_testbed_deploy::line::{DeployLine, DeployTicket, FirstMark, RootTurn};
use bmo_testbed_deploy::progress::{Outcome, read_deploy_progress};
use bmo_testbed_deploy::release::{LiveProcess, release_dir};
use bmo_testbed_deploy::run::{Ran, run_quiet};
use bmo_testbed_deploy::tree::{read_deploy_record, short};
use bmo_testbed_record::{GatePass, Record, TestRunEnd};
use serde_json::Value;

use crate::console::Face;
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
  /// The service's console face the probe asks, where the declaration brings a console screen.
  console: Option<Face>,
  /// Where each line said is told as well, for a test reading the run while it runs.
  told: Option<std::sync::mpsc::Sender<String>>,
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

    if let Some(told) = &self.told {
      let _ = told.send(line.to_owned());
    }
  }

  fn now_ms(&self) -> i64 {
    Machine.now_ms()
  }

  fn live_processes(&self) -> Option<Vec<LiveProcess>> {
    Some(Vec::new())
  }

  fn console(&self) -> &dyn ConsoleHands {
    self
      .console
      .as_ref()
      .map_or(&MachineConsole as &dyn ConsoleHands, |face| face as &dyn ConsoleHands)
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
      ..GateOutcome::default()
    }
  }
}

// A gate that says it has begun, then passes once it is let go or its letting go is dropped, as a deployment
// holding its turn runs a long gate.
struct HeldGate {
  begun: std::sync::mpsc::Sender<()>,
  go: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Gate for HeldGate {
  fn pass(&self, _: &GateAsk<'_>) -> GateOutcome {
    let _ = self.begun.send(());
    let _ = self.go.lock().unwrap().recv();

    GateOutcome {
      passed: true,
      ..GateOutcome::default()
    }
  }
}

// A held gate, the way to hear it has begun, and the way to let it go.
fn held_gate() -> (HeldGate, std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
  let (begun, has_begun) = std::sync::mpsc::channel();
  let (go, gone) = std::sync::mpsc::channel();

  (
    HeldGate {
      begun,
      go: std::sync::Mutex::new(gone),
    },
    has_begun,
    go,
  )
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

  fn build(&self, ask: &BuildAsk<'_>) -> Option<Result<(), GateFault>> {
    self
      .0
      .borrow_mut()
      .push((ask.commit.to_owned(), ask.profile.to_owned(), ask.binaries.to_vec()));
    std::fs::create_dir_all(ask.built).unwrap();
    std::fs::write(ask.built.join("app"), "built elsewhere").unwrap();
    Some(Ok(()))
  }
}

// One ask to lay units: each unit's name with its text, and the timers to start.
type LayAsked = (Vec<(String, String)>, Vec<String>);

// A gate whose machine is another, which passes, keeps the units it is asked to lay there and the timers to
// start, and answers that machine's label with the failure given.
#[derive(Default)]
struct LaysElsewhere {
  asked: RefCell<Vec<LayAsked>>,
  fails: Option<String>,
}

impl Gate for LaysElsewhere {
  fn pass(&self, _: &GateAsk<'_>) -> GateOutcome {
    GateOutcome {
      passed: true,
      ..GateOutcome::default()
    }
  }

  fn lay_units(&self, ask: &LaidUnits<'_>) -> Option<(String, Result<(), String>)> {
    self.asked.borrow_mut().push((ask.units.to_vec(), ask.timers.to_vec()));
    Some(("machine-2".to_owned(), self.fails.clone().map_or(Ok(()), Err)))
  }
}

// A gate whose machine is another, which passes, keeps each live release and commit it is asked to bring that
// machine's testbed to, and answers that machine's label with the answer given.
struct BringsElsewhere {
  asked: RefCell<Vec<(PathBuf, String)>>,
  answer: Result<Vec<String>, String>,
}

impl Gate for BringsElsewhere {
  fn pass(&self, _: &GateAsk<'_>) -> GateOutcome {
    GateOutcome {
      passed: true,
      ..GateOutcome::default()
    }
  }

  fn bring_testbed(&self, release: &Path, commit: &str) -> Option<(String, Result<Vec<String>, String>)> {
    self.asked.borrow_mut().push((release.to_path_buf(), commit.to_owned()));
    Some(("machine-2".to_owned(), self.answer.clone()))
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

  // A repository whose one Cargo workspace names a base library outside it, the library a repository of its
  // own whose main branch holds it, and nothing under the deploy root standing where either tree will read it.
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
    must(&["init", "-q", "-b", "main"], &base);
    must(&["add", "."], &base);
    must(&["commit", "-q", "-m", "library"], &base);

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

  // A deployment asked from the checkout given rather than the scene's own.
  fn run_from(&self, checkout: &Path, ask: &DeployAsk, gate: &dyn Gate, hands: &Hands) -> u8 {
    let repo = Repository::read(checkout).unwrap();
    let places = DeployPlaces::of(&repo, &self.env()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

    runtime.block_on(deploy(&repo, &places, ask, &self.env(), gate, hands, &self.hook()))
  }

  // A linked worktree of the scene's checkout at its head, as a job's checkout stands beside the primary one.
  fn job_checkout(&self, name: &str) -> PathBuf {
    let job = self.at(name);

    must(
      &["worktree", "add", "--quiet", "--detach", &job.to_string_lossy(), "HEAD"],
      &self.source,
    );
    job
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
    let codes_at = self.at("quiet-codes");
    let asks = self.at("quiet-asks");

    std::fs::write(&codes_at, codes.iter().map(|code| format!("{code}\n")).collect::<String>()).unwrap();
    self.declare_quiet_script(&format!(
      "n=$(( $(cat {asks} 2>/dev/null || echo 0) + 1 ))\necho $n > {asks}\ncode=$(sed -n \"${{n}}p\" {codes})\ncase \"$code\" in\n  1) echo 'a turn under way' ;;\n  ''|0) ;;\n  *) echo 'relation \"slots\" does not exist' >&2 ;;\nesac\nexit \"${{code:-0}}\"\n",
      asks = asks.display(),
      codes = codes_at.display(),
    ));
  }

  // The declaration's `quiet`, a script that names a turn under way while the file given stands, and 0 once it
  // is gone.
  fn declare_quiet_while(&self, held: &Path) {
    self.declare_quiet_script(&format!(
      "if [ -e {held} ]; then echo 'a turn under way'; exit 1; fi\n",
      held = held.display()
    ));
  }

  // The declaration's `quiet`, the script given, run from the release it is asked in.
  fn declare_quiet_script(&self, body: &str) {
    let script = self.at("quiet.sh");
    // A declaration that already names a quiet has it named afresh.
    let declared = std::fs::read_to_string(self.source.join("testbed.toml"))
      .unwrap()
      .lines()
      .filter(|line| !line.starts_with("quiet = "))
      .map(|line| format!("{line}\n"))
      .collect::<String>();

    std::fs::write(&script, body).unwrap();
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

// "the reason a deploy is made and the deployed version are written into the record and the event": the
// commit its release tag names records the release, and a later commit the day's nightly after it
#[test]
fn the_message_overrides_the_subject_and_the_deployed_version_is_recorded() {
  let scene = Scene::new();
  let hands = Hands::default();

  scene.write("VERSION", "3.4.0\n");
  let released = scene.push("the release");
  must(&["tag", "v3.4.0", &released], &scene.source);

  assert_eq!(
    scene.run_with(&DeployAsk::default(), &Told(true), &hands, &scene.env()),
    0,
    "{}",
    said(&hands)
  );
  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().version, "3.4.0");

  scene.write("later.txt", "a later change\n");
  let head = scene.push("a later commit subject");

  let ask = DeployAsk {
    record: Some(scene.at("testbed-data")),
    message: Some("hotfix for the placement bug".to_owned()),
    ..DeployAsk::default()
  };
  let code = scene.run_with(&ask, &Told(true), &hands, &scene.env());

  assert_eq!(code, 0, "{}", said(&hands));

  let written = read_deploy_record(&scene.at("live/deployed.json")).unwrap();
  let day = platform_day(Machine.now_ms()).format("%Y%m%d");

  assert_eq!(written.commit, head);
  assert_eq!(written.subject, "a later commit subject");
  assert_eq!(written.message, "hotfix for the placement bug");
  assert_eq!(written.version, format!("3.5.0-nightly.{day}"));

  let events = scene.events();

  assert_eq!(events.last().unwrap()["detail"]["message"], "hotfix for the placement bug");
}

// "a first deployment lays the base-library tree beside the release and the build tree" as links into its
// committed tree; "A deployment builds committed trees only": the library's checkout holding an edit half
// written changes nothing the release or the build tree reads
#[test]
fn a_first_deployment_lays_the_trees_outside_the_workspace_at_their_committed_heads() {
  let scene = Scene::over_an_outside_tree();
  let library = scene.at("base/text");
  let main = must(&["rev-parse", "main"], &library).trim().to_owned();

  std::fs::write(library.join("src/lib.rs"), "pub fn half_written(\n").unwrap();

  let hands = Hands::default();
  let code = scene.run(&hands);
  let committed = committed_tree(&library, &scene.source)
    .unwrap()
    .worktree(&scene.at("live/deploy-trees"));

  assert_eq!(code, 0, "{}", said(&hands));
  assert_eq!(
    symlink(&scene.at("live/deploy-releases/base/text")),
    Some(committed.clone()),
    "the tree the release names outside it stands as a link into its committed tree: {}",
    said(&hands)
  );
  assert_eq!(
    symlink(&scene.at("live/base/text")),
    Some(committed.clone()),
    "the tree the build tree names outside it stands as a link into its committed tree: {}",
    said(&hands)
  );
  assert_eq!(must(&["rev-parse", "HEAD"], &committed).trim(), main);
  assert_eq!(std::fs::read_to_string(committed.join("src/lib.rs")).unwrap(), "");
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

// "every deployment lays these units there once it is live"; "a unit that cannot be laid or a timer started
// there, is a warning naming the machine and what it said, and the deployment goes on"
#[test]
fn a_live_release_lays_the_units_it_marks_for_the_placed_machine_and_a_failure_there_is_a_warning() {
  let scene = Scene::new();
  let timer = "[Timer]\nOnCalendar=daily\n\n[Install]\nX-BMO-Deploy=timer\nX-BMO-Placed=timer\n";
  let service = "[Service]\nType=oneshot\nExecStart=%h/live/deploy/bin/app\n";

  scene.write("units/sweep.timer", timer);
  scene.write("units/sweep.service", service);
  scene.push("a timer the placed machine keeps too");

  let hands = Hands::default();
  let gate = LaysElsewhere {
    fails: Some("connection closed".to_owned()),
    ..LaysElsewhere::default()
  };
  let code = scene.run_with(&DeployAsk::default(), &gate, &hands, &scene.env());
  let unlaid = fill_marks(
    &DEPLOY.run.placed_unlaid,
    &[
      ("names", "sweep.timer, sweep.service"),
      ("machine", "machine-2"),
      ("said", "connection closed"),
    ],
  );

  assert_eq!(code, 0, "{}", said(&hands));
  assert_eq!(
    gate.asked.borrow().as_slice(),
    [(
      vec![
        ("sweep.timer".to_owned(), timer.to_owned()),
        ("sweep.service".to_owned(), service.to_owned())
      ],
      vec!["sweep.timer".to_owned()]
    )]
  );
  assert!(said(&hands).contains(&unlaid), "{}", said(&hands));
}

// "once it is live ... copies ... then reads that built-from back ... The deployment's line names the machine,
// the files written there and the commit read back"; "a machine that cannot be reached ... is a warning on a
// line naming the machine and what it answered: the deployment here stays live"
#[test]
fn a_live_release_brings_the_placed_testbed_to_it_and_a_machine_out_of_reach_leaves_it_live_with_a_warning()
{
  let far = vec!["~/bin/bmo-testbed".to_owned(), "~/bin/built-from".to_owned()];
  let unreached = "it cannot be reached (ssh: connect to host stand port 22: No route to host)".to_owned();

  for answer in [Ok(far.clone()), Err(unreached.clone())] {
    let scene = Scene::new();
    let hands = Hands::default();
    let gate = BringsElsewhere {
      asked: RefCell::default(),
      answer: answer.clone(),
    };
    let code = scene.run_with(&DeployAsk::default(), &gate, &hands, &scene.env());
    let head = must(&["rev-parse", "HEAD"], &scene.source);
    let shown = &head[..12];
    let line = match &answer {
      Ok(files) => fill_marks(
        &DEPLOY.run.testbed_brought,
        &[("machine", "machine-2"), ("files", &files.join(", ")), ("commit", shown)],
      ),
      Err(why) => fill_marks(
        &DEPLOY.run.testbed_unbrought,
        &[("machine", "machine-2"), ("commit", shown), ("said", why)],
      ),
    };
    let progress =
      read_deploy_progress(std::fs::read_to_string(scene.at("live/deploy-progress.json")).ok().as_deref());

    assert_eq!(code, 0, "{}", said(&hands));
    assert_eq!(progress.unwrap().ending.unwrap().outcome, Outcome::Live, "{}", said(&hands));
    let asked: Vec<(PathBuf, String)> = gate
      .asked
      .borrow()
      .iter()
      .map(|(release, commit)| (std::fs::canonicalize(release).unwrap(), commit.clone()))
      .collect();

    assert_eq!(
      asked,
      [(std::fs::canonicalize(scene.at("live/deploy")).unwrap(), head.clone())],
      "the machine's testbed is brought to the live release, at the commit put live"
    );
    assert!(said(&hands).contains(&line), "{}", said(&hands));
  }
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

// "a commit is deployed by its own declaration, never by the checkout's work tree"
#[test]
fn a_commit_goes_live_by_the_declaration_it_carries_not_the_checkouts() {
  let scene = Scene::new();
  let first = must(&["rev-parse", "HEAD"], &scene.source);
  let marker = scene.at("commit-step-ran");

  scene.declare("v2", "true");

  let declared = std::fs::read_to_string(scene.source.join("testbed.toml")).unwrap();

  scene.write(
    "testbed.toml",
    &format!(
      "{declared}\n[[deploy.step]]\nid = \"commit-only\"\ncommand = \"touch {}\"\nphase = \"after-health\"\n",
      marker.display()
    ),
  );

  let second = scene.push("second");

  // The checkout stands on the first commit, whose declaration builds v1 and has no such step.
  must(&["checkout", "--quiet", &first], &scene.source);

  let hands = Hands::default();
  let ask = DeployAsk {
    commit: Some(second.clone()),
    ..DeployAsk::default()
  };

  assert_eq!(scene.run_with(&ask, &Told(true), &hands, &scene.env()), 0, "{}", said(&hands));
  assert_eq!(std::fs::read_to_string(scene.deploy_dir().join("bin/app")).unwrap(), "echo v2\n");
  assert!(marker.exists(), "{}", said(&hands));
  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().commit, second);
}

// "a commit whose declaration stands the deployment elsewhere is refused with nothing changed"
#[test]
fn a_commit_whose_declaration_moves_the_deploy_root_is_refused() {
  let scene = Scene::new();
  let first = must(&["rev-parse", "HEAD"], &scene.source);
  let declared = std::fs::read_to_string(scene.source.join("testbed.toml")).unwrap();
  let live = scene.at("live").display().to_string();
  let moved = scene.at("moved").display().to_string();

  scene.write("testbed.toml", &declared.replace(&live, &moved));

  let second = scene.push("moved");

  must(&["checkout", "--quiet", &first], &scene.source);

  let hands = Hands::default();
  let ask = DeployAsk {
    commit: Some(second),
    ..DeployAsk::default()
  };

  assert_eq!(scene.run_with(&ask, &Told(true), &hands, &scene.env()), 2);
  assert!(said(&hands).contains("places the deployment otherwise"), "{}", said(&hands));
  assert!(!scene.deploy_dir().exists());
  assert!(!scene.at("moved").exists());
  assert!(scene.events().is_empty());
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

// "a service that stays running and never answers its health is rolled back once the wait's bound passes"
#[test]
fn a_service_that_runs_and_never_answers_its_health_is_rolled_back_once_the_bound_passes() {
  let scene = Scene::new();
  let first = must(&["rev-parse", "HEAD"], &scene.source);

  assert_eq!(scene.run(&Hands::default()), 0);

  scene.declare("v2", "false");

  let declared = std::fs::read_to_string(scene.source.join("testbed.toml")).unwrap();

  scene.write(
    "testbed.toml",
    &declared.replace("health = \"false\"\n", "health = \"false\"\nhealth_wait_seconds = 30\n"),
  );

  let second = scene.push("second");
  // Its health never answers and systemd holds the service running: only the declared bound, on a clock that
  // moves only while the deployment waits, ends the wait, after the restart and again after the rollback's.
  let hands = Hands::default();

  assert_eq!(scene.run_paused(&DeployAsk::default(), &hands), 1, "{}", said(&hands));
  assert_eq!(
    std::fs::canonicalize(scene.deploy_dir()).unwrap(),
    std::fs::canonicalize(release_dir(&scene.deploy_dir(), &first)).unwrap()
  );
  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().commit, first);
  assert_eq!(scene.names()[1..], ["deploy-rolled-back", "deploy-failed"]);

  let told = said(&hands);
  let bound = fill_marks(&DEPLOY.boot.ran_out, &[("bound", "30")]);

  assert!(
    told.contains(&fill_marks(&DEPLOY.rollback.ran_out, &[("commit", &short(&second))])),
    "{told}"
  );
  assert_eq!(told.matches(&bound).count(), 2, "{told}");

  let progress =
    read_deploy_progress(std::fs::read_to_string(scene.at("live/deploy-progress.json")).ok().as_deref())
      .unwrap();

  assert_eq!(progress.ending.unwrap().outcome, Outcome::RolledBack);
}

// "a release whose console face the probe refuses is rolled back, the point named in its events"
#[test]
fn a_release_whose_console_face_is_refused_is_rolled_back_naming_the_point() {
  let scene = Scene::new();
  let first = must(&["rev-parse", "HEAD"], &scene.source);

  assert_eq!(scene.run(&Hands::default()), 0);

  let declared = std::fs::read_to_string(scene.source.join("testbed.toml")).unwrap();

  scene.write("testbed.toml", &format!("{declared}\n[deploy.console]\nreach = \"true\"\n"));
  scene.push("second");

  // The face answers its module to a call with no role.
  let hands = Hands {
    console: Some(Face {
      open_without_role: true,
      ..Face::default()
    }),
    ..Hands::default()
  };

  assert_eq!(scene.run(&hands), 1, "{}", said(&hands));
  assert_eq!(read_deploy_record(&scene.at("live/deployed.json")).unwrap().commit, first);
  assert!(
    said(&hands).contains("the console probe refused the release at the module with no role: GET /console-bundle/sample-1a2b.js answered 200 where the contract asks 403"),
    "{}",
    said(&hands)
  );
  assert_eq!(scene.names()[1..], ["deploy-rolled-back", "deploy-failed"]);

  let events = scene.events();

  assert!(
    events[1]["detail"]["reason"]
      .as_str()
      .unwrap()
      .starts_with("rolled back after a failed deployment: the console probe refused"),
    "{}",
    events[1]
  );
  assert_eq!(events[2]["detail"]["stage"], "after");
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

// A gate standing on the footing it is given: it asks for an earlier pass on it and runs nothing where one
// stands, and otherwise passes on it, counting how many times it ran.
struct Footed {
  footing: GateFooting,
  ran: Cell<usize>,
}

impl Footed {
  fn on(toolchains: &str) -> Self {
    Self {
      footing: GateFooting {
        heads: vec!["b".repeat(40), "a".repeat(40)],
        toolchains: toolchains.to_owned(),
        machine: Some("machine-2".to_owned()),
      },
      ran: Cell::new(0),
    }
  }
}

impl Gate for Footed {
  fn pass(&self, ask: &GateAsk<'_>) -> GateOutcome {
    if let Some(earlier) = (ask.earlier)(&self.footing) {
      return passed_before(self.footing.clone(), earlier);
    }

    self.ran.set(self.ran.get() + 1);
    GateOutcome {
      passed: true,
      footing: Some(self.footing.clone()),
      ..GateOutcome::default()
    }
  }
}

impl Scene {
  // A gate's pass on the repository's `HEAD`, nothing live before it, written into the scene's record.
  fn seed_gate_pass(&self, toolchains: &str) {
    let head = must(&["rev-parse", "HEAD"], &self.source);
    let footing = Footed::on(toolchains).footing;
    let pass = GatePass {
      ground: gate_ground(&Repository::read(&self.source).unwrap().name(), &head, None, &footing),
      machine: footing.machine,
      at: Machine.now_ms() - 60 * 60 * 1000,
    };

    Record::open(&self.at("testbed-data")).unwrap().write_gate_pass(&pass).unwrap();
  }
}

// "Every later gate ... asks the record for a pass of the same repository, commit, range, heads and toolchains
// ... Where one stands the gate runs nothing ... and the deployment says on one line that the gate is left out"
#[test]
fn an_earlier_pass_on_the_same_ground_leaves_the_whole_gate_out() {
  let scene = Scene::new();
  let hands = Hands::default();
  let gate = Footed::on("cargo: rustc 1.0.0\n");

  scene.seed_gate_pass("cargo: rustc 1.0.0\n");

  assert_eq!(scene.run_asked_of(&gate, &hands), 0, "{}", said(&hands));
  assert_eq!(gate.ran.get(), 0, "{}", said(&hands));
  assert!(said(&hands).contains("the gate is left out: "), "{}", said(&hands));
  assert!(said(&hands).contains(" on machine-2 over the same range"), "{}", said(&hands));
}

// "Anything less certain runs the gate: ... another toolchain"; "A gate that passes is written into this
// machine's record"
#[test]
fn a_pass_on_another_toolchain_runs_the_gate_and_its_own_pass_is_written() {
  let scene = Scene::new();
  let hands = Hands::default();
  let head = must(&["rev-parse", "HEAD"], &scene.source);
  let gate = Footed::on("cargo: rustc 2.0.0\n");

  scene.seed_gate_pass("cargo: rustc 1.0.0\n");

  assert_eq!(scene.run_asked_of(&gate, &hands), 0, "{}", said(&hands));
  assert_eq!(gate.ran.get(), 1, "{}", said(&hands));
  assert!(!said(&hands).contains("the gate is left out"), "{}", said(&hands));

  let ground = gate_ground(&Repository::read(&scene.source).unwrap().name(), &head, None, &gate.footing);
  let written = Record::open(&scene.at("testbed-data")).unwrap().gate_passed(&ground, 0).unwrap();

  assert_eq!(written.map(|pass| pass.machine), Some(Some("machine-2".to_owned())));
}

// "A commit already live when its turn comes ends the deployment there ... exits 0: no gate, no backup, no
// release, no restart"
#[test]
fn a_commit_already_live_when_its_turn_comes_ends_live_without_a_gate() {
  let scene = Scene::new();
  let hands = Hands::default();
  let gate = Footed::on("cargo: rustc 1.0.0\n");

  assert_eq!(scene.run_asked_of(&gate, &hands), 0, "{}", said(&hands));

  let restarts = hands.systemctl.borrow().len();
  let again = Hands::default();

  assert_eq!(scene.run_asked_of(&gate, &again), 0, "{}", said(&again));
  assert_eq!(gate.ran.get(), 1, "{}", said(&again));
  assert!(
    said(&again).contains("is already live, so nothing is laid down again"),
    "{}",
    said(&again)
  );
  assert!(again.systemctl.borrow().is_empty(), "{:?}", again.systemctl.borrow());
  assert!(restarts > 0);
}

// "A commit the live commit already carries ends the deployment at its turn the same way ... laying nothing down"
#[test]
fn a_commit_the_live_one_carries_ends_live_without_laying_it_down() {
  let scene = Scene::new();
  let first = must(&["rev-parse", "HEAD"], &scene.source).trim().to_owned();
  let gate = Footed::on("cargo: rustc 1.0.0\n");

  scene.declare("v2", "true");
  scene.push("second");
  assert_eq!(scene.run_asked_of(&gate, &Hands::default()), 0);

  let back = Hands::default();
  let ask = DeployAsk {
    record: Some(scene.at("testbed-data")),
    commit: Some(first),
    ..DeployAsk::default()
  };

  assert_eq!(scene.run_with(&ask, &gate, &back, &scene.env()), 0, "{}", said(&back));
  assert!(said(&back).contains("is carried by the commit already live"), "{}", said(&back));
  assert!(back.systemctl.borrow().is_empty(), "{:?}", back.systemctl.borrow());
}

impl Scene {
  // The deployment run against the scene's record with the gate given.
  fn run_asked_of(&self, gate: &dyn Gate, hands: &Hands) -> u8 {
    let ask = DeployAsk {
      record: Some(self.at("testbed-data")),
      ..DeployAsk::default()
    };

    self.run_with(&ask, gate, hands, &self.env())
  }
}

// The line naming the machine whose connection broke, as the placed gate says it.
const BROKE: &str = "the connection to machine-2 broke";

// A gate on another machine whose connection breaks under its gate and under its build the number of times given
// before each answers: the gate passing, the build bringing a binary back. How many times each was asked.
#[derive(Default)]
struct Breaking {
  gate_breaks: usize,
  build_breaks: usize,
  gates: Cell<usize>,
  builds: Cell<usize>,
}

impl Gate for Breaking {
  fn pass(&self, _: &GateAsk<'_>) -> GateOutcome {
    let asked = self.gates.get();

    self.gates.set(asked + 1);
    GateOutcome {
      passed: asked >= self.gate_breaks,
      broken: asked < self.gate_breaks,
      reason: if asked < self.gate_breaks { BROKE.to_owned() } else { String::new() },
      ..GateOutcome::default()
    }
  }

  fn build(&self, ask: &BuildAsk<'_>) -> Option<Result<(), GateFault>> {
    let asked = self.builds.get();

    self.builds.set(asked + 1);

    if asked < self.build_breaks {
      return Some(Err(GateFault::Broken(BROKE.to_owned())));
    }

    std::fs::create_dir_all(ask.built).unwrap();
    std::fs::write(ask.built.join("app"), "built elsewhere").unwrap();
    Some(Ok(()))
  }
}

// A gate on another machine that passes and whose build there fails with Cargo's own code.
struct FailsBuildThere;

impl Gate for FailsBuildThere {
  fn pass(&self, _: &GateAsk<'_>) -> GateOutcome {
    GateOutcome {
      passed: true,
      ..GateOutcome::default()
    }
  }

  fn build(&self, _: &BuildAsk<'_>) -> Option<Result<(), GateFault>> {
    Some(Err(GateFault::Failed {
      code: Some(101),
      said: "it ran on machine-2 in the bench".to_owned(),
    }))
  }
}

// An ask whose work on the gate's machine is tried as many times as pauses are given plus one, no pause waiting.
fn retrying(pauses: usize) -> DeployAsk {
  DeployAsk {
    retry: vec![Duration::ZERO; pauses],
    ..DeployAsk::default()
  }
}

// The line saying the work is tried again: the try about to begin of every try there is.
fn retried(work: &str, next: usize, tries: usize) -> String {
  fill_marks(
    &DEPLOY.run.retrying,
    &[
      ("said", BROKE),
      ("work", work),
      ("seconds", "0"),
      ("next", &next.to_string()),
      ("tries", &tries.to_string()),
    ],
  )
}

// "where the connection breaks under one ... the deployment then says so on a line naming the machine, the work and
// the pause, waits the pause, and runs that work again from its bench: the whole gate, or the build with its
// read-back"
#[test]
fn a_gate_and_a_build_the_connection_breaks_under_are_tried_again_and_the_commit_goes_live() {
  let scene = Scene::over_an_outside_tree();
  let hands = Hands::default();
  let gate = Breaking {
    gate_breaks: 1,
    build_breaks: 2,
    ..Breaking::default()
  };
  let code = scene.run_with(&retrying(3), &gate, &hands, &scene.env());

  assert_eq!(code, 0, "{}", said(&hands));
  assert_eq!((gate.gates.get(), gate.builds.get()), (2, 3));
  assert_eq!(std::fs::read_to_string(scene.at("live/deploy/bin/app")).unwrap(), "built elsewhere");

  let lines = said(&hands);

  for line in [
    retried(&DEPLOY.run.gate_work, 2, 4),
    retried("cargo release build", 2, 4),
    retried("cargo release build", 3, 4),
  ] {
    assert!(lines.contains(&line), "{line}\n{lines}");
  }
}

// "Once the tries are spent, the stage fails naming the connection: the machine and how many tries running it
// broke on"
#[test]
fn a_connection_that_keeps_breaking_fails_the_stage_naming_it() {
  let lost = fill_marks(&DEPLOY.run.connection_lost, &[("said", BROKE), ("tries", "3")]);

  let scene = Scene::new();
  let hands = Hands::default();
  let gate = Breaking {
    gate_breaks: usize::MAX,
    ..Breaking::default()
  };
  let code = scene.run_with(&retrying(2), &gate, &hands, &scene.env());
  let head = must(&["rev-parse", "HEAD"], &scene.source);
  let failed = fill_marks(&DEPLOY.run.gate_failed, &[("commit", &head[..12]), ("reason", &lost)]);

  assert_eq!(code, 1, "{}", said(&hands));
  assert_eq!(gate.gates.get(), 3);
  assert!(said(&hands).contains(&failed), "{failed}\n{}", said(&hands));
  assert!(!scene.deploy_dir().exists(), "{}", said(&hands));

  let scene = Scene::over_an_outside_tree();
  let hands = Hands::default();
  let gate = Breaking {
    build_breaks: usize::MAX,
    ..Breaking::default()
  };
  let code = scene.run_with(&retrying(2), &gate, &hands, &scene.env());
  let unbuilt =
    fill_marks(&DEPLOY.run.build_unfinished, &[("step", "cargo release build"), ("said", &lost)]);

  assert_eq!(code, 1, "{}", said(&hands));
  assert_eq!(gate.builds.get(), 3);
  assert!(said(&hands).contains(&unbuilt), "{unbuilt}\n{}", said(&hands));
  assert!(!scene.deploy_dir().exists(), "{}", said(&hands));
}

// "A failure of the work's own is not tried again: ... a build that fails, ends its stage at once with its own exit
// code, as the build's line names it"
#[test]
fn a_build_that_fails_there_is_not_tried_again_and_names_its_own_exit_code() {
  let scene = Scene::over_an_outside_tree();
  let hands = Hands::default();
  let code = scene.run_with(&retrying(3), &FailsBuildThere, &hands, &scene.env());
  let failed = fill_marks(
    &DEPLOY.run.build_failed,
    &[
      ("step", "cargo release build"),
      ("code", "101"),
      ("said", "it ran on machine-2 in the bench"),
    ],
  );

  assert_eq!(code, 1, "{}", said(&hands));
  assert!(said(&hands).contains(&failed), "{failed}\n{}", said(&hands));
  assert!(!said(&hands).contains("trying again"), "{}", said(&hands));
}

// The scene's declared build step changed by `change`, pushed as a commit of its own.
fn rebuilt(scene: &Scene, change: impl Fn(&str) -> String) {
  let declared = std::fs::read_to_string(scene.source.join("testbed.toml")).unwrap();

  scene.write("testbed.toml", &change(&declared));
  scene.push("rebuilt");
}

// "The build tree's target directory carries the release build's mark, a file the deployment writes into it before
// its build stage begins, every time and whether or not the commit holds a Cargo workspace"
#[test]
fn the_build_tree_s_target_directory_is_marked_before_anything_is_built_into_it() {
  let scene = Scene::new();
  let hands = Hands::default();

  // The step builds only where the mark already stands.
  rebuilt(&scene, |declared| {
    declared.replace(
      "command = \"mkdir -p target/release",
      &format!("command = \"test -f target/{RELEASE_TARGET_MARK} && mkdir -p target/release"),
    )
  });

  let code = scene.run(&hands);

  assert_eq!(code, 0, "{}", said(&hands));
  assert!(scene.at("live/deploy-expand/target").join(RELEASE_TARGET_MARK).is_file());
  assert!(scene.at("live/deploy/bin/app").exists(), "{}", said(&hands));
}

// "A binary that is gone all the same when it is placed fails the deployment with every service untouched, the line
// naming the file it looked for"
#[test]
fn a_binary_gone_from_the_build_before_it_is_placed_fails_the_deployment_naming_it() {
  let scene = Scene::new();
  let hands = Hands::default();

  // The step builds the binary and takes it away again, as a removal between the build and the placing would.
  rebuilt(&scene, |declared| {
    declared.replace("> target/release/app\"", "> target/release/app && rm target/release/app\"")
  });

  let code = scene.run(&hands);
  let gone = scene.at("live/deploy-expand/target/release/app");
  let failed = fill_marks(
    &DEPLOY.run.place_failed,
    &[("why", &format!("{}: No such file or directory (os error 2)", gone.display()))],
  );

  assert_eq!(code, 1, "{}", said(&hands));
  assert!(said(&hands).contains(&failed), "{failed}\n{}", said(&hands));
  assert!(!scene.deploy_dir().exists(), "{}", said(&hands));
  assert!(scene.names().iter().any(|name| name == "deploy-failed"), "{:?}", scene.names());
}

// The ticket of a deployment that asked earlier, named by its words alone.
fn earlier_ticket() -> DeployTicket {
  DeployTicket {
    words: "an earlier deployment".to_owned(),
    ..DeployTicket::default()
  }
}

// The scene's own deployment line, its waits said every few milliseconds, with the bound given.
fn line_of(scene: &Scene, bound: Duration) -> DeployLine {
  DeployLine {
    lock: scene.at("locks/deploy-line.lock"),
    bound,
    say_every: Duration::from_millis(5),
    poll: Some(Duration::from_millis(5)),
  }
}

// "Every deployment stands in the machine's one deployment line ... the unit waits its turn in the line and ends
// with its own outcome"; "While it waits it says so"; "A deployment waiting there has written nothing into its
// progress record"
#[test]
fn a_deployment_that_must_wait_in_the_line_ends_live_once_its_turn_comes() {
  let scene = Scene::new();
  let line = line_of(&scene, Duration::from_secs(3600));
  let held = line.take(&earlier_ticket(), &|_| {}).unwrap();
  let (told, heard) = std::sync::mpsc::channel();
  let ask = DeployAsk {
    line: Some(line),
    ..DeployAsk::default()
  };

  let ((code, lines), waiting, unwritten) = std::thread::scope(|scope| {
    let deploying = scope.spawn(|| {
      let hands = Hands {
        told: Some(told),
        ..Hands::default()
      };
      let code = scene.run_with(&ask, &Told(true), &hands, &scene.env());

      (code, said(&hands))
    });
    // Its turn is let go once it has said it waits behind the holder, or once a minute has gone without it.
    let given_up = Instant::now() + Duration::from_secs(60);
    let waiting =
      std::iter::from_fn(|| heard.recv_timeout(given_up.saturating_duration_since(Instant::now())).ok())
        .find(|line| line.contains("waiting for the deployment line"));
    let unwritten = !scene.at("live/deploy-progress.json").exists();

    drop(held);
    (deploying.join().unwrap(), waiting, unwritten)
  });
  let waiting = waiting.expect("the deployment says it waits");

  assert!(waiting.contains("held by an earlier deployment"), "{waiting}");
  assert!(unwritten, "nothing is written while it waits: {lines}");
  assert_eq!(code, 0, "{lines}");
  assert!(scene.deploy_dir().exists(), "{lines}");
}

// "The deploy verb's `--first <why>` marks it"; "The deployment says both on a line once it is marked": the
// mark the run is asked with stands its ticket first and is said with who and why.
#[test]
fn a_deployment_asked_first_says_who_marked_it_and_why_and_ends_live() {
  let scene = Scene::new();
  let hands = Hands::default();
  let ask = DeployAsk {
    line: Some(line_of(&scene, Duration::from_secs(3600))),
    first: Some(FirstMark::now("ops", "the paint fix is urgent")),
    ..DeployAsk::default()
  };
  let code = scene.run_with(&ask, &Told(true), &hands, &scene.env());
  let marked = fill_marks(&DEPLOY.run.first_marked, &[("by", "ops"), ("why", "the paint fix is urgent")]);

  assert_eq!(code, 0, "{}", said(&hands));
  assert!(said(&hands).contains(&marked), "{marked}\n{}", said(&hands));
}

// "Once it has waited the policy's minutes ... it says so on a line naming the holder and goes on beside it,
// outside the line"
#[test]
fn a_deployment_whose_wait_in_the_line_passes_its_bound_goes_on_beside_the_holder_and_ends_live() {
  let scene = Scene::new();
  let hands = Hands::default();
  let line = line_of(&scene, Duration::ZERO);
  let _held = line.take(&earlier_ticket(), &|_| {}).unwrap();
  let ask = DeployAsk {
    line: Some(line),
    ..DeployAsk::default()
  };
  let code = scene.run_with(&ask, &Told(true), &hands, &scene.env());
  let passed = fill_marks(&DEPLOY.run.line_bound, &[("minutes", "0"), ("holder", "an earlier deployment")]);

  assert_eq!(code, 0, "{}", said(&hands));
  assert!(said(&hands).contains(&passed), "{passed}\n{}", said(&hands));
  assert!(scene.deploy_dir().exists(), "{}", said(&hands));
}

// The first line out of the channel given holding one of the words given, waiting until the moment given; none
// where the channel closes or the moment passes first.
fn heard_until(
  heard: &std::sync::mpsc::Receiver<String>,
  words: &[&str],
  until: Instant,
) -> Option<String> {
  std::iter::from_fn(|| heard.recv_timeout(until.saturating_duration_since(Instant::now())).ok())
    .find(|line| words.iter().any(|word| line.contains(word)))
}

// The fixed words of a policy line, up to its first mark.
fn words_of(line: &str) -> &str {
  line.split('{').next().unwrap_or_default()
}

// "One deployment of a deploy root runs at a time ... waits there until that one has ended, however long that
// takes"; "While it waits it says so, at once ... naming the deployment it waits behind"
#[test]
fn a_deployment_asked_while_one_of_its_repository_waits_for_quiet_waits_for_it_and_both_go_live() {
  let scene = Scene::new();

  assert_eq!(scene.run(&Hands::default()), 0);

  let held = scene.at("work-under-way");

  std::fs::write(&held, "").unwrap();
  scene.declare_quiet_while(&held);

  let earlier = scene.push("second");
  let ask = DeployAsk {
    root_poll: Some(Duration::from_millis(5)),
    ..DeployAsk::default()
  };
  let (told_earlier, heard_earlier) = std::sync::mpsc::channel();
  let (told_later, heard_later) = std::sync::mpsc::channel();
  let given_up = Instant::now() + Duration::from_secs(60);
  let (scene, ask) = (&scene, &ask);

  let (later, (earlier_code, earlier_said), (later_code, later_said)) = std::thread::scope(|scope| {
    let deploying_earlier = scope.spawn(move || {
      let hands = Hands {
        told: Some(told_earlier),
        ..Hands::default()
      };

      (scene.run_paused(ask, &hands), said(&hands))
    });

    heard_until(&heard_earlier, &[words_of(&DEPLOY.run.quiet_live)], given_up);
    scene.declare("v3", "true");
    scene.declare_quiet_while(&held);

    let later = scene.push("third");
    let deploying_later = scope.spawn(move || {
      let hands = Hands {
        told: Some(told_later),
        ..Hands::default()
      };

      (scene.run_paused(ask, &hands), said(&hands))
    });

    // The work under way ends once the later deployment waits behind the earlier one, or has pruned the
    // releases beside it, or a minute has gone without either.
    heard_until(
      &heard_later,
      &[words_of(&DEPLOY.run.root_waiting), words_of(&DEPLOY.run.pruned)],
      given_up,
    );
    std::fs::remove_file(&held).unwrap();

    (later, deploying_earlier.join().unwrap(), deploying_later.join().unwrap())
  });
  let log = std::fs::read_to_string(scene.at("live/deployed.log")).unwrap();
  let laid: Vec<&str> = log.lines().filter_map(|line| line.split_whitespace().nth(1)).collect();

  assert_eq!(earlier_code, 0, "{earlier_said}");
  assert_eq!(later_code, 0, "{later_said}");
  assert!(
    later_said
      .lines()
      .any(|line| line.contains(words_of(&DEPLOY.run.root_waiting)) && line.contains(&short(&earlier))),
    "{later_said}"
  );
  assert_eq!(laid[laid.len() - 2..], [earlier.as_str(), later.as_str()], "{log}");
}

// "A deployment whose turn comes lays down the newest commit asked for behind it ... the repository is checked,
// built and restarted once for all of them, and each deployment it took the commit of ends at its own turn on the
// commit already live"
#[test]
fn a_deployment_whose_turn_comes_lays_down_the_commit_a_waiting_one_asks_for_that_carries_its_own() {
  let scene = Scene::new();

  assert_eq!(scene.run(&Hands::default()), 0);

  scene.declare("v2", "true");

  let earlier = scene.push("second");

  scene.declare("v3", "true");

  let later = scene.push("third");
  let repo = Repository::read(&scene.source).unwrap();
  let root = RootTurn {
    poll: Some(Duration::from_millis(5)),
    ..RootTurn::under(&DeployPlaces::of(&repo, &scene.env()).unwrap().deploy_root)
  };
  let held = root.take(&earlier_ticket(), &|_| {});
  let asking = |commit: &str| DeployAsk {
    commit: Some(commit.to_owned()),
    root_poll: Some(Duration::from_millis(5)),
    ..DeployAsk::default()
  };
  let (ask_earlier, ask_later) = (asking(&earlier), asking(&later));
  let (told_earlier, heard_earlier) = std::sync::mpsc::channel();
  let (told_later, heard_later) = std::sync::mpsc::channel();
  let given_up = Instant::now() + Duration::from_secs(60);
  let scene = &scene;

  let ((earlier_code, earlier_said), (later_code, later_said)) = std::thread::scope(|scope| {
    let deploying_earlier = scope.spawn(move || {
      let hands = Hands {
        told: Some(told_earlier),
        ..Hands::default()
      };

      (scene.run_with(&ask_earlier, &Told(true), &hands, &scene.env()), said(&hands))
    });

    heard_until(&heard_earlier, &[words_of(&DEPLOY.run.root_waiting)], given_up);

    let deploying_later = scope.spawn(move || {
      let hands = Hands {
        told: Some(told_later),
        ..Hands::default()
      };

      (scene.run_with(&ask_later, &Told(true), &hands, &scene.env()), said(&hands))
    });

    // The turn is let go once both wait for it, the earlier one first.
    heard_until(&heard_later, &[words_of(&DEPLOY.run.root_waiting)], given_up);
    drop(held);

    (deploying_earlier.join().unwrap(), deploying_later.join().unwrap())
  });
  let log = std::fs::read_to_string(scene.at("live/deployed.log")).unwrap();
  let laid: Vec<&str> = log.lines().filter_map(|line| line.split_whitespace().nth(1)).collect();

  assert_eq!(earlier_code, 0, "{earlier_said}");
  assert_eq!(later_code, 0, "{later_said}");
  assert!(earlier_said.contains(words_of(&DEPLOY.run.joined)), "{earlier_said}");
  assert!(later_said.contains("is already live, so nothing is laid down again"), "{later_said}");
  // One release: the later commit alone was laid down after the first deploy.
  assert_eq!(laid.last().copied(), Some(later.as_str()), "{log}");
  assert!(!laid.contains(&earlier.as_str()), "{log}");
}

impl Scene {
  // A deployment of the commit given that failed in its gate, written into the scene's record as ending now.
  fn failed_in_gate(&self, commit: &str) {
    let repo = Repository::read(&self.source).unwrap();
    let now = bmo_testbed_core::queue::now_ms();
    let failed = Deployment {
      id: 0,
      repository_name: Some(repo.name()),
      unit_name: Some("bmo-deploy-1".to_owned()),
      pid: 1,
      commit: Some(commit.to_owned()),
      state: DeploymentState::Failed,
      plan: vec![],
      stages: vec![],
      steps: vec![],
      rolled_back: false,
      stopped_for: Some(StopCode::Check),
      reason: Some("the suite failed: a.test.ts".to_owned()),
      started_at: now,
      ended_at: Some(now),
    };

    Record::open(&self.at("testbed-data")).unwrap().write_deployment(&failed).unwrap();
  }

  // The deploy root's turn of the scene's repository, looked at every few milliseconds.
  fn root_turn(&self) -> RootTurn {
    let repo = Repository::read(&self.source).unwrap();

    RootTurn {
      poll: Some(Duration::from_millis(5)),
      ..RootTurn::under(&DeployPlaces::of(&repo, &self.env()).unwrap().deploy_root)
    }
  }

  // The commits laid down, in the order they went live.
  fn laid(&self) -> Vec<String> {
    let log = std::fs::read_to_string(self.at("live/deployed.log")).unwrap();

    log
      .lines()
      .filter_map(|line| line.split_whitespace().nth(1))
      .map(str::to_owned)
      .collect()
  }
}

// An ask of the commit given against the scene's record, its deploy root's turn looked at every few milliseconds.
fn asked_of(scene: &Scene, commit: &str) -> DeployAsk {
  DeployAsk {
    commit: Some(commit.to_owned()),
    record: Some(scene.at("testbed-data")),
    root_poll: Some(Duration::from_millis(5)),
    ..DeployAsk::default()
  }
}

// "A waiting commit whose newest ended deployment in the testbed's record failed in its gate is not taken ... its
// own deployment answers for it at its own turn"
#[test]
fn a_deployment_whose_turn_comes_does_not_take_a_waiting_commit_whose_gate_failed() {
  let scene = Scene::new();

  assert_eq!(scene.run(&Hands::default()), 0);

  scene.declare("v2", "true");

  let earlier = scene.push("second");

  scene.declare("v3", "true");

  let later = scene.push("third");

  scene.failed_in_gate(&later);

  let held = scene.root_turn().take(&earlier_ticket(), &|_| {});
  let (ask_earlier, ask_later) = (asked_of(&scene, &earlier), asked_of(&scene, &later));
  let (told_earlier, heard_earlier) = std::sync::mpsc::channel();
  let (told_later, heard_later) = std::sync::mpsc::channel();
  let given_up = Instant::now() + Duration::from_secs(60);
  let scene = &scene;

  let ((earlier_code, earlier_said), (later_code, later_said)) = std::thread::scope(|scope| {
    let deploying_earlier = scope.spawn(move || {
      let hands = Hands {
        told: Some(told_earlier),
        ..Hands::default()
      };

      (scene.run_with(&ask_earlier, &Told(true), &hands, &scene.env()), said(&hands))
    });

    heard_until(&heard_earlier, &[words_of(&DEPLOY.run.root_waiting)], given_up);

    let deploying_later = scope.spawn(move || {
      let hands = Hands {
        told: Some(told_later),
        ..Hands::default()
      };

      (scene.run_with(&ask_later, &Told(true), &hands, &scene.env()), said(&hands))
    });

    heard_until(&heard_later, &[words_of(&DEPLOY.run.root_waiting)], given_up);
    drop(held);

    (deploying_earlier.join().unwrap(), deploying_later.join().unwrap())
  });
  let laid = scene.laid();

  assert_eq!(earlier_code, 0, "{earlier_said}");
  assert_eq!(later_code, 0, "{later_said}");
  assert!(!earlier_said.contains(words_of(&DEPLOY.run.joined)), "{earlier_said}");
  assert_eq!(laid[laid.len() - 2..], [earlier.as_str(), later.as_str()]);
}

// "A commit whose gate failed while the deployment waited ends it at its turn ... it refuses with 2, naming that
// deployment"
#[test]
fn a_deployment_whose_commit_failed_in_its_gate_while_it_waited_ends_refused_before_its_gate() {
  let scene = Scene::new();

  assert_eq!(scene.run(&Hands::default()), 0);

  scene.declare("v2", "true");

  let commit = scene.push("second");
  let held = scene.root_turn().take(&earlier_ticket(), &|_| {});
  let ask = asked_of(&scene, &commit);
  let (told, heard) = std::sync::mpsc::channel();
  let given_up = Instant::now() + Duration::from_secs(60);
  let scene = &scene;

  let (code, lines) = std::thread::scope(|scope| {
    let deploying = scope.spawn(move || {
      let hands = Hands {
        told: Some(told),
        ..Hands::default()
      };

      (scene.run_with(&ask, &Told(true), &hands, &scene.env()), said(&hands))
    });

    heard_until(&heard, &[words_of(&DEPLOY.run.root_waiting)], given_up);
    scene.failed_in_gate(&commit);
    drop(held);
    deploying.join().unwrap()
  });

  assert_eq!(code, 2, "{lines}");
  assert!(lines.contains(words_of(&DEPLOY.run.failed_waiting)), "{lines}");
  assert!(lines.contains("bmo-deploy-1"), "{lines}");
  assert!(!scene.laid().contains(&commit), "{lines}");
}

// "Once it has picked its commit, a deployment reads its repository through the repository's primary checkout":
// a job's checkout removed while its own deployment runs its gate, and another's removed while its deployment
// waits behind that one for its deploy root's turn, leave both to go live in turn.
#[test]
fn two_deployments_of_one_repository_asked_back_to_back_from_checkouts_removed_meanwhile_both_go_live() {
  let scene = Scene::new();
  let first_job = scene.job_checkout("job-1");
  let earlier = must(&["rev-parse", "HEAD"], &first_job);

  scene.declare("v2", "true");

  let later = scene.push("second");
  let second_job = scene.job_checkout("job-2");
  let ask = DeployAsk {
    root_poll: Some(Duration::from_millis(5)),
    ..DeployAsk::default()
  };
  let (gate, has_begun, go) = held_gate();
  let (told, heard) = std::sync::mpsc::channel();
  let given_up = Instant::now() + Duration::from_secs(60);
  let (scene, ask, gate) = (&scene, &ask, &gate);
  let (first, second) = (&first_job, &second_job);

  let ((earlier_code, earlier_said), (later_code, later_said)) = std::thread::scope(|scope| {
    let deploying_earlier = scope.spawn(move || {
      let hands = Hands::default();

      (scene.run_from(first, ask, gate, &hands), said(&hands))
    });

    let _ = has_begun.recv_timeout(given_up.saturating_duration_since(Instant::now()));

    let deploying_later = scope.spawn(move || {
      let hands = Hands {
        told: Some(told),
        ..Hands::default()
      };

      (scene.run_from(second, ask, &Told(true), &hands), said(&hands))
    });

    heard_until(&heard, &[words_of(&DEPLOY.run.root_waiting)], given_up);

    for job in [first, second] {
      must(&["worktree", "remove", "--force", &job.to_string_lossy()], &scene.source);
    }

    drop(go);
    (deploying_earlier.join().unwrap(), deploying_later.join().unwrap())
  });
  let log = std::fs::read_to_string(scene.at("live/deployed.log")).unwrap();
  let laid: Vec<&str> = log.lines().filter_map(|line| line.split_whitespace().nth(1)).collect();

  assert_eq!(earlier_code, 0, "{earlier_said}");
  assert_eq!(later_code, 0, "{later_said}");
  assert!(later_said.contains(words_of(&DEPLOY.run.root_waiting)), "{later_said}");
  assert_eq!(laid, [earlier.as_str(), later.as_str()], "{log}");
}

// A deployment asked from a job's checkout waits in the line behind another repository's deployment while that
// checkout is removed, and goes live once its turn comes.
#[test]
fn a_deployment_behind_another_repositorys_in_the_line_goes_live_with_its_checkout_removed_meanwhile() {
  let scene = Scene::new();
  let other = Scene::new();
  let job = scene.job_checkout("job");
  let line = line_of(&scene, Duration::from_secs(3600));
  let (gate, has_begun, go) = held_gate();
  let (told, heard) = std::sync::mpsc::channel();
  let given_up = Instant::now() + Duration::from_secs(60);
  let ask = DeployAsk {
    line: Some(line),
    ..DeployAsk::default()
  };
  let (scene, other, ask, gate, job) = (&scene, &other, &ask, &gate, &job);

  let ((other_code, other_said), (code, lines)) = std::thread::scope(|scope| {
    let deploying_other = scope.spawn(move || {
      let hands = Hands::default();

      (other.run_from(&other.source, ask, gate, &hands), said(&hands))
    });

    let _ = has_begun.recv_timeout(given_up.saturating_duration_since(Instant::now()));

    let deploying = scope.spawn(move || {
      let hands = Hands {
        told: Some(told),
        ..Hands::default()
      };

      (scene.run_from(job, ask, &Told(true), &hands), said(&hands))
    });

    heard_until(&heard, &[words_of(&DEPLOY.run.line_waiting)], given_up);
    must(&["worktree", "remove", "--force", &job.to_string_lossy()], &scene.source);
    drop(go);
    (deploying_other.join().unwrap(), deploying.join().unwrap())
  });

  assert_eq!(other_code, 0, "{other_said}");
  assert_eq!(code, 0, "{lines}");
  assert!(lines.contains(words_of(&DEPLOY.run.line_waiting)), "{lines}");
  assert!(
    lines.contains(&fill_marks(&DEPLOY.run.primary, &[("checkout", &scene.source.to_string_lossy())])),
    "{lines}"
  );
  assert!(scene.deploy_dir().exists(), "{lines}");
}

// "A deployment is under way where ... the testbed's record holds it for the repository without an end, and its
// process lives"
#[test]
fn a_release_the_testbeds_record_holds_under_way_outlasts_a_later_deployments_prune() {
  let scene = Scene::new();
  let first = must(&["rev-parse", "HEAD"], &scene.source);
  let ask = DeployAsk {
    record: Some(scene.at("testbed-data")),
    ..DeployAsk::default()
  };
  let deploy = |version: &str| {
    let hands = Hands::default();

    scene.declare(version, "true");
    scene.push(version);
    assert_eq!(scene.run_with(&ask, &Told(true), &hands, &scene.env()), 0, "{}", said(&hands));
  };

  assert_eq!(scene.run_with(&ask, &Told(true), &Hands::default(), &scene.env()), 0);
  deploy("v2");
  Record::open(&scene.at("testbed-data"))
    .unwrap()
    .write_deployment(&Deployment {
      id: 0,
      repository_name: Some("sample".to_owned()),
      unit_name: None,
      pid: i64::from(std::process::id()),
      commit: Some(first.clone()),
      state: DeploymentState::Running,
      plan: Vec::new(),
      stages: Vec::new(),
      steps: Vec::new(),
      rolled_back: false,
      stopped_for: None,
      reason: None,
      started_at: Machine.now_ms(),
      ended_at: None,
    })
    .unwrap();
  deploy("v3");

  assert!(release_dir(&scene.deploy_dir(), &first).exists());
}
