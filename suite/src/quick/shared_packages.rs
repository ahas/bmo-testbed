//! The packages the shared versions pin (docs/checks.md, 「The quick checks」, `shared-versions`): every entry
//! of a checkout's package manifests naming a pinned package is the pinned release's. An entry naming a file
//! names the pinned file, and the bytes standing there hash to the pinned sha256; any other entry is the
//! pinned version as it is written.
//!
//! The pins are read by the TOML reader, a manifest by the JSON reader and a file's bytes hashed by SHA-256.

use std::collections::BTreeMap;
use std::path::Path;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::{QUICK_CHECKS, SharedVersionsWords};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::files::{checkout_files_where, read_text};

fn words() -> &'static SharedVersionsWords {
  &QUICK_CHECKS.shared_versions
}

/// A package the shared versions pin: the version a package manifest writes, the file its release is packed
/// in and that file's sha256.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagePin {
  pub version: String,
  pub file: String,
  pub sha256: String,
}

/// The packages the shared versions' text pins, by name; none where it holds no such table, or why the table
/// does not read.
pub fn read_pins(text: &str) -> Result<BTreeMap<String, PackagePin>, String> {
  let table: toml::Table = text.parse().map_err(|why: toml::de::Error| why.to_string())?;

  match table.get(&words().packages) {
    Some(packages) => packages.clone().try_into().map_err(|why: toml::de::Error| why.to_string()),
    None => Ok(BTreeMap::new()),
  }
}

/// How an entry naming a pinned package differs from the pinned release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Off {
  /// A version, a range or a tag that is not the pinned version.
  Version,
  /// A file other than the pinned one.
  OtherFile,
  /// The file it names does not stand at its path.
  Missing(String),
  /// The file it names hashes otherwise: its path and its sha256.
  OtherBytes(String, String),
  /// A value that reads as neither a version nor a file.
  Unread,
}

/// One entry of a package manifest naming a pinned package: where it stands, what it writes, and how it
/// differs, where it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageEntry {
  pub manifest: String,
  pub list: String,
  pub name: String,
  pub found: String,
  pub off: Option<Off>,
}

impl PackageEntry {
  /// What the entry should write instead and why, as the check's line says it after the entry.
  pub fn why(&self, pin: &PackagePin) -> Option<String> {
    let words = words();
    let line = match self.off.as_ref()? {
      Off::Version | Off::Unread => &words.package_version,
      Off::OtherFile => &words.package_other_file,
      Off::Missing(path) => {
        return Some(fill_marks(&words.package_missing, &[("path", path), ("file", &pin.file)]));
      }
      Off::OtherBytes(path, found) => {
        return Some(fill_marks(
          &words.package_other_bytes,
          &[("path", path), ("found", found), ("file", &pin.file), ("wanted", &pin.sha256)],
        ));
      }
    };

    Some(fill_marks(line, &[("version", &pin.version), ("file", &pin.file)]))
  }

  /// The entry as the report names it: as written, marked where its file is missing or differs.
  pub fn shown(&self) -> String {
    let words = words();

    match &self.off {
      Some(Off::Missing(_)) => fill_marks(&words.report_missing_file, &[("entry", &self.found)]),
      Some(Off::OtherBytes(..)) => fill_marks(&words.report_other_bytes, &[("entry", &self.found)]),
      _ => self.found.clone(),
    }
  }
}

fn sha256_of(path: &Path) -> Option<String> {
  std::fs::read(path)
    .ok()
    .map(|bytes| Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect())
}

// How one entry differs from its pin, read from the manifest's folder.
fn off(folder: &Path, folder_shown: &str, value: &Value, pin: &PackagePin) -> Option<Off> {
  let Some(written) = value.as_str() else {
    return Some(Off::Unread);
  };
  let Some(path) = written.strip_prefix(&words().file_prefix) else {
    return (written != pin.version).then_some(Off::Version);
  };
  let named = Path::new(path);

  if named.file_name().and_then(|name| name.to_str()) != Some(pin.file.as_str()) {
    return Some(Off::OtherFile);
  }

  let shown = Path::new(folder_shown).join(named).to_string_lossy().into_owned();

  match sha256_of(&folder.join(named)) {
    None => Some(Off::Missing(shown)),
    Some(found) if found != pin.sha256 => Some(Off::OtherBytes(shown, found)),
    Some(_) => None,
  }
}

/// Every entry of the checkout's package manifests naming a pinned package, in each list naming packages; a
/// manifest that does not read as a JSON object is named by its path and why.
pub fn hold_packages(
  root: &Path,
  pins: &BTreeMap<String, PackagePin>,
) -> (Vec<PackageEntry>, Vec<(String, String)>) {
  let words = words();
  let mut entries = Vec::new();
  let mut unread = Vec::new();

  if pins.is_empty() {
    return (entries, unread);
  }

  let manifests =
    checkout_files_where(root, |path| path.rsplit('/').next() == Some(words.package_manifest.as_str()));

  for manifest in manifests {
    let document: Value = match serde_json::from_str(&read_text(root, &manifest)) {
      Ok(Value::Object(document)) => Value::Object(document),
      Ok(_) => {
        unread.push((manifest, words.not_object.clone()));
        continue;
      }
      Err(why) => {
        unread.push((manifest, why.to_string()));
        continue;
      }
    };
    let folder_shown = Path::new(&manifest)
      .parent()
      .map(|folder| folder.to_string_lossy().into_owned())
      .unwrap_or_default();
    let folder = root.join(&folder_shown);

    for list in &words.package_lists {
      let Some(listed) = document.get(list).and_then(Value::as_object) else {
        continue;
      };

      for (name, pin) in pins {
        let Some(value) = listed.get(name) else {
          continue;
        };
        let found = value.as_str().map_or_else(|| value.to_string(), str::to_owned);

        entries.push(PackageEntry {
          manifest: manifest.clone(),
          list: list.clone(),
          name: name.clone(),
          found,
          off: off(&folder, &folder_shown, value, pin),
        });
      }
    }
  }

  (entries, unread)
}
