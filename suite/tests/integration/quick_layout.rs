//! The layout check (docs/checks.md, 「The quick checks」): the README's folder table against the tree's
//! top-level folders, folders named for a language, and manifests against their workspace's members.

use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use bmo_testbed_suite::quick::layout::{
  checkout_layout, language_faults, manifest_faults, readme_folders, table_faults, top_folders,
};

use crate::quick::{checkout, repository};

fn owned(given: &[&str]) -> Vec<String> {
  given.iter().map(|one| (*one).to_owned()).collect()
}

// "a top-level folder is missing from the README's folder table, or the table names a folder the tree
// does not hold"
#[test]
fn the_folder_table_names_every_top_level_folder_and_no_other() {
  let readme = "# x\n\n| Name | What |\n|---|---|\n| `a` | b |\n\n## Folders\n\n| Folder | Role |\n|---|---|\n| `app/` | the app |\n| `gone/` | nothing |\n";
  let named = readme_folders(readme).unwrap();
  let tree = top_folders(&owned(&["app/a.rs", "chat/b.ts", "README.md"]));

  assert_eq!(named, ["app", "gone"]);
  assert_eq!(
    table_faults(&named, &tree),
    [
      "chat/: a top-level folder the README.md folder table does not name",
      "README.md: the folder table names gone/, which the tree does not hold",
    ]
  );
  assert!(
    readme_folders("# x\n\nno table\n")
      .unwrap_err()
      .contains("no table opens with a Folder column")
  );
}

// "a top-level folder, or a folder directly under one, is named for a language": the policy's two levels,
// and a third where the levels named are three
#[test]
fn a_top_level_folder_or_one_directly_under_it_is_named_for_no_language() {
  let files = owned(&["rust/a.rs", "app/typescript/a.ts", "app/core/python/a.py", "client/python/a.py"]);
  let languages = owned(&["rust", "typescript", "python"]);
  let levels = QUICK_CHECKS.layout.levels;

  assert_eq!(levels, 2);
  assert_eq!(
    language_faults(&files, &owned(&["client"]), &languages, levels),
    ["app/typescript/: a folder named for a language", "rust/: a folder named for a language"]
  );
  assert!(
    language_faults(&files, &owned(&["client"]), &languages, levels + 1)
      .contains(&"app/core/python/: a folder named for a language".to_owned())
  );
}

// "a folder holding a manifest is no member of its workspace, or a member holds none"
#[test]
fn a_manifests_folder_is_a_member_and_a_member_holds_one_but_inside_a_project_of_its_own() {
  let files = owned(&[
    "Cargo.toml",
    "app/server/Cargo.toml",
    "app/stray/Cargo.toml",
    "tools/face/server/Cargo.toml",
  ]);

  assert_eq!(
    manifest_faults(
      &files,
      "Cargo.toml",
      &owned(&["app/server", "chat/acts"]),
      &owned(&["tools/face"]),
      "Cargo"
    ),
    [
      "app/stray/Cargo.toml: its folder is no member of the Cargo workspace",
      "chat/acts/: a member of the Cargo workspace holding no Cargo.toml",
    ]
  );
}

// "a member may be a glob, reaching every folder it matches and owing none"
#[test]
fn a_member_may_be_a_glob_reaching_every_folder_it_matches_and_owing_none() {
  let files = owned(&["crates/a/Cargo.toml", "crates/b/Cargo.toml", "other/c/Cargo.toml"]);

  assert_eq!(
    manifest_faults(&files, "Cargo.toml", &owned(&["crates/*", "empty/*"]), &[], "Cargo"),
    ["other/c/Cargo.toml: its folder is no member of the Cargo workspace"]
  );
}

// "the check over a checkout reads its README, its Cargo members and its package workspace"
#[test]
fn the_check_over_a_checkout_reads_its_readme_its_cargo_members_and_its_package_workspace() {
  let readme = "# x\n\n| Folder | Role |\n|---|---|\n| `crates/` | the crates |\n| `web/` | the pages |\n";
  let dir = checkout(&[
    ("testbed.toml", "testbed = 0\n"),
    ("README.md", readme),
    ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
    ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
    ("package.json", "{ \"name\": \"root\", \"workspaces\": [\"web/site\"] }\n"),
    ("web/site/package.json", "{ \"name\": \"site\" }\n"),
    ("web/stray/package.json", "{ \"name\": \"stray\" }\n"),
    ("python/a.py", ""),
  ]);
  let found = checkout_layout(&repository(&dir));

  assert_eq!(found.paths, 8);
  assert_eq!(
    found.findings,
    [
      "python/: a top-level folder the README.md folder table does not name",
      "python/: a folder named for a language",
      "web/stray/package.json: its folder is no member of the package workspace",
    ]
  );
}
