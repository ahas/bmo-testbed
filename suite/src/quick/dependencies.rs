//! The dependency check (docs/checks.md, 「The quick checks」): every dependency a manifest of the checkout
//! names is one the declaration allows. A registry crate or package passes, and so does one inside the
//! repository; one from another repository passes only where it is a declared base library, or a
//! dev-dependency on the testbed's test kit.
//!
//! A crate's manifest is read by the TOML reader and a package's by the JSON reader. A package's version
//! spec is npm's own short syntax, told apart here by its leading word alone: a protocol, a path or a
//! hosted repository's `owner/name`; no maintained reader of it ships for Rust.

use std::collections::BTreeMap;
use std::path::Path;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use serde_json::Value;

use super::files::{checkout_files_where, read_text};

// The tables of a crate's manifest naming what it depends on, at its top and under each `[target.…]`,
// each with whether it names dev-dependencies.
const CARGO_TABLES: [(&str, bool); 3] =
  [("dependencies", false), ("dev-dependencies", true), ("build-dependencies", false)];

// The lists of a package's manifest naming what it depends on, each with whether it names
// dev-dependencies.
const PACKAGE_LISTS: [(&str, bool); 4] = [
  ("dependencies", false),
  ("devDependencies", true),
  ("optionalDependencies", false),
  ("peerDependencies", false),
];

/// Where a dependency comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
  /// A registry's crate or package.
  Registry,
  /// A folder of this repository, or another package of its workspace.
  Inside,
  /// Another repository, by what names it: a path outside the root, a git address.
  Foreign(String),
}

/// One dependency a manifest names: the manifest, the name it is known by, whether it is a
/// dev-dependency, and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
  pub manifest: String,
  pub name: String,
  pub dev: bool,
  pub origin: Origin,
}

fn joined(folder: &str, path: &str) -> String {
  if folder.is_empty() { path.to_owned() } else { format!("{folder}/{path}") }
}

/// A path from the root with its `.` and `..` worked out, or none where it climbs above the root or
/// begins at the file system's.
pub fn normal(path: &str) -> Option<String> {
  if path.starts_with('/') || path.starts_with('~') {
    return None;
  }

  let mut parts: Vec<&str> = Vec::new();

  for part in path.split('/') {
    match part {
      "" | "." => {}
      ".." => {
        parts.pop()?;
      }
      _ => parts.push(part),
    }
  }

  Some(parts.join("/"))
}

// Where a path a manifest in `folder` names leads: one beginning at the file system's root or the home
// leads out of the checkout whatever folder names it.
fn path_origin(folder: &str, path: &str) -> Origin {
  let from_root = if path.starts_with(['/', '~']) { None } else { normal(&joined(folder, path)) };

  match from_root {
    Some(_) => Origin::Inside,
    None => Origin::Foreign(path.to_owned()),
  }
}

// Where a crate's dependency entry, written in a manifest in `folder`, comes from; `shared` is the
// workspace's own table and the folder it stands in, which an entry taken from the workspace reads.
fn cargo_origin(
  folder: &str,
  name: &str,
  entry: &toml::Value,
  shared: Option<(&str, &toml::Table)>,
) -> Origin {
  if entry.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
    return match shared.and_then(|(at, table)| table.get(name).map(|entry| (at, entry))) {
      Some((at, entry)) => cargo_origin(at, name, entry, None),
      None => Origin::Registry,
    };
  }

  if let Some(git) = entry.get("git").and_then(toml::Value::as_str) {
    return Origin::Foreign(git.to_owned());
  }

  match entry.get("path").and_then(toml::Value::as_str) {
    Some(path) => path_origin(folder, path),
    None => Origin::Registry,
  }
}

/// Every dependency a crate's manifest in `folder` names, in every table, a target's included; `shared`
/// is the workspace's own table of dependencies and the folder it stands in.
pub fn cargo_dependencies(
  manifest: &str,
  folder: &str,
  table: &toml::Table,
  shared: Option<(&str, &toml::Table)>,
) -> Vec<Dependency> {
  let at_top = CARGO_TABLES.iter().filter_map(|(name, dev)| table.get(*name).map(|one| (one, *dev)));
  let per_target = table
    .get("target")
    .and_then(toml::Value::as_table)
    .into_iter()
    .flat_map(|targets| targets.values())
    .flat_map(|target| {
      CARGO_TABLES
        .iter()
        .filter_map(move |(name, dev)| target.get(*name).map(|one| (one, *dev)))
    });

  at_top
    .chain(per_target)
    .filter_map(|(listed, dev)| listed.as_table().map(|listed| (listed, dev)))
    .flat_map(|(listed, dev)| {
      listed.iter().map(move |(key, entry)| {
        // A dependency renamed, in the manifest or in the workspace entry it takes, is known by the
        // package it names.
        let taken = (entry.get("workspace").and_then(toml::Value::as_bool) == Some(true))
          .then(|| shared.and_then(|(_, table)| table.get(key)))
          .flatten();
        let name = [Some(entry), taken]
          .into_iter()
          .flatten()
          .find_map(|one| one.get("package").and_then(toml::Value::as_str))
          .unwrap_or(key);

        Dependency {
          manifest: manifest.to_owned(),
          name: name.to_owned(),
          dev,
          origin: cargo_origin(folder, key, entry, shared),
        }
      })
    })
    .collect()
}

// Where a package's version spec, written in a manifest in `folder`, comes from.
fn package_origin(folder: &str, spec: &str) -> Origin {
  if spec.starts_with("workspace:") {
    return Origin::Inside;
  }

  for protocol in ["file:", "link:", "portal:"] {
    if let Some(path) = spec.strip_prefix(protocol) {
      return path_origin(folder, path);
    }
  }

  if spec.starts_with('.') || spec.starts_with('/') || spec.starts_with('~') {
    return path_origin(folder, spec);
  }

  // A registry's own protocol, an alias or a catalog entry, names a registry package; any other
  // protocol, or a hosted repository's `owner/name`, names another repository.
  if ["npm:", "jsr:", "catalog:"].iter().any(|registry| spec.starts_with(registry)) {
    Origin::Registry
  } else if spec.contains(':') || spec.contains('/') {
    Origin::Foreign(spec.to_owned())
  } else {
    Origin::Registry
  }
}

/// Every dependency a package's manifest in `folder` names, in every list.
pub fn package_dependencies(manifest: &str, folder: &str, package: &Value) -> Vec<Dependency> {
  PACKAGE_LISTS
    .iter()
    .filter_map(|(list, dev)| package[*list].as_object().map(|listed| (listed, *dev)))
    .flat_map(|(listed, dev)| {
      listed.iter().map(move |(name, spec)| Dependency {
        manifest: manifest.to_owned(),
        name: name.clone(),
        dev,
        origin: package_origin(folder, spec.as_str().unwrap_or_default()),
      })
    })
    .collect()
}

/// The fault in one dependency, where the declaration does not allow it: one from another repository
/// that is no declared base library, nor a dev-dependency on the test kit.
pub fn dependency_fault(dependency: &Dependency, base_libraries: &[String]) -> Option<String> {
  let words = &QUICK_CHECKS.dependencies;
  let Origin::Foreign(source) = &dependency.origin else {
    return None;
  };
  let known = |names: &[String]| names.contains(&dependency.name);

  if known(base_libraries) || (dependency.dev && known(&words.test_kit)) {
    return None;
  }

  let line = if dependency.dev { &words.foreign_dev } else { &words.foreign };

  Some(fill_marks(
    line,
    &[("manifest", &dependency.manifest), ("name", &dependency.name), ("source", source)],
  ))
}

/// What the check came to: how many dependencies it read and a line for each it does not allow, or for
/// each manifest that does not read.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DependencyFindings {
  pub dependencies: usize,
  pub findings: Vec<String>,
}

fn file_name(path: &str) -> &str {
  path.rsplit('/').next().unwrap_or_default()
}

fn folder_of(path: &str) -> &str {
  path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// The check over every crate's and package's manifest the checkout tracks or holds new.
pub fn checkout_dependencies(root: &Path, base_libraries: &[String]) -> DependencyFindings {
  let unreadable =
    |path: &str, why: String| fill_marks(&QUICK_CHECKS.unreadable, &[("path", path), ("why", &why)]);
  let manifests =
    checkout_files_where(root, |path| matches!(file_name(path), "Cargo.toml" | "package.json"));
  let mut crates: BTreeMap<String, toml::Table> = BTreeMap::new();
  let mut found = DependencyFindings::default();
  let mut dependencies = Vec::new();

  for manifest in &manifests {
    let text = read_text(root, manifest);

    if file_name(manifest) == "package.json" {
      match serde_json::from_str::<Value>(&text) {
        Ok(package) => {
          dependencies.extend(package_dependencies(manifest, folder_of(manifest), &package));
        }
        Err(why) => found.findings.push(unreadable(manifest, why.to_string())),
      }
    } else {
      match text.parse::<toml::Table>() {
        Ok(table) => {
          crates.insert(folder_of(manifest).to_owned(), table);
        }
        Err(why) => found.findings.push(unreadable(manifest, why.to_string())),
      }
    }
  }

  for (folder, table) in &crates {
    // The workspace a crate takes shared entries from: the nearest folder at or above its own whose
    // manifest declares one.
    let mut above = Some(folder.as_str());
    let mut shared = None;

    while let Some(at) = above {
      if let Some(table) = crates
        .get(at)
        .and_then(|manifest| manifest.get("workspace"))
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table)
      {
        shared = Some((at, table));
        break;
      }

      above = (!at.is_empty()).then(|| folder_of(at));
    }

    dependencies.extend(cargo_dependencies(&joined(folder, "Cargo.toml"), folder, table, shared));
  }

  found.dependencies = dependencies.len();
  found
    .findings
    .extend(dependencies.iter().filter_map(|one| dependency_fault(one, base_libraries)));
  found
}
