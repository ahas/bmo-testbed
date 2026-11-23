//! The stand-ins: a call's words read into a request, and the answer printed in systemd's own forms.

use bmo_testbed_supervise::control::{Answer, Request};
use bmo_testbed_supervise::serve::{lay_stand_ins, serve};
use bmo_testbed_supervise::stand_in::{
  Systemctl, journal, show, systemctl_request, systemctl_said, unit_name,
};
use bmo_testbed_supervise::state::UnitState;
use clap::Parser;

use crate::support::Bench;

fn words(line: &[&str]) -> Systemctl {
  Systemctl::try_parse_from(std::iter::once("systemctl").chain(line.iter().copied())).unwrap()
}

// "systemctl's verbs become the supervisor's requests, a bare name read as a service"
#[test]
fn systemctls_verbs_become_requests() {
  assert_eq!(unit_name("bmo-web"), "bmo-web.service");
  assert_eq!(unit_name("backup.timer"), "backup.timer");
  assert_eq!(
    systemctl_request(&words(&["--user", "restart", "a.service", "b"])),
    Some(Request::Restart {
      units: vec!["a.service".to_owned(), "b.service".to_owned()]
    })
  );
  assert_eq!(
    systemctl_request(&words(&["--user", "enable", "--now", "/tree/herdr.service"])),
    Some(Request::Enable {
      units: vec!["/tree/herdr.service".to_owned()],
      now: true
    })
  );
  assert_eq!(
    systemctl_request(&words(&["--user", "is-active", "--quiet", "x"])),
    Some(Request::Show {
      units: vec!["x.service".to_owned()]
    })
  );
  assert_eq!(systemctl_request(&words(&["--user", "isolate", "x"])), None);
}

// "show prints the asked properties as systemd does, and is-active ends with its code"
#[test]
fn show_and_is_active_print_as_systemd() {
  let mut running = UnitState::loaded("a.service", "A", true);

  running.set("active", "running");
  running.main_pid = 42;

  let shown =
    show(std::slice::from_ref(&running), &["ActiveState".to_owned(), "MainPID".to_owned()], false);

  assert_eq!(shown, "ActiveState=active\nMainPID=42");
  assert_eq!(show(std::slice::from_ref(&running), &["SubState".to_owned()], true), "running");

  let answer = Answer {
    states: vec![UnitState::loaded("b.service", "", false)],
    ..Answer::ok()
  };
  let (code, out) = systemctl_said(&words(&["--user", "is-active", "--quiet", "b"]), &answer);

  assert_eq!((code, out.as_str()), (3, ""));

  let (code, out) = systemctl_said(
    &words(&["--user", "show", "-p", "ActiveState,SubState,MainPID,NRestarts,ExecMainStatus", "a"]),
    &Answer {
      states: vec![running],
      ..Answer::ok()
    },
  );

  assert_eq!(code, 0);
  assert_eq!(out, "ActiveState=active\nSubState=running\nMainPID=42\nNRestarts=0\nExecMainStatus=0");
}

// "journalctl prints the last lines of the units' logs in the order they were written"
#[test]
fn journalctl_prints_the_last_lines() {
  let bench = Bench::new();
  let logs = bench.places.logs();

  std::fs::create_dir_all(&logs).unwrap();
  std::fs::write(
    logs.join("a.service.log"),
    "2026-01-01T00:00:01.000Z one\n2026-01-01T00:00:03.000Z three\n",
  )
  .unwrap();
  std::fs::write(logs.join("b.service.log"), "2026-01-01T00:00:02.000Z two\n").unwrap();

  let units = vec!["a.service".to_owned(), "b.service".to_owned()];

  assert_eq!(journal(&logs, &units, Some(2), true), vec!["two", "three"]);
  assert_eq!(
    journal(&logs, &units[..1], None, false),
    vec!["2026-01-01T00:00:01.000Z a.service: one", "2026-01-01T00:00:03.000Z a.service: three"]
  );
}

// "the supervisor serves its socket, refuses a second one on its data root and lays its stand-ins"
#[tokio::test(flavor = "multi_thread")]
async fn the_supervisor_serves_its_socket_alone() {
  let bench = Bench::new();
  let binary = bench.path("bmo-testbed");

  std::fs::write(&binary, "").unwrap();
  lay_stand_ins(&bench.places, &binary).unwrap();

  for name in ["systemctl", "systemd-run", "journalctl"] {
    assert_eq!(std::fs::read_link(bench.places.stand_ins().join(name)).unwrap(), binary);
  }

  let (ready, readied) = tokio::sync::oneshot::channel();
  let (places, specifiers, at) = (bench.places.clone(), bench.specifiers(), binary.clone());
  let served =
    tokio::spawn(async move { serve(places, specifiers, &at, true, Some(ready), &|_| {}).await.is_ok() });

  readied.await.unwrap();

  let second = serve(bench.places.clone(), bench.specifiers(), &binary, true, None, &|_| {}).await;

  assert!(second.unwrap_err().said().contains("another supervisor"));

  let socket = bench.places.socket();
  let answer = bmo_testbed_supervise::control::ask(&socket, &Request::ShowEnvironment).await.unwrap();

  assert!(answer.out.contains(&format!("BMO_SUPERVISOR={}", socket.display())));

  bmo_testbed_supervise::control::ask(&socket, &Request::Shutdown).await.unwrap();
  assert!(served.await.unwrap());
}
