//! A project of its own in a checkout (docs/checks.md, 「Which tests a change runs」): where it installs, and
//! which of its paths a workspace package standing inside it keeps.

use std::path::Path;
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_suite::workspace::{own_project_install_dirs, own_project_of, own_projects};

// A checkout with one workspace package and one project of its own whose screen has a lockfile of its own,
// and a tool folder holding a workspace package of its own.
fn checkout() -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path();

  for (path, text) in [
    ("package.json", r#"{ "name": "root", "workspaces": ["server", "tools/bench/verbs"] }"#),
    ("bun.lock", ""),
    ("server/package.json", r#"{ "name": "@sample/server" }"#),
    ("server/main.ts", ""),
    ("lab/package.json", r#"{ "name": "lab", "scripts": { "check": "bun test" } }"#),
    ("lab/bun.lock", ""),
    ("lab/src/thing.ts", ""),
    ("lab/client/package.json", r#"{ "name": "lab-client" }"#),
    ("lab/client/bun.lock", ""),
    ("lab/client/node_modules/dep/bun.lock", ""),
    ("tools/bench/package.json", r#"{ "name": "bench", "scripts": { "check": "bun test" } }"#),
    ("tools/bench/bun.lock", ""),
    ("tools/bench/tool.toml", ""),
    ("tools/bench/verbs/package.json", r#"{ "name": "@sample/bench" }"#),
    ("tools/bench/verbs/verb.ts", ""),
    (".gitignore", "node_modules/\n"),
  ] {
    write(root, path, text);
  }

  let done = Command::new("git")
    .args(["init", "-q"])
    .current_dir(root)
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_SYSTEM", "/dev/null")
    .stdin(Stdio::null())
    .output()
    .unwrap();

  assert!(done.status.success());
  dir
}

fn write(root: &Path, file: &str, text: &str) {
  let path = root.join(file);

  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
}

#[test]
fn a_project_of_its_own_is_installed_from_each_of_its_lockfiles_its_own_directory_first() {
  let dir = checkout();

  assert_eq!(
    own_project_install_dirs(&Repository::read(dir.path()).unwrap()),
    ["lab", "lab/client", "tools/bench"]
  );
}

#[test]
fn a_workspace_package_standing_inside_a_project_of_its_own_keeps_its_paths() {
  let dir = checkout();
  let projects = own_projects(&Repository::read(dir.path()).unwrap());
  let bench = projects.iter().find(|one| one.dir == "tools/bench").unwrap();

  assert_eq!(bench.members, ["tools/bench/verbs"]);
  assert!(own_project_of(&projects, "tools/bench/verbs/verb.ts").is_none());
  assert_eq!(
    own_project_of(&projects, "tools/bench/verbsmore/verb.ts").map(|one| one.dir.as_str()),
    Some("tools/bench")
  );
  assert_eq!(
    own_project_of(&projects, "tools/bench/tool.toml").map(|one| one.dir.as_str()),
    Some("tools/bench")
  );
}
