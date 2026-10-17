//! The change check (docs/checks.md, 「The change check」): what a change named by its files and one read
//! since a commit each select, over a scratch checkout, and what a passing run and a failing one print, every
//! command answered by a stand-in; and a tool checked by its manifest, a package declaring a check and a
//! project of its own, whose steps are run as they stand.

use std::path::Path;
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::deploy_steps::DeployStep;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::policy::{fill_marks, fill_shown};
use bmo_testbed_suite::check::{
  CheckAsk, CheckPlan, default_log, default_since, plan, run_check, run_step,
};
use bmo_testbed_suite::static_check::TimedRun;

fn write(root: &Path, file: &str, text: &str) {
  let path = root.join(file);

  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
}

// A git command in the scratch repository, with no configuration of the account's and an author of its own.
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

// The declaration of the scratch checkout: the quick checks on, a type check every change owes, the web
// front end's own type check only a change under it reaches, and a tools directory.
const DECLARED: &str = r#"
testbed = 0

[test]
languages = ["bun"]

[check]
quick = ["layout"]

[[check.step]]
id = "types"
command = "bunx tsc --noEmit"

[[check.step]]
id = "web-types"
command = "bun run typecheck"
dir = "web"
when = ["web/**"]

[tools]
dir = "tools"
"#;

// Two packages, each a module and a test that loads it, and a web front end, committed on main; then a
// branch whose own commit changes the first package's module. The commit main stood on is answered beside
// the checkout.
fn checkout() -> (tempfile::TempDir, String) {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path();

  for (path, text) in [
    ("testbed.toml", DECLARED),
    ("package.json", r#"{ "name": "fixture", "workspaces": ["web", "packages/*"] }"#),
    ("web/package.json", r#"{ "name": "web" }"#),
    ("web/app.config.ts", "export default {};\n"),
    ("packages/util/package.json", r#"{ "name": "@sample/util" }"#),
    ("packages/util/numbers.ts", "export const one = 1;\n"),
    (
      "packages/util/numbers.test.ts",
      "import { one } from \"./numbers\";\n\nconsole.log(one);\n",
    ),
    ("packages/other/package.json", r#"{ "name": "@sample/other" }"#),
    ("packages/other/other.ts", "export const two = 2;\n"),
    (
      "packages/other/other.test.ts",
      "import { two } from \"./other\";\n\nconsole.log(two);\n",
    ),
  ] {
    write(root, path, text);
  }

  git(root, &["init", "-q", "-b", "main"]);
  git(root, &["add", "-A"]);
  git(root, &["commit", "-qm", "base"]);

  let base = git(root, &["rev-parse", "HEAD"]);

  git(root, &["switch", "-qc", "work/1"]);
  write(root, "packages/util/numbers.ts", "export const one = 11;\n");
  git(root, &["commit", "-qam", "the branch's own"]);

  (dir, base)
}

fn repo(root: &Path) -> Repository {
  Repository::read(root).unwrap()
}

fn names(plan: &CheckPlan) -> Vec<String> {
  plan.steps.iter().map(|step| step.name.clone()).collect()
}

fn files(given: &[&str]) -> CheckAsk {
  CheckAsk {
    since: None,
    files: given.iter().map(|file| (*file).to_owned()).collect(),
  }
}

#[test]
fn a_change_read_since_the_commit_its_branch_was_made_from_selects_the_tests_that_load_what_it_changed() {
  let (dir, base) = checkout();
  let repo = repo(dir.path());
  let since = |commit: &str| CheckAsk {
    since: Some(commit.to_owned()),
    files: vec![],
  };
  let named = plan(&repo, &since(&base)).unwrap();
  let defaulted = plan(&repo, &CheckAsk::default()).unwrap();

  assert_eq!(default_since(&repo), base);
  assert_eq!(named.changed, ["packages/util/numbers.ts"]);
  assert_eq!(named.touched.tests, ["packages/util/numbers.test.ts"]);
  assert_eq!(defaulted.touched, named.touched);

  // The quick checks and the declared steps the change reaches first, the web front end's left out of a
  // change that never reaches it, and nothing of Cargo's in a checkout with no workspace of crates.
  assert_eq!(names(&named), [DeployStep::Quick.name(), "types", "bun test"]);
  assert_eq!(
    named.steps.iter().find(|step| step.tests).unwrap().step.command,
    ["bun", "test", "./packages/util/numbers.test.ts"]
  );

  // Read since HEAD on the branch, nothing is changed and no test is owed.
  let still = plan(&repo, &since("HEAD")).unwrap();

  assert!(still.changed.is_empty());
  assert!(!still.steps.iter().any(|step| step.tests));
}

// "the touched test files are one step for each folder they run from, the folder of their runner's
// configuration or else the root, each handed by its path from there"
#[test]
fn touched_test_files_run_one_step_per_folder_of_their_configuration() {
  let (dir, _) = checkout();
  let root = dir.path();

  write(root, "packages/util/bunfig.toml", "");
  write(root, "packages/other/bunfig.toml", "");
  write(root, "top.test.ts", "console.log(0);\n");

  let planned = plan(
    &repo(root),
    &files(&["packages/util/numbers.test.ts", "packages/other/other.test.ts", "top.test.ts"]),
  )
  .unwrap();
  let tests: Vec<(String, String, Vec<String>)> = planned
    .steps
    .iter()
    .filter(|step| step.tests)
    .map(|step| (step.name.clone(), step.step.dir.display().to_string(), step.step.command.clone()))
    .collect();
  let ran = |name: &str, dir: &str, file: &str| {
    (name.to_owned(), dir.to_owned(), ["bun", "test", file].map(str::to_owned).to_vec())
  };

  assert_eq!(
    tests,
    [
      ran("bun test", ".", "./top.test.ts"),
      ran("bun test in packages/other", "packages/other", "./other.test.ts"),
      ran("bun test in packages/util", "packages/util", "./numbers.test.ts"),
    ]
  );
}

#[test]
fn a_change_named_by_its_files_selects_by_them_alone_whatever_the_branch_committed() {
  let (dir, _) = checkout();
  let repo = repo(dir.path());
  let given = plan(&repo, &files(&["packages/other/other.ts"])).unwrap();

  assert_eq!(given.from, "1 file(s) given");
  assert_eq!(given.touched.tests, ["packages/other/other.test.ts"]);

  // A change under the web front end reaches the step declared for it, run in its directory.
  let web = plan(&repo, &files(&["web/app.config.ts"])).unwrap();

  assert!(names(&web).contains(&"web-types in web".to_owned()), "{:?}", names(&web));

  // A test file given is a test the change owes itself.
  let test = plan(&repo, &files(&["packages/util/numbers.test.ts"])).unwrap();

  assert_eq!(test.touched.tests, ["packages/util/numbers.test.ts"]);
}

fn ran(code: i32, output: &str) -> TimedRun {
  TimedRun {
    code,
    output: output.to_owned(),
    waited_seconds: 0.0,
    ran_seconds: 1.0,
  }
}

#[test]
fn a_passing_run_says_each_step_went_through_and_what_the_log_verb_read_of_the_tests() {
  let (dir, base) = checkout();
  let log = dir.path().join("check.log");
  let planned = plan(
    &repo(dir.path()),
    &CheckAsk {
      since: Some(base),
      files: vec![],
    },
  )
  .unwrap();
  let checked = run_check(dir.path(), &planned, &log, &|_, step| {
    if step.step.command[1..] == ["log", &log.to_string_lossy()] {
      ran(0, "pass 3 · fail 0 · files 1\n")
    } else {
      ran(0, "fine\n")
    }
  });

  let words = &SUITE.check;
  let tests = planned.steps.iter().find(|step| step.tests).unwrap();

  assert_eq!(checked.code, 0, "{:?}", checked.lines);
  assert_eq!(
    checked.lines[0],
    fill_shown(
      &words.heading,
      &[
        ("changed", &planned.changed.len()),
        ("from", &planned.from),
        ("tests", &planned.touched.tests.len()),
        ("tested", &planned.touched.tested_crates.len()),
        ("checked", &planned.touched.checked_crates.len()),
        ("projects", &planned.touched.projects.len()),
        ("tools", &planned.touched.tools.len()),
      ],
    )
  );
  assert!(
    planned.from.starts_with(&fill_shown(&words.since, &[("since", &"")])),
    "{}",
    planned.from
  );
  assert!(
    checked.lines.contains(&fill_shown(
      &words.step,
      &[("outcome", &words.ok), ("name", &tests.name), ("code", &0), ("ran", &1), ("waited", &0)],
    )),
    "{:?}",
    checked.lines
  );
  // The readers of the check's output match a step's line by these openings.
  assert!(checked.lines.iter().any(|line| line.starts_with("ok bun test")), "{:?}", checked.lines);
  assert!(checked.lines.iter().all(|line| !line.starts_with("FAILED")), "{:?}", checked.lines);
  assert!(checked.lines.contains(&"pass 3 · fail 0 · files 1".to_owned()), "{:?}", checked.lines);
  assert_eq!(checked.lines.last().unwrap(), &fill_shown(&words.log, &[("path", &log.display())]));
  assert!(std::fs::read_to_string(&log).unwrap().contains(&fill_shown(
    &words.logged,
    &[("name", &tests.name), ("command", &tests.step.command.join(" ")), ("code", &0)],
  )));
}

// "the first lines a failing static step printed": as many as the policy names, the rest left to the log
#[test]
fn a_failing_static_step_shows_as_many_of_its_lines_as_the_policy_names() {
  let (dir, base) = checkout();
  let log = dir.path().join("check.log");
  let shown = SUITE.numbers.check_shown_lines;
  let output: String = (1..=shown + 5).map(|at| format!("fault {at}\n")).collect();
  let planned = plan(
    &repo(dir.path()),
    &CheckAsk {
      since: Some(base),
      files: vec![],
    },
  )
  .unwrap();
  let checked = run_check(dir.path(), &planned, &log, &|_, step| {
    if step.step.command.iter().any(|word| word.contains("tsc")) {
      ran(2, &output)
    } else {
      ran(0, "fine\n")
    }
  });
  let faults: Vec<&String> = checked.lines.iter().filter(|line| line.starts_with("  fault ")).collect();

  assert_eq!(faults.len(), shown, "{:?}", checked.lines);
  assert_eq!(faults.last().map(|line| line.as_str()), Some(format!("  fault {shown}").as_str()));
  assert!(std::fs::read_to_string(&log).unwrap().contains(&format!("fault {}", shown + 5)));
}

#[test]
fn a_failing_run_runs_every_step_to_its_end_and_names_what_failed_without_the_tests_own_output() {
  let (dir, base) = checkout();
  let log = dir.path().join("check.log");
  let planned = plan(
    &repo(dir.path()),
    &CheckAsk {
      since: Some(base),
      files: vec![],
    },
  )
  .unwrap();
  let checked = run_check(dir.path(), &planned, &log, &|_, step| {
    let command = &step.step.command;

    if command[1..] == ["log", &log.to_string_lossy()] {
      ran(1, "pass 2 · fail 1 · files 1\n\n1 failure(s) the log names:\n  numbers > counts\n")
    } else if command.iter().any(|word| word.contains("tsc")) {
      ran(2, "packages/util/numbers.ts(1,1): error TS2322: wrong\n")
    } else if step.tests {
      ran(1, "(fail) numbers > counts\nthe tests' own noise\n")
    } else {
      ran(0, "fine\n")
    }
  });

  assert_eq!(checked.code, 1);
  assert!(
    checked
      .lines
      .iter()
      .any(|line| line.starts_with("FAILED types") && line.contains("exit 2")),
    "{:?}",
    checked.lines
  );
  assert!(
    checked.lines.contains(&fill_shown(
      &SUITE.check.step,
      &[
        ("outcome", &SUITE.check.failed),
        ("name", &"types"),
        ("code", &2),
        ("ran", &1),
        ("waited", &0)
      ],
    )),
    "{:?}",
    checked.lines
  );
  assert!(
    checked
      .lines
      .contains(&"  packages/util/numbers.ts(1,1): error TS2322: wrong".to_owned()),
    "{:?}",
    checked.lines
  );
  assert!(
    checked.lines.iter().any(|line| line.starts_with("FAILED bun test")),
    "{:?}",
    checked.lines
  );
  assert!(checked.lines.contains(&"  numbers > counts".to_owned()), "{:?}", checked.lines);
  assert!(
    !checked.lines.iter().any(|line| line.contains("the tests' own noise")),
    "{:?}",
    checked.lines
  );
  assert!(std::fs::read_to_string(&log).unwrap().contains("the tests' own noise"));
}

// Two tools checked by their manifest beside the packages: `shout`, whose check writes the folder it runs in
// into a file there, and `quiet`, whose manifest names no check.
fn with_tools(root: &Path) {
  let manifest = |name: &str, check: &str| {
    format!(
      "[tool]\nname = \"{name}\"\n{check}run = [\"python3\", \"{name}.py\"]\n\n[[entry]]\nname = \"{name}\"\ndescription = \"Says a text back.\"\n"
    )
  };

  for name in ["shout", "quiet"] {
    write(root, &format!("tools/{name}/{name}.py"), "print('said')\n");
  }

  write(root, "tools/shout/tool.toml", &manifest("shout", "check = 'pwd -P > checked'\n"));
  write(root, "tools/quiet/tool.toml", &manifest("quiet", ""));
}

#[test]
fn a_change_inside_a_tool_no_crate_or_package_holds_runs_its_manifest_s_check_in_its_folder_once() {
  let (dir, _) = checkout();
  let root = dir.path();

  with_tools(root);

  let planned = plan(&repo(root), &files(&["tools/shout/shout.py", "tools/shout/tool.toml"])).unwrap();

  assert_eq!(names(&planned), [DeployStep::Quick.name(), "types", "tools/shout/ check"]);

  let step = planned.steps.last().unwrap();
  let placed = root.join("tools/shout");

  assert_eq!(step.step.dir, Path::new("tools/shout"));

  let done = run_step(root, step);

  assert_eq!(done.code, 0, "{}", done.output);
  assert_eq!(
    std::fs::read_to_string(placed.join("checked")).unwrap().trim(),
    placed.canonicalize().unwrap().to_string_lossy()
  );
}

#[test]
fn a_tool_whose_manifest_names_no_check_fails_its_step_with_why() {
  let (dir, _) = checkout();
  let root = dir.path();

  with_tools(root);

  let planned = plan(&repo(root), &files(&["tools/quiet/quiet.py"])).unwrap();
  let step = planned.steps.iter().find(|step| step.name == "tools/quiet/ check").unwrap();
  let done = run_step(root, step);

  assert_ne!(done.code, 0);
  assert_eq!(
    done.output.trim(),
    fill_marks(&SUITE.file.no_tool_check, &[("manifest", "tool.toml"), ("key", "tool.check")])
  );
}

// A package whose folder holds a Python part beside it, `relay/server`, which it reads and checks by the
// command its own section declares.
fn with_package_check(root: &Path) {
  write(
    root,
    "package.json",
    r#"{ "name": "fixture", "workspaces": ["web", "packages/*", "relay/relay"] }"#,
  );
  write(
    root,
    "relay/relay/package.json",
    r#"{ "name": "relay", "bmo": { "reads": ["relay/server/**"], "check": "pwd -P > ../server/checked" } }"#,
  );
  write(root, "relay/relay/relay.ts", "export const relay = 1;\n");
  write(root, "relay/server/serve.py", "print('served')\n");
}

#[test]
fn a_change_a_package_owns_or_reads_runs_the_check_it_declares_inside_its_own_folder() {
  let (dir, _) = checkout();
  let root = dir.path();

  with_package_check(root);

  let repo = repo(root);

  for (file, owed) in [
    ("relay/server/serve.py", true),
    ("relay/relay/relay.ts", true),
    ("packages/util/numbers.ts", false),
  ] {
    let planned = plan(&repo, &files(&[file])).unwrap();

    assert_eq!(names(&planned).contains(&"relay/relay/ check".to_owned()), owed, "{file}");
  }

  let planned = plan(&repo, &files(&["relay/server/serve.py"])).unwrap();
  let step = planned.steps.iter().find(|step| step.name == "relay/relay/ check").unwrap();
  let done = run_step(root, step);

  assert_eq!(done.code, 0, "{}", done.output);
  assert_eq!(
    std::fs::read_to_string(root.join("relay/server/checked")).unwrap().trim(),
    root.join("relay/relay").canonicalize().unwrap().to_string_lossy()
  );
}

// Two projects of their own beside the workspace, each with its own manifest and lockfile: `site`, whose
// manifest names the check script, and `bare`, whose manifest names none.
fn with_projects(root: &Path) {
  write(
    root,
    "site/package.json",
    r#"{ "name": "site", "scripts": { "check": "tsc && bun test" } }"#,
  );
  write(root, "site/bun.lock", "{}\n");
  write(root, "site/page.ts", "export const page = 1;\n");
  write(root, "bare/package.json", r#"{ "name": "bare" }"#);
  write(root, "bare/bun.lock", "{}\n");
  write(root, "bare/page.ts", "export const page = 1;\n");
  git(root, &["add", "-A"]);
}

#[test]
fn a_change_inside_a_project_of_its_own_runs_its_check_script_there_and_only_that() {
  let (dir, _) = checkout();
  let root = dir.path();

  with_projects(root);

  let planned = plan(&repo(root), &files(&["site/page.ts"])).unwrap();

  assert_eq!(names(&planned), [DeployStep::Quick.name(), "types", "site/ check"]);
  assert_eq!(planned.touched.projects.len(), 1);
  assert!(planned.touched.tests.is_empty());

  let step = planned.steps.last().unwrap();

  assert_eq!(step.step.command, ["bun", "run", "check"]);
  assert_eq!(step.step.dir, Path::new("site"));
}

#[test]
fn a_project_whose_manifest_names_no_check_script_fails_its_step_with_why() {
  let (dir, _) = checkout();
  let root = dir.path();

  with_projects(root);

  let planned = plan(&repo(root), &files(&["bare/page.ts"])).unwrap();
  let step = planned.steps.iter().find(|step| step.name == "bare/ check").unwrap();
  let done = run_step(root, step);

  assert_ne!(done.code, 0);
  assert_eq!(done.output.trim(), fill_marks(&SUITE.file.no_check, &[("script", "check")]));
}

#[test]
fn a_log_left_unnamed_is_one_file_per_checkout_under_the_data_root() {
  let data = Path::new("/data");
  let one = default_log(data, Path::new("/repos/a/sample"));
  let other = default_log(data, Path::new("/repos/b/sample"));

  assert_ne!(one, other);
  assert_eq!(one.parent(), Some(data.join("testbed").as_path()));
  assert!(one.file_name().unwrap().to_string_lossy().starts_with("check-sample-"));
  assert_eq!(default_log(data, Path::new("/repos/a/sample")), one);
}
