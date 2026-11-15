//! The service coming up after a restart (docs/deploy.md, 「What the deploy run does, in order」, step 11):
//! the wait for the declared health until it answers, the unit gives up starting or the wait's bound passes,
//! systemd's account of the unit, the reading of it running, and the lines the deploy output keeps where it
//! did not come up.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_deploy::boot::{
  BootScene, HealthEnd, HealthWait, HealthWatch, StayUp, UnitStatus, ask_health, boot_report, boot_scene,
  stands_up, stay_report, unit_gave_up, unit_status, wait_for_health,
};
use bmo_testbed_deploy::run::Ran;

fn watch(command: &str, cwd: &Path) -> HealthWatch<'static> {
  HealthWatch {
    poll_ms: 20,
    check_every_ms: 0,
    ..HealthWatch::new(command, cwd, 600)
  }
}

// A watch whose every ask answers at once as `ask` says, counted, so a wait runs on the test's clock alone.
fn answering<'a>(asked: &'a Cell<u32>, ask: impl Fn() -> (bool, String) + 'a) -> HealthWatch<'a> {
  HealthWatch {
    ask: Box::new(move || {
      asked.set(asked.get() + 1);

      let answer = ask();

      Box::pin(async move { answer })
    }),
    gave_up: Some(Box::new(|| false)),
    ..HealthWatch::new("app health", Path::new("/"), 600)
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

  assert_eq!(waited.end, HealthEnd::Answered);
  assert_eq!(waited.asks, 1);
  assert_eq!(waited.last, "exit 0: healthy");
}

#[tokio::test]
async fn the_health_is_asked_again_until_it_answers() {
  let dir = tempfile::tempdir().unwrap();
  // Fails twice, then answers: each ask leaves a mark, and the third finds two.
  let line = "echo x >> asks; test $(wc -l < asks) -ge 3";
  let waited = wait_for_health(&watch(line, dir.path())).await;

  assert!(waited.answered());
  assert_eq!(waited.asks, 3);
}

#[tokio::test]
async fn an_ask_that_takes_its_time_is_waited_for_until_it_ends() {
  let dir = tempfile::tempdir().unwrap();
  let started = std::time::Instant::now();
  let waited = wait_for_health(&watch("sleep 1; echo late", dir.path())).await;

  assert!(waited.answered());
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

  assert_eq!(waited.end, HealthEnd::GaveUp);
  assert_eq!(waited.asks, 1);
  assert_eq!(waited.last, "exit 1: not yet");
}

// A service that stays running and never says it is ready: every ask answers 1 and the unit never gives up,
// so only the bound ends the wait, on the paused clock at the moment it passes.
#[tokio::test(start_paused = true)]
async fn a_health_that_never_answers_ends_the_wait_once_its_bound_passes() {
  let asked = Cell::new(0);
  let watch = answering(&asked, || (false, "exit 1: the service is not ready".to_owned()));
  let waited = wait_for_health(&watch).await;

  assert_eq!(waited.end, HealthEnd::RanOut);
  assert_eq!((waited.waited_ms, waited.bound_ms), (600_000, 600_000));
  assert_eq!(waited.last, "exit 1: the service is not ready");
  // Asked at the start and after every rest until the bound: one ask more than the rests that fit in it.
  assert_eq!(u64::from(waited.asks), 600_000 / DEPLOY.numbers.health_poll_ms + 1);
  assert_eq!(asked.get(), waited.asks);
}

// An ask that never ends, such as one hanging on a service that took the connection and never answers, is
// cut where the bound passes rather than holding the wait for ever.
#[tokio::test(start_paused = true)]
async fn an_ask_still_running_when_the_bound_passes_is_cut_there() {
  let watch = HealthWatch {
    ask: Box::new(|| Box::pin(std::future::pending())),
    ..HealthWatch::new("app health", Path::new("/"), 600)
  };
  let waited = wait_for_health(&watch).await;

  assert_eq!(waited.end, HealthEnd::RanOut);
  assert_eq!((waited.asks, waited.waited_ms), (1, 600_000));
  assert_eq!(waited.last, DEPLOY.boot.cut);
}

// Whether a process stands, a zombie not counted: one whose parent was ended is reaped by the system.
fn stands(pid: &str) -> bool {
  std::fs::read_to_string(format!("/proc/{pid}/stat"))
    .is_ok_and(|stat| stat.rsplit_once(") ").is_some_and(|(_, rest)| !rest.starts_with('Z')))
}

// The command starts a process that outlives the shell's own answer; the ask is dropped once that process has
// written its id, as the wait drops an ask the bound cut.
#[tokio::test]
async fn a_dropped_ask_ends_every_process_its_command_started() {
  let dir = tempfile::tempdir().unwrap();
  let pid_at = dir.path().join("pid");
  let line = "sleep 300 & echo $! > pid.tmp && mv pid.tmp pid; wait";
  let written = async {
    for _ in 0..1000 {
      if let Ok(pid) = std::fs::read_to_string(&pid_at) {
        return pid.trim().to_owned();
      }

      tokio::time::sleep(Duration::from_millis(10)).await;
    }

    panic!("the health command never wrote its process's id");
  };
  let pid = tokio::select! {
    asked = ask_health(line.to_owned(), PathBuf::from(dir.path())) => panic!("the ask ended: {asked:?}"),
    pid = written => pid,
  };
  let mut gone = false;

  for _ in 0..500 {
    if !stands(&pid) {
      gone = true;
      break;
    }

    tokio::time::sleep(Duration::from_millis(10)).await;
  }

  if !gone {
    // Taken down here all the same, so a failing run leaves nothing behind.
    std::process::Command::new("kill").args(["-9", &pid]).status().unwrap();
  }

  assert!(gone, "the process {pid} the command started outlived its dropped ask");
}

#[test]
fn systemd_is_read_one_property_to_a_line() {
  let shown = |_: &[String]| Ran {
    ok: true,
    code: Some(0),
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
    code: Some(1),
    out: "no bus".to_owned(),
  };

  assert_eq!(unit_status(&refused, "app.service"), None);

  let partial = |_: &[String]| Ran {
    ok: true,
    code: Some(0),
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
      code: Some(0),
      out: "ActiveState=failed\nSubState=failed\nMainPID=0\nNRestarts=5\nExecMainStatus=101".to_owned(),
    },
    _ => Ran {
      ok: true,
      code: Some(0),
      out: "2026-10-14T05:15:00+0000 host app[41]: panicked at the start\n\n".to_owned(),
    },
  }
}

#[test]
fn a_health_that_did_not_answer_is_told_with_the_unit_and_its_journal() {
  let wait = HealthWait {
    command: "curl -fs localhost:8080/health".to_owned(),
    end: HealthEnd::GaveUp,
    waited_ms: 12_400,
    bound_ms: 600_000,
    asks: 7,
    last: "exit 7".to_owned(),
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
        ("end", &fill_shown(&words.gave_up, &[("unit", &"app.service")])),
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

#[test]
fn a_health_whose_bound_passed_says_the_bound() {
  let wait = HealthWait {
    command: "app health".to_owned(),
    end: HealthEnd::RanOut,
    waited_ms: 600_000,
    bound_ms: 600_000,
    asks: 1201,
    last: "exit 1: the service is not ready".to_owned(),
  };
  let lines = boot_report(&boot_scene(&machine, "app.service", 0, wait));
  let words = &DEPLOY.boot;

  assert_eq!(
    lines[0],
    fill_shown(
      &words.waited,
      &[
        ("waited", &600),
        ("command", &"app health"),
        ("asks", &1201),
        ("last", &"exit 1: the service is not ready"),
        ("end", &fill_shown(&words.ran_out, &[("bound", &600)])),
      ],
    )
  );
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
