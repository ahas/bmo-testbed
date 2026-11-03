//! The static check (docs/checks.md, 「The static check」): which steps a change owes, how they run, a
//! command's waits read back, and where a selected run begins.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::{Declaration, Repository};
use bmo_testbed_core::policy::deploy_steps::DeployStep;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::static_check::STATIC_CHECK;
use bmo_testbed_core::turn::{Lane, NOT_RUN_EXIT_CODE, TURN_TIMES_VARIABLE, times_words};
use bmo_testbed_suite::cargo::cargo_invocation;
use bmo_testbed_suite::crates::CrateDeclaration;
use bmo_testbed_suite::static_check::{
  CheckStep, OwnPackages, StepDone, TimedRun, cargo_format, cargo_lint, check_steps, newest_vetted,
  quick_check, run_static_check, run_timed, step_words,
};

fn write(root: &Path, file: &str, text: &str) {
  let path = root.join(file);

  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
}

// A git command in a scratch repository, with no configuration of the account's and an author of its own.
fn git(at: &Path, args: &[&str]) -> String {
  let done = Command::new("git")
    .args(args)
    .current_dir(at)
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_SYSTEM", "/dev/null")
    .env("GIT_AUTHOR_NAME", "tester")
    .env("GIT_AUTHOR_EMAIL", "tester@example.com")
    .env("GIT_COMMITTER_NAME", "tester")
    .env("GIT_COMMITTER_EMAIL", "tester@example.com")
    .stdin(Stdio::null())
    .output()
    .unwrap();

  assert!(
    done.status.success(),
    "git {}: {}",
    args.join(" "),
    String::from_utf8_lossy(&done.stderr)
  );
  String::from_utf8_lossy(&done.stdout).trim().to_owned()
}

// A declaration enabling a quick check and declaring two steps: one every change owes, and one only a
// change under the web front end or the server it stands on reaches.
const DECLARED: &str = r#"
testbed = 0

[check]
quick = ["layout"]

[[check.step]]
id = "types"
command = "tsc --noEmit"

[[check.step]]
id = "web-types"
command = "bun run typecheck"
dir = "web"
lane = "narrow"
when = ["web/**", "server/**"]
retry_out_of_memory = true
"#;

// A committed repository with a declaration of its own.
struct Space {
  dir: tempfile::TempDir,
}

impl Space {
  fn new() -> Self {
    Self::with(&[
      ("testbed.toml", DECLARED),
      ("packages/util/numbers.ts", "export const one = 1;\n"),
      ("server/conversation.ts", "export const round = 1;\n"),
      ("web/app.ts", "export default {};\n"),
    ])
  }

  fn with(files: &[(&str, &str)]) -> Self {
    let dir = tempfile::tempdir().unwrap();

    for (path, text) in files {
      write(dir.path(), path, text);
    }

    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "start"]);

    Self { dir }
  }

  fn root(&self) -> &Path {
    self.dir.path()
  }

  fn repo(&self) -> Repository {
    Repository::read(self.root()).unwrap()
  }

  fn head(&self) -> String {
    git(self.root(), &["rev-parse", "HEAD"])
  }

  fn commit(&self, path: &str, text: &str) -> String {
    write(self.root(), path, text);
    git(self.root(), &["add", "-A"]);
    git(self.root(), &["commit", "-qm", &format!("change {path}")]);
    self.head()
  }
}

fn ids(steps: &[CheckStep]) -> Vec<&str> {
  steps.iter().map(|step| step.id.as_str()).collect()
}

mod which_steps_a_change_owes {
  use super::*;

  // "the quick checks first, then the declared steps the change reaches, in their order"
  #[test]
  fn the_quick_checks_first_then_the_declared_steps_the_change_reaches_in_their_order() {
    let space = Space::new();
    let start = space.head();

    space.commit("packages/util/numbers.ts", "export const one = 2;\n");
    assert_eq!(ids(&check_steps(&space.repo(), Some(&start))), [DeployStep::Quick.name(), "types"]);

    space.commit("server/conversation.ts", "export const round = 2;\n");
    assert_eq!(
      ids(&check_steps(&space.repo(), Some(&start))),
      [DeployStep::Quick.name(), "types", "web-types"]
    );
  }

  // "a declared step runs its command through sh in the directory, the lane and with the retry it declares"
  #[test]
  fn a_declared_step_runs_its_command_through_sh_in_the_directory_the_lane_and_with_the_retry_it_declares()
  {
    let space = Space::new();
    let steps = check_steps(&space.repo(), None);
    let web = steps.iter().find(|step| step.id == "web-types").unwrap();
    let types = steps.iter().find(|step| step.id == "types").unwrap();

    assert_eq!(web.command, ["sh", "-c", "bun run typecheck"]);
    assert_eq!(web.dir, PathBuf::from("web"));
    assert_eq!(web.lane, Some(Lane::Narrow));
    assert!(web.retry_out_of_memory);
    assert_eq!(types.dir, PathBuf::from("."));
    assert_eq!(types.lane, None);
    assert!(!types.retry_out_of_memory);
    assert_eq!(steps[0], quick_check());
  }

  // "the Cargo workspace's formatting and lints where the change marks a crate, after the declared steps"
  #[test]
  fn the_cargo_workspace_s_formatting_and_lints_where_the_change_marks_a_crate() {
    let space = Space::new();

    space.commit("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/*\"]\n");
    space.commit(
      "crates/alpha/Cargo.toml",
      "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );

    let start = space.commit("crates/alpha/src/lib.rs", "pub fn one() -> u8 { 1 }\n");

    space.commit("packages/util/numbers.ts", "export const one = 2;\n");
    assert_eq!(ids(&check_steps(&space.repo(), Some(&start))), [DeployStep::Quick.name(), "types"]);

    space.commit("crates/alpha/src/lib.rs", "pub fn one() -> u8 { 2 }\n");

    let steps = check_steps(&space.repo(), Some(&start));

    assert_eq!(ids(&steps), [DeployStep::Quick.name(), "types", "cargo format", "cargo lint"]);
    assert_eq!(steps[2..], [cargo_format(&OwnPackages::Workspace), cargo_lint(&OwnPackages::Workspace)]);
  }

  // "a run with no range and one that cannot be read owe every step"
  #[test]
  fn a_run_with_no_range_and_one_that_cannot_be_read_owe_every_step() {
    let space = Space::new();
    let whole = [DeployStep::Quick.name(), "types", "web-types"];

    assert_eq!(ids(&check_steps(&space.repo(), None)), whole);
    assert_eq!(ids(&check_steps(&space.repo(), Some("no-such-commit"))), whole);
  }

  // "a repository enabling no quick check and declaring no step owes nothing"
  #[test]
  fn a_repository_enabling_no_quick_check_and_declaring_no_step_owes_nothing() {
    let space = Space::with(&[("server/conversation.ts", "export const round = 1;\n")]);
    let bare = Repository {
      root: space.root().to_path_buf(),
      declaration: Declaration::default(),
    };

    assert!(check_steps(&bare, None).is_empty());
  }
}

mod running_the_check {
  use super::*;

  fn done(code: i32, output: String) -> TimedRun {
    TimedRun {
      code,
      output,
      waited_seconds: 0.0,
      ran_seconds: 0.0,
    }
  }

  fn steps() -> Vec<CheckStep> {
    check_steps(&Space::new().repo(), None)
  }

  // "runs the steps in order, writes each one's output, and passes where all did"
  #[test]
  fn runs_the_steps_in_order_writes_each_one_s_output_and_passes_where_all_did() {
    let steps = steps();
    let mut written: Vec<String> = Vec::new();
    let mut told: Vec<StepDone> = Vec::new();
    let ran: RefCell<Vec<(PathBuf, PathBuf)>> = RefCell::new(Vec::new());
    let result = run_static_check(
      Path::new("/checkout"),
      &steps[1..],
      &mut |text| written.push(text.to_owned()),
      &|root, step| {
        ran.borrow_mut().push((root.to_path_buf(), step.dir.clone()));
        done(0, step.command.join(" "))
      },
      &mut |one| told.push(one.clone()),
    );

    assert_eq!(result, None);
    assert_eq!(
      ran.into_inner(),
      [
        (PathBuf::from("/checkout"), PathBuf::from(".")),
        (PathBuf::from("/checkout"), PathBuf::from("web"))
      ]
    );
    assert_eq!(told[0].id, "types");
    assert_eq!(
      written[0],
      format!(
        "{}\n{}\n",
        fill_marks(
          &STATIC_CHECK.ended,
          &[
            ("step", &step_words(&steps[1])),
            ("code", "0"),
            ("times", &times_words(told[0].waited_seconds, told[0].ran_seconds)),
          ],
        ),
        steps[1].command.join(" ")
      )
    );
  }

  // "every step runs to its end, and the first that failed is named in one line"
  #[test]
  fn every_step_runs_to_its_end_and_the_first_that_failed_is_named_in_one_line() {
    let steps = steps();
    let ran: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let result = run_static_check(
      Path::new("/checkout"),
      &steps,
      &mut |_| {},
      &|_, step| {
        ran.borrow_mut().push(step.id.clone());
        done(if step.id == "types" { 0 } else { 2 }, "error: a sample failure".to_owned())
      },
      &mut |_| {},
    );

    assert_eq!(
      result,
      Some(fill_marks(&STATIC_CHECK.failed, &[("step", &step_words(&steps[0])), ("code", "2")]))
    );
    assert_eq!(ran.into_inner(), ids(&steps));
  }

  // "a step whose turn never came is not run rather than failed, and a failure elsewhere outranks it"
  #[test]
  fn a_step_whose_turn_never_came_is_not_run_rather_than_failed_and_a_failure_elsewhere_outranks_it() {
    let steps = steps();
    let not_run = i32::from(NOT_RUN_EXIT_CODE);
    let mut told: Vec<(String, i32)> = Vec::new();
    let result = run_static_check(
      Path::new("/checkout"),
      &steps[2..],
      &mut |_| {},
      &|_, _| done(not_run, String::new()),
      &mut |one| told.push((one.id.clone(), one.code)),
    );

    assert_eq!(
      result,
      Some(fill_marks(
        &STATIC_CHECK.not_had,
        &[("mark", &STATIC_CHECK.not_run), ("step", &step_words(&steps[2]))]
      ))
    );
    assert_eq!(told, [("web-types".to_owned(), not_run)]);

    let result = run_static_check(
      Path::new("/checkout"),
      &steps[1..],
      &mut |_| {},
      &|_, step| done(if step.id == "types" { not_run } else { 1 }, String::new()),
      &mut |_| {},
    );

    assert_eq!(
      result,
      Some(fill_marks(&STATIC_CHECK.failed, &[("step", &step_words(&steps[2])), ("code", "1")]))
    );
  }
}

mod a_run_with_its_waits_read_back {
  use super::*;

  // "takes its wait from what the heavy commands under it wrote, and the rest of its time as its run"
  #[test]
  fn takes_its_wait_from_what_the_heavy_commands_under_it_wrote_and_the_rest_of_its_time_as_its_run() {
    let times = serde_json::json!({
      "command": "build",
      "lane": "narrow",
      "waitedSeconds": 0.25,
      "ranSeconds": 1,
      "at": 1,
    });
    let script = format!("printf '%s\\n' \"$1\" >> \"${TURN_TIMES_VARIABLE}\"; echo built");
    let command = vec!["sh".to_owned(), "-c".to_owned(), script, "sh".to_owned(), times.to_string()];
    let run = run_timed(&command, &std::env::temp_dir(), &[]);

    assert_eq!(run.code, 0);
    assert_eq!(run.output, "built\n");
    assert!(run.waited_seconds > 0.0 && run.waited_seconds <= 0.25);
    assert!(run.ran_seconds >= 0.0);
  }
}

mod where_a_selected_run_begins {
  use super::*;

  // "the newest commit HEAD descends from among the vetted ones, passing over what it does not carry"
  #[test]
  fn the_newest_commit_head_descends_from_among_the_vetted_ones_passing_over_what_it_does_not_carry() {
    let space = Space::new();
    let first = space.head();
    let second = space.commit("server/conversation.ts", "export const round = 2;\n");

    space.commit("server/conversation.ts", "export const round = 3;\n");

    assert_eq!(
      newest_vetted(space.root(), &[Some(first.clone()), Some(second.clone()), Some("f".repeat(40)), None]),
      Some(second)
    );
    assert_eq!(newest_vetted(space.root(), &[Some(first.clone())]), Some(first));
  }

  // "none it carries is a whole run"
  #[test]
  fn none_it_carries_is_a_whole_run() {
    let space = Space::new();

    assert_eq!(newest_vetted(space.root(), &[Some("f".repeat(40)), None]), None);
    assert_eq!(newest_vetted(space.root(), &[]), None);
  }
}

mod the_gate_judges_the_repository_alone {
  use super::*;

  const FORMATTED: &str = "pub fn one() -> u8 {\n    1\n}\n";
  const UNFORMATTED: &str = "pub fn   one( ) -> u8 { 1 }\n";

  fn package(name: &str, dependencies: &str) -> String {
    format!(
      "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependencies}"
    )
  }

  // Two repositories side by side: a sibling whose workspace holds a client and a host, and the deployed one,
  // whose crate names the sibling's client by a path, the way a workspace reaches a sibling repository.
  struct Siblings {
    dir: tempfile::TempDir,
  }

  impl Siblings {
    fn new() -> Self {
      let dir = tempfile::tempdir().unwrap();
      let sibling = dir.path().join("sibling");
      let deployed = dir.path().join("deployed");

      write(
        &sibling,
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"client\", \"host\"]\n",
      );
      write(&sibling, "client/Cargo.toml", &package("sibling-client", ""));
      write(&sibling, "client/src/lib.rs", FORMATTED);
      write(&sibling, "host/Cargo.toml", &package("sibling-host", ""));
      write(&sibling, "host/src/lib.rs", FORMATTED);
      write(&deployed, "testbed.toml", "testbed = 0\n");
      write(&deployed, "Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/app\"]\n");
      write(
        &deployed,
        "crates/app/Cargo.toml",
        &package("app", "sibling-client = { path = \"../../../sibling/client\" }\n"),
      );
      write(&deployed, "crates/app/src/lib.rs", FORMATTED);

      for root in [&sibling, &deployed] {
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "start"]);
      }

      Self { dir }
    }

    fn sibling(&self) -> PathBuf {
      self.dir.path().join("sibling")
    }

    fn deployed(&self) -> PathBuf {
      self.dir.path().join("deployed")
    }

    // The deployed repository's formatting step run as the Cargo verb runs it: whether it passed.
    fn formatting_passes(&self) -> bool {
      let repo = Repository::read(&self.deployed()).unwrap();
      let steps = check_steps(&repo, None);
      let step = steps.iter().find(|step| step.id == DeployStep::CargoFormat.name()).unwrap();
      let invocation = cargo_invocation(&self.deployed(), &step.command[2..]);
      let done = invocation.command(&self.deployed()).output().unwrap();

      done.status.success()
    }
  }

  // "a sibling's unformatted uncommitted file does not fail the deployed repository's formatting"
  #[test]
  fn a_sibling_s_unformatted_uncommitted_file_does_not_fail_the_deployed_repository_s_formatting() {
    let siblings = Siblings::new();

    write(&siblings.sibling(), "host/src/lib.rs", UNFORMATTED);
    write(&siblings.sibling(), "client/src/lib.rs", UNFORMATTED);

    assert!(siblings.formatting_passes());
  }

  // "the deployed repository's own unformatted file still fails its formatting"
  #[test]
  fn the_deployed_repository_s_own_unformatted_file_still_fails_its_formatting() {
    let siblings = Siblings::new();

    write(&siblings.deployed(), "crates/app/src/lib.rs", UNFORMATTED);

    assert!(!siblings.formatting_passes());
  }

  fn member(name: &str, dir: &str) -> CrateDeclaration {
    CrateDeclaration {
      name: name.to_owned(),
      dir: dir.to_owned(),
      ..CrateDeclaration::default()
    }
  }

  // "a workspace inside its root is judged whole, and one reaching outside names its own members"
  #[test]
  fn a_workspace_inside_its_root_is_judged_whole_and_one_reaching_outside_names_its_own_members() {
    let inside = [member("app", "crates/app"), member("root", "")];

    assert_eq!(OwnPackages::among(&inside), OwnPackages::Workspace);

    let reaching = [
      member("app", "crates/app"),
      member("other", "/elsewhere/other"),
      member("tool", "tools/tool"),
    ];
    let own = OwnPackages::among(&reaching);

    assert_eq!(own, OwnPackages::Named(vec!["app".to_owned(), "tool".to_owned()]));
    assert_eq!(cargo_format(&own).command[1..], ["cargo", "fmt", "--check", "-p", "app", "-p", "tool"]);
    assert_eq!(
      cargo_lint(&own).command[1..],
      [
        "cargo",
        "clippy",
        "-p",
        "app",
        "-p",
        "tool",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings"
      ]
    );
    assert_eq!(
      cargo_lint(&OwnPackages::Workspace).command[1..],
      ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"]
    );
    assert_eq!(cargo_format(&OwnPackages::Workspace).command[1..], ["cargo", "fmt", "--check"]);
    assert!(!OwnPackages::Named(Vec::new()).any());
  }
}
