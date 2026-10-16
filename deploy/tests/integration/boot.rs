//! The service coming up after a restart (docs/deploy.md, 「What the deploy run does, in order」, step 10):
//! the wait for the declared health, systemd's account of the unit, the watch of it staying up, and the
//! lines the deploy output keeps where it did not.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;

use bmo_testbed_deploy::boot::{
  BootScene, HealthWait, HealthWatch, StayUp, UnitStatus, boot_report, boot_scene, stay_report, stays_up,
  unit_gave_up, unit_status, wait_for_health,
};
use bmo_testbed_deploy::run::Ran;

fn watch(command: &str, cwd: &Path, limit_ms: u64) -> HealthWatch<'static> {
  HealthWatch {
    ask_limit_ms: 2_000,
    poll_ms: 20,
    check_every_ms: 0,
    ..HealthWatch::new(command, cwd, limit_ms)
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
  let waited = wait_for_health(&watch("echo healthy", dir.path(), 5_000)).await;

  assert!(waited.answered && !waited.gave_up);
  assert_eq!(waited.asks, 1);
  assert_eq!(waited.last, "exit 0: healthy");
}

#[tokio::test]
async fn the_health_is_asked_again_until_it_answers() {
  let dir = tempfile::tempdir().unwrap();
  // Fails twice, then answers: each ask leaves a mark, and the third finds two.
  let line = "echo x >> asks; test $(wc -l < asks) -ge 3";
  let waited = wait_for_health(&watch(line, dir.path(), 5_000)).await;

  assert!(waited.answered);
  assert_eq!(waited.asks, 3);
}

#[tokio::test]
async fn a_wait_that_runs_out_keeps_what_the_last_ask_heard() {
  let dir = tempfile::tempdir().unwrap();
  let waited = wait_for_health(&watch("echo 'not yet'; echo more; exit 3", dir.path(), 150)).await;

  assert!(!waited.answered && !waited.gave_up);
  assert!(waited.asks > 1, "{waited:?}");
  assert!(waited.waited_ms >= 150);
  assert_eq!(waited.last, "exit 3: not yet");
}

#[tokio::test]
async fn an_ask_that_does_not_answer_in_time_counts_as_no_answer() {
  let dir = tempfile::tempdir().unwrap();
  let waited = wait_for_health(&HealthWatch {
    ask_limit_ms: 1_000,
    ..watch("sleep 30", dir.path(), 10)
  })
  .await;

  assert!(!waited.answered);
  assert_eq!(waited.asks, 1);
  assert_eq!(waited.last, "no answer within 1s");
}

#[tokio::test]
async fn the_wait_ends_where_the_unit_gave_up_starting() {
  let dir = tempfile::tempdir().unwrap();
  let waited = wait_for_health(&HealthWatch {
    gave_up: Some(Box::new(|| true)),
    ..watch("exit 1", dir.path(), 60_000)
  })
  .await;

  assert!(!waited.answered && waited.gave_up);
  assert_eq!(waited.asks, 1);
  assert_eq!(waited.last, "exit 1");
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
  assert!(!unit_gave_up(Some(&read)));

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
  assert!(unit_gave_up(Some(&read)));
  assert!(unit_gave_up(Some(&status("inactive", "dead", "0", "0"))));
  assert!(!unit_gave_up(None));
}

#[tokio::test(start_paused = true)]
async fn a_service_running_throughout_on_one_process_stayed_up() {
  let read = readings(vec![running("41", "2"), running("41", "2"), None, running("41", "2")]);

  assert_eq!(stays_up(3_000, 1_000, &read).await, StayUp::Stayed);
}

#[tokio::test(start_paused = true)]
async fn a_service_that_stops_changes_process_or_restarts_fell() {
  let went = readings(vec![running("41", "0"), Some(status("activating", "auto-restart", "0", "1"))]);

  assert!(matches!(
    stays_up(3_000, 1_000, &went).await,
    StayUp::Fell { why, .. } if why == "the unit went activating/auto-restart"
  ));

  let changed = readings(vec![running("41", "0"), running("42", "0")]);

  assert!(matches!(
    stays_up(3_000, 1_000, &changed).await,
    StayUp::Fell { why, .. } if why == "its main process changed from 41 to 42"
  ));

  let restarted = readings(vec![running("41", "1"), running("41", "3")]);

  assert!(matches!(
    stays_up(3_000, 1_000, &restarted).await,
    StayUp::Fell { why, .. } if why == "systemd restarted it 2 time(s)"
  ));
}

#[tokio::test(start_paused = true)]
async fn a_hold_with_no_reading_is_no_stay() {
  let not_running = readings(vec![Some(status("failed", "failed", "0", "0"))]);

  assert!(matches!(
    stays_up(3_000, 1_000, &not_running).await,
    StayUp::Fell { why, .. } if why == "the unit is failed/failed"
  ));

  let silent = readings(vec![None]);

  assert!(matches!(
    stays_up(3_000, 1_000, &silent).await,
    StayUp::Fell { why, status: None } if why == "systemd could not be asked for the unit"
  ));

  let quiet_after = readings(vec![running("41", "0"), None]);

  assert!(matches!(
    stays_up(3_000, 1_000, &quiet_after).await,
    StayUp::Fell { why, .. } if why == "systemd could not be asked for the unit during the hold"
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
    limit_ms: 300_000,
    answered: false,
    waited_ms: 12_400,
    asks: 7,
    last: "exit 7".to_owned(),
    gave_up: true,
  };
  let scene: BootScene = boot_scene(&machine, "app.service", 1_760_418_900_000, wait);
  let lines = boot_report(&scene);

  assert_eq!(
    lines[0],
    "waited 12s of 300s for the health `curl -fs localhost:8080/health`: 7 asks, the last heard exit 7; \
     app.service gave up starting"
  );
  assert_eq!(
    lines[1],
    "app.service: failed/failed, main process 0, 5 automatic restarts, last exit status 101"
  );
  assert!(lines[2].starts_with("machine: load "), "{}", lines[2]);
  assert_eq!(lines[3], "app.service's journal since the restart, its last 40 lines at most:");
  assert_eq!(lines[4], "  2026-10-14T05:15:00+0000 host app[41]: panicked at the start");
  assert_eq!(lines.len(), 5);
}

#[test]
fn a_service_that_did_not_stay_up_is_told_with_why() {
  let lines = stay_report(&machine, "app.service", 0, 30_000, "systemd restarted it 1 time(s)", None);

  assert_eq!(
    lines[0],
    "app.service answered and did not stay up for 30s: systemd restarted it 1 time(s)"
  );
  assert!(lines[1].starts_with("app.service: failed/failed"), "{}", lines[1]);
  assert_eq!(lines.len(), 4);
}
