//! The set's release (docs/deploy.md, 「The set's release」): the set read from its folder, every value that
//! declares a unit's own version set where it stands and nothing else, the release made in git, commit, push
//! and tag, and the step the commits since the last release propose.

use std::path::Path;
use std::process::{Command, Stdio};

use bmo_testbed_suite::release::{
  Standing, Step, machine_git, make_release, member_lines, plan_files, propose, read_member, read_set,
  refusal, step_of, subjects_since_release,
};
use semver::Version;

fn version(text: &str) -> Version {
  Version::parse(text).unwrap()
}

fn files(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
  pairs.iter().map(|(path, text)| ((*path).to_owned(), (*text).to_owned())).collect()
}

fn edited<'a>(plan: &'a bmo_testbed_suite::release::Plan, path: &str) -> &'a str {
  plan.edits.iter().find(|(one, _)| one == path).map(|(_, text)| text.as_str()).unwrap()
}

fn tags(listed: &[&[&str]]) -> Vec<Vec<String>> {
  listed.iter().map(|one| one.iter().map(|tag| (*tag).to_owned()).collect()).collect()
}

// "The last release is the newest tag v<major>.<minor>.<patch> carrying no pre-release that more than half of
// the set's repositories hold, so an upstream's own tags kept in a fork never count"
#[test]
fn the_last_release_is_the_newest_release_most_repositories_hold() {
  assert_eq!(Standing::of(&tags(&[&[], &[], &["v0.27.4", "v0.27.5"]])), Standing::Before);
  assert_eq!(
    Standing::of(&tags(&[
      &["v1.0.0", "v1.1.0"],
      &["v1.0.0", "v1.1.0"],
      &["v1.0.0", "v0.27.5", "v2.0.0-rc.1"]
    ])),
    Standing::At(version("1.1.0"))
  );
  assert_eq!(Standing::of(&tags(&[&["v1.0.0"], &["v1.0.0"], &[]])), Standing::At(version("1.0.0")));
}

// "a unit's own version, which is the last release's version, or before the first release 0.1.0 or
// 0.1.0-rev.<n>"
#[test]
fn an_own_version_is_the_base_and_its_revisions_before_the_first_release_and_the_release_after() {
  let before = Standing::Before;
  let after = Standing::At(version("1.0.0"));

  assert!(before.owns(&version("0.1.0")));
  assert!(before.owns(&version("0.1.0-rev.77")));
  assert!(before.owns(&version("0.1.0-rev.73+source")));
  assert!(!before.owns(&version("0.27.5")));
  assert!(!before.owns(&version("0.1.0-beta.1")));
  assert!(!before.owns(&version("0.1.1")));
  assert!(after.owns(&version("1.0.0")));
  assert!(!after.owns(&version("0.1.0-rev.77")));
  assert!(!after.owns(&version("1.1.0-nightly.20261124")));
}

// "a Cargo.toml's [package] and [workspace.package] version, and in each of its dependency tables the version
// of an entry taken from a sibling by path, its operator kept", every other byte as it stood
#[test]
fn a_cargo_manifest_moves_its_own_versions_and_its_siblings_pins_and_nothing_else() {
  let manifest = "# The workspace.\n[workspace.package]\nversion = \"0.1.0-rev.12\" # the unit's\nedition = \"2024\"\n\n[workspace.dependencies]\nsibling = { path = \"../sibling\", version = \"=0.1.0-rev.5\" }\nregistry = \"0.1.0\"\nfork = { path = \"../fork\", version = \"0.27.5\" }\n\n[target.'cfg(unix)'.dependencies]\nnear = { path = \"near\", version = \"0.1.0\" }\n";
  let plan = plan_files(&files(&[("Cargo.toml", manifest)]), &Standing::Before, &version("1.0.0"), &[]);

  assert_eq!(
    edited(&plan, "Cargo.toml"),
    "# The workspace.\n[workspace.package]\nversion = \"1.0.0\" # the unit's\nedition = \"2024\"\n\n[workspace.dependencies]\nsibling = { path = \"../sibling\", version = \"=1.0.0\" }\nregistry = \"0.1.0\"\nfork = { path = \"../fork\", version = \"0.27.5\" }\n\n[target.'cfg(unix)'.dependencies]\nnear = { path = \"near\", version = \"1.0.0\" }\n"
  );
  assert_eq!(
    plan
      .changes
      .iter()
      .map(|one| (one.what.as_str(), one.from.as_str()))
      .collect::<Vec<_>>(),
    [
      ("[workspace.package] version", "0.1.0-rev.12"),
      ("workspace.dependencies sibling version", "=0.1.0-rev.5"),
      ("target.cfg(unix).dependencies near version", "0.1.0")
    ]
  );
}

// A table the manifest does not hold is never added on the way to a value: only the value that stands moves.
#[test]
fn a_manifest_holding_no_package_table_gains_none() {
  let manifest = "[workspace]\nmembers = []\n\n[workspace.dependencies]\nsibling = { path = \"../sibling\", version = \"=0.1.0-rev.5\" }\n";
  let project = "[project]\nname = \"kit\"\nversion = \"0.1.0\"\n";
  let plan = plan_files(
    &files(&[("Cargo.toml", manifest), ("pyproject.toml", project)]),
    &Standing::Before,
    &version("1.0.0"),
    &[],
  );

  assert_eq!(edited(&plan, "Cargo.toml"), manifest.replace("=0.1.0-rev.5", "=1.0.0"));
  assert_eq!(edited(&plan, "pyproject.toml"), project.replace("0.1.0", "1.0.0"));
}

// "a Cargo.lock's packages read from a path, never a registry's"
#[test]
fn a_lock_moves_its_path_packages_and_leaves_the_registrys() {
  let lock = "version = 4\n\n[[package]]\nname = \"own\"\nversion = \"0.1.0-rev.3\"\n\n[[package]]\nname = \"shared\"\nversion = \"0.1.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n";
  let plan = plan_files(&files(&[("Cargo.lock", lock)]), &Standing::Before, &version("1.0.0"), &[]);

  assert_eq!(
    edited(&plan, "Cargo.lock"),
    "version = 4\n\n[[package]]\nname = \"own\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"shared\"\nversion = \"0.1.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
  );
  assert_eq!(plan.changes.len(), 1);
}

// "in a JSON document, each member of its top-level object whose value is the unit's own version, a build suffix
// such as +source kept, and an OpenAPI document's info.version", the layout as it stood and the packages a
// manifest names left
#[test]
fn a_json_document_moves_its_top_level_own_versions_and_info_version_in_its_own_layout() {
  let package = "{\n  \"name\": \"client\",\n  \"version\": \"0.1.0-rev.9\",\n  \"dependencies\": { \"sibling\": \"0.1.0-rev.9\", \"pack\": \"file:vendor/pack-0.1.0-rev.73.tgz\" }\n}\n";
  let layer = "{\n  \"layer\": \"0.1.0-rev.73+source\",\n  \"shared\": [\"vue\"]\n}\n";
  let openapi = "{\"openapi\": \"3.1.0\", \"info\": {\"title\": \"t\", \"version\": \"0.1.0-rev.2\"}}";
  let plan = plan_files(
    &files(&[
      ("client/package.json", package),
      ("shared/layer.json", layer),
      ("docs/openapi.json", openapi),
    ]),
    &Standing::Before,
    &version("1.0.0"),
    &[],
  );

  assert_eq!(
    edited(&plan, "client/package.json"),
    package.replacen("\"version\": \"0.1.0-rev.9\"", "\"version\": \"1.0.0\"", 1)
  );
  assert_eq!(
    edited(&plan, "shared/layer.json"),
    "{\n  \"layer\": \"1.0.0+source\",\n  \"shared\": [\"vue\"]\n}\n"
  );
  assert_eq!(
    edited(&plan, "docs/openapi.json"),
    "{\"openapi\": \"3.1.0\", \"info\": {\"title\": \"t\", \"version\": \"1.0.0\"}}"
  );
}

// "a pyproject.toml's [project] version", "a VERSION file" and "the version after Fixed at on the line that
// fixes it"; every other file naming an own version is named as left as it stands
#[test]
fn the_project_the_version_file_and_the_fixed_line_move_and_other_mentions_are_left() {
  let plan = plan_files(
    &files(&[
      ("VERSION", "0.1.0-rev.4\n"),
      ("sdk/pyproject.toml", "[project]\nname = \"kit\"\nversion = \"0.1.0-rev.9\"\n"),
      (
        "docs/api.md",
        "# API\n\n**Fixed at 0.1.0-rev.8.** The rest.\n\nfixed at version `0.1.0-rev.8` too.\n",
      ),
      ("tests/version.rs", "assert_eq!(version, \"0.1.0-rev.8\");\n"),
      ("bun.lock", "\"pack-0.1.0-rev.73.tgz\"\n"),
    ]),
    &Standing::Before,
    &version("1.0.0"),
    &[],
  );

  assert_eq!(edited(&plan, "VERSION"), "1.0.0\n");
  assert_eq!(edited(&plan, "sdk/pyproject.toml"), "[project]\nname = \"kit\"\nversion = \"1.0.0\"\n");
  assert_eq!(
    edited(&plan, "docs/api.md"),
    "# API\n\n**Fixed at 1.0.0.** The rest.\n\nfixed at version `1.0.0` too.\n"
  );
  assert_eq!(plan.left, ["tests/version.rs"]);
}

// "A file under a node_modules/ or a fixtures/ folder, or one the declaration's [check.shared_versions] exclude
// reaches, is not read"; an upstream's version is no unit's own
#[test]
fn excluded_files_and_upstream_versions_are_left_unread() {
  let plan = plan_files(
    &files(&[
      ("packages/upstream/package.json", "{\"version\": \"0.1.0\"}"),
      ("tests/fixtures/package.json", "{\"version\": \"0.1.0\"}"),
      ("app/node_modules/x/package.json", "{\"version\": \"0.1.0\"}"),
      ("forked/package.json", "{\"version\": \"0.27.5\"}"),
    ]),
    &Standing::Before,
    &version("1.0.0"),
    &["packages/**".to_owned()],
  );

  assert_eq!(plan, Default::default());
}

// After the first release the release's own version is what moves, and a nightly or a revision is no unit's.
#[test]
fn after_a_release_the_releases_version_moves_to_the_next() {
  let plan = plan_files(
    &files(&[
      ("Cargo.toml", "[package]\nname = \"one\"\nversion = \"1.0.0\"\n"),
      ("VERSION", "0.1.0-rev.3\n"),
    ]),
    &Standing::At(version("1.0.0")),
    &version("1.1.0"),
    &[],
  );

  assert_eq!(edited(&plan, "Cargo.toml"), "[package]\nname = \"one\"\nversion = \"1.1.0\"\n");
  assert!(plan.edits.iter().all(|(path, _)| path != "VERSION"));
}

// "a file that does not read refuses the whole release"; a version below the last release, or one a
// repository already holds a tag of, is refused
#[test]
fn a_file_that_does_not_read_or_a_version_not_above_the_last_release_is_refused() {
  let plan = plan_files(
    &files(&[("Cargo.toml", "[package\nversion = \"0.1.0\"")]),
    &Standing::Before,
    &version("1.0.0"),
    &[],
  );

  assert_eq!(plan.unread.len(), 1);
  assert!(refusal(&version("0.1.0"), &Standing::Before, &[], &[]).is_some());
  assert!(refusal(&version("1.0.0-rc.1"), &Standing::Before, &[], &[]).is_some());
  assert!(refusal(&version("1.0.0"), &Standing::At(version("1.0.0")), &[], &[]).is_some());
  assert!(
    refusal(&version("1.0.0"), &Standing::Before, &["one".to_owned()], &[])
      .unwrap()
      .contains("v1.0.0")
  );
  assert_eq!(refusal(&version("1.0.0"), &Standing::Before, &[], &[]), None);
}

// "one marked breaking, or opening with a word the policy names for taking away or renaming, proposes a major
// release; one opening with a word for adding, a minor one; any other, a patch"
#[test]
fn a_subject_proposes_its_step_and_the_strongest_leads() {
  assert_eq!(step_of("Remove the old route"), Step::Major);
  assert_eq!(step_of("feat(api)!: drop a field"), Step::Major);
  assert_eq!(step_of("Mend a thing\n\nBREAKING CHANGE: gone"), Step::Major);
  assert_eq!(step_of("Add a route"), Step::Minor);
  assert_eq!(step_of("feat: a route"), Step::Minor);
  assert_eq!(step_of("Mend the reader"), Step::Patch);
  assert_eq!(step_of("fix: the reader"), Step::Patch);

  let proposal = propose("one", &["Mend a".to_owned(), "Add b".to_owned(), "Rename c".to_owned()]);

  assert_eq!(proposal.counts, [1, 1, 1]);
  assert_eq!(proposal.strongest, Some((Step::Major, "Rename c".to_owned())));
  assert_eq!(Step::Major.from(&Standing::Before), version("1.0.0"));
  assert_eq!(Step::Minor.from(&Standing::At(version("1.2.3"))), version("1.3.0"));
  assert_eq!(Step::Patch.from(&Standing::At(version("1.2.3"))), version("1.2.4"));
}

fn git(dir: &Path, args: &[&str]) -> String {
  let ran = Command::new("git").args(args).current_dir(dir).stdin(Stdio::null()).output().unwrap();

  assert!(ran.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ran.stderr));
  String::from_utf8_lossy(&ran.stdout).trim().to_owned()
}

// A repository at `dir` holding `files`, pushed to a bare remote named for it under `remotes`.
fn repository(dir: &Path, remotes: &Path, name: &str, files: &[(&str, &str)]) {
  let remote = remotes.join(format!("{name}.git"));

  std::fs::create_dir_all(dir).unwrap();
  std::fs::create_dir_all(&remote).unwrap();
  git(&remote, &["init", "--quiet", "--bare", "--initial-branch=main"]);
  git(dir, &["init", "--quiet", "--initial-branch=main"]);
  git(dir, &["config", "user.name", "Tester"]);
  git(dir, &["config", "user.email", "tester@example.com"]);

  for (path, text) in files {
    let at = dir.join(path);

    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, text).unwrap();
  }

  git(dir, &["add", "."]);
  git(dir, &["commit", "--quiet", "-m", "Add the first files"]);
  git(dir, &["remote", "add", "origin", &remote.to_string_lossy()]);
  git(dir, &["push", "--quiet", "origin", "main"]);
  git(dir, &["fetch", "--quiet", "origin"]);
}

// "The set's repositories", "Each is read as its remote's main holds it", "The release, in order": a set of an
// index, two units, a second clone and a repository with no remote, released for real against bare remotes
#[test]
fn a_release_over_a_set_commits_pushes_and_tags_every_repository() {
  let dir = tempfile::tempdir().unwrap();
  let set = dir.path().join("set");
  let remotes = dir.path().join("remotes");

  repository(&set, &remotes, "index", &[("README.md", "The index.\n")]);
  std::fs::write(set.join(".gitignore"), "*/\n").unwrap();
  git(&set, &["add", "."]);
  git(&set, &["commit", "--quiet", "-m", "Ignore the units"]);
  git(&set, &["push", "--quiet", "origin", "main"]);
  repository(
    &set.join("services/one"),
    &remotes,
    "unit-one",
    &[
      ("Cargo.toml", "[package]\nname = \"one\"\nversion = \"0.1.0-rev.4\"\n"),
      ("VERSION", "0.1.0-rev.4\n"),
      ("docs/api.md", "**Fixed at 0.1.0-rev.4.**\n"),
    ],
  );
  repository(
    &set.join("services/two"),
    &remotes,
    "unit-two",
    &[("package.json", "{\"version\": \"0.1.0\"}\n")],
  );
  repository(&set.join("tools/loose"), &remotes, "loose", &[("README.md", "Loose.\n")]);
  git(&set.join("tools/loose"), &["remote", "remove", "origin"]);
  git(
    dir.path(),
    &[
      "clone",
      "--quiet",
      &remotes.join("unit-two.git").to_string_lossy(),
      &set.join("services/two-copy").to_string_lossy(),
    ],
  );
  git(
    &set.join("services/one"),
    &[
      "worktree",
      "add",
      "--quiet",
      "-b",
      "work",
      &set.join("services/one-work").to_string_lossy(),
    ],
  );

  let read = read_set(&set, &machine_git);
  let names: Vec<&str> = read.members.iter().map(|one| one.name.as_str()).collect();
  let passed: Vec<(&str, &str)> =
    read.passed_over.iter().map(|one| (one.folder.as_str(), one.why.as_str())).collect();

  assert_eq!(names, ["index", "unit-one", "unit-two"]);
  assert_eq!(read.members[2].folder, "services/two");
  assert_eq!(passed.len(), 2, "{passed:?}");
  assert!(passed[0].0 == "services/two-copy" && passed[0].1.contains("second clone"), "{passed:?}");
  assert!(passed[1].0 == "tools/loose" && passed[1].1.contains("no origin"), "{passed:?}");

  let to = version("1.0.0");
  let reads: Vec<_> = read
    .members
    .iter()
    .map(|member| read_member(member, &Standing::Before, &to, &machine_git).unwrap())
    .collect();

  assert_eq!(reads[0].plan.changes.len(), 0);
  assert_eq!(
    reads[1].plan.changes.iter().map(|one| one.path.as_str()).collect::<Vec<_>>(),
    ["Cargo.toml", "VERSION", "docs/api.md"]
  );
  assert_eq!(
    member_lines(&reads[0], &to).last().unwrap(),
    "  then: tag v1.0.0 on origin/main's head and push the tag"
  );
  assert!(
    member_lines(&reads[2], &to)
      .iter()
      .any(|line| line == "  package.json: version 0.1.0 → 1.0.0")
  );

  let index_head = reads[0].head.clone();
  let scratch = dir.path().join("scratch");
  let commits = make_release(&reads, &to, &scratch, &machine_git).unwrap();

  assert_eq!(commits, 2);
  assert!(!scratch.exists());

  for (name, released) in [("unit-one", true), ("unit-two", true), ("index", false)] {
    let remote = remotes.join(format!("{name}.git"));
    let tagged = git(&remote, &["rev-list", "-n", "1", "v1.0.0"]);

    assert_eq!(tagged, git(&remote, &["rev-parse", "main"]), "{name}");
    assert_eq!(
      git(&remote, &["log", "-1", "--format=%s", "main"]) == "Release 1.0.0",
      released,
      "{name}"
    );
  }

  assert_eq!(git(&remotes.join("index.git"), &["rev-parse", "main"]), index_head);
  assert_eq!(git(&remotes.join("unit-one.git"), &["show", "main:VERSION"]), "1.0.0");
  assert_eq!(
    git(&remotes.join("unit-two.git"), &["show", "main:package.json"]),
    "{\"version\": \"1.0.0\"}"
  );
  // Each clone's own working tree is left as it stood.
  assert_eq!(std::fs::read_to_string(set.join("services/one/VERSION")).unwrap(), "0.1.0-rev.4\n");

  // The next proposal counts from the release's tag.
  git(&set.join("services/one"), &["fetch", "--quiet", "--tags", "origin"]);
  assert_eq!(subjects_since_release(&read.members[1], &machine_git).unwrap(), Vec::<String>::new());
}
