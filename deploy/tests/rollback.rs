//! Rollback (docs/deploy.md, 「Rollback」): the deploy tree flipped back onto the release live before, the
//! unit links relinked, the record written back and the service started again, through the tree's own
//! hands on a real tree and a stand-in for the services; and what the deploy output and the events say.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_deploy::release::{flip, release_dir};
use bmo_testbed_deploy::rollback::{
  BootFailure, DeployTree, FailedDeploy, NotRolled, Rollback, ServiceHands, deploy_failure_notice,
  journal_command, roll_back, rolled_back_detail,
};
use bmo_testbed_deploy::tree::{DeployRecord, read_deploy_record, short};

use crate::stand::{git, mark_built, must};

struct Stand {
  root: tempfile::TempDir,
  deploy_dir: PathBuf,
  unit_dir: PathBuf,
  live: String,
  failed: String,
}

impl Stand {
  fn linked(&self) -> PathBuf {
    std::fs::read_link(&self.deploy_dir).unwrap()
  }

  fn record(&self) -> PathBuf {
    self.root.path().join("deployed.json")
  }

  fn tree<'a>(&self, said: &'a dyn Fn(&str)) -> DeployTree<'a> {
    DeployTree {
      deploy_dir: self.deploy_dir.clone(),
      record_path: self.record(),
      log_path: self.root.path().join("deployed.log"),
      unit_dir: self.unit_dir.clone(),
      git: &git,
      say: said,
    }
  }
}

// The live commit keeps its unit at the root; the failed one moved it under `units/`, and the deployment
// relinked the installed unit through the deploy tree to follow it. Each commit has a release of its own,
// built, and the deploy tree names the failed one.
fn stand() -> Stand {
  let root = tempfile::tempdir().unwrap();
  let source = root.path().join("source");
  let deploy_dir = root.path().join("deploy");
  let unit_dir = root.path().join("unit-dir");

  std::fs::create_dir_all(&source).unwrap();
  std::fs::create_dir_all(&unit_dir).unwrap();
  std::fs::create_dir_all(root.path().join("data")).unwrap();
  must(&["init", "--quiet", "-b", "main"], &source);
  std::fs::write(source.join(".gitignore"), "bin/\n").unwrap();
  std::fs::write(source.join("app.service"), "[Service]\n").unwrap();
  must(&["add", "."], &source);
  must(&["commit", "--quiet", "-m", "live"], &source);

  let live = must(&["rev-parse", "HEAD"], &source);

  std::fs::create_dir_all(source.join("units")).unwrap();
  must(&["mv", "app.service", "units/app.service"], &source);
  must(&["commit", "--quiet", "-m", "failed"], &source);

  let failed = must(&["rev-parse", "HEAD"], &source);

  for commit in [&live, &failed] {
    let release = release_dir(&deploy_dir, commit);

    must(&["worktree", "add", "--quiet", "--detach", &release.to_string_lossy(), commit], &source);
    mark_built(&release, commit);
  }

  flip(&deploy_dir, &release_dir(&deploy_dir, &failed)).unwrap();
  std::os::unix::fs::symlink(deploy_dir.join("units/app.service"), unit_dir.join("app.service")).unwrap();

  Stand {
    root,
    deploy_dir,
    unit_dir,
    live,
    failed,
  }
}

// The services, answering as a test sets them and keeping what they were asked.
struct Services {
  stops: bool,
  restarts: bool,
  answers: bool,
  steps: RefCell<Vec<&'static str>>,
  asked: Cell<u32>,
  said: RefCell<Vec<String>>,
}

impl Services {
  fn new() -> Services {
    Services {
      stops: true,
      restarts: true,
      answers: true,
      steps: RefCell::new(vec![]),
      asked: Cell::new(0),
      said: RefCell::new(vec![]),
    }
  }
}

impl ServiceHands for Services {
  fn stop(&self) -> bool {
    self.steps.borrow_mut().push("stop");
    self.stops
  }

  fn restart(&self) -> bool {
    self.steps.borrow_mut().push("restart");
    self.restarts
  }

  async fn answers(&self) -> bool {
    self.asked.set(self.asked.get() + 1);
    self.answers
  }

  fn say(&self, line: &str) {
    self.said.borrow_mut().push(line.to_owned());
  }

  fn now_iso(&self) -> String {
    "2026-09-25T07:00:00.000Z".to_owned()
  }
}

fn failure(live: &str, failed: &str) -> FailedDeploy {
  FailedDeploy {
    commit: failed.to_owned(),
    failure: BootFailure::NotAnswering,
    previous: Some(DeployRecord {
      commit: live.to_owned(),
      subject: "live".to_owned(),
      message: "the live reason".to_owned(),
      version: "1.0.0".to_owned(),
      at: "2026-09-25T05:00:00.000Z".to_owned(),
      backup: "old-backup".to_owned(),
    }),
    written: DeployRecord {
      commit: failed.to_owned(),
      subject: "failed".to_owned(),
      message: "the failed reason".to_owned(),
      version: "1.1.0".to_owned(),
      at: "2026-09-25T06:59:00.000Z".to_owned(),
      backup: "new-backup".to_owned(),
    },
    before_start: false,
    unit: "app.service".to_owned(),
  }
}

fn quiet(_: &str) {}

#[tokio::test]
async fn a_failed_deployment_is_flipped_back_relinked_recorded_and_restarted() {
  let at = stand();
  let services = Services::new();
  let failed = failure(&at.live, &at.failed);
  let outcome = roll_back(&failed, &mut at.tree(&quiet), &services).await;

  assert_eq!(
    outcome,
    Rollback::RolledBack {
      to: at.live.clone(),
      answers: true,
    }
  );
  assert_eq!(*services.steps.borrow(), ["stop", "restart"]);
  assert_eq!(services.asked.get(), 1);
  assert_eq!(at.linked(), Path::new("deploy-releases").join(short(&at.live)));
  assert_eq!(
    *services.said.borrow(),
    [
      fill_shown(&DEPLOY.rolling.back_to, &[("commit", &short(&at.live))]),
      fill_shown(&DEPLOY.rolling.answers, &[("commit", &short(&at.live))]),
    ]
  );

  // The installed unit follows the file back to where the live commit keeps it.
  assert_eq!(
    std::fs::read_link(at.unit_dir.join("app.service")).unwrap(),
    at.deploy_dir.join("app.service")
  );

  // The record names the live commit again and keeps the backup the failed deployment took.
  let record = read_deploy_record(&at.record()).unwrap();

  assert_eq!(
    record,
    DeployRecord {
      commit: at.live.clone(),
      subject: "live".to_owned(),
      message: "the live reason".to_owned(),
      version: "1.0.0".to_owned(),
      at: "2026-09-25T07:00:00.000Z".to_owned(),
      backup: "new-backup".to_owned(),
    }
  );
  assert_eq!(
    std::fs::read_to_string(at.root.path().join("deployed.log")).unwrap(),
    format!(
      "2026-09-25T07:00:00.000Z {} new-backup rollback of {}: live\n",
      at.live,
      short(&at.failed)
    )
  );

  let notice = deploy_failure_notice(&failed, &outcome);
  let lines: Vec<&str> = notice.lines().collect();

  assert_eq!(
    lines[0],
    format!(
      "deployment failed: the service on {} gave up starting before its health answered.",
      short(&at.failed)
    )
  );
  assert_eq!(
    lines[1],
    format!("Rolled back to the earlier release {}, and the service answers again.", short(&at.live))
  );
  assert_eq!(lines[2], "Cause: journalctl --user -u app.service -n 40");
}

#[tokio::test]
async fn a_service_silent_on_the_release_before_too_is_said_to_be_down() {
  let at = stand();
  let services = Services {
    answers: false,
    stops: false,
    ..Services::new()
  };
  let failed = failure(&at.live, &at.failed);
  let outcome = roll_back(&failed, &mut at.tree(&quiet), &services).await;

  assert_eq!(
    outcome,
    Rollback::RolledBack {
      to: at.live.clone(),
      answers: false,
    }
  );
  assert_eq!(
    *services.said.borrow(),
    [
      fill_shown(&DEPLOY.rolling.back_to, &[("commit", &short(&at.live))]),
      DEPLOY.rolling.would_not_stop.clone(),
      fill_shown(&DEPLOY.rolling.silent, &[("commit", &short(&at.live))]),
    ]
  );
  assert!(
    deploy_failure_notice(&failed, &outcome).contains("but the service still does not answer; it is down.")
  );
}

#[tokio::test]
async fn a_restart_refused_on_the_release_before_asks_no_health() {
  let at = stand();
  let services = Services {
    restarts: false,
    ..Services::new()
  };
  let outcome = roll_back(&failure(&at.live, &at.failed), &mut at.tree(&quiet), &services).await;

  assert_eq!(
    outcome,
    Rollback::RolledBack {
      to: at.live.clone(),
      answers: false,
    }
  );
  assert_eq!(services.asked.get(), 0);
  assert_eq!(at.linked(), Path::new("deploy-releases").join(short(&at.live)));
}

#[tokio::test]
async fn a_deployment_with_a_command_before_the_start_is_never_rolled_back_by_itself() {
  let at = stand();
  let services = Services::new();
  let failed = FailedDeploy {
    before_start: true,
    ..failure(&at.live, &at.failed)
  };
  let outcome = roll_back(&failed, &mut at.tree(&quiet), &services).await;

  assert!(matches!(
    &outcome,
    Rollback::NotRolled {
      why: NotRolled::BeforeStart,
      detail,
    } if *detail == DEPLOY.rolling.before_start
  ));
  assert!(services.steps.borrow().is_empty());
  assert_eq!(at.linked(), Path::new("deploy-releases").join(short(&at.failed)));

  let notice = deploy_failure_notice(&failed, &outcome);

  assert!(notice.lines().nth(1).unwrap().ends_with("The service is down."), "{notice}");
  assert_eq!(rolled_back_detail(&failed, &outcome), None);
}

#[tokio::test]
async fn a_first_deployment_has_nothing_to_go_back_to() {
  let at = stand();
  let services = Services::new();

  for previous in [None, Some(failure(&at.live, &at.failed).written)] {
    let failed = FailedDeploy {
      previous,
      ..failure(&at.live, &at.failed)
    };
    let outcome = roll_back(&failed, &mut at.tree(&quiet), &services).await;

    assert!(matches!(
      &outcome,
      Rollback::NotRolled {
        why: NotRolled::First,
        detail,
      } if *detail == DEPLOY.rolling.first
    ));
    assert!(
      deploy_failure_notice(&failed, &outcome)
        .contains("No release was live before, so nothing was rolled back. The service is down.")
    );
  }

  assert!(services.steps.borrow().is_empty());
}

#[tokio::test]
async fn a_release_before_that_is_gone_or_unfinished_is_not_flipped_onto() {
  let at = stand();
  let live_release = release_dir(&at.deploy_dir, &at.live);

  std::fs::remove_file(live_release.join("bin/built-from")).unwrap();

  let services = Services::new();
  let failed = failure(&at.live, &at.failed);
  let outcome = roll_back(&failed, &mut at.tree(&quiet), &services).await;

  let Rollback::NotRolled {
    why: NotRolled::Tree,
    detail,
  } = &outcome
  else {
    panic!("{outcome:?}");
  };

  assert_eq!(
    *detail,
    fill_shown(
      &DEPLOY.rolling.not_built,
      &[("commit", &short(&at.live)), ("release", &live_release.display())]
    )
  );
  assert_eq!(*services.steps.borrow(), ["stop"]);
  assert_eq!(at.linked(), Path::new("deploy-releases").join(short(&at.failed)));
  assert!(deploy_failure_notice(&failed, &outcome).contains("The deploy tree could not be rolled back: "));
}

#[tokio::test]
async fn a_service_that_answered_and_was_not_running_is_told_so() {
  let at = stand();
  let failed = FailedDeploy {
    failure: BootFailure::DiedAfterStart,
    ..failure(&at.live, &at.failed)
  };
  let outcome = roll_back(&failed, &mut at.tree(&quiet), &Services::new()).await;

  assert_eq!(
    deploy_failure_notice(&failed, &outcome).lines().next().unwrap(),
    format!(
      "deployment failed: the service on {} answered and was not running when it was read.",
      short(&at.failed)
    )
  );

  let refused = FailedDeploy {
    failure: BootFailure::RestartRefused,
    ..failure(&at.live, &at.failed)
  };

  assert_eq!(
    deploy_failure_notice(&refused, &outcome).lines().next().unwrap(),
    format!("deployment failed: the service could not be restarted on {}.", short(&at.failed))
  );
}

#[test]
fn the_rolled_back_event_names_both_commits_and_the_journal() {
  let failed = failure(&"a".repeat(40), &"b".repeat(40));
  let outcome = Rollback::RolledBack {
    to: "a".repeat(40),
    answers: true,
  };
  let detail = rolled_back_detail(&failed, &outcome).unwrap();

  assert_eq!(detail["failed_commit"], "b".repeat(40));
  assert_eq!(detail["back_to"], "a".repeat(40));
  assert_eq!(detail["answers"], true);
  assert_eq!(detail["journal"], journal_command(&failed));
  assert_eq!(
    detail["reason"],
    "rolled back after a failed deployment: the service on bbbbbbbbbbbb did not come up, so it is back on the \
     earlier release aaaaaaaaaaaa"
  );
}
