//! A deployment followed through the testbed's record (docs/deploy.md, 「The hosted deploy tool」): its end
//! read off the record's deployment events, a call attached to a deployment already under way or asked to
//! attach, a read that fails taken as no outcome, and the answer each ending makes. The restarts the
//! deployment outlives are made, not read: the executable ended mid-call as its tool host ends it at a stop
//! and started anew, and the record cut away from a call that follows it and put back.

use std::io::Write as _;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{LazyLock, mpsc};
use std::time::{Duration, Instant, SystemTime};

use bmo_testbed_client::{Deployment, DeploymentState, StopCode};
use bmo_testbed_core::turn::Env;
use bmo_testbed_deploy::tree::DeployRecord;
use bmo_testbed_record::{Record, record_path};
use deploy_tool::{Answer, Arguments, Begun, Course, Ground, deploy, recorded};
use serde_json::{Value, json};

use crate::stand::{
  COMMIT, HANDED_OFF, call, checkout, env, logged, record, record_path as deploy_record_path, registry,
  script, testbed, text, write,
};

const UNIT: &str = "bmo-deploy-1793328193818";

// The moment this test binary's deployments start from: now, so the record keeps them, since it takes away
// deployments older than its policy's days as it is written.
static NOW_MS: LazyLock<i64> = LazyLock::new(|| {
  let since = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();

  i64::try_from(since.as_millis()).unwrap()
});

// A deployment of the repository `agency` at the commit, in the unit, run by this test's own process so the
// record reads it as running while it is; `nth` tells one run from another, which the record knows by its
// process and its start.
fn deployment(state: DeploymentState, nth: i64) -> Deployment {
  Deployment {
    id: 0,
    repository_name: Some("bmo-agency".to_owned()),
    unit_name: Some(UNIT.to_owned()),
    pid: i64::from(std::process::id()),
    commit: Some(COMMIT.to_owned()),
    state,
    plan: Vec::new(),
    stages: Vec::new(),
    steps: Vec::new(),
    rolled_back: false,
    stopped_for: None,
    reason: None,
    started_at: *NOW_MS + nth,
    ended_at: None,
  }
}

fn failed_in_build() -> Deployment {
  Deployment {
    stopped_for: Some(StopCode::Build),
    reason: Some("the release build failed".to_owned()),
    ..deployment(DeploymentState::Failed, 1)
  }
}

// What a call stands on: a checkout of `agency` in the registry, the testbed binary's stand-in, a unit the
// service manager holds active for as long as the test runs, and a journal of one line.
struct Stand {
  dir: tempfile::TempDir,
  bin: PathBuf,
  registry: PathBuf,
}

impl Stand {
  fn new(verb: &str) -> Self {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    let root = checkout(dir.path(), "agency", "bmo-agency");
    let registry = registry(dir.path(), &[&root]);

    testbed(&bin, verb);
    // Each time the unit is asked of is written down.
    script(
      &bin,
      "systemctl",
      "mkdir -p \"$STAND_LOG\"\necho x >> \"$STAND_LOG/asked\"\nif [ -e \"$STAND_LOG/ended\" ]; then echo inactive; exit 3; fi\necho active",
    );
    script(&bin, "journalctl", "echo 'build: cargo failed'");

    Self { dir, bin, registry }
  }

  fn ground(&self) -> Ground {
    Ground {
      env: env(self.dir.path(), &self.bin, &self.registry),
      testbed: self.bin.join("bmo-testbed"),
      poll: Duration::from_millis(10),
      asker: None,
    }
  }

  fn record(&self) -> PathBuf {
    record_path(&self.dir.path().join("data"))
  }

  fn write(&self, deployment: &Deployment) {
    Record::open_file(&self.record()).unwrap().write_deployment(deployment).unwrap();
  }

  // Whether the service manager's stand-in has been asked of the unit.
  fn asked(&self) -> bool {
    !logged(self.dir.path(), "asked").is_empty()
  }

  // Ends a call the test started, however the test ends, its panics included: from then on the unit answers
  // inactive, so a call still asking of it stops following within one turn.
  fn ending(&self) -> Ending<'_> {
    Ending(self)
  }

  // Whether the testbed binary's stand-in ran.
  fn ran(&self) -> bool {
    !logged(self.dir.path(), "words").is_empty()
  }
}

struct Ending<'a>(&'a Stand);

impl Drop for Ending<'_> {
  fn drop(&mut self) {
    write(&self.0.dir.path().join("log").join("ended"), "");
  }
}

// A call made in this process on a thread of its own, never joined: its answer is awaited no longer than a
// deadline, so a call that never ends fails the test instead of holding it, and goes with the test binary.
struct Pending {
  answered: mpsc::Receiver<Answer>,
  answer: Option<Answer>,
}

impl Pending {
  fn start(ground: &Ground, arguments: Arguments) -> Self {
    let (sender, answered) = mpsc::channel();
    let ground = ground.clone();

    std::thread::spawn(move || {
      let _ = sender.send(deploy(&arguments, &ground));
    });

    Self {
      answered,
      answer: None,
    }
  }

  // Whether the call has answered.
  fn ended(&mut self) -> bool {
    if self.answer.is_none() {
      self.answer = self.answered.try_recv().ok();
    }

    self.answer.is_some()
  }

  // Its answer, awaited no longer than `until`'s deadline.
  fn answer(mut self) -> Answer {
    until(|| self.ended());
    self.answer.take().unwrap()
  }
}

fn asked(attach: Option<bool>) -> Arguments {
  Arguments {
    repository: "agency".to_owned(),
    commit: "0123456".to_owned(),
    why: "Ship the queue fix".to_owned(),
    dry_run: None,
    attach,
    first: None,
  }
}

// Waits, bounded, for `ready` to hold. The bound is a failure's and never a wait: a condition that holds ends
// the wait at once, and the deployment gate's loaded machine takes seconds to start each stand-in a turn asks.
fn until(mut ready: impl FnMut() -> bool) {
  let deadline = Instant::now() + Duration::from_secs(60);

  while !ready() {
    assert!(Instant::now() < deadline, "the condition never held");
    std::thread::sleep(Duration::from_millis(5));
  }
}

// "the deployment the verb started is followed through the record's events to the end the record holds,
// while its unit is still active"
#[test]
fn the_verbs_deployment_is_followed_to_the_end_the_record_holds() {
  let stand = Stand::new(&format!("echo '{HANDED_OFF}'"));
  let ground = stand.ground();

  // A deployment the record held before the call began is never taken for the call's own.
  stand.write(&Deployment {
    unit_name: Some("bmo-deploy-1".to_owned()),
    ..deployment(DeploymentState::Live, 0)
  });

  let answer = {
    let _ending = stand.ending();
    let call = Pending::start(&ground, asked(None));

    until(|| stand.ran());
    stand.write(&deployment(DeploymentState::Running, 1));
    stand.write(&failed_in_build());
    call.answer()
  };
  let text = answer.plain();

  assert!(answer.is_error);
  assert!(
    text.starts_with(&format!(
      "not live: deployment 2 in {UNIT} ended failed, stopped in build: the release build failed"
    )),
    "{text}"
  );
  assert!(text.ends_with("The last lines of its journal:\nbuild: cargo failed"), "{text}");
}

// "a deployment the record holds as live answers its commit, its subject and reason from the deployment
// record, the commit live before the call where the call started it, and its unit and id"
#[test]
fn a_live_deployment_answers_its_commit_and_where_it_ran() {
  let live = Deployment {
    id: 7,
    ..deployment(DeploymentState::Live, 1)
  };
  let after: DeployRecord =
    serde_json::from_str(&record(COMMIT, "Fix the queue", "Ship it", "2026-10-30T10:00:00")).unwrap();
  let other: DeployRecord = serde_json::from_str(&record(
    "fedcba9876543210fedcba9876543210fedcba98",
    "Later work",
    "Then",
    "2026-10-31T10:00:00",
  ))
  .unwrap();
  let journal = || unreachable!();

  assert_eq!(
    recorded(&live, Begun::Started(None), Some(&after), &journal).plain(),
    format!(
      "live: {COMMIT} (Fix the queue)\nprevious: none recorded\nmessage: Ship it\nunit: {UNIT}\ndeployment: 7"
    )
  );
  assert_eq!(
    recorded(&live, Begun::Attached, Some(&other), &journal).plain(),
    format!("live: {COMMIT}\nunit: {UNIT}\ndeployment: 7")
  );
}

// "a deployment whose process went without ending it is answered gone, with no stage and no reason"
#[test]
fn a_deployment_whose_process_went_is_answered_gone() {
  let stand = Stand::new("exit 0");
  let mut course = Course::before_start(&stand.record());

  course.run_in(UNIT);
  stand.write(&Deployment {
    // No process stands under this number.
    pid: i64::from(i32::MAX),
    ..deployment(DeploymentState::Running, 1)
  });
  course.read();

  let gone = course.ended().expect("a deployment whose process went has ended");
  let text = recorded(gone, Begun::Attached, None, &String::new).plain();

  assert_eq!(gone.state, DeploymentState::Gone);
  assert!(text.contains("ended gone, stopped in no stage named: its run gave no reason"), "{text}");
}

// "a record that cannot be read is no outcome: the next turn reads it again from the same number"
#[test]
fn a_record_that_cannot_be_read_is_read_again() {
  let dir = tempfile::tempdir().unwrap();
  let blocked = dir.path().join("record");
  let at: PathBuf = blocked.join("record.sqlite");

  // A file where the record's folder must stand, so the record cannot be opened.
  write(&blocked, "");

  let mut course = Course::before_start(&at);

  course.run_in(UNIT);
  course.read();
  assert!(course.deployment().is_none());

  std::fs::remove_file(&blocked).unwrap();
  Record::open_file(&at)
    .unwrap()
    .write_deployment(&deployment(DeploymentState::Running, 1))
    .unwrap();
  course.read();
  assert_eq!(course.deployment().map(|found| found.state), Some(DeploymentState::Running));
  assert!(course.ended().is_none());

  Record::open_file(&at).unwrap().write_deployment(&failed_in_build()).unwrap();
  course.read();
  assert_eq!(course.ended().map(|found| found.state), Some(DeploymentState::Failed));
}

// "a call of a commit whose deployment is still running follows it and runs no verb"
#[test]
fn a_deployment_under_way_is_followed_rather_than_started_again() {
  let stand = Stand::new(&format!("echo '{HANDED_OFF}'"));
  let ground = stand.ground();

  stand.write(&deployment(DeploymentState::Running, 1));

  let answer = {
    let _ending = stand.ending();
    let call = Pending::start(&ground, asked(None));

    // The call is following the deployment once it asks of its unit.
    until(|| stand.asked());
    stand.write(&Deployment {
      ended_at: Some(*NOW_MS + 2),
      ..deployment(DeploymentState::Live, 1)
    });
    call.answer()
  };

  assert!(!answer.is_error, "{}", answer.plain());
  assert_eq!(answer.plain().lines().next(), Some(format!("live: {COMMIT}").as_str()));
  assert!(!stand.ran());
}

// "a call with attach answers the newest deployment of the commit, and refuses where the record holds
// none, starting nothing either way"
#[test]
fn a_call_with_attach_answers_the_newest_deployment_of_the_commit() {
  let stand = Stand::new(&format!("echo '{HANDED_OFF}'"));
  let ground = stand.ground();

  // The unit answers inactive throughout, so no call of this test can follow it for longer than a turn.
  drop(stand.ending());

  let none = deploy(&asked(Some(true)), &ground);

  assert!(none.is_error);
  assert!(
    none.plain().contains("holds no deployment of bmo-agency at 0123456"),
    "{}",
    none.plain()
  );

  stand.write(&failed_in_build());
  // A newer deployment of another repository is passed over.
  stand.write(&Deployment {
    repository_name: Some("bmo-other".to_owned()),
    ..deployment(DeploymentState::Live, 2)
  });
  write(
    &deploy_record_path(stand.dir.path(), "agency"),
    &record(COMMIT, "Fix the queue", "Ship it", "2026-10-30T10:00:00"),
  );

  let answer = deploy(&asked(Some(true)), &ground);

  assert!(answer.is_error);
  assert!(answer.plain().starts_with("not live: deployment 1 in"), "{}", answer.plain());
  assert!(!stand.ran());
}

// A process of the executable handed one call, in a process group of its own as the tool host starts a call.
struct Call {
  child: Child,
  reaped: bool,
}

impl Call {
  fn start(request: &Value, env: &Env) -> Self {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deploy-tool"))
      .env_clear()
      .envs(env)
      .process_group(0)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn()
      .expect("the tool starts");

    child.stdin.take().unwrap().write_all(request.to_string().as_bytes()).unwrap();
    Self {
      child,
      reaped: false,
    }
  }

  // Its whole group ended by `SIGKILL`, as the tool host ends a call at its stop, and the process reaped.
  fn cut(&mut self) -> ExitStatus {
    end_group(self.child.id());
    self.reaped = true;
    self.child.wait().unwrap()
  }

  // Its answer, waited for no longer than `until`'s deadline.
  fn answer(mut self) -> Value {
    until(|| self.child.try_wait().unwrap().is_some());
    self.reaped = true;

    let mut out = String::new();

    std::io::Read::read_to_string(self.child.stdout.as_mut().unwrap(), &mut out).unwrap();
    serde_json::from_str(&out).unwrap_or_else(|why| panic!("{why}: {out}"))
  }
}

// A call the test did not end itself is ended with its group, however the test ends.
impl Drop for Call {
  fn drop(&mut self) {
    if !self.reaped {
      end_group(self.child.id());
      let _ = self.child.wait();
    }
  }
}

fn end_group(group: u32) {
  let _ = Command::new("kill").args(["-KILL", "--", &format!("-{group}")]).status();
}

// The process standing for a deployment's unit, which the deploy verb's stand-in starts in a session of its own
// as the service manager starts a unit apart from its caller; ended however the test ends.
struct Unit(u32);

impl Unit {
  fn alive(&self) -> bool {
    std::fs::read_to_string(format!("/proc/{}/stat", self.0))
      .is_ok_and(|stat| !stat.rsplit(')').next().unwrap_or_default().trim_start().starts_with('Z'))
  }
}

impl Drop for Unit {
  fn drop(&mut self) {
    let _ = Command::new("kill").args(["-KILL", &self.0.to_string()]).status();
  }
}

// "when its host stops, the call is ended with it; the deployment runs on in its own unit, and a call with
// attach, started anew, answers what came of it" (docs/deploy.md, 「The hosted deploy tool」)
#[test]
fn a_call_ended_at_its_hosts_stop_leaves_the_deployment_going_and_a_new_call_attaches_to_its_outcome() {
  let stand = Stand::new(&format!(
    "setsid sleep 300 </dev/null >/dev/null 2>&1 &\necho $! > \"$STAND_LOG/unit\"\necho x >> \"$STAND_LOG/runs\"\necho '{HANDED_OFF}'"
  ));
  let env = stand.ground().env;
  let mut first = Call::start(
    &call(json!({ "repository": "agency", "commit": "0123456", "why": "Ship the queue fix" })),
    &env,
  );

  until(|| !logged(stand.dir.path(), "unit").trim().is_empty());

  let unit = Unit(logged(stand.dir.path(), "unit").trim().parse().unwrap());
  let running = Deployment {
    pid: i64::from(unit.0),
    ..deployment(DeploymentState::Running, 1)
  };

  stand.write(&running);
  // The call follows the deployment once it asks of its unit.
  until(|| stand.asked());

  let ended = first.cut();

  assert_eq!(ended.signal(), Some(9), "{ended:?}");
  assert!(unit.alive(), "the deployment's unit went with the call");

  // The deployment goes live with no call following it.
  stand.write(&Deployment {
    state: DeploymentState::Live,
    ended_at: Some(*NOW_MS + 2),
    ..running
  });

  let answer = Call::start(
    &call(
      json!({ "repository": "agency", "commit": "0123456", "why": "Ship the queue fix", "attach": true }),
    ),
    &env,
  )
  .answer();
  let said = text(&answer);

  assert_eq!(answer["isError"], false, "{said}");
  assert_eq!(said.lines().next(), Some(format!("live: {COMMIT}").as_str()), "{said}");
  assert!(said.ends_with(&format!("unit: {UNIT}\ndeployment: 1")), "{said}");
  // The deploy verb ran for the first call alone.
  assert_eq!(logged(stand.dir.path(), "runs").lines().count(), 1);
}

// "a record that cannot be opened or read is read again at the next turn, from the same number": the record
// cut away while a call follows a deployment, the call goes on, and once the record is back it answers the end
// the record holds
#[test]
fn a_record_cut_while_a_call_follows_it_is_read_again_until_the_deployment_ends() {
  let stand = Stand::new(&format!("echo '{HANDED_OFF}'"));
  let ground = stand.ground();
  let folder = stand.record().parent().unwrap().to_path_buf();
  let away = folder.with_extension("away");
  let turns = || logged(stand.dir.path(), "asked").lines().count();

  let answer = {
    let _ending = stand.ending();
    let mut call = Pending::start(&ground, asked(None));

    until(|| stand.ran());
    stand.write(&deployment(DeploymentState::Running, 1));

    let written = turns();

    until(|| turns() >= written + 2);

    // The record cut: its folder taken away and a file standing where it stood, so it cannot be opened.
    std::fs::rename(&folder, &away).unwrap();
    write(&folder, "");

    let cut = turns();

    until(|| turns() >= cut + 3);
    assert!(!call.ended(), "a record that could not be read ended the call");

    std::fs::remove_file(&folder).unwrap();
    std::fs::rename(&away, &folder).unwrap();
    stand.write(&failed_in_build());
    call.answer()
  };

  assert!(answer.is_error);
  assert!(
    answer
      .plain()
      .starts_with(&format!("not live: deployment 1 in {UNIT} ended failed, stopped in build")),
    "{}",
    answer.plain()
  );
}

// "a resume from before the record's window reads the deployment by its id instead": the events of the
// deployment followed gone past the window before the course read them, the deployment is found by its unit
#[test]
fn a_resume_from_before_the_window_finds_the_deployment_by_its_unit() {
  let dir = tempfile::tempdir().unwrap();
  let at = dir.path().join("record").join("record.sqlite");
  let write = |deployment: &Deployment| {
    Record::open_file(&at).unwrap().write_deployment(deployment).unwrap();
  };
  let another = |nth: i64| Deployment {
    unit_name: Some("bmo-deploy-1".to_owned()),
    ..deployment(DeploymentState::Live, nth)
  };

  write(&another(0));

  let mut course = Course::before_start(&at);

  course.run_in(UNIT);
  write(&deployment(DeploymentState::Running, 1));
  write(&failed_in_build());

  // Every event so far aged past the window, so the next write takes them away.
  rusqlite::Connection::open(&at)
    .unwrap()
    .execute("UPDATE events SET at = 0", [])
    .unwrap();
  write(&another(2));
  course.read();

  let ended = course.ended().expect("the deployment is found past the window");

  assert_eq!((ended.id, ended.state), (2, DeploymentState::Failed));
}
