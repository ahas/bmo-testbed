//! The shared versions check (docs/checks.md, 「The quick checks」): every entry of the checkout's Cargo
//! manifests that names a listed crate is the shared line byte for byte, or takes the workspace's adding
//! nothing but `optional`, and every registry package of its locks on a semver line the snapshot names
//! stands at the snapshot's version; an exception the declaration names stands in for the shared line or
//! version of its crate. Every entry of its package manifests naming a pinned package is the pinned release's
//! (`shared_packages`). The shared versions are read from the root of the checkout the machine's registry
//! lists under the repository name the declaration gives.
//!
//! A manifest is read by the format-keeping TOML reader, since the rule holds the line as written, not
//! only what it means; a lock by the TOML reader; a version by the semver reader.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::{Repository, SharedException};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::{QUICK_CHECKS, SharedVersionsWords};
use bmo_testbed_core::turn::Env;
use bmo_testbed_core::verb::data_root_of;
use semver::Version;
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, TableLike};

use super::Findings;
use super::files::{checkout_files_where, read_text};
use super::shared_packages::{PackagePin, hold_packages, read_pins};
use crate::dependents::{read_registry, registry_path};

// The tables of a crate's manifest naming what it depends on, at its top and under each `[target.…]`.
const CARGO_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

fn words() -> &'static SharedVersionsWords {
  &QUICK_CHECKS.shared_versions
}

/// The shared versions: the line each listed crate is written with, as the text after its key, the
/// versions the snapshot locks each crate below them at, and the packages pinned to one release's file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Shared {
  pub lines: BTreeMap<String, String>,
  pub snapshot: BTreeMap<String, Vec<Version>>,
  pub packages: BTreeMap<String, PackagePin>,
}

// An entry's value as it is written, its whitespace and comments either side left out; an entry written
// as a table of its own is shown as the inline table it stands for.
fn written(item: &Item) -> String {
  match item {
    Item::Value(value) => {
      let mut value = value.clone();

      value.decor_mut().clear();
      value.to_string()
    }
    other => match other.as_table_like() {
      Some(table) => {
        let pairs: Vec<String> =
          table.iter().map(|(key, value)| format!("{key} = {}", written(value))).collect();

        format!("{{ {} }}", pairs.join(", "))
      }
      None => other.to_string().trim().to_owned(),
    },
  }
}

fn version_of(text: &str) -> Result<Version, String> {
  Version::parse(text).map_err(|why| format!("{text}: {why}"))
}

/// The shared versions a file's text holds, or why it does not read as them.
pub fn read_shared(text: &str) -> Result<Shared, String> {
  let document: DocumentMut = text.parse().map_err(|why: toml_edit::TomlError| why.to_string())?;
  let mut shared = Shared::default();

  if let Some(lines) = document.get(&words().lines).and_then(Item::as_table_like) {
    for (name, item) in lines.iter() {
      shared.lines.insert(name.to_owned(), written(item));
    }
  }

  if let Some(snapshot) = document.get(&words().snapshot).and_then(Item::as_table_like) {
    for (name, item) in snapshot.iter() {
      let versions = match (item.as_str(), item.as_array()) {
        (Some(one), _) => vec![version_of(one)?],
        (None, Some(many)) => many
          .iter()
          .map(|one| one.as_str().map_or_else(|| Err(format!("{name}: {one}")), version_of))
          .collect::<Result<_, _>>()?,
        (None, None) => return Err(format!("{name} = {}", written(item))),
      };

      shared.snapshot.insert(name.to_owned(), versions);
    }
  }

  shared.packages = read_pins(text)?;

  Ok(shared)
}

/// The root of the checkout the registry lists under `repository`'s declared name, or why none is found.
pub fn shared_root(repository: &str, env: &Env) -> Result<PathBuf, String> {
  let registry = registry_path(&data_root_of(env), env);
  let shown = registry.to_string_lossy();
  let checkouts = read_registry(&registry, env)
    .map_err(|fault| fill_marks(&words().unread_registry, &[("registry", &shown), ("why", &fault.why)]))?;
  let mut roots: Vec<PathBuf> = checkouts
    .iter()
    .filter_map(|checkout| Repository::read(checkout).ok())
    .filter(|checkout| checkout.name() == repository)
    .map(|checkout| std::fs::canonicalize(&checkout.root).unwrap_or(checkout.root))
    .collect();

  roots.sort();
  roots.dedup();

  match roots.as_slice() {
    [] => Err(fill_marks(&words().no_checkout, &[("registry", &shown), ("repository", repository)])),
    [root] => Ok(root.clone()),
    many => {
      let named: Vec<String> = many.iter().map(|root| root.to_string_lossy().into_owned()).collect();

      Err(fill_marks(
        &words().two_checkouts,
        &[("registry", &shown), ("repository", repository), ("roots", &named.join(", "))],
      ))
    }
  }
}

/// The shared versions standing at the root of the checkout the registry lists under `repository`.
pub fn find_shared(repository: &str, env: &Env) -> Result<Shared, String> {
  let path = shared_root(repository, env)?.join(&words().file);
  let shown = path.to_string_lossy();
  let text = std::fs::read_to_string(&path)
    .map_err(|why| fill_marks(&words().not_shared, &[("path", &shown), ("why", &why.to_string())]))?;

  read_shared(&text).map_err(|why| fill_marks(&words().not_shared, &[("path", &shown), ("why", &why)]))
}

/// One way a checkout differs from the shared versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Divergence {
  /// A manifest's entry for a listed crate that is not its line, or that adds to the workspace's line.
  Line {
    manifest: String,
    table: String,
    name: String,
    found: String,
    wanted: String,
    inherited: bool,
  },
  /// A package manifest's entry for a pinned package that is not the pinned release's, and what to write.
  Package {
    manifest: String,
    list: String,
    name: String,
    found: String,
    why: String,
  },
  /// A lock's package standing at another version of the snapshot's line.
  Locked {
    lock: String,
    name: String,
    found: String,
    wanted: String,
  },
  /// An exception that does not stand, or a file that does not read.
  Other(String),
}

impl Divergence {
  /// The line the check prints for it.
  pub fn said(&self) -> String {
    let words = words();

    match self {
      Self::Line {
        manifest,
        table,
        name,
        found,
        wanted,
        inherited,
      } => fill_marks(
        if *inherited { &words.inherited } else { &words.line },
        &[
          ("manifest", manifest),
          ("table", table),
          ("name", name),
          ("found", found),
          ("wanted", wanted),
        ],
      ),
      Self::Locked {
        lock,
        name,
        found,
        wanted,
      } => {
        fill_marks(&words.locked, &[("lock", lock), ("name", name), ("found", found), ("wanted", wanted)])
      }
      Self::Package {
        manifest,
        list,
        name,
        found,
        why,
      } => fill_marks(
        &words.package,
        &[("manifest", manifest), ("list", list), ("name", name), ("found", found), ("why", why)],
      ),
      Self::Other(said) => said.clone(),
    }
  }
}

/// What holding a checkout to the shared versions came to: how many entries and packages it read, how each
/// that differs differs, and what it writes for each pinned package it names, as the report shows it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Held {
  pub entries: usize,
  pub divergences: Vec<Divergence>,
  pub packages: BTreeMap<String, BTreeSet<String>>,
}

fn file_name(path: &str) -> &str {
  path.rsplit('/').next().unwrap_or_default()
}

// The line an exception writes, as the text after its key, or why it does not read as one line for the
// crate.
fn exception_line(name: &str, line: &str) -> Result<String, String> {
  let document: DocumentMut = line.parse().map_err(|why: toml_edit::TomlError| why.to_string())?;
  let entries: Vec<(&str, &Item)> = document.iter().collect();

  match entries.as_slice() {
    [(key, item)] if *key == name && item.is_value() => Ok(written(item)),
    _ => Err(line.to_owned()),
  }
}

// A table an item holds, inline or standing on its own.
fn under(item: Option<&Item>) -> Option<&dyn TableLike> {
  item.and_then(Item::as_table_like)
}

// The dependency tables of a manifest, each with the name the faults give it: the workspace's own, those
// at the top and those under each target.
fn dependency_tables(document: &DocumentMut) -> Vec<(String, &dyn TableLike)> {
  let mut tables: Vec<(String, &dyn TableLike)> = Vec::new();

  if let Some(listed) =
    under(document.get("workspace")).and_then(|workspace| under(workspace.get("dependencies")))
  {
    tables.push(("workspace.dependencies".to_owned(), listed));
  }

  for name in CARGO_TABLES {
    if let Some(listed) = under(document.get(name)) {
      tables.push((name.to_owned(), listed));
    }
  }

  if let Some(targets) = under(document.get("target")) {
    for (target, item) in targets.iter() {
      for name in CARGO_TABLES {
        if let Some(listed) = item.as_table_like().and_then(|target| under(target.get(name))) {
          tables.push((format!("target.{target}.{name}"), listed));
        }
      }
    }
  }

  tables
}

// Whether an entry takes its workspace's line.
fn inherits(item: &Item) -> bool {
  item
    .as_table_like()
    .and_then(|entry| entry.get("workspace"))
    .and_then(Item::as_bool)
    .unwrap_or(false)
}

// The faults of one manifest's entries against the lines they are held to, and how many entries it read.
fn hold_manifest(manifest: &str, document: &DocumentMut, held_to: &BTreeMap<String, String>) -> Held {
  let words = words();
  let mut held = Held::default();

  for (table, listed) in dependency_tables(document) {
    for (key, item) in listed.iter() {
      let renamed = item.as_table_like().and_then(|entry| entry.get("package")).and_then(Item::as_str);
      let name = renamed.unwrap_or(key);
      let Some(line) = held_to.get(name) else {
        continue;
      };

      held.entries += 1;

      let found = written(item);
      let entry = item.as_table_like();
      let wanted = if inherits(item) {
        let adds = entry
          .into_iter()
          .flat_map(|entry| entry.iter())
          .any(|(key, _)| !words.inherit_keys.iter().any(|kept| kept == key));

        if !adds {
          continue;
        }

        let optional = entry.and_then(|entry| entry.get("optional")).and_then(Item::as_bool) == Some(true);
        let inherit = if optional { &words.inherit_optional } else { &words.inherit };

        (fill_marks(inherit, &[("name", name)]), true)
      } else if found == *line && renamed.is_none() {
        continue;
      } else {
        (format!("{name} = {line}"), false)
      };

      held.divergences.push(Divergence::Line {
        manifest: manifest.to_owned(),
        table: table.clone(),
        name: key.to_owned(),
        found,
        wanted: wanted.0,
        inherited: wanted.1,
      });
    }
  }

  held
}

/// A lock's packages as Cargo writes them; every other key is left unread.
#[derive(Debug, Deserialize)]
struct Lock {
  #[serde(default)]
  package: Vec<LockedPackage>,
}

#[derive(Debug, Deserialize)]
struct LockedPackage {
  name: String,
  version: String,
  source: Option<String>,
}

// Whether two versions are on one semver line: the same major, or below 1 the same minor, or below 0.1
// the same patch.
fn one_line(left: &Version, right: &Version) -> bool {
  left.major == right.major
    && (left.major > 0 || (left.minor == right.minor && (left.minor > 0 || left.patch == right.patch)))
}

// The faults of one lock's registry packages against the snapshot, but where an exception names the
// version; how many it read; and the versions of each crate it holds.
fn hold_lock(
  path: &str,
  lock: &Lock,
  snapshot: &BTreeMap<String, Vec<Version>>,
  exceptions: &BTreeMap<String, SharedException>,
  holds: &mut BTreeMap<String, BTreeSet<String>>,
) -> Held {
  let words = words();
  let mut held = Held::default();

  for package in &lock.package {
    let from_registry = package
      .source
      .as_deref()
      .is_some_and(|source| words.registry_sources.iter().any(|start| source.starts_with(start.as_str())));

    if !from_registry {
      continue;
    }

    holds.entry(package.name.clone()).or_default().insert(package.version.clone());

    let (Some(versions), Ok(version)) = (snapshot.get(&package.name), Version::parse(&package.version))
    else {
      continue;
    };
    let Some(wanted) = versions.iter().find(|one| one_line(one, &version)) else {
      continue;
    };

    held.entries += 1;

    let excepted = exceptions
      .get(&package.name)
      .is_some_and(|exception| exception.locked.contains(&package.version));

    if *wanted != version && !excepted {
      held.divergences.push(Divergence::Locked {
        lock: path.to_owned(),
        name: package.name.clone(),
        found: package.version.clone(),
        wanted: wanted.to_string(),
      });
    }
  }

  held
}

/// The checkout at `root` held to the shared versions, with the exceptions its declaration names.
pub fn hold(root: &Path, shared: &Shared, exceptions: &BTreeMap<String, SharedException>) -> Held {
  let words = words();
  let mut held = Held::default();
  let mut held_to = shared.lines.clone();

  for (name, exception) in exceptions {
    let Some(shared_line) = shared.lines.get(name) else {
      if exception.line.is_some() || !shared.snapshot.contains_key(name) {
        held
          .divergences
          .push(Divergence::Other(fill_marks(&words.not_listed, &[("name", name)])));
      }

      continue;
    };
    let Some(line) = &exception.line else {
      continue;
    };

    match exception_line(name, line) {
      Ok(line) if line == *shared_line => {
        held
          .divergences
          .push(Divergence::Other(fill_marks(&words.same_line, &[("name", name)])));
      }
      Ok(line) => {
        held_to.insert(name.clone(), line);
      }
      Err(why) => {
        held
          .divergences
          .push(Divergence::Other(fill_marks(&words.bad_line, &[("name", name), ("why", &why)])));
      }
    }
  }

  let unreadable = |line: &str, path: &str, why: String| {
    Divergence::Other(fill_marks(line, &[("path", path), ("why", &why)]))
  };
  let add = |one: Held, held: &mut Held| {
    held.entries += one.entries;
    held.divergences.extend(one.divergences);
  };

  for manifest in checkout_files_where(root, |path| file_name(path) == "Cargo.toml") {
    match read_text(root, &manifest).parse::<DocumentMut>() {
      Ok(document) => add(hold_manifest(&manifest, &document, &held_to), &mut held),
      Err(why) => held.divergences.push(unreadable(&words.not_manifest, &manifest, why.to_string())),
    }
  }

  let mut holds = BTreeMap::new();

  for lock in checkout_files_where(root, |path| file_name(path) == words.lock) {
    match toml::from_str::<Lock>(&read_text(root, &lock)) {
      Ok(read) => add(hold_lock(&lock, &read, &shared.snapshot, exceptions, &mut holds), &mut held),
      Err(why) => held.divergences.push(unreadable(&words.not_lock, &lock, why.to_string())),
    }
  }

  for (name, exception) in exceptions {
    for version in &exception.locked {
      if !holds.get(name).is_some_and(|held| held.contains(version)) {
        held.divergences.push(Divergence::Other(fill_marks(
          &words.not_locked,
          &[("name", name), ("version", version)],
        )));
      }
    }
  }

  let (entries, unread_manifests) = hold_packages(root, &shared.packages);

  for (manifest, why) in unread_manifests {
    held.divergences.push(unreadable(&words.not_package_manifest, &manifest, why));
  }

  for entry in entries {
    held.entries += 1;
    held.packages.entry(entry.name.clone()).or_default().insert(entry.shown());

    if let Some(why) = shared.packages.get(&entry.name).and_then(|pin| entry.why(pin)) {
      held.divergences.push(Divergence::Package {
        manifest: entry.manifest,
        list: entry.list,
        name: entry.name,
        found: entry.found,
        why,
      });
    }
  }

  held
}

/// The check over the repository: its manifests and locks held to the shared versions its declaration
/// names, with its exceptions; a declaration naming none holds nothing.
pub fn checkout_shared_versions(repo: &Repository, env: &Env) -> Findings {
  let Some(declared) = &repo.declaration.check.shared_versions else {
    return Findings::default();
  };

  match find_shared(&declared.repository, env) {
    Ok(shared) => {
      let held = hold(&repo.root, &shared, &declared.exceptions);

      Findings {
        paths: held.entries,
        findings: held.divergences.iter().map(Divergence::said).collect(),
      }
    }
    Err(why) => Findings {
      paths: 0,
      findings: vec![why],
    },
  }
}

/// What `--report` came to: a line for each checkout of the registry, how many of them differ, and how
/// many were read.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
  pub rows: Vec<String>,
  pub differs: usize,
  pub all: usize,
}

// Each pinned package the checkout names, with what it writes for it beside the pinned version.
fn report_packages(held: &Held, shared: &Shared) -> Vec<String> {
  let words = words();

  held
    .packages
    .iter()
    .filter_map(|(name, entries)| {
      let pin = shared.packages.get(name)?;
      let entries: Vec<String> = entries.iter().cloned().collect();

      Some(fill_marks(
        &words.report_package,
        &[
          ("name", name),
          ("entries", &entries.join(&words.report_list_joiner)),
          ("wanted", &pin.version),
        ],
      ))
    })
    .collect()
}

// One checkout's line in the report.
fn report_row(name: &str, held: &Held, shared: &Shared) -> String {
  let words = words();
  let packages = report_packages(held, shared);

  if held.divergences.is_empty() {
    let clean = fill_marks(&words.report_clean, &[("name", name), ("count", &held.entries.to_string())]);

    return std::iter::once(clean).chain(packages).collect::<Vec<_>>().join(&words.report_joiner);
  }

  let mut crates: Vec<&str> = Vec::new();
  let mut versions: Vec<String> = Vec::new();
  let mut others = 0;

  for divergence in &held.divergences {
    match divergence {
      Divergence::Line { name, .. } => {
        if !crates.contains(&name.as_str()) {
          crates.push(name);
        }
      }
      Divergence::Locked {
        name,
        found,
        wanted,
        ..
      } => versions.push(fill_marks(
        &words.report_version,
        &[("name", name), ("found", found), ("wanted", wanted)],
      )),
      Divergence::Package { .. } => {}
      Divergence::Other(_) => others += 1,
    }
  }

  let joined = |items: &[String]| items.join(&words.report_list_joiner);
  let crates: Vec<String> = crates.into_iter().map(str::to_owned).collect();
  let parts: Vec<String> = [
    (!crates.is_empty()).then(|| fill_marks(&words.report_lines, &[("crates", &joined(&crates))])),
    (!versions.is_empty()).then(|| fill_marks(&words.report_locked, &[("versions", &joined(&versions))])),
    (others > 0).then(|| fill_marks(&words.report_others, &[("count", &others.to_string())])),
  ]
  .into_iter()
  .flatten()
  .chain(packages)
  .collect();

  fill_marks(
    &words.report_differs,
    &[
      ("name", name),
      ("count", &held.divergences.len().to_string()),
      ("parts", &parts.join(&words.report_joiner)),
    ],
  )
}

/// Every checkout of the machine's registry held to the shared versions the repository's declaration
/// names, each with the exceptions its own declaration names, whether or not it enables the check; or why
/// the shared versions or the registry could not be read.
pub fn shared_versions_report(repo: &Repository, env: &Env) -> Result<Report, String> {
  let words = words();
  let Some(declared) = &repo.declaration.check.shared_versions else {
    return Ok(Report::default());
  };
  let shared = find_shared(&declared.repository, env)?;
  let registry = registry_path(&data_root_of(env), env);
  let checkouts = read_registry(&registry, env).map_err(|fault| {
    fill_marks(
      &words.unread_registry,
      &[("registry", &registry.to_string_lossy()), ("why", &fault.why)],
    )
  })?;
  let mut report = Report::default();

  for checkout in checkouts {
    report.all += 1;

    match Repository::read(&checkout) {
      Ok(there) => {
        let none = BTreeMap::new();
        let exceptions = there
          .declaration
          .check
          .shared_versions
          .as_ref()
          .map_or(&none, |shared| &shared.exceptions);
        let held = hold(&there.root, &shared, exceptions);

        report.differs += usize::from(!held.divergences.is_empty());
        report.rows.push(report_row(&there.name(), &held, &shared));
      }
      Err(refusal) => {
        report.differs += 1;
        report.rows.push(fill_marks(
          &words.report_unread,
          &[("path", &checkout.to_string_lossy()), ("why", &refusal.to_string())],
        ));
      }
    }
  }

  Ok(report)
}
