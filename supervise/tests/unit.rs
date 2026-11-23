//! A unit file read into what the supervisor runs.

use std::path::PathBuf;
use std::time::Duration;

use bmo_testbed_supervise::unit::{Ending, KillMode, Kind, Restart, ServiceType, line, read};

use crate::support::Bench;

const SERVER: &str = r#"[Unit]
Description=A stand-in server
After=network-online.target other.service
ConditionPathExists=%h/settings.toml
StartLimitIntervalSec=300
StartLimitBurst=3

[Service]
Type=simple
WorkingDirectory=%h/tree
ExecStartPre=-/bin/true
ExecStart=%h/tree/bin/server --config "%h/a b.toml" \
  serve
Environment=PATH=%h/bin:/usr/bin "GREETING=hello there"
Environment=PLAIN=1
EnvironmentFile=-%h/missing.env
Restart=on-failure
RestartSec=2
TimeoutStopSec=infinity
KillMode=process
SuccessExitStatus=75 SIGUSR1
OOMPolicy=continue
MemoryMax=65%

[Install]
WantedBy=default.target
X-BMO-Deploy=bot
"#;

// "a service is read with its specifiers filled, its words split and its policy kept"
#[test]
fn a_service_is_read_with_specifiers_words_and_policy() {
  let bench = Bench::new();
  let home = bench.dir.path().to_string_lossy().into_owned();
  let unit = read("server.service", SERVER, &bench.specifiers()).unwrap();
  let Kind::Service(service) = &unit.kind else { panic!("a service") };

  assert_eq!(unit.description, "A stand-in server");
  assert_eq!(unit.after, vec!["network-online.target", "other.service"]);
  assert_eq!(unit.conditions, vec![(PathBuf::from(format!("{home}/settings.toml")), false)]);
  assert_eq!(unit.mark.as_deref(), Some("bot"));
  assert_eq!(unit.not_applied, vec!["OOMPolicy", "MemoryMax"]);
  assert_eq!(service.kind, ServiceType::Simple);
  assert_eq!(
    service.start.as_ref().unwrap().words,
    vec![
      format!("{home}/tree/bin/server"),
      "--config".to_owned(),
      format!("{home}/a b.toml"),
      "serve".to_owned()
    ]
  );
  assert_eq!(service.start_pre.len(), 1);
  assert!(service.start_pre[0].overlook_failure);
  assert_eq!(service.working_directory, Some((PathBuf::from(format!("{home}/tree")), false)));
  assert_eq!(
    service.environment,
    vec![
      ("PATH".to_owned(), format!("{home}/bin:/usr/bin")),
      ("GREETING".to_owned(), "hello there".to_owned()),
      ("PLAIN".to_owned(), "1".to_owned()),
    ]
  );
  assert_eq!(service.environment_files, vec![(PathBuf::from(format!("{home}/missing.env")), true)]);
  assert_eq!(service.restart, Restart::OnFailure);
  assert_eq!(service.restart_after, Duration::from_secs(2));
  assert_eq!(service.stop_bound, None);
  assert_eq!(service.kill_mode, KillMode::Process);
  assert_eq!(service.start_limit_interval, Duration::from_secs(300));
  assert_eq!(service.start_limit_burst, 3);
  assert_eq!(service.ending(Some(75), None, false), Ending::Clean);
  assert_eq!(service.ending(None, Some(libc::SIGUSR1), false), Ending::Clean);
  assert_eq!(service.ending(Some(1), None, false), Ending::Code);
  assert_eq!(service.ending(None, Some(libc::SIGTERM), false), Ending::Clean);
  assert_eq!(service.ending(None, Some(libc::SIGKILL), false), Ending::Signal);
}

// "a unit that names nothing takes systemd's defaults"
#[test]
fn a_bare_service_takes_the_defaults() {
  let bench = Bench::new();
  let unit =
    read("bare.service", "[Service]\nType=oneshot\nExecStart=/bin/true\n", &bench.specifiers()).unwrap();
  let Kind::Service(service) = &unit.kind else { panic!("a service") };

  assert_eq!(service.kind, ServiceType::Oneshot);
  assert_eq!(service.restart, Restart::No);
  assert_eq!(service.stop_bound, Some(Duration::from_secs(90)));
  assert_eq!(service.kill_mode, KillMode::Group);
  assert_eq!(service.start_limit_burst, 5);
  assert!(unit.mark.is_none());
}

// "a timer reads its calendars, its spans and the service it starts"
#[test]
fn a_timer_reads_its_calendars_and_its_service() {
  let bench = Bench::new();
  let text = "[Timer]\nOnCalendar=*-*-* 01/2:10:00\nOnCalendar=Sun *-*-* 03:30:00\nPersistent=true\nRandomizedDelaySec=60\n";
  let unit = read("backup.timer", text, &bench.specifiers()).unwrap();
  let Kind::Timer(timer) = &unit.kind else { panic!("a timer") };

  assert_eq!(timer.calendars.len(), 2);
  assert!(timer.persistent);
  assert_eq!(timer.randomized_delay, Duration::from_secs(60));
  assert_eq!(timer.unit, "backup.service");
}

// "a value that does not read refuses the unit, naming the key"
#[test]
fn a_value_that_does_not_read_refuses_the_unit() {
  let bench = Bench::new();
  let refused = read(
    "odd.service",
    "[Service]\nExecStart=/bin/true\nRestart=sometimes\n",
    &bench.specifiers(),
  )
  .unwrap_err();

  assert!(refused.contains("Restart=sometimes"), "{refused}");
  assert!(read("odd.timer", "[Timer]\nOnCalendar=*-*-* 25:00\n", &bench.specifiers()).is_err());
}

// "a command line's prefixes are taken off it"
#[test]
fn a_command_lines_prefixes_are_taken_off() {
  let read = line("-@/bin/sh -c 'tail -F log & exec server'").unwrap();

  assert!(read.overlook_failure);
  assert_eq!(read.words, vec!["/bin/sh", "-c", "tail -F log & exec server"]);
  assert!(line("   ").is_err());
}
