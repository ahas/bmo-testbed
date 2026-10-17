//! The deployment's verbs and the daily run as a shell runs them (docs/boundary.md, 「The verbs」;
//! docs/deploy.md, 「Where a deployment runs」): what each refuses before it touches anything, a dry run said
//! in the caller's process, and the unit each hands itself to, to a stand-in `systemd-run` that starts
//! nothing. Nothing here starts a unit or reaches a service.

use std::path::{Path, PathBuf};

use bmo_testbed_cli::full::{read_start, runs_folder, write_start};
use bmo_testbed_core::policy::daily::DAILY;
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::policy::{fill_marks, fill_shown};
use bmo_testbed_core::queue::{LockWait, now_ms, take_lock};

use crate::stand::{Ran, git, install, ran, testbed};

// A repository with a remote, its declaration deploying into the scene's own places.
struct Scene {
  dir: tempfile::TempDir,
  repo: PathBuf,
  data: PathBuf,
}

impl Scene {
  fn new(extra: &str) -> Self {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let repo = dir.path().join("repo");
    let data = dir.path().join("testbed-data");

    git(dir.path(), &["init", "-q", "--bare", "-b", "main", &origin.to_string_lossy()]);
    git(dir.path(), &["clone", "-q", &origin.to_string_lossy(), &repo.to_string_lossy()]);
    std::fs::create_dir_all(dir.path().join("runtime")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
      repo.join("testbed.toml"),
      format!(
        "testbed = 0\nname = \"sample\"\n\n[deploy]\nroot = \"{}\"\ndata_root = \"{}\"\n{extra}",
        dir.path().join("live").display(),
        dir.path().join("runtime").display()
      ),
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    git(&repo, &["push", "-q", "origin", "HEAD:main"]);

    Self { dir, repo, data }
  }

  // A stand-in `systemd-run` that writes down the line it was handed and starts nothing, ending with `exit`.
  fn systemd_run(&self, exit: i32) -> PathBuf {
    let bin = self.dir.path().join("bin");
    let record = self.dir.path().join("line");

    std::fs::create_dir_all(&bin).unwrap();
    install(
      &format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\nexit {exit}\n", record.display()),
      &bin.join("systemd-run"),
    );
    bin
  }

  fn run(&self, args: &[&str], env: &[(&str, &str)], stand_in: Option<&Path>) -> Ran {
    let mut command = testbed(&self.repo, &self.data, args);

    // The user units read are the scene's own, never the machine's.
    command.env("XDG_CONFIG_HOME", self.dir.path().join("config"));

    if let Some(bin) = stand_in {
      command.env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    }

    for (name, value) in env {
      command.env(name, value);
    }

    ran(&mut command)
  }

  fn line(&self) -> Vec<String> {
    std::fs::read_to_string(self.dir.path().join("line"))
      .unwrap_or_default()
      .lines()
      .map(str::to_owned)
      .collect()
  }
}

fn unit_of(line: &[String]) -> String {
  line.iter().find_map(|word| word.strip_prefix("--unit=")).unwrap_or_default().to_owned()
}

fn numbered(unit: &str, prefix: &str) -> bool {
  unit
    .strip_prefix(prefix)
    .is_some_and(|number| !number.is_empty() && number.chars().all(|one| one.is_ascii_digit()))
}

// "the deploy verbs refuse a job's shell before anything is done"
#[test]
fn the_deploy_verb_refuses_a_jobs_shell_before_anything_is_done() {
  let scene = Scene::new("");
  let bin = scene.systemd_run(0);

  for args in [&["deploy"][..], &["deploy", "--dry-run"]] {
    let refused = scene.run(args, &[("CLAUDE_PID", "4242")], Some(&bin));

    assert_eq!(refused.code, 2, "{args:?}: {}", refused.err);
    assert!(refused.err.contains(&DEPLOY.run.job), "{args:?}: {}", refused.err);
  }

  assert!(scene.line().is_empty());
  assert_eq!(std::fs::read_dir(scene.dir.path().join("runtime")).unwrap().count(), 0);
}

// "the deployment called from anywhere goes to a unit of its own and ends at once"
#[test]
fn the_deployment_goes_to_a_unit_of_its_own_and_ends_at_once() {
  let scene = Scene::new("");
  let bin = scene.systemd_run(0);
  let done = scene.run(&["deploy", "--no-check"], &[], Some(&bin));
  let line = scene.line();
  let unit = unit_of(&line);

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(numbered(&unit, "bmo-deploy-"), "{unit}");
  assert!(line.contains(&format!("--setenv=BMO_DEPLOY_UNIT={unit}")));
  assert!(line.contains(&"--collect".to_owned()));
  assert!(line.contains(&"--setenv=BMO_TESTBED_FRONT=1".to_owned()));
  assert!(line.contains(&format!("--setenv=BMO_DATA_DIR={}", scene.data.display())));
  // The unit finds its deploy tree in the declaration again, never in the release a caller stands in.
  assert!(!line.iter().any(|word| word.starts_with("--setenv=BMO_DEPLOY_DIR=")), "{line:?}");
  assert_eq!(
    line[line.len() - 3..],
    [
      env!("CARGO_BIN_EXE_bmo-testbed").to_owned(),
      "deploy".to_owned(),
      "--no-check".to_owned()
    ]
  );
  assert!(done.out.contains(&fill_shown(&DEPLOY.unit.notice, &[("unit", &unit)])), "{}", done.out);
}

// "a unit that cannot be started is a refusal, and nothing runs in the caller's place"
#[test]
fn a_deployment_whose_unit_cannot_be_started_is_refused() {
  let scene = Scene::new("");
  let bin = scene.systemd_run(1);
  let done = scene.run(&["deploy"], &[], Some(&bin));

  assert_eq!(done.code, 2, "{}", done.out);
  // What systemd-run's stand-in printed follows the policy's line, so the line is matched up to it.
  let refused = DEPLOY.unit.not_started.split("{unit}").nth(1).unwrap().split("{said}").next().unwrap();

  assert!(done.out.contains(refused), "{}", done.out);
  assert!(!scene.dir.path().join("live").exists());
}

// "a dry run says in the caller's process what it would do, and starts no unit"
#[test]
fn a_dry_run_says_in_the_callers_process_what_it_would_do() {
  let scene = Scene::new("");
  let bin = scene.systemd_run(0);
  // A caller standing in some release of another repository carries that release in BMO_DEPLOY_DIR.
  let elsewhere = scene.dir.path().join("elsewhere/deploy");
  let done = scene.run(
    &["deploy", "--dry-run"],
    &[("BMO_DEPLOY_DIR", &elsewhere.to_string_lossy())],
    Some(&bin),
  );

  assert_eq!(done.code, 0, "{}\n{}", done.out, done.err);
  assert!(done.out.contains("--dry-run:"), "{}", done.out);
  assert!(scene.line().is_empty());
  assert!(!scene.dir.path().join("live").exists());
}

// "a value naming nothing is refused before anything is done"
#[test]
fn a_commit_or_a_command_naming_nothing_is_refused() {
  let scene = Scene::new("");

  for args in [
    &["deploy", "--commit", ""][..],
    &["deploy", "--before-start", " "],
    &["full", "--due", "25:61"],
  ] {
    assert_eq!(scene.run(args, &[], None).code, 2, "{args:?}");
  }
}

// "backups refuses a repository that declares none, and keeps local what no remote is declared for"
#[test]
fn backups_refuses_none_declared_and_keeps_local_with_no_remote() {
  let none = Scene::new("");
  let refused = none.run(&["backups"], &[], None);

  assert_eq!(refused.code, 2);
  assert!(refused.err.contains(&VERBS.backups.none_declared), "{}", refused.err);

  let local = Scene::new("\n[deploy.backup]\ndir = \"/nowhere/backups\"\n");
  let kept = local.run(&["backups"], &[], None);

  assert_eq!(kept.code, 0, "{}", kept.err);
  assert!(kept.out.contains("stay on this machine"), "{}", kept.out);
}

// "the daily run called from anywhere goes to a unit of its own and ends at once"
#[test]
fn the_daily_run_goes_to_a_unit_of_its_own_and_ends_at_once() {
  let scene = Scene::new("");
  let bin = scene.systemd_run(0);
  let done = scene.run(&["full", "--due", "00:00"], &[], Some(&bin));
  let line = scene.line();
  let unit = unit_of(&line);

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(numbered(&unit, "bmo-full-suite-"), "{unit}");
  assert!(line.contains(&format!(
    "--setenv=BMO_FULL_CHECK_DIR={}",
    scene.dir.path().join("live/full-check").display()
  )));
  assert_eq!(
    line[line.len() - 2..],
    [env!("CARGO_BIN_EXE_bmo-testbed").to_owned(), "full".to_owned()]
  );
  assert!(done.out.contains(&fill_marks(&DAILY.run.started, &[("unit", &unit)])), "{}", done.out);
}

// "a daily run already started since the time asked starts none, and a second inside its unit is refused"
#[test]
fn a_run_already_started_since_the_due_time_starts_none_and_a_second_is_refused() {
  let scene = Scene::new("");
  let bin = scene.systemd_run(0);
  let folder = runs_folder(&scene.data, "sample");

  write_start(&folder, now_ms());
  assert!(read_start(&folder).is_some());

  let not_due = scene.run(&["full", "--due", "00:00"], &[], Some(&bin));

  assert_eq!(not_due.code, 0, "{}", not_due.err);
  assert!(scene.line().is_empty(), "{:?}", scene.line());

  let _held = take_lock(&scene.data.join("locks/full-sample.lock"), 1.0, LockWait::default()).unwrap();
  let second = scene.run(&["full"], &[("BMO_DEPLOY_UNIT", "bmo-full-suite-1")], None);

  assert_eq!(second.code, 2, "{}", second.err);
  assert!(second.err.contains(&DAILY.run.refused), "{}", second.err);
}
