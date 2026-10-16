//! The tools checked by their manifest (docs/checks.md, 「Which tests a change runs」): a folder under the
//! declared tools directory holding the declared manifest, that no crate of the workspace, no workspace
//! package and no project of its own holds, checked by the command the manifest names under the declared
//! key. Each tool declares itself by its manifest; nothing here keeps a list of tools.

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;

use crate::crates::{CrateDeclaration, owning_crate};
use crate::files::checkout_files;
use crate::workspace::{OwnProject, PackageDeclaration, own_project_of, owning_package};

/// A command to run and the variables it is handed.
pub type CheckCommand = (Vec<String>, Vec<(String, String)>);

/// A tool checked by its manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedTool {
  /// Its folder from the checkout's root.
  pub dir: String,
  /// The check command its manifest names, or why there is none to run.
  pub check: Result<String, String>,
}

impl CheckedTool {
  /// Whether a path lies in its folder.
  pub fn holds(&self, path: &str) -> bool {
    path.starts_with(&format!("{}/", self.dir))
  }

  /// What its check runs, inside its folder through `sh`, and what it is handed. None where its manifest
  /// gives it no check to run.
  pub fn command(&self) -> Result<CheckCommand, String> {
    let check = self.check.clone()?;

    Ok((vec!["sh".to_owned(), "-c".to_owned(), check], Vec::new()))
  }
}

/// The check a package declares, run as a tool's is: inside the package's directory through `sh`. None
/// where it declares none.
pub fn package_check(package: &PackageDeclaration) -> Option<CheckedTool> {
  let check = package.check.clone()?;

  Some(CheckedTool {
    dir: package.dir.clone(),
    check: Ok(check),
  })
}

// The command a manifest's text names under a dotted key, or why it names none.
fn manifest_check(text: &str, manifest: &str, key: &str) -> Result<String, String> {
  let table: toml::Table = text.parse().map_err(|why: toml::de::Error| {
    fill_marks(&SUITE.file.unread_manifest, &[("manifest", manifest), ("why", why.message())])
  })?;
  let mut words = key.split('.');
  let first = words.next().and_then(|word| table.get(word));
  let found = words.fold(first, |at, word| at.and_then(|value| value.get(word)));

  found
    .and_then(toml::Value::as_str)
    .map(str::to_owned)
    .ok_or_else(|| fill_marks(&SUITE.file.no_tool_check, &[("manifest", manifest), ("key", key)]))
}

/// Every tool checked by its manifest in the repository: each outermost folder under its declared tools
/// directory holding the declared manifest that none of these crates, packages and projects holds. A
/// repository declaring no tools directory has none.
pub fn checked_tools(
  repo: &Repository,
  packages: &[PackageDeclaration],
  projects: &[OwnProject],
  crates: &[CrateDeclaration],
) -> Vec<CheckedTool> {
  let tools = &repo.declaration.tools;
  let Some(under) = tools.dir.as_ref() else {
    return Vec::new();
  };
  let under = format!("{}/", under.to_string_lossy().trim_end_matches('/'));
  let manifest = tools.manifest.as_str();
  let held = |path: &str| {
    owning_crate(crates, path).is_some()
      || owning_package(packages, path).is_some()
      || own_project_of(projects, path).is_some()
  };
  let mut found: Vec<String> = checkout_files(&repo.root)
    .into_iter()
    .filter(|path| path.starts_with(&under) && !held(path))
    .filter_map(|path| path.strip_suffix(&format!("/{manifest}")).map(str::to_owned))
    .collect();
  let mut dirs: Vec<String> = Vec::new();

  found.sort();

  for dir in found {
    if !dirs.iter().any(|above| dir.starts_with(&format!("{above}/"))) {
      dirs.push(dir);
    }
  }

  dirs
    .into_iter()
    .map(|dir| {
      let check = std::fs::read_to_string(repo.root.join(&dir).join(manifest))
        .map_err(|why| {
          fill_marks(&SUITE.file.unread_manifest, &[("manifest", manifest), ("why", &why.to_string())])
        })
        .and_then(|text| manifest_check(&text, manifest, &tools.check_key));

      CheckedTool { dir, check }
    })
    .collect()
}

/// The tool checked by its manifest a path lies in, or none.
pub fn checked_tool_of<'a>(tools: &'a [CheckedTool], path: &str) -> Option<&'a CheckedTool> {
  tools.iter().find(|one| one.holds(path))
}
