//! The set's version names (bmo-chat's `docs/conventions.md`, 「Versions」; docs/deploy.md, 「The deployed
//! version」): the version a tree declares, the tag that names a release, the day in the platform's time zone,
//! and the nightly a deployment between releases is named for.

use chrono::{DateTime, FixedOffset, NaiveDate};
use semver::{BuildMetadata, Prerelease, Version};

use crate::policy::versions::VERSIONS;

/// The tag that names a release: the version after the policy's prefix, as `v1.0.0` or `v1.1.0-beta.2`.
pub fn release_tag(version: &str) -> String {
  format!("{}{version}", VERSIONS.tag_prefix)
}

/// The day a moment in epoch milliseconds falls on in the platform's time zone, the policy's distance from
/// UTC.
pub fn platform_day(at_ms: i64) -> NaiveDate {
  let zone = FixedOffset::east_opt(VERSIONS.platform_offset_minutes * 60)
    .unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC is an offset"));

  DateTime::from_timestamp_millis(at_ms)
    .unwrap_or_default()
    .with_timezone(&zone)
    .date_naive()
}

/// The version main leads to from the version it declares, the next unreleased one: the set's first release
/// where the declared version is from before it, a major of 0; the version itself where that is an alpha, since
/// main stays on the alpha's line until its first beta; else the minor above, a stable release's, a beta's or a
/// release candidate's whose line has a branch of its own.
pub fn leads_to(declared: &Version) -> Version {
  let alpha = declared.pre.as_str().split('.').next().is_some_and(|word| word == VERSIONS.alpha);

  if declared.major == 0 {
    Version::new(1, 0, 0)
  } else if alpha {
    Version::new(declared.major, declared.minor, declared.patch)
  } else {
    Version::new(declared.major, declared.minor + 1, 0)
  }
}

/// The day's nightly of the version main leads to from `declared`: that version, then the policy's
/// pre-release and the day, as `1.1.0-0.nightly.20261124` after `1.0.0`. Semver orders it after the release
/// before it and before every alpha, beta, release candidate and release of the version it leads to.
pub fn nightly(declared: &Version, day: NaiveDate) -> Version {
  let pre = format!("{}.{}", VERSIONS.nightly, day.format("%Y%m%d"));

  Version {
    pre: Prerelease::new(&pre).unwrap_or(Prerelease::EMPTY),
    build: BuildMetadata::EMPTY,
    ..leads_to(declared)
  }
}

/// The version a deployment of a commit names: the declared version where a tag among `tags` names that
/// release, else the day's nightly of the version main leads to from it. A commit declaring none names none,
/// and a declared version that is no semantic version is named as it is written.
pub fn deployed_version(declared: &str, tags: &[String], day: NaiveDate) -> String {
  let declared = declared.trim();

  if declared.is_empty() {
    return String::new();
  }

  let Ok(version) = Version::parse(declared) else {
    return declared.to_owned();
  };
  let tag = release_tag(declared);

  if tags.iter().any(|one| one.trim() == tag) {
    return declared.to_owned();
  }

  nightly(&version, day).to_string()
}

/// The version a tree declares at its root, read through `read`, which answers a file's text by its path
/// from the root or none where the tree holds no such file: the version file's content, else the version
/// the root Cargo manifest names for its workspace's packages or its package, else the root package
/// manifest's `version`; empty where none of them names one.
pub fn declared_version(read: &dyn Fn(&str) -> Option<String>) -> String {
  if let Some(text) = read(&VERSIONS.version_file)
    && !text.trim().is_empty()
  {
    return text.trim().to_owned();
  }

  let cargo = read(&VERSIONS.cargo_manifest)
    .and_then(|text| text.parse::<toml::Table>().ok())
    .and_then(|manifest| {
      let workspace = manifest.get("workspace").and_then(|workspace| workspace.get("package"));
      let package = manifest.get("package");

      [workspace, package]
        .into_iter()
        .flatten()
        .find_map(|table| table.get("version").and_then(toml::Value::as_str).map(str::to_owned))
    });

  if let Some(version) = cargo {
    return version;
  }

  read(&VERSIONS.package_manifest)
    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    .and_then(|manifest| manifest.get("version").and_then(serde_json::Value::as_str).map(str::to_owned))
    .unwrap_or_default()
}
