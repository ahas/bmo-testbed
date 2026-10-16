//! The layout check (docs/checks.md, 「The quick checks」): the README's folder table names every
//! top-level folder and nothing else, no folder of the top two levels is named for a language, and every
//! folder holding a `Cargo.toml` or a `package.json` is a member of its workspace or a project of its own,
//! and every member holds one. A folder inside a project of its own keeps that project's layout, and
//! passes.

use std::collections::BTreeSet;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use markdown::mdast::Node;
use markdown::{ParseOptions, to_mdast};

use super::Findings;
use super::files::{checkout_files_where, read_text};
use crate::selection::glob_matches;
use crate::workspace::{own_projects, workspace_declarations};

fn inside(path: &str, folder: &str) -> bool {
  path == folder || path.starts_with(&format!("{folder}/"))
}

// The folder a file stands in, the root's being the empty one.
fn folder_of(file: &str) -> &str {
  file.rsplit_once('/').map_or("", |(folder, _)| folder)
}

fn tables<'a>(node: &'a Node, found: &mut Vec<&'a Node>) {
  if matches!(node, Node::Table(_)) {
    found.push(node);
  }

  for child in node.children().into_iter().flatten() {
    tables(child, found);
  }
}

/// The folders the README's folder table names, each without its closing `/`: the first cell of every
/// row of the table whose header opens with the policy's head cell.
pub fn readme_folders(text: &str) -> Result<Vec<String>, String> {
  let words = &QUICK_CHECKS.layout;
  let tree = to_mdast(text, &ParseOptions::gfm()).map_err(|why| {
    fill_marks(&words.not_markdown, &[("readme", &words.readme), ("why", &why.to_string())])
  })?;
  let mut found = Vec::new();

  tables(&tree, &mut found);

  let first_cell = |row: &Node| row.children().and_then(|cells| cells.first()).map(Node::to_string);
  let table = found
    .into_iter()
    .filter_map(Node::children)
    .find(|rows| rows.first().and_then(first_cell).is_some_and(|head| head.trim() == words.table_head))
    .ok_or_else(|| {
      fill_marks(&words.no_table, &[("readme", &words.readme), ("head", &words.table_head)])
    })?;

  Ok(
    table
      .iter()
      .skip(1)
      .filter_map(first_cell)
      .map(|cell| cell.trim().trim_end_matches('/').to_owned())
      .collect(),
  )
}

/// Every top-level folder among the files.
pub fn top_folders(files: &[String]) -> BTreeSet<String> {
  files
    .iter()
    .filter_map(|file| file.split_once('/').map(|(top, _)| top.to_owned()))
    .collect()
}

/// Where the README's folder table and the tree's top-level folders part.
pub fn table_faults(named: &[String], tree: &BTreeSet<String>) -> Vec<String> {
  let words = &QUICK_CHECKS.layout;
  let named_set: BTreeSet<&String> = named.iter().collect();
  let missing = tree
    .iter()
    .filter(|folder| !named_set.contains(folder))
    .map(|folder| fill_marks(&words.unnamed, &[("folder", folder), ("readme", &words.readme)]));
  let gone = named_set
    .iter()
    .filter(|folder| !tree.contains(**folder))
    .map(|folder| fill_marks(&words.gone, &[("folder", folder), ("readme", &words.readme)]));

  missing.chain(gone).collect()
}

/// Every folder of the top two levels named for a language, a folder directly under one of the folders
/// passed aside.
pub fn language_faults(files: &[String], passed: &[String], languages: &[String]) -> Vec<String> {
  let named_for_one = |name: &str| languages.iter().any(|language| language.eq_ignore_ascii_case(name));
  let mut folders = BTreeSet::new();

  for file in files {
    let parts: Vec<&str> = file.split('/').collect();

    for depth in 1..parts.len().min(3) {
      folders.insert(parts[..depth].join("/"));
    }
  }

  folders
    .into_iter()
    .filter(|folder| {
      let name = folder.rsplit('/').next().unwrap_or_default();

      named_for_one(name) && (!folder.contains('/') || !passed.iter().any(|outer| inside(folder, outer)))
    })
    .map(|folder| fill_marks(&QUICK_CHECKS.layout.language, &[("folder", &folder)]))
    .collect()
}

/// Every folder holding the manifest named `file` that no member reaches, and every member named
/// outright holding none; a member is a folder or a glob of folders, and the root's own manifest and
/// those inside the folders passed pass.
pub fn manifest_faults(
  files: &[String],
  file: &str,
  members: &[String],
  passed: &[String],
  workspace: &str,
) -> Vec<String> {
  let words = &QUICK_CHECKS.layout;
  let held: BTreeSet<&str> = files
    .iter()
    .filter(|path| path.rsplit('/').next() == Some(file))
    .map(|path| folder_of(path))
    .filter(|folder| !folder.is_empty())
    .collect();
  let mut faults: Vec<String> = held
    .iter()
    .filter(|folder| !members.iter().any(|member| glob_matches(member, folder)))
    .filter(|folder| !passed.iter().any(|outer| inside(folder, outer)))
    .map(|folder| {
      fill_marks(&words.not_member, &[("folder", folder), ("file", file), ("workspace", workspace)])
    })
    .collect();

  faults.extend(
    members
      .iter()
      .filter(|member| glob::Pattern::escape(member) == **member && !held.contains(member.as_str()))
      .map(|member| {
        fill_marks(&words.no_manifest, &[("member", member), ("file", file), ("workspace", workspace)])
      }),
  );
  faults
}

/// The check over the checkout's tracked and new files.
pub fn checkout_layout(repo: &Repository) -> Findings {
  let root = repo.root.as_path();
  let words = &QUICK_CHECKS.layout;
  let files = checkout_files_where(root, |_| true);
  let projects: Vec<String> = own_projects(repo).into_iter().map(|one| one.dir).collect();
  let mut findings = match readme_folders(&read_text(root, &words.readme)) {
    Ok(named) => table_faults(&named, &top_folders(&files)),
    Err(why) => vec![why],
  };

  findings.extend(language_faults(&files, &projects, &words.languages));

  // The Cargo workspace: its members as the root's manifest lists them.
  let cargo: toml::Table = read_text(root, "Cargo.toml").parse().unwrap_or_default();
  let crates: Vec<String> = cargo
    .get("workspace")
    .and_then(|workspace| workspace.get("members"))
    .and_then(toml::Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(toml::Value::as_str)
    .map(|member| member.trim_end_matches('/').to_owned())
    .collect();

  findings.extend(manifest_faults(&files, "Cargo.toml", &crates, &projects, "Cargo"));

  // The package workspace: the packages its entries reach, and the projects of their own beside it.
  let package_dirs: Vec<String> = workspace_declarations(root)
    .into_iter()
    .map(|one| one.dir)
    .chain(projects.iter().cloned())
    .collect();

  findings.extend(manifest_faults(&files, "package.json", &package_dirs, &projects, "package"));

  Findings {
    paths: files.len(),
    findings,
  }
}
