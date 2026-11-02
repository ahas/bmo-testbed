//! The dependency check (docs/checks.md, 「The quick checks」): where each dependency a manifest names
//! comes from, and which of those from another repository the declaration allows.

use bmo_testbed_suite::quick::dependencies::{
  Dependency, DependencyFindings, Origin, cargo_dependencies, checkout_dependencies, dependency_fault,
  normal, package_dependencies,
};

use crate::quick::checkout;

fn origins(found: &[Dependency]) -> Vec<(&str, bool, Origin)> {
  found.iter().map(|one| (one.name.as_str(), one.dev, one.origin.clone())).collect()
}

fn dependency(name: &str, dev: bool, origin: Origin) -> Dependency {
  Dependency {
    manifest: "app/Cargo.toml".to_owned(),
    name: name.to_owned(),
    dev,
    origin,
  }
}

fn foreign(source: &str) -> Origin {
  Origin::Foreign(source.to_owned())
}

// "a path is worked out from the root, and one climbing above it or starting elsewhere leads out"
#[test]
fn a_path_is_worked_out_from_the_root_and_one_climbing_above_it_or_starting_elsewhere_leads_out() {
  assert_eq!(normal("crates/a/../b/./c").as_deref(), Some("crates/b/c"));
  assert_eq!(normal("a/..").as_deref(), Some(""));
  assert_eq!(normal("a/../.."), None);
  assert_eq!(normal("/opt/lib"), None);
  assert_eq!(normal("~/lib"), None);
}

// "a crate's dependencies are read from every table, a target's included, renamed ones by their package"
#[test]
fn a_crates_dependencies_are_read_from_every_table_a_targets_included_renamed_ones_by_their_package() {
  let manifest: toml::Table = r#"
[dependencies]
near = { path = "../near" }
far = { path = "../../../far" }
serde = "1"
other = { package = "real-name", git = "https://example.com/other.git" }
[dev-dependencies]
kit = { path = "/opt/kit" }
[target.'cfg(unix)'.build-dependencies]
built = { path = "../../../built" }
"#
  .parse()
  .unwrap();
  let found = cargo_dependencies("crates/app/Cargo.toml", "crates/app", &manifest, None);
  let mut found = origins(&found);

  found.sort_by(|a, b| a.0.cmp(b.0));
  assert_eq!(
    found,
    [
      ("built", false, foreign("../../../built")),
      ("far", false, foreign("../../../far")),
      ("kit", true, foreign("/opt/kit")),
      ("near", false, Origin::Inside),
      ("real-name", false, foreign("https://example.com/other.git")),
      ("serde", false, Origin::Registry),
    ]
  );
}

// "an entry taken from the workspace is read where the workspace names it, from the workspace's folder"
#[test]
fn an_entry_taken_from_the_workspace_is_read_where_the_workspace_names_it_from_its_folder() {
  let manifest: toml::Table =
    "[dependencies]\nshared.workspace = true\nlib = { workspace = true }\nalias = { workspace = true }\nserde.workspace = true\n"
      .parse()
      .unwrap();
  let workspace: toml::Table = "shared = { path = \"crates/shared\" }\nlib = { path = \"../lib\" }\nalias = { package = \"real-lib\", path = \"../real\" }\nserde = \"1\"\n"
    .parse()
    .unwrap();
  let found = cargo_dependencies("crates/app/Cargo.toml", "crates/app", &manifest, Some(("", &workspace)));
  let mut found = origins(&found);

  found.sort_by(|a, b| a.0.cmp(b.0));
  assert_eq!(
    found,
    [
      ("lib", false, foreign("../lib")),
      ("real-lib", false, foreign("../real")),
      ("serde", false, Origin::Registry),
      ("shared", false, Origin::Inside),
    ]
  );
}

// "a package's spec is a registry's, a workspace's, a path's or another repository's"
#[test]
fn a_packages_spec_is_a_registrys_a_workspaces_a_paths_or_another_repositorys() {
  let package: serde_json::Value = serde_json::from_str(
    r#"{
      "dependencies": {
        "plain": "^1.2.0",
        "sibling": "workspace:*",
        "near": "file:../near",
        "far": "file:../../far",
        "linked": "link:../../linked",
        "relative": "../../relative",
        "alias": "npm:other@1",
        "hosted": "owner/name",
        "remote": "git+https://example.com/remote.git"
      },
      "devDependencies": { "kit": "file:../../kit" },
      "peerDependencies": { "peer": "*" }
    }"#,
  )
  .unwrap();
  let found = package_dependencies("web/package.json", "web", &package);
  let mut found = origins(&found);

  found.sort_by(|a, b| a.0.cmp(b.0));
  assert_eq!(
    found,
    [
      ("alias", false, Origin::Registry),
      ("far", false, foreign("../../far")),
      ("hosted", false, foreign("owner/name")),
      ("kit", true, foreign("../../kit")),
      ("linked", false, foreign("../../linked")),
      ("near", false, Origin::Inside),
      ("peer", false, Origin::Registry),
      ("plain", false, Origin::Registry),
      ("relative", false, foreign("../../relative")),
      ("remote", false, foreign("git+https://example.com/remote.git")),
      ("sibling", false, Origin::Inside),
    ]
  );
}

// "a foreign dependency passes as a declared base library, or as a dev-dependency on the test kit"
#[test]
fn a_foreign_dependency_passes_as_a_declared_base_library_or_as_a_dev_dependency_on_the_test_kit() {
  let base = ["bmo-chat".to_owned()];

  assert_eq!(dependency_fault(&dependency("serde", false, Origin::Registry), &base), None);
  assert_eq!(dependency_fault(&dependency("near", false, Origin::Inside), &base), None);
  assert_eq!(dependency_fault(&dependency("bmo-chat", false, foreign("../chat")), &base), None);
  assert_eq!(
    dependency_fault(&dependency("bmo-testbed-test-support", true, foreign("../testbed")), &base),
    None
  );
  assert_eq!(
    dependency_fault(&dependency("bmo-testbed-test-support", false, foreign("../testbed")), &base)
      .as_deref(),
    Some(
      "app/Cargo.toml: bmo-testbed-test-support comes from another repository (../testbed) and is no declared base library"
    )
  );
  assert_eq!(
    dependency_fault(&dependency("stray", true, foreign("https://example.com/stray.git")), &base)
      .as_deref(),
    Some(
      "app/Cargo.toml: stray comes from another repository (https://example.com/stray.git) and is neither a declared base library nor the test kit"
    )
  );
}

// "the check reads every manifest of a checkout, crates taking shared entries from their workspace"
#[test]
fn the_check_reads_every_manifest_of_a_checkout_crates_taking_shared_entries_from_their_workspace() {
  let dir = checkout(&[
    (
      "Cargo.toml",
      "[workspace]\nmembers = [\"crates/*\"]\n[workspace.dependencies]\nbmo-chat = { path = \"../chat\" }\nstray = { path = \"../stray\" }\n",
    ),
    (
      "crates/a/Cargo.toml",
      "[package]\nname = \"a\"\n[dependencies]\nbmo-chat.workspace = true\nstray.workspace = true\nb = { path = \"../b\" }\n",
    ),
    (
      "crates/b/Cargo.toml",
      "[package]\nname = \"b\"\n[dev-dependencies]\nbmo-testbed-scratch-database = { path = \"../../../testbed/scratch-database\" }\n",
    ),
    (
      "web/package.json",
      "{ \"devDependencies\": { \"@bmo-testbed/testkit\": \"file:../../testbed/testkit\" }, \"dependencies\": { \"left\": \"^1\" } }\n",
    ),
    ("broken/package.json", "{ not json"),
  ]);
  let found = checkout_dependencies(dir.path(), &["bmo-chat".to_owned()]);

  assert_eq!(found.dependencies, 6);
  assert_eq!(found.findings.len(), 2);
  assert!(found.findings[0].starts_with("broken/package.json: "));
  assert_eq!(
    found.findings[1],
    "crates/a/Cargo.toml: stray comes from another repository (../stray) and is no declared base library"
  );
}

// "a checkout naming no manifest has no dependency to fault"
#[test]
fn a_checkout_naming_no_manifest_has_no_dependency_to_fault() {
  let dir = checkout(&[("notes.txt", "")]);

  assert_eq!(checkout_dependencies(dir.path(), &[]), DependencyFindings::default());
}
