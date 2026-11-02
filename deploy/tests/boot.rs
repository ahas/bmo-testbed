//! The service coming up after a restart (docs/deploy.md, 「What the deploy run does, in order」, step 10):
//! the wait for the declared health until it answers or the unit gives up starting, systemd's account of the
//! unit, the reading of it running, and the lines the deploy output keeps where it did not come up.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_deploy::boot::{
  BootScene, HealthWait, HealthWatch, StayUp, UnitStatus, boot_report, boot_scene, stands_up, stay_report,
  unit_gave_up, unit_status, wait_for_health,
};
use bmo_testbed_deploy::run::Ran;

fn watch(command: &str, cwd: &Path) -> HealthWatch<'static> {
  HealthWatch {
    poll_ms: 20,
    check_every_ms: 0,
    ..HealthWatch::new(command, cwd)
  }
}

fn status(active: &str, sub: &str, pid: &str, restarts: &str) -> UnitStatus {
  UnitStatus {
    active: active.to_owned(),
    sub: sub.to_owned(),
    pid: pid.to_owned(),
    restarts: restarts.to_owned(),
    exit_status: "0".to_owned(),
  }
}

fn running(pid: &str, restarts: &str) -> Option<UnitStatus> {
  Some(status("active", "running", pid, restarts))
}

// The readings systemd gives one after another, the last given again once they run out.
fn readings(list: Vec<Option<UnitStatus>>) -> impl Fn() -> Option<UnitStatus> {
  let left = RefCell::new(VecDeque::from(list));

  move || {
    let mut left = left.borrow_mut();

    if left.len() > 1 {
      left.pop_front().flatten()
    } else {
      left.front().cloned().flatten()
    }
  }
}

#[tokio::test]
async fn the_wait_ends_at_the_first_answer_of_zero() {
  let dir = tempfile::tempdir().unwrap();
  let waited = wait_for_health(&watch("echo healthy", dir.path())).await;

  assert!(waited.answered && !waited.gave_up);
  assert_eq!(waited.asks, 1);
  assert_eq!(waited.last, "exit 0: healthy");
}

#[tokio::test]
async fn the_health_is_asked_again_until_it_answers() {
  let dir = tempfile::tempdir().unwrap();
  // Fails twice, then answers: each ask leaves a mark, and the third finds two.
  let line = "echo x >> asks; test $(wc -l < asks) -ge 3";
  let waited = wait_for_health(&watch(line, dir.path())).await;

  assert!(waited.answered);
  assert_eq!(waited.asks, 3);
}

#[tokio::test]
async fn an_ask_that_takes_its_time_is_waited_for_until_it_ends() {
  let dir = tempfile::tempdir().unwrap();
  let started = std::time::Instant::now();
  let waited = wait_for_health(&watch("sleep 1; echo late", dir.path())).await;

  assert!(waited.answered);
  assert_eq!(waited.asks, 1);
  assert_eq!(waited.last, "exit 0: late");
  assert!(started.elapsed() >= std::time::Duration::from_secs(1));
}

#[tokio::test]
async fn the_wait_ends_where_the_unit_gave_up_starting() {
  let dir = tempfile::tempdir().unwrap();
  let waited = wait_for_health(&HealthWatch {
    gave_up: Some(Box::new(|| true)),
    ..watch("echo 'not yet'; echo more; exit 1", dir.path())
  })
  .await;

  assert!(!waited.answered && waited.gave_up);
  assert_eq!(waited.asks, 1);
  assert_eq!(waited.last, "exit 1: not yet");
}

#[test]
fn systemd_is_read_one_property_to_a_line() {
  let shown = |_: &[String]| Ran {
    ok: true,
    out: "ActiveState=activating\nSubState=auto-restart\nMainPID=0\nNRestarts=4\nExecMainStatus=1"
      .to_owned(),
  };
  let read = unit_status(&shown, "app.service").unwrap();

  assert_eq!(
    read,
    UnitStatus {
      active: "activating".to_owned(),
      sub: "auto-restart".to_owned(),
      pid: "0".to_owned(),
      restarts: "4".to_owned(),
      exit_status: "1".to_owned(),
    }
  );
  assert!(!unit_gave_up(Some(&read), Some(&read)));

  let refused = |_: &[String]| Ran {
    ok: false,
    out: "no bus".to_owned(),
  };

  assert_eq!(unit_status(&refused, "app.service"), None);

  let partial = |_: &[String]| Ran {
    ok: true,
    out: "ActiveState=failed".to_owned(),
  };
  let read = unit_status(&partial, "app.service").unwrap();

  assert_eq!(read.pid, "?");
  assert!(unit_gave_up(None, Some(&read)));
  assert!(unit_gave_up(None, Some(&status("inactive", "dead", "0", "0"))));
  assert!(!unit_gave_up(None, None));
}

#[test]
fn a_unit_systemd_restarted_since_the_wait_began_gave_up_starting() {
  let first = status("activating", "start", "41", "0");

  assert!(!unit_gave_up(Some(&first), Some(&status("active", "running", "41", "0"))));
  assert!(unit_gave_up(Some(&first), Some(&status("activating", "auto-restart", "0", "1"))));
  assert!(!unit_gave_up(None, Some(&status("activating", "auto-restart", "0", "1"))));
}

#[test]
fn a_service_read_running_is_up_and_one_not_running_or_unread_is_not() {
  let words = &DEPLOY.boot;

  assert_eq!(stands_up(&readings(vec![running("41", "2")])), StayUp::Stayed);
  assert!(matches!(
    stands_up(&readings(vec![Some(status("failed", "failed", "0", "0"))])),
    StayUp::Fell { why, .. } if why == fill_shown(&words.unit_is, &[("active", &"failed"), ("sub", &"failed")])
  ));
  assert!(matches!(
    stands_up(&readings(vec![None])),
    StayUp::Fell { why, status: None } if why == words.unasked
  ));
}

// systemd and the journal stood in by one runner answering by the command it is handed.
fn machine(words: &[String]) -> Ran {
  match words[0].as_str() {
    "systemctl" => Ran {
      ok: true,
      out: "ActiveState=failed\nSubState=failed\nMainPID=0\nNRestarts=5\nExecMainStatus=101".to_owned(),
    },
    _ => Ran {
      ok: true,
      out: "2026-10-14T05:15:00+0000 host app[41]: panicked at the start\n\n".to_owned(),
    },
  }
}

#[test]
fn a_health_that_did_not_answer_is_told_with_the_unit_and_its_journal() {
  let wait = HealthWait {
    command: "curl -fs localhost:8080/health".to_owned(),
    answered: false,
    waited_ms: 12_400,
    asks: 7,
    last: "exit 7".to_owned(),
    gave_up: true,
  };
  let scene: BootScene = boot_scene(&machine, "app.service", 1_760_418_900_000, wait);
  let lines = boot_report(&scene);
  let words = &DEPLOY.boot;

  assert_eq!(
    lines[0],
    fill_shown(
      &words.waited,
      &[
        ("waited", &12),
        ("command", &"curl -fs localhost:8080/health"),
        ("asks", &7),
        ("last", &"exit 7"),
        ("gave_up", &fill_shown(&words.gave_up, &[("unit", &"app.service")])),
      ],
    )
  );
  assert_eq!(lines[1], failed_unit_line());
  assert_eq!(lines[2], fill_shown(&words.machine, &[("strain", &scene.strain)]));
  assert_eq!(
    lines[3],
    fill_shown(&words.journal, &[("unit", &"app.service"), ("lines", &DEPLOY.numbers.journal_lines)])
  );
  assert_eq!(lines[4], "  2026-10-14T05:15:00+0000 host app[41]: panicked at the start");
  assert_eq!(lines.len(), 5);
}

// The unit line the stand-in systemd's failed unit is told by.
fn failed_unit_line() -> String {
  fill_shown(
    &DEPLOY.boot.unit,
    &[
      ("unit", &"app.service"),
      ("active", &"failed"),
      ("sub", &"failed"),
      ("pid", &0),
      ("process", &""),
      ("restarts", &5),
      ("status", &101),
    ],
  )
}

#[test]
fn a_service_not_read_running_is_told_with_why() {
  let why = fill_shown(&DEPLOY.boot.unit_is, &[("active", &"failed"), ("sub", &"failed")]);
  let lines = stay_report(&machine, "app.service", 0, &why, None);

  assert_eq!(lines[0], fill_shown(&DEPLOY.boot.fell, &[("unit", &"app.service"), ("why", &why)]));
  assert_eq!(lines[1], failed_unit_line());
  assert_eq!(lines.len(), 4);
}
