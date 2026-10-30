//! A call put through to the deploy verb and waited out (docs/deploy.md, 「The hosted deploy tool」): the
//! verb run in the checkout outside the harness, the unit read off its line and waited on, and the
//! deployment record judged, with stand-ins for the testbed binary and the service manager on the path.

use std::path::Path;
use std::time::Duration;

use bmo_testbed_deploy::tree::DeployRecord;
use deploy_tool::{Arguments, Ground, deploy, outcome, release_testbed, unit_of};
use serde_json::json;

use crate::stand::{
  COMMIT, HANDED_OFF, call, checkout, env, logged, record, record_path, registry, script, testbed, text,
  tool, write,
};

fn arguments(commit: &str) -> serde_json::Value {
  json!({ "repository": "agency", "commit": commit, "why": "Ship the queue fix" })
}

// The service manager's stand-ins: a unit that has ended, and a journal of two lines.
fn ended_unit(bin: &Path) {
  script(bin, "systemctl", "echo inactive\nexit 3");
  script(bin, "journalctl", "echo 'gate: the suite failed'\necho 'deployment failed'");
}

// The testbed binary's work where the deployment goes live: the record written, and the line handing off.
fn going_live(record_at: &Path, written: &str) -> String {
  format!(
    "mkdir -p '{}'\ncat > '{}' <<'END'\n{written}\nEND\necho '{HANDED_OFF}'",
    record_at.parent().unwrap().display(),
    record_at.display()
  )
}

fn recorded(commit: &str, subject: &str, at: &str) -> DeployRecord {
  serde_json::from_str(&record(commit, subject, "Because", at)).unwrap()
}

// "the unit is the word of the verb's line that names a deployment's unit"
#[test]
fn the_unit_is_read_off_the_verbs_line() {
  assert_eq!(unit_of(HANDED_OFF).as_deref(), Some("bmo-deploy-1793328193818"));
  assert_eq!(unit_of("deploy: nothing to say"), None);
  assert_eq!(unit_of("bmo-deploy- alone"), None);
}

// "the release's own binary stands beside the placed folder, and none where the executable stands
// elsewhere"
#[test]
fn the_releases_own_binary_is_found_beside_the_placed_folder() {
  let dir = tempfile::tempdir().unwrap();
  let release = dir.path().join("release");
  let exe = release.join("tools/deploy/.placed/deploy-tool");

  write(&exe, "");
  assert_eq!(release_testbed(&exe), None);

  write(&release.join("bin/bmo-testbed"), "");
  assert_eq!(release_testbed(&exe), Some(release.join("bin/bmo-testbed")));
  assert_eq!(release_testbed(&release.join("bin/deploy-tool")), None);
}

// "a record naming the commit, written after the call began, is live"
#[test]
fn a_record_naming_the_commit_and_written_anew_is_live() {
  let before = recorded("fedcba9876543210fedcba9876543210fedcba98", "Earlier work", "2026-10-29T10:00:00");
  let after = recorded(COMMIT, "Fix the queue", "2026-10-30T10:00:00");
  let answer = outcome("0123456", "bmo-deploy-7", Some(&before), Some(&after), &|| unreachable!());
  let lines: Vec<String> = answer.plain().lines().map(str::to_owned).collect();

  assert!(!answer.is_error);
  assert_eq!(lines[0], format!("live: {COMMIT} (Fix the queue)"));
  assert!(lines[1].contains("fedcba9876543210") && lines[1].contains("Earlier work"), "{lines:?}");
  assert_eq!(lines[2], "message: Because");
  assert_eq!(lines[3], "unit: bmo-deploy-7");
  assert_eq!(
    outcome("0123456", "bmo-deploy-7", None, Some(&after), &String::new)
      .plain()
      .lines()
      .nth(1),
    Some("previous: none recorded")
  );
}

// "a record still the one from before, or naming another commit, is not live and carries the journal"
#[test]
fn a_record_from_before_or_of_another_commit_is_not_live() {
  let same = recorded(COMMIT, "Fix the queue", "2026-10-29T10:00:00");
  let other = recorded("fedcba9876543210fedcba9876543210fedcba98", "Earlier work", "2026-10-29T10:00:00");
  let journal = || "the journal's tail".to_owned();
  let unchanged = outcome(COMMIT, "bmo-deploy-7", Some(&same), Some(&same), &journal);
  let elsewhere = outcome("0123456", "bmo-deploy-7", Some(&other), Some(&other), &journal);
  let none = outcome("0123456", "bmo-deploy-7", None, None, &journal);

  for answer in [&unchanged, &elsewhere, &none] {
    assert!(answer.is_error);
    assert!(answer.plain().starts_with("not live: bmo-deploy-7"), "{}", answer.plain());
    assert!(answer.plain().ends_with("the journal's tail"), "{}", answer.plain());
  }

  assert!(elsewhere.plain().contains("names fedcba98"), "{}", elsewhere.plain());
  assert!(none.plain().contains("names nothing"), "{}", none.plain());
}

// "the verb runs in the checkout with the commit and the reason, outside the harness, and its unit is
// waited out to a live record"
#[test]
fn the_verb_runs_in_the_checkout_and_its_live_record_is_answered() {
  let dir = tempfile::tempdir().unwrap();
  let bin = dir.path().join("bin");
  let root = checkout(dir.path(), "agency", "bmo-agency");
  let at = registry(dir.path(), &[&root]);
  let written = record(COMMIT, "Fix the queue", "Ship the queue fix", "2026-10-30T10:00:00");

  write(
    &record_path(dir.path(), "agency"),
    &record(
      "fedcba9876543210fedcba9876543210fedcba98",
      "Earlier work",
      "Before",
      "2026-10-29T10:00:00",
    ),
  );
  testbed(&bin, &going_live(&record_path(dir.path(), "agency"), &written));
  ended_unit(&bin);

  let ran = tool(&call(arguments("0123456789ab")).to_string(), &env(dir.path(), &bin, &at));
  let answer = ran.answer();

  assert_eq!(ran.code, 0, "{}", ran.err);
  assert_eq!(answer["isError"], false, "{}", text(&answer));
  assert_eq!(text(&answer).lines().next(), Some(format!("live: {COMMIT} (Fix the queue)").as_str()));
  assert!(text(&answer).contains("previous: fedcba98"), "{}", text(&answer));
  assert_eq!(
    logged(dir.path(), "words").lines().collect::<Vec<_>>(),
    ["deploy", "--commit=0123456789ab", "--message=Ship the queue fix"]
  );
  assert_eq!(
    std::fs::canonicalize(logged(dir.path(), "cwd").trim()).unwrap(),
    std::fs::canonicalize(&root).unwrap()
  );
  assert_eq!(logged(dir.path(), "harness"), "none none");
}

// "a deployment whose record does not name the commit answers an error with the unit's journal"
#[test]
fn a_deployment_that_did_not_go_live_answers_its_journal() {
  let dir = tempfile::tempdir().unwrap();
  let bin = dir.path().join("bin");
  let root = checkout(dir.path(), "agency", "agency");
  let at = registry(dir.path(), &[&root]);

  write(
    &record_path(dir.path(), "agency"),
    &record(
      "fedcba9876543210fedcba9876543210fedcba98",
      "Earlier work",
      "Before",
      "2026-10-29T10:00:00",
    ),
  );
  testbed(&bin, &format!("echo '{HANDED_OFF}'"));
  ended_unit(&bin);

  let answer = tool(&call(arguments("0123456")).to_string(), &env(dir.path(), &bin, &at)).answer();

  assert_eq!(answer["isError"], true);
  assert!(text(&answer).starts_with("not live: bmo-deploy-1793328193818"), "{}", text(&answer));
  assert!(text(&answer).contains("gate: the suite failed\ndeployment failed"), "{}", text(&answer));
}

// "a refusal of the verb answers an error carrying what it said"
#[test]
fn a_refusal_of_the_verb_answers_what_it_said() {
  let dir = tempfile::tempdir().unwrap();
  let bin = dir.path().join("bin");
  let root = checkout(dir.path(), "agency", "agency");
  let at = registry(dir.path(), &[&root]);

  testbed(&bin, "echo 'refused: the remote has never seen 0123456' >&2\nexit 2");

  let answer = tool(&call(arguments("0123456")).to_string(), &env(dir.path(), &bin, &at)).answer();

  assert_eq!(answer["isError"], true);
  assert!(text(&answer).contains("the deploy verb refused"), "{}", text(&answer));
  assert!(text(&answer).contains("the remote has never seen 0123456"), "{}", text(&answer));
}

// "a dry run answers what the verb said, and waits on no unit"
#[test]
fn a_dry_run_answers_what_the_verb_said() {
  let dir = tempfile::tempdir().unwrap();
  let bin = dir.path().join("bin");
  let root = checkout(dir.path(), "agency", "agency");
  let at = registry(dir.path(), &[&root]);
  let mut asked = arguments("0123456");

  asked["dry_run"] = json!(true);
  testbed(&bin, "echo 'dry run: would deploy 0123456 over fedcba98'");
  script(&bin, "systemctl", "echo active");

  let answer = tool(&call(asked).to_string(), &env(dir.path(), &bin, &at)).answer();

  assert_eq!(answer["isError"], false, "{}", text(&answer));
  assert_eq!(text(&answer), "dry run: would deploy 0123456 over fedcba98");
  assert_eq!(logged(dir.path(), "words").lines().last(), Some("--dry-run"));
}

// "a repository whose declaration deploys nothing is refused before the verb runs"
#[test]
fn a_repository_that_deploys_nothing_is_refused() {
  let dir = tempfile::tempdir().unwrap();
  let bin = dir.path().join("bin");
  let root = dir.path().join("plain");

  write(&root.join("testbed.toml"), "testbed = 0\nname = \"agency\"\n");
  testbed(&bin, "exit 0");

  let at = registry(dir.path(), &[&root]);
  let answer = tool(&call(arguments("0123456")).to_string(), &env(dir.path(), &bin, &at)).answer();

  assert_eq!(answer["isError"], true);
  assert!(text(&answer).contains(&root.to_string_lossy().into_owned()), "{}", text(&answer));
  assert!(logged(dir.path(), "words").is_empty());
}

// "the unit is asked again while it is active, and a unit active past the wait is answered as still
// running"
#[test]
fn the_unit_is_waited_on_while_it_is_active() {
  let dir = tempfile::tempdir().unwrap();
  let bin = dir.path().join("bin");
  let root = checkout(dir.path(), "agency", "agency");
  let at = registry(dir.path(), &[&root]);
  let asked = Arguments {
    repository: "agency".to_owned(),
    commit: "0123456".to_owned(),
    why: "Ship it".to_owned(),
    dry_run: None,
  };
  let count = dir.path().join("asked");

  testbed(
    &bin,
    &going_live(
      &record_path(dir.path(), "agency"),
      &record(COMMIT, "Fix the queue", "Ship it", "2026-10-30T10:00:00"),
    ),
  );
  ended_unit(&bin);
  script(
    &bin,
    "systemctl",
    &format!(
      "echo x >> '{0}'\nif [ \"$(wc -l < '{0}')\" -lt 3 ]; then echo active; else echo inactive; exit 3; fi",
      count.display()
    ),
  );

  let ground = Ground {
    env: env(dir.path(), &bin, &at),
    testbed: bin.join("bmo-testbed"),
    poll: Duration::from_millis(10),
    wait: Duration::from_secs(60),
  };
  let answer = deploy(&asked, &ground);

  assert!(!answer.is_error, "{}", answer.plain());
  assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 3);

  script(&bin, "systemctl", "echo active");

  let still = deploy(
    &asked,
    &Ground {
      wait: Duration::ZERO,
      ..ground
    },
  );

  assert!(still.is_error);
  assert!(still.plain().contains("bmo-deploy-1793328193818 is still running"), "{}", still.plain());
}
