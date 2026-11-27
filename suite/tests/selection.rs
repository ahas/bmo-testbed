//! Which units a change reaches (docs/checks.md, 「Which tests a change runs」): a module's imports read
//! off its source by oxc, followed to the checkout's own files, and the rule over a scratch workspace.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_suite::crates::{CrateDeclaration, workspace_crates};
use bmo_testbed_suite::imports::{WorkspacePackage, module_specifiers, resolve_import};
use bmo_testbed_suite::log::failed_files_of;
use bmo_testbed_suite::selection::{
  ChangedPaths, Layout, MarkReason, Selection, changed_paths, glob_matches, import_closure, select_crates,
  select_packages, selected_tests, selection_lines, tracked_paths,
};
use bmo_testbed_suite::tool_checks::CheckedTool;
use bmo_testbed_suite::workspace::{OwnProject, git_lines, own_projects};

fn write(root: &Path, file: &str, text: &str) {
  let path = root.join(file);

  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
}

fn strings(given: &[&str]) -> Vec<String> {
  given.iter().map(|one| (*one).to_owned()).collect()
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

#[test]
fn a_module_names_what_it_imports_exports_from_loads_requires_and_mocks_and_nothing_its_text_merely_mentions()
 {
  let source = r#"
    import { a } from "./a";
    import "./side";
    import type { T } from "@sample/types/t";
    export { b } from "../b";
    export * from "./all";
    const late = await import("./late");
    const old = require(`./old`);
    mock.module("./mocked", () => ({}));
    // import { c } from "./in-a-comment";
    const text = 'import { d } from "./in-a-string"';
    import("./a");
  "#;

  assert_eq!(
    module_specifiers("x.ts", source),
    ["./a", "./side", "@sample/types/t", "../b", "./all", "./late", "./old", "./mocked"]
  );
}

#[test]
fn a_single_file_component_names_what_its_scripts_import() {
  let source = r#"<script setup lang="ts">
import One from "./One.vue";
const two: number = 2;
</script>

<template><div>{{ two }}</div></template>
"#;

  assert_eq!(module_specifiers("Page.vue", source), ["./One.vue"]);
}

#[test]
fn an_import_resolves_to_the_checkout_file_the_runtime_would_load() {
  let root = tempfile::tempdir().unwrap();
  let packages = [WorkspacePackage {
    dir: "packages/util".to_owned(),
    name: "@sample/util".to_owned(),
  }];

  write(root.path(), "scripts/run.ts", "");
  write(root.path(), "scripts/lib/index.ts", "");
  write(root.path(), "packages/util/numbers.ts", "");
  write(root.path(), "packages/util/old.ts", "");

  let found = |spec: &str| resolve_import(root.path(), "scripts/run.ts", spec, &packages);

  assert_eq!(found("./lib").as_deref(), Some("scripts/lib/index.ts"));
  assert_eq!(found("@sample/util/numbers").as_deref(), Some("packages/util/numbers.ts"));
  assert_eq!(found("../packages/util/old.js").as_deref(), Some("packages/util/old.ts"));
  assert_eq!(found("@sample/other/thing"), None);
  assert_eq!(found("zod"), None);
}

#[test]
fn a_glob_keeps_a_star_inside_one_directory_and_lets_two_cross_them() {
  assert!(glob_matches("prompts/**", "prompts/persona/guide.md"));
  assert!(glob_matches("docs/*.md", "docs/web.md"));
  assert!(!glob_matches("docs/*.md", "docs/lore/one.md"));
  assert!(!glob_matches("prompts/**", "docs/prompts.md"));
}

// A workspace shaped like this repository's intended one, cut down to what each rule needs: base packages
// under `packages/`, the bot in `server/`, the verbs in `scripts/`, the console in `client/`, and `checks`
// over everything. The agent launches `scripts/task.ts`; the bot launches the console's browser driver and
// the task runner. The repository tooling reads the graph, the bot reads the prompts and `checks` reads the
// documents. Beside them, `lab` is a project of its own: its own manifest and lockfile, listed by no
// workspace. The workspace is bun's, as its declaration says.
const BUN: &str = "testbed = 0\n\n[test]\nlanguages = [\"bun\"]\n";

const MANIFESTS: [(&str, &str); 11] = [
  (
    "package.json",
    r#"{ "name": "fixture", "workspaces": ["server", "scripts", "client", "packages/*"] }"#,
  ),
  ("packages/util/package.json", r#"{ "name": "@sample/util" }"#),
  (
    "packages/agent/package.json",
    r#"{ "name": "@sample/agent", "dependencies": { "@sample/util": "workspace:*" }, "bmo": { "runs": ["scripts/task.ts"] } }"#,
  ),
  (
    "packages/chat/package.json",
    r#"{ "name": "@sample/chat", "dependencies": { "@sample/agent": "workspace:*", "zod": "^4.6.5" } }"#,
  ),
  (
    "packages/repo/package.json",
    r#"{ "name": "@sample/repo", "dependencies": { "@sample/agent": "workspace:*" }, "bmo": { "reads": ["graph/**"] } }"#,
  ),
  (
    "server/package.json",
    r#"{
      "name": "@sample/bot",
      "dependencies": { "@sample/chat": "workspace:*", "@sample/repo": "workspace:*" },
      "devDependencies": { "@sample/agent": "workspace:*" },
      "bmo": { "reads": ["prompts/**"], "runs": ["client/browser/touch.ts", "scripts/task-runner.ts"] }
    }"#,
  ),
  (
    "scripts/package.json",
    r#"{ "name": "@sample/scripts", "dependencies": { "@sample/bot": "workspace:*" } }"#,
  ),
  (
    "client/package.json",
    r#"{ "name": "web", "dependencies": { "@sample/bot": "workspace:*", "@sample/chat": "workspace:*" } }"#,
  ),
  (
    "packages/checks/package.json",
    r#"{ "name": "@sample/checks", "bmo": { "always": true, "reads": ["docs/**", "*.md", "graph/**"] } }"#,
  ),
  ("lab/package.json", r#"{ "name": "lab", "scripts": { "check": "bun test" } }"#),
  ("lab/bun.lock", "{}"),
];

const SOURCES: [(&str, &str); 10] = [
  ("packages/util/numbers.ts", "export const one = 1;\n"),
  (
    "packages/agent/launcher.ts",
    "import { one } from \"@sample/util/numbers\";\n\nexport const launch = one;\n",
  ),
  ("packages/chat/tellbot.ts", "export const tell = 1;\n"),
  (
    "scripts/task.ts",
    "import { one } from \"../packages/util/numbers\";\n\nconsole.log(one);\n",
  ),
  (
    "scripts/task-runner.ts",
    "import { launch } from \"@sample/agent/launcher\";\n\nconsole.log(launch);\n",
  ),
  ("scripts/edit.ts", "console.log(1);\n"),
  (
    "client/browser/touch.ts",
    "import { page } from \"./page\";\n\nexport const touch = page;\n",
  ),
  ("client/browser/page.ts", "export const page = 1;\n"),
  ("client/app/domain/events.ts", "export const events = 1;\n"),
  ("lab/src/thing.ts", "export const thing = 1;\n"),
];

const TESTS: [&str; 10] = [
  "packages/util/numbers.test.ts",
  "packages/agent/launcher.test.ts",
  "packages/chat/tellbot.test.ts",
  "packages/repo/graph.test.ts",
  "packages/checks/wording.test.ts",
  "server/rounds.test.ts",
  "scripts/task.test.ts",
  "client/app/domain/events.test.ts",
  "stray.test.ts",
  "lab/src/thing.test.ts",
];

const PACKAGES: [&str; 8] = [
  "@sample/agent",
  "@sample/bot",
  "@sample/chat",
  "@sample/checks",
  "@sample/repo",
  "@sample/scripts",
  "@sample/util",
  "web",
];

// A scratch checkout holding these files, none of them committed: what it holds is what git lists.
fn scratch(files: &[(&str, &str)]) -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();

  for (path, text) in files {
    write(dir.path(), path, text);
  }

  git(dir.path(), &["init", "-q", "-b", "main"]);
  dir
}

fn repo(root: &Path) -> Repository {
  Repository::read(root).unwrap()
}

// The scratch workspace and the layout its package files declare.
struct Fixture {
  dir: tempfile::TempDir,
  layout: Layout,
}

impl Fixture {
  fn new() -> Self {
    let files: Vec<(&str, &str)> =
      MANIFESTS.iter().chain(&SOURCES).chain(&[("testbed.toml", BUN)]).copied().collect();
    let dir = scratch(&files);
    let layout = Layout::of_checkout(&repo(dir.path())).unwrap();

    Self { dir, layout }
  }

  fn root(&self) -> &Path {
    self.dir.path()
  }

  fn repo(&self) -> Repository {
    repo(self.root())
  }

  fn select(&self, changed: &[&str]) -> Selection {
    select_packages(self.root(), &self.layout, &strings(changed))
  }

  // Every marked package with the first reason it was marked for.
  fn marked(&self, changed: &[&str]) -> BTreeMap<String, MarkReason> {
    first_reasons(self.select(changed).marked)
  }
}

fn first_reasons(marked: BTreeMap<String, Vec<MarkReason>>) -> BTreeMap<String, MarkReason> {
  marked.into_iter().map(|(name, reasons)| (name, reasons[0].clone())).collect()
}

fn reasons<const N: usize>(pairs: [(&str, MarkReason); N]) -> BTreeMap<String, MarkReason> {
  pairs.into_iter().map(|(name, reason)| (name.to_owned(), reason)).collect()
}

fn owner(path: &str) -> MarkReason {
  MarkReason::Owner {
    path: path.to_owned(),
  }
}

fn root_file(path: &str) -> MarkReason {
  MarkReason::Root {
    path: path.to_owned(),
  }
}

fn reads(path: &str, glob: &str) -> MarkReason {
  MarkReason::Reads {
    path: path.to_owned(),
    glob: glob.to_owned(),
  }
}

fn unclaimed(path: &str) -> MarkReason {
  MarkReason::Unclaimed {
    path: path.to_owned(),
  }
}

fn runs(launched: &str, path: &str) -> MarkReason {
  MarkReason::Runs {
    launched: launched.to_owned(),
    path: path.to_owned(),
  }
}

fn dependent(on: &str) -> MarkReason {
  MarkReason::Dependent { on: on.to_owned() }
}

fn names<V>(marked: &BTreeMap<String, V>) -> Vec<&str> {
  marked.keys().map(String::as_str).collect()
}

mod the_packages {
  use super::*;

  // A package as its manifest declares it: its name, the packages it depends on, what it reads and runs,
  // and whether it is marked always.
  type Declared<'a> = (&'a str, Vec<String>, Vec<String>, Vec<String>, bool);

  // "each package with its name, the workspace packages it depends on, and its bmo section"
  #[test]
  fn each_package_with_its_name_the_workspace_packages_it_depends_on_and_its_bmo_section() {
    let fixture = Fixture::new();
    let read: Vec<Declared> = fixture
      .layout
      .packages
      .iter()
      .map(|one| {
        (
          one.name.as_str(),
          one.dependencies.clone(),
          one.reads.clone(),
          one.runs.clone(),
          one.always,
        )
      })
      .collect();

    assert_eq!(
      read,
      [
        (
          "@sample/bot",
          strings(&["@sample/chat", "@sample/repo", "@sample/agent"]),
          strings(&["prompts/**"]),
          strings(&["client/browser/touch.ts", "scripts/task-runner.ts"]),
          false
        ),
        ("@sample/scripts", strings(&["@sample/bot"]), vec![], vec![], false),
        ("web", strings(&["@sample/bot", "@sample/chat"]), vec![], vec![], false),
        (
          "@sample/agent",
          strings(&["@sample/util"]),
          vec![],
          strings(&["scripts/task.ts"]),
          false
        ),
        ("@sample/chat", strings(&["@sample/agent"]), vec![], vec![], false),
        ("@sample/checks", vec![], strings(&["docs/**", "*.md", "graph/**"]), vec![], true),
        ("@sample/repo", strings(&["@sample/agent"]), strings(&["graph/**"]), vec![], false),
        ("@sample/util", vec![], vec![], vec![], false),
      ]
    );
  }
}

mod the_rule {
  use super::*;

  // "owner: a changed file marks the package that holds it, and the packages that depend on it after"
  #[test]
  fn owner_a_changed_file_marks_the_package_that_holds_it_and_the_packages_that_depend_on_it_after() {
    assert_eq!(
      Fixture::new().marked(&["packages/chat/tellbot.ts"]),
      reasons([
        ("@sample/chat", owner("packages/chat/tellbot.ts")),
        ("@sample/bot", dependent("@sample/chat")),
        ("@sample/scripts", dependent("@sample/bot")),
        ("web", dependent("@sample/bot")),
        ("@sample/checks", MarkReason::Always),
      ])
    );
  }

  // "a package's own manifest is its own path"
  #[test]
  fn a_package_s_own_manifest_is_its_own_path() {
    assert_eq!(
      names(&Fixture::new().marked(&["packages/repo/package.json"])),
      ["@sample/bot", "@sample/checks", "@sample/repo", "@sample/scripts", "web"]
    );
  }

  // "root configuration marks every package"
  #[test]
  fn root_configuration_marks_every_package() {
    let all = Fixture::new().marked(&["bun.lock"]);

    assert_eq!(names(&all), PACKAGES);
    assert_eq!(all["@sample/util"], root_file("bun.lock"));
  }

  // "root configuration is what the declared languages name": node's lockfile is root configuration where
  // node is declared and an unclaimed path where bun is, and bun's runner configuration the other way round.
  #[test]
  fn root_configuration_is_what_the_declared_languages_name() {
    let bun = Fixture::new();
    let node = Fixture::new();

    write(node.root(), "testbed.toml", "testbed = 0\n\n[test]\nlanguages = [\"node\"]\n");

    let node_layout = Layout::of_checkout(&node.repo()).unwrap();
    let by_node =
      |path: &str| first_reasons(select_packages(node.root(), &node_layout, &strings(&[path])).marked);
    let unclaimed = |path: &str| MarkReason::Unclaimed {
      path: path.to_owned(),
    };

    assert_eq!(by_node("package-lock.json")["@sample/util"], root_file("package-lock.json"));
    assert_eq!(by_node("bunfig.toml")["@sample/util"], unclaimed("bunfig.toml"));
    assert_eq!(bun.marked(&["bunfig.toml"])["@sample/util"], root_file("bunfig.toml"));
    assert_eq!(bun.marked(&["package-lock.json"])["@sample/util"], unclaimed("package-lock.json"));
  }

  // "reads: a path a package reads marks that package, and the mark stops there"
  #[test]
  fn reads_a_path_a_package_reads_marks_that_package_and_the_mark_stops_there() {
    let fixture = Fixture::new();

    assert_eq!(
      fixture.marked(&["graph/console.json"]),
      reasons([
        ("@sample/checks", reads("graph/console.json", "graph/**")),
        ("@sample/repo", reads("graph/console.json", "graph/**")),
      ])
    );
    assert_eq!(
      fixture.marked(&["prompts/rules/answer-shape.md"]),
      reasons([
        ("@sample/bot", reads("prompts/rules/answer-shape.md", "prompts/**")),
        ("@sample/checks", MarkReason::Always),
      ])
    );
  }

  // "an unclaimed path — in no package and read by none — marks every package"
  #[test]
  fn an_unclaimed_path_in_no_package_and_read_by_none_marks_every_package() {
    let all = Fixture::new().marked(&["unclaimed/notes.txt"]);

    assert_eq!(names(&all), PACKAGES);
    assert_eq!(all["@sample/chat"], unclaimed("unclaimed/notes.txt"));
    assert_eq!(
      all["@sample/chat"].text(),
      fill_marks(&SUITE.reasons.unclaimed, &[("path", "unclaimed/notes.txt")])
    );
  }

  // "runs: a launched file marks the package that launches it, and so does a file it loads"
  #[test]
  fn runs_a_launched_file_marks_the_package_that_launches_it_and_so_does_a_file_it_loads() {
    let fixture = Fixture::new();
    let itself = &fixture.marked(&["scripts/task.ts"])["@sample/agent"];

    assert_eq!(*itself, runs("scripts/task.ts", "scripts/task.ts"));
    assert_eq!(itself.text(), fill_marks(&SUITE.reasons.runs, &[("launched", "scripts/task.ts")]));
    assert_eq!(
      fixture.marked(&["client/browser/page.ts"])["@sample/bot"],
      runs("client/browser/touch.ts", "client/browser/page.ts")
    );
    // Through a workspace package's name as well as a relative path.
    assert_eq!(
      fixture.marked(&["packages/agent/launcher.ts"])["@sample/bot"],
      runs("scripts/task-runner.ts", "packages/agent/launcher.ts")
    );
  }

  // "runs: a file beside a launched one that it never loads marks nothing through it"
  #[test]
  fn runs_a_file_beside_a_launched_one_that_it_never_loads_marks_nothing_through_it() {
    let fixture = Fixture::new();

    assert_eq!(
      import_closure(fixture.root(), "client/browser/touch.ts", &fixture.layout, &mut HashMap::new()),
      ["client/browser/touch.ts", "client/browser/page.ts"]
    );
    assert_eq!(
      fixture.marked(&["scripts/edit.ts"]),
      reasons([("@sample/scripts", owner("scripts/edit.ts")), ("@sample/checks", MarkReason::Always)])
    );
    assert_eq!(
      fixture.marked(&["client/app/domain/events.ts"]),
      reasons([("web", owner("client/app/domain/events.ts")), ("@sample/checks", MarkReason::Always)])
    );
  }

  // "dependents: the mark climbs through every package that depends on a marked one, to a fixed point"
  #[test]
  fn dependents_the_mark_climbs_through_every_package_that_depends_on_a_marked_one_to_a_fixed_point() {
    assert_eq!(names(&Fixture::new().marked(&["packages/util/numbers.ts"])), PACKAGES);
  }

  // "always: checks is marked by any change at all"
  #[test]
  fn always_checks_is_marked_by_any_change_at_all() {
    let fixture = Fixture::new();

    assert_eq!(
      fixture.marked(&["docs/web.md"]),
      reasons([("@sample/checks", reads("docs/web.md", "docs/**"))])
    );
    assert_eq!(fixture.marked(&["packages/util/numbers.ts"])["@sample/checks"], MarkReason::Always);
  }
}

mod the_crates {
  use super::*;

  // Three crates: beta stands on alpha, gamma on nothing. The bot's tests start alpha's binary, so its
  // `runs` names alpha's directory.
  fn crates() -> Vec<CrateDeclaration> {
    let one = |name: &str, dependencies: &[&str], reads: &[&str]| CrateDeclaration {
      dir: format!("crates/{name}"),
      name: name.to_owned(),
      dependencies: strings(dependencies),
      reads: strings(reads),
      binaries: Vec::new(),
      library: true,
      tests: Vec::new(),
    };

    vec![
      one("alpha", &[], &["packages/util/cases.json"]),
      one("beta", &["alpha"], &[]),
      one("gamma", &[], &[]),
    ]
  }

  fn with_crates(fixture: &Fixture) -> Layout {
    let mut layout = fixture.layout.clone();

    layout.crates = crates();

    for one in layout.packages.iter_mut().filter(|one| one.name == "@sample/bot") {
      one.runs.push("crates/alpha".to_owned());
    }

    layout
  }

  fn crates_marked(changed: &[&str]) -> BTreeMap<String, MarkReason> {
    let layout = Layout {
      crates: crates(),
      ..Layout::default()
    };

    first_reasons(select_crates(&layout, &strings(changed)))
  }

  // "a path inside a crate marks that crate, and the crates that depend on it after"
  #[test]
  fn a_path_inside_a_crate_marks_that_crate_and_the_crates_that_depend_on_it_after() {
    assert_eq!(
      crates_marked(&["crates/alpha/src/lib.rs"]),
      reasons([("alpha", owner("crates/alpha/src/lib.rs")), ("beta", dependent("alpha"))])
    );
    assert_eq!(
      crates_marked(&["crates/gamma/Cargo.toml"]),
      reasons([("gamma", owner("crates/gamma/Cargo.toml"))])
    );
  }

  // "the workspace's root files mark every crate, and a path no crate holds and none reads marks none"
  #[test]
  fn the_workspace_s_root_files_mark_every_crate_and_a_path_no_crate_holds_or_reads_marks_none() {
    assert_eq!(
      crates_marked(&["Cargo.lock"]),
      reasons([
        ("alpha", root_file("Cargo.lock")),
        ("beta", root_file("Cargo.lock")),
        ("gamma", root_file("Cargo.lock")),
      ])
    );
    assert_eq!(
      names(&crates_marked(&[".cargo/config.toml", "rustfmt.toml"])),
      ["alpha", "beta", "gamma"]
    );

    assert!(crates_marked(&["crates/README.md"]).is_empty());
  }

  // "a path outside the workspace marks a crate whose reads match it, and the mark stops there"
  #[test]
  fn a_path_outside_the_workspace_marks_a_crate_whose_reads_match_it_and_the_mark_stops_there() {
    assert_eq!(
      crates_marked(&["packages/util/cases.json", "packages/util/numbers.ts"]),
      reasons([("alpha", reads("packages/util/cases.json", "packages/util/cases.json"))])
    );
  }

  // "a crate's path marks no package as unclaimed, only the one whose runs name that crate"
  #[test]
  fn a_crate_s_path_marks_no_package_as_unclaimed_only_the_one_whose_runs_name_that_crate() {
    let fixture = Fixture::new();
    let layout = with_crates(&fixture);
    let selection = select_packages(fixture.root(), &layout, &strings(&["crates/alpha/src/lib.rs"]));

    assert_eq!(names(&selection.crates), ["alpha", "beta"]);
    assert_eq!(
      first_reasons(selection.marked),
      reasons([
        ("@sample/bot", runs("crates/alpha", "crates/alpha/src/lib.rs")),
        ("@sample/scripts", dependent("@sample/bot")),
        ("web", dependent("@sample/bot")),
        ("@sample/checks", MarkReason::Always),
      ])
    );
    assert!(
      !select_packages(fixture.root(), &layout, &strings(&["crates/gamma/src/lib.rs"]))
        .marked
        .contains_key("@sample/bot")
    );
  }

  // "a path inside a crate's directory marks that crate, wherever the workspace lists it"
  #[test]
  fn a_path_inside_a_tool_s_crate_marks_that_crate_and_no_package_as_unclaimed() {
    let fixture = Fixture::new();
    let mut layout = with_crates(&fixture);

    layout.crates.push(CrateDeclaration {
      dir: "tools/delta".to_owned(),
      name: "delta-tool".to_owned(),
      dependencies: strings(&["alpha"]),
      reads: Vec::new(),
      binaries: strings(&["delta-tool"]),
      library: true,
      tests: Vec::new(),
    });

    let selection = select_packages(
      fixture.root(),
      &layout,
      &strings(&["tools/delta/src/lib.rs", "tools/delta/tool.toml"]),
    );

    assert_eq!(
      first_reasons(selection.crates),
      reasons([("delta-tool", owner("tools/delta/src/lib.rs"))])
    );
    assert_eq!(first_reasons(selection.marked), reasons([("@sample/checks", MarkReason::Always)]));
  }

  // "its first lines name every crate, the marked ones with why"
  #[test]
  fn its_first_lines_name_every_crate_the_marked_ones_with_why() {
    let fixture = Fixture::new();
    let layout = with_crates(&fixture);
    let lines = selection_lines(
      &layout,
      &select_packages(fixture.root(), &layout, &strings(&["crates/beta/src/lib.rs"])),
    );
    let said = &SUITE.selection;

    assert_eq!(
      lines[lines.len() - 3..],
      [
        fill_marks(&said.crate_unmarked, &[("dir", "crates/alpha")]),
        fill_marks(
          &said.crate_marked,
          &[("dir", "crates/beta"), ("reason", &owner("crates/beta/src/lib.rs").text())]
        ),
        fill_marks(&said.crate_unmarked, &[("dir", "crates/gamma")]),
      ]
    );
  }

  // "are read from the workspace as Cargo reads it, each with the crates of the workspace it depends on"
  #[test]
  fn are_read_from_the_workspace_as_cargo_reads_it_each_with_the_crates_of_the_workspace_it_depends_on() {
    let cargo = tempfile::tempdir().unwrap();
    let files = [
      ("Cargo.toml", "[workspace]\nresolver = \"3\"\nmembers = [\"crates/*\"]\n"),
      (
        "crates/alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[package.metadata.bmo]\nreads = [\"docs/alpha.md\"]\n",
      ),
      ("crates/alpha/src/lib.rs", ""),
      (
        "crates/beta/Cargo.toml",
        "[package]\nname = \"beta\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nalpha = { path = \"../alpha\" }\n",
      ),
      ("crates/beta/src/main.rs", "fn main() {}\n"),
    ];

    for (path, text) in files {
      write(cargo.path(), path, text);
    }

    assert_eq!(
      workspace_crates(cargo.path()).unwrap(),
      [
        CrateDeclaration {
          dir: "crates/alpha".to_owned(),
          name: "alpha".to_owned(),
          dependencies: vec![],
          reads: strings(&["docs/alpha.md"]),
          binaries: vec![],
          library: true,
          tests: Vec::new(),
        },
        CrateDeclaration {
          dir: "crates/beta".to_owned(),
          name: "beta".to_owned(),
          dependencies: strings(&["alpha"]),
          reads: vec![],
          binaries: strings(&["beta"]),
          library: false,
          tests: Vec::new(),
        },
      ]
    );
    assert!(Fixture::new().layout.crates.is_empty());
  }
}

mod a_selected_run {
  use super::*;

  // "the tests of marked packages, and a test inside no package, and no other"
  #[test]
  fn the_tests_of_marked_packages_and_a_test_inside_no_package_and_no_other() {
    let fixture = Fixture::new();
    let selection = fixture.select(&["packages/chat/tellbot.ts"]);

    assert_eq!(
      selected_tests(&fixture.layout, &selection, &strings(&TESTS)),
      [
        "packages/chat/tellbot.test.ts",
        "packages/checks/wording.test.ts",
        "server/rounds.test.ts",
        "scripts/task.test.ts",
        "client/app/domain/events.test.ts",
        "stray.test.ts",
      ]
    );
  }

  // "its first lines name every package, the marked ones with why and how many more reasons"
  #[test]
  fn its_first_lines_name_every_package_the_marked_ones_with_why_and_how_many_more_reasons() {
    let fixture = Fixture::new();
    let lines = selection_lines(
      &fixture.layout,
      &fixture.select(&["packages/chat/tellbot.ts", "server/package.json", "prompts/persona/guide.md"]),
    );
    let said = &SUITE.selection;

    assert_eq!(lines[0], fill_marks(&said.heading, &[("marked", "5"), ("packages", "8")]));
    assert!(lines.contains(&fill_marks(
      &said.marked,
      &[
        ("name", "@sample/bot"),
        ("reason", &owner("server/package.json").text()),
        ("more", &fill_marks(&said.more, &[("count", "1")])),
      ],
    )));
    assert!(lines.contains(&fill_marks(&said.unmarked, &[("name", "@sample/util")])));
  }
}

mod a_project_of_its_own {
  use super::*;

  // "is read from its own manifest and lockfile, beside the packages and none of them"
  #[test]
  fn is_read_from_its_own_manifest_and_lockfile_beside_the_packages_and_none_of_them() {
    let fixture = Fixture::new();

    assert_eq!(
      fixture.layout.projects,
      [OwnProject {
        dir: "lab".to_owned(),
        check: true,
        members: vec![],
        browser: vec![],
      }]
    );
    assert_eq!(fixture.layout.owner_of("lab/src/thing.ts"), None);
  }

  // "a tool folder carrying its own manifest and lockfile is one, and its check is its project's"
  #[test]
  fn a_tool_folder_carrying_its_own_manifest_and_lockfile_is_one_and_its_check_is_its_project_s() {
    let dir = scratch(&[
      ("testbed.toml", BUN),
      ("package.json", r#"{ "name": "fixture", "workspaces": ["packages/*"] }"#),
      ("packages/util/package.json", r#"{ "name": "@sample/util" }"#),
      ("tools/bench/package.json", r#"{ "name": "bench", "scripts": { "check": "bun test" } }"#),
      ("tools/bench/bun.lock", "{}"),
      ("tools/bench/src/bench.ts", "export const bench = 1;\n"),
      ("tools/plain/package.json", r#"{ "name": "plain" }"#),
    ]);

    assert_eq!(
      own_projects(&repo(dir.path())),
      [OwnProject {
        dir: "tools/bench".to_owned(),
        check: true,
        members: vec![],
        browser: vec![],
      }]
    );
  }

  // "its lockfile is the one a declared language names": a node-declared repository finds the folder carrying
  // `package-lock.json` and not the one carrying `bun.lock`, and one declaring no such language finds none
  #[test]
  fn its_lockfile_is_the_one_a_declared_language_names() {
    let files = |declared: &'static str| {
      scratch(&[
        ("testbed.toml", declared),
        ("package.json", r#"{ "name": "fixture", "workspaces": ["packages/*"] }"#),
        ("package-lock.json", "{}"),
        ("packages/util/package.json", r#"{ "name": "@sample/util" }"#),
        ("site/package.json", r#"{ "name": "site", "scripts": { "check": "npm test" } }"#),
        ("site/package-lock.json", "{}"),
        ("lab/package.json", r#"{ "name": "lab" }"#),
        ("lab/bun.lock", "{}"),
      ])
    };
    let dirs = |declared: &'static str| {
      let dir = files(declared);

      own_projects(&repo(dir.path())).into_iter().map(|one| one.dir).collect::<Vec<_>>()
    };

    assert_eq!(dirs("testbed = 0\n\n[test]\nlanguages = [\"node\"]\n"), ["site"]);
    assert_eq!(dirs(BUN), ["lab"]);
    assert_eq!(
      dirs("testbed = 0\n\n[test]\nlanguages = [\"cargo\", \"bun\", \"node\"]\n"),
      ["lab", "site"]
    );
    assert!(dirs("testbed = 0\n\n[test]\nlanguages = [\"cargo\"]\n").is_empty());
  }

  // "a workspace package standing inside it is the workspace's, not the project's"
  #[test]
  fn a_workspace_package_standing_inside_it_is_the_workspace_s_not_the_project_s() {
    let dir = scratch(&[
      ("testbed.toml", BUN),
      (
        "package.json",
        r#"{ "name": "fixture", "workspaces": ["packages/*", "tools/bench/verbs"] }"#,
      ),
      (
        "packages/checks/package.json",
        r#"{ "name": "@sample/checks", "bmo": { "always": true } }"#,
      ),
      (
        "tools/bench/package.json",
        r#"{ "name": "bench", "scripts": { "check": "bun test ./src" } }"#,
      ),
      ("tools/bench/bun.lock", "{}"),
      ("tools/bench/src/bench.ts", "export const bench = 1;\n"),
      ("tools/bench/verbs/package.json", r#"{ "name": "@sample/bench" }"#),
      ("tools/bench/verbs/verb.ts", "export const verb = 1;\n"),
    ]);
    let layout = Layout::of_checkout(&repo(dir.path())).unwrap();

    assert_eq!(layout.projects[0].members, ["tools/bench/verbs"]);

    let inside = select_packages(dir.path(), &layout, &strings(&["tools/bench/verbs/verb.ts"]));

    assert!(inside.projects.is_empty(), "{:?}", inside.projects);
    assert_eq!(names(&inside.marked), ["@sample/bench", "@sample/checks"]);
    assert_eq!(
      selected_tests(
        &layout,
        &inside,
        &strings(&["tools/bench/verbs/verb.test.ts", "tools/bench/src/bench.test.ts"])
      ),
      ["tools/bench/verbs/verb.test.ts"]
    );

    let beside = select_packages(dir.path(), &layout, &strings(&["tools/bench/src/bench.ts"]));

    assert_eq!(
      beside.projects,
      BTreeMap::from([("tools/bench".to_owned(), "tools/bench/src/bench.ts".to_owned())])
    );
    assert_eq!(names(&beside.marked), ["@sample/checks"]);
  }

  // "a change inside it marks it for its own check and no package but the one always marked"
  #[test]
  fn a_change_inside_it_marks_it_for_its_own_check_and_no_package_but_the_one_always_marked() {
    let fixture = Fixture::new();
    let selection = fixture.select(&["lab/src/thing.ts", "lab/package.json"]);

    assert_eq!(selection.projects, BTreeMap::from([("lab".to_owned(), "lab/src/thing.ts".to_owned())]));
    assert_eq!(names(&selection.marked), ["@sample/checks"]);
    assert!(selection_lines(&fixture.layout, &selection).contains(&fill_marks(
      &SUITE.selection.project_marked,
      &[("dir", "lab"), ("path", "lab/src/thing.ts")]
    )));
  }

  // "its tests are its check's, never a selected run's"
  #[test]
  fn its_tests_are_its_check_s_never_a_selected_run_s() {
    let fixture = Fixture::new();
    let selection = fixture.select(&["lab/src/thing.ts"]);

    assert!(
      !selected_tests(&fixture.layout, &selection, &strings(&TESTS))
        .contains(&"lab/src/thing.test.ts".to_owned())
    );
    assert!(
      selection_lines(&fixture.layout, &fixture.select(&["packages/util/numbers.ts"]))
        .contains(&fill_marks(&SUITE.selection.project_unmarked, &[("dir", "lab")]))
    );
  }

  // "the declaration names one discovery would not find, and with discovery off only those"
  #[test]
  fn the_declaration_names_one_discovery_would_not_find_and_with_discovery_off_only_those() {
    let fixture = Fixture::new();
    let dirs = |declared: &str| {
      write(fixture.root(), "testbed.toml", declared);
      own_projects(&fixture.repo()).into_iter().map(|one| one.dir).collect::<Vec<_>>()
    };

    write(fixture.root(), "site/package.json", r#"{ "name": "site" }"#);

    assert_eq!(dirs(&format!("{BUN}\n[projects]\npaths = [\"site\"]\n")), ["lab", "site"]);
    assert_eq!(dirs(&format!("{BUN}\n[projects]\ndiscover = false\npaths = [\"site\"]\n")), ["site"]);
  }
}

mod a_tool_checked_by_its_manifest {
  use super::*;

  // Beside the fixture's packages: `scan`, a Python tool with a check of its own; `bare`, one whose manifest
  // names none; `held`, a tool the crate `held-tool` holds; and `loose`, a folder with no manifest.
  fn own() -> (Fixture, Layout) {
    let fixture = Fixture::new();
    let manifest = |name: &str, check: &str| {
      format!(
        "[tool]\nname = \"{name}\"\n{check}run = [\"python3\", \"{name}.py\"]\n\n[[entry]]\nname = \"{name}\"\ndescription = \"Reads a page.\"\n"
      )
    };

    write(fixture.root(), "testbed.toml", &format!("{BUN}\n[tools]\ndir = \"tools\"\n"));
    write(
      fixture.root(),
      "tools/scan/tool.toml",
      &manifest("scan", "check = \"python3 -m unittest\"\n"),
    );
    write(fixture.root(), "tools/scan/scan.py", "print(1)\n");
    write(fixture.root(), "tools/bare/tool.toml", &manifest("bare", ""));
    write(fixture.root(), "tools/held/tool.toml", &manifest("held", "check = \"cargo test\"\n"));
    write(fixture.root(), "tools/loose/notes.txt", "nothing\n");

    let held = CrateDeclaration {
      dir: "tools/held".to_owned(),
      name: "held-tool".to_owned(),
      dependencies: Vec::new(),
      reads: Vec::new(),
      binaries: Vec::new(),
      library: true,
      tests: Vec::new(),
    };
    let layout = Layout::of_parts(&fixture.repo(), vec![held]);

    (fixture, layout)
  }

  // "is a tool folder with a manifest that no crate holds, read with its manifest's check or why it has none"
  #[test]
  fn is_a_tool_folder_with_a_manifest_that_no_crate_holds_read_with_its_manifest_s_check_or_why_it_has_none()
   {
    let (_fixture, layout) = own();

    assert_eq!(
      layout.tools,
      [
        CheckedTool {
          dir: "tools/bare".to_owned(),
          check: Err(fill_marks(
            &SUITE.file.no_tool_check,
            &[("manifest", "tool.toml"), ("key", "tool.check")]
          )),
        },
        CheckedTool {
          dir: "tools/scan".to_owned(),
          check: Ok("python3 -m unittest".to_owned()),
        },
      ]
    );
  }

  // "a change inside it marks it for its own check and no package but the one always marked"
  #[test]
  fn a_change_inside_it_marks_it_for_its_own_check_and_no_package_but_the_one_always_marked() {
    let (fixture, layout) = own();
    let selection =
      select_packages(fixture.root(), &layout, &strings(&["tools/scan/scan.py", "tools/scan/tool.toml"]));

    assert_eq!(
      selection.tools,
      BTreeMap::from([("tools/scan".to_owned(), "tools/scan/scan.py".to_owned())])
    );
    assert_eq!(names(&selection.marked), ["@sample/checks"]);

    let lines = selection_lines(&layout, &selection);

    assert!(lines.contains(&fill_marks(
      &SUITE.selection.tool_marked,
      &[("dir", "tools/scan"), ("path", "tools/scan/scan.py")]
    )));
    assert!(lines.contains(&fill_marks(&SUITE.selection.tool_unmarked, &[("dir", "tools/bare")])));
  }

  // "a path in a tool folder that is neither one still marks every package as unclaimed"
  #[test]
  fn a_path_in_a_tool_folder_that_is_neither_one_still_marks_every_package_as_unclaimed() {
    let (fixture, layout) = own();
    let selection = select_packages(fixture.root(), &layout, &strings(&["tools/loose/notes.txt"]));

    assert!(selection.tools.is_empty());
    assert_eq!(names(&selection.marked), PACKAGES);
  }
}

// A change spanning the web console, its documents and its graph, then the same change with the task runner
// the bot launches and a chat module beside it: whether the bot's tests, one of them flaky, are reached.
mod a_change_spanning_the_console {
  use super::*;

  const WHOLE: [&str; 16] = [
    "client/app/components/TodoPanel.test.ts",
    "client/app/components/TodoPanel.vue",
    "client/app/domain/events.test.ts",
    "client/app/domain/events.ts",
    "client/server/api/events.get.ts",
    "client/server/utils/sse-frames.test.ts",
    "client/server/utils/sse-frames.ts",
    "client/shared/config.ts",
    "docs/web.md",
    "graph/bot.json",
    "graph/console.json",
    "graph/scripts.json",
    "scripts/task-runner.ts",
    "scripts/task.ts",
    "packages/chat/tellbot.test.ts",
    "packages/chat/tellbot.ts",
  ];

  // "the console, its documents and its graph select the console, checks and repo, and not the bot"
  #[test]
  fn the_console_its_documents_and_its_graph_select_the_console_checks_and_repo_and_not_the_bot() {
    let fixture = Fixture::new();
    let console: Vec<&str> = WHOLE
      .into_iter()
      .filter(|path| !path.starts_with("scripts/") && !path.starts_with("packages/"))
      .collect();
    let selection = fixture.select(&console);

    assert_eq!(names(&selection.marked), ["@sample/checks", "@sample/repo", "web"]);
    assert!(
      !selected_tests(&fixture.layout, &selection, &strings(&TESTS))
        .contains(&"server/rounds.test.ts".to_owned())
    );
  }

  // "the task runner the bot launches and a chat module beside it are reached by the bot's tests"
  #[test]
  fn the_task_runner_the_bot_launches_and_a_chat_module_beside_it_are_reached_by_the_bot_s_tests() {
    let fixture = Fixture::new();
    let selection = fixture.select(&WHOLE);

    assert_eq!(
      selection.marked["@sample/bot"],
      [runs("scripts/task-runner.ts", "scripts/task-runner.ts")]
    );
    assert!(selection.marked["@sample/chat"].contains(&owner("packages/chat/tellbot.ts")));
    assert!(
      selected_tests(&fixture.layout, &selection, &strings(&TESTS))
        .contains(&"server/rounds.test.ts".to_owned())
    );
  }
}

mod the_changed_paths {
  use super::*;

  // The fixture committed whole as the range's start.
  fn committed() -> (Fixture, String) {
    let fixture = Fixture::new();

    git(fixture.root(), &["init", "-q", "-b", "main"]);
    git(fixture.root(), &["add", "-A"]);
    git(fixture.root(), &["commit", "-qm", "start"]);

    let start = git(fixture.root(), &["rev-parse", "HEAD"]);

    (fixture, start)
  }

  // "are the range's commits, the tracked changes not yet committed and the files not yet tracked, a rename both ways"
  #[test]
  fn are_the_range_s_commits_the_tracked_changes_not_yet_committed_and_the_files_not_yet_tracked_a_rename_both_ways()
   {
    let (fixture, start) = committed();
    let repo = fixture.root();

    std::fs::rename(repo.join("scripts/edit.ts"), repo.join("scripts/edits.ts")).unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "a rename"]);
    write(repo, "packages/util/numbers.ts", "export const one = 2;\n");
    write(repo, "notes.md", "new\n");

    assert_eq!(
      changed_paths(repo, &start).unwrap(),
      ChangedPaths::Paths(vec![
        "notes.md".to_owned(),
        "packages/util/numbers.ts".to_owned(),
        "scripts/edit.ts".to_owned(),
        "scripts/edits.ts".to_owned(),
      ])
    );
  }

  // "a base the checkout cannot read narrows nothing"
  #[test]
  fn a_base_the_checkout_cannot_read_narrows_nothing() {
    let (fixture, _) = committed();

    assert!(matches!(
      changed_paths(fixture.root(), "no-such-commit"),
      Ok(ChangedPaths::CommitNotInHistory)
    ));
  }
  // "a range that cannot be narrowed counts every tracked path"
  #[test]
  fn a_range_that_cannot_be_narrowed_counts_every_tracked_path() {
    let (fixture, _) = committed();
    let tracked = git_lines(fixture.root(), &["ls-files"]).unwrap();
    let counted = tracked_paths(fixture.root()).unwrap();

    assert!(!tracked.is_empty());
    assert!(tracked.iter().all(|path| counted.contains(path)), "{counted:?}");
  }
}

// A batch of commits held by a flaky test it never reached, run for real: a workspace carries a bot test that
// always fails, a batch of commits is selected over its range from the live commit, and the tests it
// selected run with bun in that workspace, so the selection and the run are the real ones.
mod a_flaky_test_in_a_package_the_batch_does_not_reach {
  use super::*;

  const PASS: &str =
    "import { expect, test } from \"bun:test\";\n\ntest(\"holds\", () => expect(1).toBe(1));\n";
  const FAIL: &str = "import { expect, test } from \"bun:test\";\n\ntest(\"fails now and then\", () => expect(1).toBe(2));\n";

  // A committed workspace whose bot carries the failing test, and the live commit it starts from.
  struct Space {
    dir: tempfile::TempDir,
    live: String,
  }

  impl Space {
    fn new() -> Self {
      let dir = tempfile::tempdir().unwrap();
      let files = [
        (
          "package.json",
          r#"{ "name": "fixture", "workspaces": ["server", "client", "packages/*"] }"#,
        ),
        ("packages/util/package.json", r#"{ "name": "@sample/util" }"#),
        ("packages/util/numbers.test.ts", PASS),
        (
          "server/package.json",
          r#"{ "name": "@sample/bot", "dependencies": { "@sample/util": "workspace:*" } }"#,
        ),
        ("server/rounds.test.ts", FAIL),
        ("server/conversation.ts", "export const round = 1;\n"),
        (
          "client/package.json",
          r#"{ "name": "web", "dependencies": { "@sample/bot": "workspace:*" } }"#,
        ),
        ("client/app/domain/events.ts", "export const events = 1;\n"),
        ("client/app/domain/events.test.ts", PASS),
      ];

      for (path, text) in files {
        write(dir.path(), path, text);
      }

      git(dir.path(), &["init", "-q", "-b", "main"]);
      git(dir.path(), &["add", "-A"]);
      git(dir.path(), &["commit", "-qm", "live"]);

      let live = git(dir.path(), &["rev-parse", "HEAD"]);

      Self { dir, live }
    }

    fn commit(&self, path: &str, text: &str) {
      write(self.dir.path(), path, text);
      git(self.dir.path(), &["add", "-A"]);
      git(self.dir.path(), &["commit", "-qm", &format!("change {path}")]);
    }

    // The batch's suite over the workspace: the tests its range selects, run with bun and read back as the
    // suite's own log is.
    fn batch_run(&self) -> (Vec<String>, i32, Vec<String>) {
      let dir = self.dir.path();
      let layout = Layout::of_checkout(&repo(dir)).unwrap();
      let changed = match changed_paths(dir, &self.live) {
        Ok(ChangedPaths::Paths(paths)) => paths,
        _ => panic!("unexpected changed_paths result"),
      };
      let selection = select_packages(dir, &layout, &changed);
      let tests = git_lines(dir, &["ls-files", "*.test.ts"]).unwrap();
      let ran = selected_tests(&layout, &selection, &tests);
      let run = Command::new("bun")
        .arg("test")
        .args(ran.iter().map(|file| format!("./{file}")))
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
      let log = format!("{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));

      (ran, run.status.code().unwrap_or(1), failed_files_of(&log))
    }
  }

  // "a batch of console commits runs the console's tests alone and goes live"
  #[test]
  fn a_batch_of_console_commits_runs_the_console_s_tests_alone_and_goes_live() {
    let space = Space::new();

    space.commit("client/app/domain/events.ts", "export const events = 2;\n");
    space.commit("client/app/domain/events.ts", "export const events = 3;\n");

    let (ran, code, _) = space.batch_run();

    assert_eq!(ran, ["client/app/domain/events.test.ts"]);
    assert_eq!(code, 0);
  }

  // "while a batch that reaches the bot runs its flaky test, and is held by it"
  #[test]
  fn while_a_batch_that_reaches_the_bot_runs_its_flaky_test_and_is_held_by_it() {
    let space = Space::new();

    space.commit("client/app/domain/events.ts", "export const events = 2;\n");
    space.commit("server/conversation.ts", "export const round = 2;\n");

    let (ran, code, failed_files) = space.batch_run();

    assert!(ran.contains(&"server/rounds.test.ts".to_owned()));
    assert_ne!(code, 0);
    assert_eq!(failed_files, ["server/rounds.test.ts"]);
  }
}
