//! The supervisor over stand-in units: started, watched, restarted on a crash, stopped, and their health read.

use std::time::Duration;

use bmo_testbed_supervise::control::Request;
use bmo_testbed_supervise::manager::Event;
use bmo_testbed_supervise::state::Health;

use crate::support::{Bench, alive, ask, read, state, units, until};

fn simple(command: &str, extra: &str) -> String {
  format!("[Service]\nExecStart=/bin/sh -c '{command}'\n{extra}\n[Install]\nWantedBy=default.target\n")
}

// "an enabled unit starts with the supervisor, its output kept in its log, and a stop ends its whole group"
#[tokio::test(flavor = "multi_thread")]
async fn an_enabled_unit_starts_logs_and_stops_with_its_group() {
  let bench = Bench::new();

  bench.unit(
    "server.service",
    &simple("echo started; sleep 300 & echo $! > child.pid; wait", "WorkingDirectory=%h"),
    true,
  );
  bench.unit("idle.service", &simple("sleep 300", ""), false);

  let events = bench.run();
  let running = until(&events, "server.service", |state| state.sub_state == "running").await;
  let child_pid = {
    let mut read_pid = String::new();

    for _ in 0..100 {
      read_pid = read(&bench.path("child.pid"));

      if !read_pid.trim().is_empty() {
        break;
      }

      tokio::time::sleep(Duration::from_millis(50)).await;
    }

    read_pid.trim().parse::<u32>().unwrap()
  };

  assert!(running.main_pid > 0);
  assert_eq!(running.unit_file_state, "enabled");
  assert_eq!(state(&events, "idle.service").await.active_state, "inactive");
  until(&events, "server.service", |_| {
    read(&bench.places.log_of("server.service")).contains("started")
  })
  .await;

  let stopped = ask(
    &events,
    Request::Stop {
      units: units(&["server.service"]),
    },
  )
  .await;

  assert_eq!(stopped.code, 0);
  assert_eq!(state(&events, "server.service").await.active_state, "inactive");
  assert!(!alive(running.main_pid));

  // The child the shell left behind went with the group.
  for _ in 0..40 {
    if !alive(child_pid) {
      break;
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  assert!(!alive(child_pid));
}

// "a crashed unit is started again after its pause, and the restarts are counted"
#[tokio::test(flavor = "multi_thread")]
async fn a_crashed_unit_is_started_again() {
  let bench = Bench::new();

  bench.unit("crashy.service", &simple("sleep 300", "Restart=always\nRestartSec=100ms"), true);

  let events = bench.run();
  let first = until(&events, "crashy.service", |state| state.sub_state == "running").await;

  // SAFETY: the test kills the process it started.
  unsafe { libc::kill(first.main_pid as i32, libc::SIGKILL) };

  let again =
    until(&events, "crashy.service", |state| state.n_restarts == 1 && state.sub_state == "running").await;

  assert_ne!(again.main_pid, first.main_pid);

  ask(
    &events,
    Request::Stop {
      units: units(&["crashy.service"]),
    },
  )
  .await;
}

// "a unit that keeps failing hits its start limit and stays failed until it is reset"
#[tokio::test(flavor = "multi_thread")]
async fn a_unit_that_keeps_failing_hits_its_start_limit() {
  let bench = Bench::new();

  bench.unit(
    "failing.service",
    "[Unit]\nStartLimitBurst=3\nStartLimitIntervalSec=60\n[Service]\nExecStart=/bin/sh -c 'exit 3'\nRestart=on-failure\nRestartSec=10ms\n",
    false,
  );

  let events = bench.run();

  ask(
    &events,
    Request::Start {
      units: units(&["failing.service"]),
    },
  )
  .await;

  let failed = until(&events, "failing.service", |state| state.result == "start-limit-hit").await;

  assert_eq!(failed.active_state, "failed");
  assert_eq!(failed.n_restarts, 3);

  ask(
    &events,
    Request::ResetFailed {
      units: units(&["failing.service"]),
    },
  )
  .await;
  assert_eq!(state(&events, "failing.service").await.active_state, "inactive");
}

// "a oneshot's start answers once it has ended, with its failure where it failed"
#[tokio::test(flavor = "multi_thread")]
async fn a_oneshot_start_answers_once_it_has_ended() {
  let bench = Bench::new();

  bench.unit(
    "job.service",
    "[Service]\nType=oneshot\nExecStart=/bin/sh -c 'sleep 0.2; touch %h/done'\n",
    false,
  );
  bench.unit("bad.service", "[Service]\nType=oneshot\nExecStart=/bin/sh -c 'exit 4'\n", false);

  let events = bench.run();
  let done = ask(
    &events,
    Request::Start {
      units: units(&["job.service"]),
    },
  )
  .await;

  assert_eq!(done.code, 0);
  assert!(bench.path("done").exists());
  assert_eq!(state(&events, "job.service").await.active_state, "inactive");

  let bad = ask(
    &events,
    Request::Start {
      units: units(&["bad.service"]),
    },
  )
  .await;

  assert_eq!(bad.code, 1);

  let failed = state(&events, "bad.service").await;

  assert_eq!((failed.active_state.as_str(), failed.exec_main_status), ("failed", 4));
}

// "a restart stops the running process and starts a new one, and an unknown unit is not found"
#[tokio::test(flavor = "multi_thread")]
async fn a_restart_replaces_the_process() {
  let bench = Bench::new();

  bench.unit("server.service", &simple("sleep 300", ""), true);

  let events = bench.run();
  let first = until(&events, "server.service", |state| state.sub_state == "running").await;
  let restarted = ask(
    &events,
    Request::Restart {
      units: units(&["server.service"]),
    },
  )
  .await;
  let now = state(&events, "server.service").await;

  assert_eq!(restarted.code, 0);
  assert_eq!(now.sub_state, "running");
  assert_ne!(now.main_pid, first.main_pid);
  assert!(!alive(first.main_pid));
  assert_eq!(
    ask(
      &events,
      Request::Start {
        units: units(&["missing.service"])
      }
    )
    .await
    .code,
    5
  );
  assert_eq!(state(&events, "missing.service").await.load_state, "not-found");

  ask(
    &events,
    Request::Stop {
      units: units(&["server.service"]),
    },
  )
  .await;
}

// "a unit whose condition does not hold is left inactive, and one with its environment reads it"
#[tokio::test(flavor = "multi_thread")]
async fn conditions_and_environment_files_are_read_as_the_unit_starts() {
  let bench = Bench::new();

  std::fs::write(bench.path("unit.env"), "GREETING='hello $there'\n").unwrap();
  bench.unit(
    "waits.service",
    "[Unit]\nConditionPathExists=%h/absent\n[Service]\nExecStart=/bin/sleep 300\n[Install]\nWantedBy=default.target\n",
    true,
  );
  bench.unit(
    "speaks.service",
    "[Service]\nType=oneshot\nEnvironmentFile=%h/unit.env\nEnvironment=PATH=/usr/bin:/bin\nExecStart=/bin/sh -c 'echo \"$GREETING\" > %h/said; echo \"$PATH\" > %h/path; echo \"$BMO_SUPERVISOR\" > %h/socket'\n",
    false,
  );

  let events = bench.run();

  assert_eq!(state(&events, "waits.service").await.active_state, "inactive");
  assert_eq!(
    ask(
      &events,
      Request::Start {
        units: units(&["speaks.service"])
      }
    )
    .await
    .code,
    0
  );
  assert_eq!(read(&bench.path("said")).trim(), "hello $there");
  assert!(
    read(&bench.path("path")).starts_with(&format!("{}:/usr/bin", bench.places.stand_ins().display()))
  );
  assert_eq!(read(&bench.path("socket")).trim(), bench.places.socket().to_string_lossy());
}

// "a transient run is answered once it ends with its code, and let go of with --collect"
#[tokio::test(flavor = "multi_thread")]
async fn a_transient_run_waits_and_collects() {
  let bench = Bench::new();
  let events = bench.run();
  let ran = ask(
    &events,
    Request::Run {
      unit: Some("bmo-deploy-1".to_owned()),
      words: vec!["/bin/sh".to_owned(), "-c".to_owned(), "echo \"$MARK\"; exit 7".to_owned()],
      cwd: Some(bench.dir.path().to_path_buf()),
      env: vec![("MARK".to_owned(), "marked".to_owned())],
      collect: true,
      wait: true,
    },
  )
  .await;

  assert_eq!(ran.code, 7);
  assert!(read(&bench.places.log_of("bmo-deploy-1.service")).contains("marked"));
  assert_eq!(state(&events, "bmo-deploy-1.service").await.load_state, "not-found");
}

// "enabling links the unit as systemd does, and --now starts it"
#[tokio::test(flavor = "multi_thread")]
async fn enabling_links_and_starts() {
  let bench = Bench::new();
  let file = bench.path("elsewhere.service");

  std::fs::write(&file, simple("sleep 300", "")).unwrap();

  let events = bench.run();
  let enabled = ask(
    &events,
    Request::Enable {
      units: vec![file.to_string_lossy().into_owned()],
      now: true,
    },
  )
  .await;

  assert_eq!(enabled.code, 0, "{enabled:?}");
  assert!(enabled.out.contains("default.target.wants"));
  assert!(bench.places.enabled("elsewhere.service"));
  assert_eq!(state(&events, "elsewhere.service").await.sub_state, "running");

  let disabled = ask(
    &events,
    Request::Disable {
      units: units(&["elsewhere.service"]),
      now: true,
    },
  )
  .await;

  assert_eq!(disabled.code, 0);
  assert!(!bench.places.enabled("elsewhere.service"));
  assert_eq!(state(&events, "elsewhere.service").await.active_state, "inactive");
}

// "a bot unit's health is read from its deploy tree's declaration"
#[tokio::test(flavor = "multi_thread")]
async fn a_bot_units_health_is_read_from_its_deploy_tree() {
  let bench = Bench::new();
  let tree = bench.path("service/deploy");

  std::fs::create_dir_all(tree.join("units")).unwrap();
  std::fs::write(
    tree.join("testbed.toml"),
    "testbed = 0\nname = \"stand-in\"\n\n[deploy]\nroot = \"~/service\"\ndata_root = \"~/service/data\"\nhealth = \"test -f healthy\"\n",
  )
  .unwrap();
  std::fs::write(tree.join("units/bot.service"), simple("sleep 300", "") + "X-BMO-Deploy=bot\n").unwrap();
  std::os::unix::fs::symlink(tree.join("units/bot.service"), bench.places.unit_dir.join("bot.service"))
    .unwrap();
  bench.places.enable("bot.service").unwrap();

  let events = bench.run();

  until(&events, "bot.service", |state| state.sub_state == "running").await;
  events.send(Event::HealthDue).unwrap();
  until(&events, "bot.service", |state| state.health == Health::Unhealthy).await;
  std::fs::write(tree.join("healthy"), "").unwrap();
  events.send(Event::HealthDue).unwrap();
  until(&events, "bot.service", |state| state.health == Health::Healthy).await;

  ask(
    &events,
    Request::Stop {
      units: units(&["bot.service"]),
    },
  )
  .await;
}

// "a shutdown stops every unit and ends the supervisor, its state file saying so"
#[tokio::test(flavor = "multi_thread")]
async fn a_shutdown_stops_every_unit() {
  let bench = Bench::new();

  bench.unit("one.service", &simple("sleep 300", ""), true);
  bench.unit(
    "two.service",
    &simple("trap \"exit 0\" TERM; sleep 300 & wait", "After=one.service"),
    true,
  );

  let events = bench.run();
  let one = until(&events, "one.service", |state| state.sub_state == "running").await;
  let two = until(&events, "two.service", |state| state.sub_state == "running").await;
  let ended = ask(&events, Request::Shutdown).await;

  assert_eq!(ended.code, 0);
  assert!(!alive(one.main_pid) && !alive(two.main_pid));

  let snapshot: serde_json::Value = serde_json::from_str(&read(&bench.places.state())).unwrap();

  assert_eq!(snapshot["pid"], 0);
}
