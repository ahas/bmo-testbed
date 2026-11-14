//! The packages the shared versions pin (docs/checks.md, 「The quick checks」, `shared-versions`): every entry
//! of a checkout's package manifests naming a pinned package is the pinned release's or one of the earlier
//! releases the file names it compatible with. An entry naming a file names one of those releases' files, and
//! the bytes standing there hash to that release's sha256; any other entry is one of their versions as it is
//! written.
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

/// One release of a package: the version a package manifest writes, the file it is packed in and that
/// file's sha256.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
  pub version: String,
  pub file: String,
  pub sha256: String,
}

/// A package the shared versions pin: the release a consumer moves onto, by its version, file and sha256, and
/// the earlier releases it stays compatible with, which a consumer that has not moved may still vendor.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagePin {
  pub version: String,
  pub file: String,
  pub sha256: String,
  #[serde(skip)]
  pub compatible: Vec<Release>,
}

impl PackagePin {
  // The pinned release, then each compatible one.
  fn releases(&self) -> impl Iterator<Item = Release> + '_ {
    let pinned = Release {
      version: self.version.clone(),
      file: self.file.clone(),
      sha256: self.sha256.clone(),
    };

    std::iter::once(pinned).chain(self.compatible.iter().cloned())
  }
}

/// The packages the shared versions' text pins, by name, each with the compatible releases the file names for
/// it; none where it holds no such table, or why a table does not read.
pub fn read_pins(text: &str) -> Result<BTreeMap<String, PackagePin>, String> {
  let table: toml::Table = text.parse().map_err(|why: toml::de::Error| why.to_string())?;
  let words = words();
  let mut pins: BTreeMap<String, PackagePin> = match table.get(&words.packages) {
    Some(packages) => packages.clone().try_into().map_err(|why: toml::de::Error| why.to_string())?,
    None => BTreeMap::new(),
  };
  let compatible: BTreeMap<String, Vec<Release>> = match table.get(&words.compatible) {
    Some(compatible) => compatible.clone().try_into().map_err(|why: toml::de::Error| why.to_string())?,
    None => BTreeMap::new(),
  };

  for (name, releases) in compatible {
    let Some(pin) = pins.get_mut(&name) else {
      return Err(fill_marks(&words.compatible_unpinned, &[("name", &name)]));
    };

    pin.compatible = releases;
  }

  Ok(pins)
}

/// How an entry naming a pinned package differs from the pinned release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Off {
  /// A version, a range or a tag that is not the pinned version.
  Version,
  /// A file other than the pinned one.
  OtherFile,
  /// The file it names does not stand at its path: the path and the release whose file it names.
  Missing(String, Release),
  /// The file it names hashes otherwise: its path, its sha256 and the release whose file it names.
  OtherBytes(String, String, Release),
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
      Off::Missing(path, release) => {
        return Some(fill_marks(&words.package_missing, &[("path", path), ("file", &release.file)]));
      }
      Off::OtherBytes(path, found, release) => {
        return Some(fill_marks(
          &words.package_other_bytes,
          &[("path", path), ("found", found), ("file", &release.file), ("wanted", &release.sha256)],
        ));
      }
    };

    Some(fill_marks(line, &[("version", &pin.version), ("file", &pin.file)]))
  }

  /// The entry as the report names it: as written, marked where its file is missing or differs.
  pub fn shown(&self) -> String {
    let words = words();

    match &self.off {
      Some(Off::Missing(..)) => fill_marks(&words.report_missing_file, &[("entry", &self.found)]),
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
    return (!pin.releases().any(|release| release.version == written)).then_some(Off::Version);
  };
  let named = Path::new(path);
  let file_name = named.file_name().and_then(|name| name.to_str());
  let Some(release) = pin.releases().find(|release| Some(release.file.as_str()) == file_name) else {
    return Some(Off::OtherFile);
  };

  let shown = Path::new(folder_shown).join(named).to_string_lossy().into_owned();

  match sha256_of(&folder.join(named)) {
    None => Some(Off::Missing(shown, release)),
    Some(found) if found != release.sha256 => Some(Off::OtherBytes(shown, found, release)),
    Some(_) => None,
  }
}

/// Every entry of the checkout's package manifests that `reads` takes naming a pinned package, in each list
/// naming packages; a manifest that does not read as a JSON object is named by its path and why.
pub fn hold_packages(
  root: &Path,
  pins: &BTreeMap<String, PackagePin>,
  reads: impl Fn(&str) -> bool,
) -> (Vec<PackageEntry>, Vec<(String, String)>) {
  let words = words();
  let mut entries = Vec::new();
  let mut unread = Vec::new();

  if pins.is_empty() {
    return (entries, unread);
  }

  let manifests = checkout_files_where(root, |path| {
    path.rsplit('/').next() == Some(words.package_manifest.as_str()) && reads(path)
  });

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
