//! The `suite` verb as a caller runs it (docs/suite.md; docs/testbed.md, 「The turn」): one run of the suite
//! at a time on this machine, each waiting its turn before it starts anything, a run whose turn never comes
//! running nothing, and a range that is no commit refused before anything runs. Each run is the binary in a
//! checkout of the test's own, on a turn file of the test's own.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::Duration;

use bmo_testbed_client::{CommandRun, TestRunKind, TestRunState};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::queue::{LockWait, take_lock};
use bmo_testbed_core::turn::{Env, Lane, NOT_RUN_EXIT_CODE, SEATS_VARIABLE, TurnAsk, take_turn};
use bmo_testbed_record::Record;

use crate::stand::{Ran, checkout, finished, git, testbed, until};

// A test file that holds its run open until the test lets it go: it says it began, waits for the release
// file in the probe directory its environment names, and says it ended.
const PROBE: &str = r#"import { test } from "bun:test";
import { existsSync, writeFileSync } from "node:fs";
import { join } from "node:path";

test("holds the run until it is let go", async () => {
  const probe = process.env.SUITE_TURN_PROBE ?? "";
  writeFileSync(join(probe, "began"), "");
  while (!existsSync(join(probe, "release"))) await Bun.sleep(20);
  writeFileSync(join(probe, "ended"), "");
}, 170000);
"#;

// A checkout of bun tests holding the probe file.
fn repository(dir: &Path) -> PathBuf {
  let repo = dir.join("repo");

  checkout(&repo, "testbed = 0\n\n[test]\nlanguages = [\"bun\"]\n");
  std::fs::write(repo.join("probe.test.ts"), PROBE).unwrap();
  repo
}

// A run of the suite in a process group of its own, on the test's own turn file, standing where no service
// manager can be reached; with a probe directory where the probe file is to report to, in a wide lane of one
// seat, so probed runs take the lane one at a time.
fn suite_run(repo: &Path, data: &Path, args: &[&str], probe: Option<&Path>) -> Child {
  let mut command = testbed(repo, data, &[&["suite"][..], args].concat());

  if let Some(probe) = probe {
    command.env("SUITE_TURN_PROBE", probe).env(SEATS_VARIABLE, "1");
  }

  std::os::unix::process::CommandExt::process_group(&mut command, 0);
  command.spawn().unwrap()
}

// A run ended, and what it left in its own group taken with it.
fn ended(run: Child) -> Ran {
  let group = run.id();
  let done = finished(run);

  // SAFETY: what the run left in its own group goes with it.
  unsafe { libc::kill(-(group as i32), libc::SIGKILL) };
  done
}

fn queued(lock: &Path, pid: u32) -> bool {
  std::fs::read_dir(lock.with_extension("lock.queue"))
    .map(|names| {
      names
        .flatten()
        .any(|name| name.file_name().to_string_lossy().ends_with(&format!("-{pid}")))
    })
    .unwrap_or(false)
}

fn mark(line: &str) -> String {
  format!("{} {line}", SUITE.lines.mark)
}

// A line of the run as it opens, up to the first mark that holds what the run fills in.
fn opening(template: &str, marks: &[(&str, &str)]) -> String {
  let mut all = vec![("mark", SUITE.lines.mark.as_str())];

  all.extend_from_slice(marks);
  fill_marks(template, &all).split('{').next().unwrap_or_default().to_owned()
}

fn verdict_of(said: &str) -> Option<&str> {
  let ok = mark(&format!("{} in ", SUITE.ending.ok));
  let failed = mark(&format!("{} in ", SUITE.ending.failed));

  said.lines().rfind(|line| line.starts_with(&ok) || line.starts_with(&failed))
}

fn verdict(outcome: &str, note: &str) -> String {
  fill_marks(
    &SUITE.ending.verdict,
    &[("mark", &SUITE.lines.mark), ("outcome", outcome), ("seconds", "0"), ("note", note)],
  )
}

// A test file that fails when any of the deployment's own variables reaches it.
const STRIPPED: &str = r#"import { expect, test } from "bun:test";

test("the deployment's variables do not reach a test", () => {
  expect(process.env.BMO_DATA_DIR).toBeUndefined();
  expect(process.env.BMO_DEPLOY_PROGRESS).toBeUndefined();
  expect(process.env.BMO_DEPLOY_UNIT).toBeUndefined();
  expect(process.env.BMO_TESTBED_FRONT).toBeUndefined();
});
"#;

// "the deployment's own variables are taken away from every test": the suite runs in a deployment's
// environment, and the test file it runs reads none of it.
#[test]
fn the_deployments_own_variables_are_taken_away_from_every_test() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");

  checkout(&repo, "testbed = 0\n\n[test]\nlanguages = [\"bun\"]\n");
  std::fs::write(repo.join("environment.test.ts"), STRIPPED).unwrap();

  let mut command = testbed(&repo, dir.path(), &["suite"]);

  command
    .env("BMO_DEPLOY_PROGRESS", dir.path().join("deploy-progress.json"))
    .env("BMO_DEPLOY_UNIT", "bmo-deploy-1")
    .env("BMO_TESTBED_FRONT", "1");

  std::os::unix::process::CommandExt::process_group(&mut command, 0);
  let run = command.spawn().unwrap();
  let done = ended(run);

  assert_eq!(done.code, 0, "{}", done.err);
}

// "a run of the suite waits for the turn another run holds before it starts anything": a run started while
// another holds the turn starts nothing until the turn is let go, and only then does its own clock start.
#[test]
fn a_run_of_the_suite_waits_for_the_turn_another_run_holds() {
  let dir = tempfile::tempdir().unwrap();
  let repo = repository(dir.path());
  let lock = dir.path().join("testbed.lock");
  let holder = take_lock(&lock, 5.0, LockWait::default()).unwrap();
  let mut run = suite_run(&repo, dir.path(), &["no-such-test-file-791.ts"], None);

  until("the run's ticket", || queued(&lock, run.id()), Duration::from_secs(20));
  assert!(run.try_wait().unwrap().is_none());

  drop(holder);

  let done = ended(run);
  let lines: Vec<&str> = done.err.lines().filter(|line| line.starts_with(&mark(""))).collect();
  let taken = opening(&LANES.settled.own, &[]);

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(lines[0].starts_with(&opening(&SUITE.lines.turn, &[("words", &taken)])), "{}", done.err);
  // A run where no service manager can be reached says it stands where it was started.
  assert!(lines[1].starts_with(&opening(&SUITE.lines.fence_none, &[("why", "")])), "{}", done.err);
  assert!(lines[2].starts_with(&opening(&SUITE.lines.cargo_jobs, &[])), "{}", done.err);
  assert_eq!(
    verdict_of(&done.err),
    Some(verdict(&SUITE.ending.ok, &fill_marks(&SUITE.ending.all_passed, &[("files", "0")])).as_str())
  );
}

// "a run whose harness goes away before its turn comes runs nothing and says it was not run": one command
// holds every seat of the machine, and the run waits behind it until its harness is gone.
#[test]
fn a_run_whose_turn_does_not_come_runs_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let repo = repository(dir.path());
  let lock = dir.path().join("testbed.lock");
  let holder = take_turn(&TurnAsk {
    command: "the close".to_owned(),
    lane: Lane::Wide,
    say: &|_| {},
    path: Some(lock.clone()),
    env: Env::from([
      ("BMO_TESTBED_SCOPE".to_owned(), "place-3".to_owned()),
      // A lane of one seat, which the holder fills.
      ("BMO_TESTBED_SEATS".to_owned(), "1".to_owned()),
    ]),
    checkout: "/checkout".to_owned(),
    poll: None,
    notice_seconds: None,
  });
  let mut harness = std::process::Command::new("sleep").arg("300").spawn().unwrap();
  let mut command = testbed(&repo, dir.path(), &["suite", "no-such-test-file-791.ts"]);

  command.env("CLAUDE_PID", harness.id().to_string()).env("BMO_TESTBED_SEATS", "1");
  std::os::unix::process::CommandExt::process_group(&mut command, 0);

  let run = command.spawn().unwrap();

  until("the run's ticket", || queued(&lock, run.id()), Duration::from_secs(20));
  harness.kill().unwrap();
  harness.wait().unwrap();

  let done = ended(run);

  drop(holder);

  let held_by = ["place-3", "/checkout", "the close"].join(&LANES.entry.joiner);
  let not_had = fill_marks(&LANES.waiting.held_by, &[("who", &held_by)]);

  assert_eq!(done.code, i32::from(NOT_RUN_EXIT_CODE), "{}", done.err);
  assert!(done.err.contains(&not_had), "{}", done.err);
  assert_eq!(
    verdict_of(&done.err),
    Some(verdict(&SUITE.ending.failed, &SUITE.ending.not_run).as_str())
  );
}

// "the first unseated ticket takes any free seat": in a wide lane of one seat, two runs started at once run
// one after the other, the second starting its tests only once the first has ended. The probe file holds each
// run open until the test lets it go.
#[test]
fn two_runs_started_at_once_in_one_seat_run_one_after_the_other() {
  let dir = tempfile::tempdir().unwrap();
  let repo = repository(dir.path());
  let lock = dir.path().join("testbed.lock");
  let probes: Vec<PathBuf> = ["a", "b"].map(|name| dir.path().join(name)).to_vec();

  for probe in &probes {
    std::fs::create_dir_all(probe).unwrap();
  }

  let runs: Vec<Child> = probes
    .iter()
    .map(|probe| suite_run(&repo, dir.path(), &["probe.test.ts"], Some(probe)))
    .collect();
  let began = |probe: &Path| probe.join("began").exists();
  let release = |probe: &Path| std::fs::write(probe.join("release"), "").unwrap();
  let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    until("one run's probe", || probes.iter().any(|probe| began(probe)), Duration::from_secs(180));

    let first = probes.iter().position(|probe| began(probe)).unwrap();
    let second = 1 - first;

    // The other run stands in line, its probe not begun, until the first has ended.
    until("the second run's ticket", || queued(&lock, runs[second].id()), Duration::from_secs(20));
    assert!(!began(&probes[second]));

    release(&probes[first]);
    until("the second run's probe", || began(&probes[second]), Duration::from_secs(180));
    assert!(probes[first].join("ended").exists());
    release(&probes[second]);
    second
  }));

  for probe in &probes {
    release(probe);
  }

  let done: Vec<Ran> = runs.into_iter().map(ended).collect();
  let second = checked.unwrap();
  let ok = mark(&format!("{} in ", SUITE.ending.ok));

  assert_eq!(done.iter().map(|run| run.code).collect::<Vec<_>>(), [0, 0], "{}", done[0].err);
  assert!(
    done
      .iter()
      .all(|run| verdict_of(&run.err).is_some_and(|verdict| verdict.starts_with(&ok)))
  );
  assert!(done[second].err.lines().any(|line| line.starts_with(&mark("turn"))));
}

// A test that fails the first time it runs in a directory and passes every time after.
const FLAKY: &str = r#"import { expect, test } from "bun:test";
import { existsSync, writeFileSync } from "node:fs";

test("passes the second time", () => {
  const seen = process.env.SUITE_FLAKY_MARK ?? "";
  if (!existsSync(seen)) {
    writeFileSync(seen, "");
    expect(1).toBe(2);
  }
});
"#;

// "a test file runs from the folder of its runner's configuration": a file whose folder's bunfig.toml
// preloads what its test reads passes in a run of the suite started at the repository's root.
#[test]
fn a_test_file_runs_from_the_folder_of_its_runner_s_configuration() {
  let dir = tempfile::tempdir().unwrap();
  let repo = repository(dir.path());
  let data = dir.path().join("data");

  std::fs::create_dir_all(repo.join("tests/ts/unit")).unwrap();
  std::fs::create_dir_all(&data).unwrap();
  std::fs::write(repo.join("tests/ts/bunfig.toml"), "[test]\npreload = [\"./preload.ts\"]\n").unwrap();
  std::fs::write(repo.join("tests/ts/preload.ts"), "process.env.SUITE_PRELOADED = \"yes\";\n").unwrap();
  std::fs::write(
    repo.join("tests/ts/unit/reads.test.ts"),
    "import { expect, test } from \"bun:test\";\n\ntest(\"reads what the preload set\", () => {\n  \
     expect(process.env.SUITE_PRELOADED).toBe(\"yes\");\n});\n",
  )
  .unwrap();

  let mut command = testbed(&repo, &data, &["suite", "--json", "tests/ts/"]);

  command.env_remove("SUITE_PRELOADED");
  std::os::unix::process::CommandExt::process_group(&mut command, 0);

  let done = ended(command.spawn().unwrap());

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(done.err.contains("tests/ts/unit/reads.test.ts"), "{}", done.err);

  // With --json the run's command run stands on standard output: its own log, and its test run in the record.
  let answer: CommandRun = serde_json::from_str(done.out.trim()).expect("one command run");
  let kept = Record::open(&data)
    .unwrap()
    .test_run(answer.test_run_id.expect("a test run is kept"))
    .unwrap()
    .unwrap();

  assert_eq!(answer.exit_code, 0);
  assert!(
    std::fs::read_to_string(&answer.log_path)
      .unwrap()
      .contains("tests/ts/unit/reads.test.ts")
  );
  assert_eq!(
    (kept.kind, kept.state, kept.passed_count),
    (TestRunKind::Suite, TestRunState::Passed, 1)
  );
  assert_eq!(kept.repository_name.as_deref(), Some("repo"));
}

// "the browser files a project of its own declares run in the browser lane, once the project's check has
// ended": the check installs the package the browser file imports, in a copy that holds no packages yet. The
// check installs only once the declaration's own browser file has run, which a lane holding the project's file
// at its head would run only after that file, so the order is the sweep's and no race of start-up times.
#[test]
fn the_browser_files_a_project_declares_run_once_its_check_has_installed_it() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let data = dir.path().join("data");

  checkout(
    &repo,
    "testbed = 0\n\n[test]\nlanguages = [\"bun\"]\nbrowser = [\"browser/*.test.ts\"]\n",
  );
  std::fs::create_dir_all(repo.join("lab/tests")).unwrap();
  std::fs::create_dir_all(repo.join("browser")).unwrap();
  std::fs::create_dir_all(&data).unwrap();
  std::fs::write(
    repo.join("browser/declared.test.ts"),
    "import { test } from \"bun:test\";\nimport { writeFileSync } from \"node:fs\";\n\n\
     test(\"the declared browser file runs\", () => {\n  writeFileSync(`${import.meta.dir}/../ran`, \"\");\n});\n",
  )
  .unwrap();
  std::fs::write(
    repo.join("lab/package.json"),
    r#"{ "name": "lab", "scripts": { "check": "sh install.sh" }, "bmo": { "browser": ["tests/*.test.ts"] } }"#,
  )
  .unwrap();
  std::fs::write(repo.join("lab/bun.lock"), "{}").unwrap();
  std::fs::write(
    repo.join("lab/install.sh"),
    "i=0\n\
     while [ ! -e ../ran ] && [ \"$i\" -lt 200 ]; do sleep 0.05; i=$((i + 1)); done\n\
     mkdir -p node_modules/installed\n\
     printf '{ \"name\": \"installed\", \"main\": \"index.ts\" }' > node_modules/installed/package.json\n\
     printf 'export const installed = true;\\n' > node_modules/installed/index.ts\n",
  )
  .unwrap();
  std::fs::write(
    repo.join("lab/tests/page.test.ts"),
    "import { expect, test } from \"bun:test\";\nimport { installed } from \"installed\";\n\n\
     test(\"the page answers\", () => {\n  expect(installed).toBe(true);\n});\n",
  )
  .unwrap();

  let mut command = testbed(&repo, &data, &["suite"]);

  std::os::unix::process::CommandExt::process_group(&mut command, 0);

  let done = ended(command.spawn().unwrap());
  let headings: Vec<&str> = done
    .err
    .lines()
    .filter(|line| line.ends_with("lab/") || line.ends_with("lab/tests/page.test.ts"))
    .collect();

  assert_eq!(done.code, 0, "{}", done.err);
  assert_eq!(headings.len(), 2, "{}", done.err);
}

// A checkout holding one project of its own, `road/`, whose check script runs its tests, and the test file
// given under it.
fn project_with_tests(dir: &Path, test_file: &str) -> (PathBuf, PathBuf) {
  let repo = dir.join("repo");
  let data = dir.join("data");

  checkout(&repo, "testbed = 0\n\n[test]\nlanguages = [\"bun\"]\n");
  std::fs::create_dir_all(repo.join("road/tests")).unwrap();
  std::fs::create_dir_all(&data).unwrap();
  std::fs::write(
    repo.join("road/package.json"),
    r#"{ "name": "road", "scripts": { "check": "bun test" } }"#,
  )
  .unwrap();
  std::fs::write(repo.join("road/bun.lock"), "{}").unwrap();
  std::fs::write(repo.join("road/tests/road.test.ts"), test_file).unwrap();
  (repo, data)
}

// A whole run of the suite in the checkout, ended.
fn whole_run(repo: &Path, data: &Path) -> Ran {
  let mut command = testbed(repo, data, &["suite"]);

  std::os::unix::process::CommandExt::process_group(&mut command, 0);
  ended(command.spawn().unwrap())
}

// "a project of its own runs its own tests, through its check script and in no other way": a test failing
// under it fails the run as the project's unit, and the file is no unit of the run's own.
#[test]
fn a_project_s_failing_test_fails_the_run_through_its_check_script_alone() {
  let dir = tempfile::tempdir().unwrap();
  let (repo, data) = project_with_tests(
    dir.path(),
    "import { expect, test } from \"bun:test\";\n\ntest(\"the road holds\", () => {\n  \
     expect(1).toBe(2);\n});\n",
  );
  let done = whole_run(&repo, &data);
  let failed = fill_marks(&SUITE.ending.failed_files, &[("files", "road/")]);

  assert_eq!(done.code, 1, "{}", done.err);
  assert!(done.err.lines().any(|line| line == failed), "{}", done.err);
  assert!(done.err.contains("the road holds"), "{}", done.err);
  assert!(!done.err.lines().any(|line| line.ends_with("road/tests/road.test.ts")), "{}", done.err);
}

// "a test that needs a program the machine lacks is the test's to skip, and the verdict names how many tests
// the units skipped": a project's check script that skips a test passes, its verdict saying so.
#[test]
fn a_test_a_project_s_check_skips_is_named_on_the_verdict() {
  let dir = tempfile::tempdir().unwrap();
  let (repo, data) = project_with_tests(
    dir.path(),
    "import { test } from \"bun:test\";\n\nconsole.error(\"skipped: no such program on this machine\");\n\n\
     test.skip(\"drives a program this machine lacks\", () => {});\n",
  );
  let done = whole_run(&repo, &data);
  let skipped = fill_marks(&SUITE.ending.skipped_count, &[("count", "1")]);

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(verdict_of(&done.err).is_some_and(|line| line.ends_with(&skipped)), "{}", done.err);
  assert!(done.err.contains("skipped: no such program on this machine"), "{}", done.err);
}

// "a project of its own: the test files its tests' output names as holding a failed test ... The steps of the
// check before its tests do not run again": a project whose one file fails the first time runs that file once
// more by itself, its check's earlier step and its other file run once, and the run passes.
#[test]
fn a_project_runs_once_more_only_its_failed_test_files() {
  let dir = tempfile::tempdir().unwrap();
  let (repo, data) = project_with_tests(dir.path(), FLAKY);

  std::fs::write(
    repo.join("road/package.json"),
    r#"{ "name": "road", "scripts": { "check": "echo step >> ../steps && bun test" } }"#,
  )
  .unwrap();
  std::fs::write(
    repo.join("road/tests/steady.test.ts"),
    "import { test } from \"bun:test\";\nimport { appendFileSync } from \"node:fs\";\n\n\
     test(\"steady\", () => {\n  appendFileSync(`${import.meta.dir}/../../steady`, \"ran\\n\");\n});\n",
  )
  .unwrap();

  let mut command = testbed(&repo, &data, &["suite", "--again"]);

  command
    .env("SUITE_FLAKY_MARK", dir.path().join("seen"))
    .env_remove("BMO_TESTBED_EVENT_HOOK");
  std::os::unix::process::CommandExt::process_group(&mut command, 0);

  let done = ended(command.spawn().unwrap());
  let files = fill_marks(&SUITE.again.files, &[("names", "tests/road.test.ts")]);
  let once_more = fill_marks(
    &SUITE.again.once_more,
    &[("mark", &SUITE.lines.mark), ("unit", "road/"), ("what", &files)],
  );

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(done.err.lines().any(|line| line == once_more), "{}", done.err);
  assert_eq!(std::fs::read_to_string(repo.join("steps")).unwrap(), "step\n");
  assert_eq!(std::fs::read_to_string(repo.join("steady")).unwrap(), "ran\n");
}

// "with --again what failed runs once more, and what passed only then is written down and sent as an event"
#[test]
fn a_run_once_more_passes_on_its_second_run_and_tells_of_the_flaky_test() {
  let dir = tempfile::tempdir().unwrap();
  let repo = repository(dir.path());
  let data = dir.path().join("data");
  let event = dir.path().join("event");

  std::fs::write(repo.join("flaky.test.ts"), FLAKY).unwrap();
  std::fs::create_dir_all(&data).unwrap();

  let mut command = testbed(&repo, &data, &["suite", "--again", "flaky.test.ts"]);

  command
    .env("SUITE_FLAKY_MARK", dir.path().join("seen"))
    .env("BMO_TESTBED_EVENT_HOOK", format!("f() {{ cat > {}; }}; f", event.display()));
  std::os::unix::process::CommandExt::process_group(&mut command, 0);

  let done = ended(command.spawn().unwrap());
  let once_more = fill_marks(
    &SUITE.again.once_more,
    &[("mark", &SUITE.lines.mark), ("unit", "flaky.test.ts"), ("what", &SUITE.again.whole)],
  );
  let record = std::fs::read_to_string(data.join("testbed/suite-flaky.jsonl")).unwrap();
  let payload: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&event).unwrap()).unwrap();

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(done.err.contains(&once_more), "{}", done.err);
  assert!(record.contains("passes the second time"), "{record}");
  assert_eq!(payload["event"], "suite-flaky");
  assert_eq!(
    payload["detail"]["tests"],
    serde_json::json!([{ "test": "passes the second time", "unit": "flaky.test.ts" }])
  );
}

// "count prints the tests the tree holds"
#[test]
fn the_count_prints_the_tests_the_tree_holds() {
  let dir = tempfile::tempdir().unwrap();
  let repo = repository(dir.path());

  crate::stand::git(&repo, &["add", "probe.test.ts"]);
  crate::stand::git(&repo, &["commit", "-q", "-m", "probe"]);

  let done = crate::stand::ran(&mut testbed(&repo, dir.path(), &["count"]));
  let tree =
    fill_marks(&bmo_testbed_core::policy::count::COUNT.tree, &[("typescript", "1"), ("rust", "0")]);

  assert_eq!((done.code, done.out.trim()), (0, tree.as_str()), "{}", done.err);
}

// "the suite refuses a range that is no commit before it runs anything"
#[test]
fn the_suite_refuses_a_range_that_is_no_commit_before_it_runs_anything() {
  let dir = tempfile::tempdir().unwrap();
  let repo = repository(dir.path());
  let done = ended(suite_run(&repo, dir.path(), &["--since", "no-such-commit-777"], None));
  let opening = fill_marks(&SUITE.selection.not_a_range, &[("since", "no-such-commit-777"), ("why", "")]);

  assert_eq!(done.code, 2, "{}", done.err);
  assert!(done.err.contains(opening.trim_end()), "{}", done.err);
}

// "a run that tests a crate brings the lock's path packages to their manifests for the whole run and puts the
// lock back as it was found": a sibling crate read by a path has moved its version past the one the
// checkout's committed lock names, and the crate's tests, which run Cargo holding its lock, still pass.
#[test]
fn a_crate_s_tests_pass_where_a_sibling_s_version_moved_past_the_lock() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let sibling = dir.path().join("sibling");
  let data = dir.path().join("data");
  let manifest = |name: &str, version: &str, more: &str| {
    format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2021\"\n{more}")
  };
  let lock = |version: &str| {
    format!(
      "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n\n\
       [[package]]\nname = \"sample-app\"\nversion = \"0.1.0\"\ndependencies = [\n \"sample-sibling\",\n]\n\n\
       [[package]]\nname = \"sample-sibling\"\nversion = \"{version}\"\n"
    )
  };

  std::fs::create_dir_all(sibling.join("src")).unwrap();
  std::fs::create_dir_all(&data).unwrap();
  std::fs::write(sibling.join("Cargo.toml"), manifest("sample-sibling", "0.2.0", "")).unwrap();
  std::fs::write(sibling.join("src/lib.rs"), "pub fn one() -> u8 {\n  1\n}\n").unwrap();
  checkout(&repo, "testbed = 0\n\n[test]\nlanguages = [\"cargo\"]\n");

  for (path, text) in [
    (
      "Cargo.toml",
      manifest(
        "sample-app",
        "0.1.0",
        "\n[dependencies]\nsample-sibling = { path = \"../sibling\" }\n\n[workspace]\n",
      ),
    ),
    ("Cargo.lock", lock("0.1.0")),
    ("src/lib.rs", "pub use sample_sibling::one;\n".to_owned()),
    (
      "tests/one.rs",
      "#[test]\nfn one() {\n  assert_eq!(sample_app::one(), 1);\n}\n".to_owned(),
    ),
  ] {
    std::fs::create_dir_all(repo.join(path).parent().unwrap()).unwrap();
    std::fs::write(repo.join(path), text).unwrap();
  }

  git(&repo, &["add", "-A"]);
  git(&repo, &["commit", "-qm", "crate"]);

  let mut command = testbed(&repo, &data, &["suite"]);

  command.env("BMO_CARGO_TARGET_DIR", dir.path().join("cargo-target"));
  std::os::unix::process::CommandExt::process_group(&mut command, 0);

  let done = ended(command.spawn().unwrap());

  assert_eq!(done.code, 0, "{}", done.err);
  assert!(done.err.contains("sample-sibling 0.1.0 -> 0.2.0"), "{}", done.err);
  assert_eq!(std::fs::read_to_string(repo.join("Cargo.lock")).unwrap(), lock("0.1.0"));
}

// "the crates' tests are built once, by one Cargo command naming every crate the run reaches", and "a crate's
// unit runs the test binaries written down for it, from the folder Cargo would run it from and with the
// variables Cargo would hand it": a workspace of two crates is built by one command, the first crate's test
// reads its package and folder as Cargo hands them, and the second crate's failing test fails its unit alone.
// With --again, "a binary whose own result names the tests that failed runs only those ... from the same build,
// so nothing is built again".
#[test]
fn the_crates_tests_run_from_one_build_with_what_cargo_would_hand_them() {
  let dir = tempfile::tempdir().unwrap();
  let repo = dir.path().join("repo");
  let data = dir.path().join("data");
  let manifest =
    |name: &str| format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
  let one_test = "#[test]\nfn reads_what_cargo_hands_it() {\n  \
    let folder = std::env::var(\"CARGO_MANIFEST_DIR\").unwrap();\n  \
    assert_eq!(std::env::var(\"CARGO_PKG_NAME\").unwrap(), \"sample-one\");\n  \
    assert_eq!(std::env::current_dir().unwrap(), std::path::PathBuf::from(folder));\n}\n";

  std::fs::create_dir_all(&data).unwrap();
  checkout(&repo, "testbed = 0\n\n[test]\nlanguages = [\"cargo\"]\n");

  for (path, text) in [
    ("Cargo.toml", "[workspace]\nresolver = \"2\"\nmembers = [\"one\", \"two\"]\n".to_owned()),
    (
      "Cargo.lock",
      "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n\n\
       [[package]]\nname = \"sample-one\"\nversion = \"0.1.0\"\n\n\
       [[package]]\nname = \"sample-two\"\nversion = \"0.1.0\"\n"
        .to_owned(),
    ),
    ("one/Cargo.toml", manifest("sample-one")),
    ("one/src/lib.rs", "pub fn one() -> u8 {\n  1\n}\n".to_owned()),
    ("one/tests/one.rs", one_test.to_owned()),
    ("two/Cargo.toml", manifest("sample-two")),
    (
      "two/src/lib.rs",
      "#[test]\nfn fails() {\n  assert_eq!(1 + 1, 3);\n}\n\n#[test]\nfn holds() {\n  assert_eq!(1 + 1, 2);\n}\n"
        .to_owned(),
    ),
  ] {
    std::fs::create_dir_all(repo.join(path).parent().unwrap()).unwrap();
    std::fs::write(repo.join(path), text).unwrap();
  }

  git(&repo, &["add", "-A"]);
  git(&repo, &["commit", "-qm", "crates"]);

  let mut command = testbed(&repo, &data, &["suite", "--again"]);

  command
    .env("BMO_CARGO_TARGET_DIR", dir.path().join("cargo-target"))
    .env_remove("BMO_TESTBED_EVENT_HOOK");
  std::os::unix::process::CommandExt::process_group(&mut command, 0);

  let done = ended(command.spawn().unwrap());
  let built =
    |what: &str| fill_marks(&SUITE.lines.building_tests, &[("what", what), ("count", "2"), ("code", "0")]);
  let unit = |outcome: &str, file: &str| {
    done
      .err
      .lines()
      .any(|line| line.starts_with(outcome) && line.ends_with(&format!("s  {file}")))
  };

  assert_eq!(done.code, 1, "{}", done.err);
  assert_eq!(done.err.matches(&built(&SUITE.lines.library_tests)).count(), 1, "{}", done.err);
  assert_eq!(done.err.matches(&built(&SUITE.lines.tests)).count(), 1, "{}", done.err);
  assert!(unit(&SUITE.file.ok, "one/"), "{}", done.err);
  assert!(unit(&SUITE.file.redo, "two/"), "{}", done.err);
  assert!(unit(&SUITE.file.failed, "two/"), "{}", done.err);
  assert!(done.err.contains("reads_what_cargo_hands_it ... ok"), "{}", done.err);

  let narrowed = fill_marks(&SUITE.again.tests, &[("names", "fails")]);
  let once_more = fill_marks(
    &SUITE.again.once_more,
    &[("mark", &SUITE.lines.mark), ("unit", "two/"), ("what", &narrowed)],
  );
  let second = done.err.split(&once_more).nth(1).unwrap_or_default();

  assert!(done.err.lines().any(|line| line == once_more), "{}", done.err);
  assert!(second.contains("running 1 test\n"), "{}", done.err);
  assert!(!second.contains("holds"), "{}", done.err);
}
