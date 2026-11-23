//! The set's version names (bmo-chat's `docs/conventions.md`, 「Versions」; docs/deploy.md, 「The deployed
//! version」): the version a tree declares, the tag that names a release, the day in the platform's time zone,
//! and the nightly a deployment between releases is named for.

use chrono::{DateTime, FixedOffset, NaiveDate};
use semver::{BuildMetadata, Prerelease, Version};

use crate::policy::versions::VERSIONS;

/// The tag that names a release: the version after the policy's prefix, as `v1.0.0`.
pub fn release_tag(version: &str) -> String {
  format!("{}{version}", VERSIONS.tag_prefix)
}

/// The release a tag names, or none where the tag is no release tag: the prefix, then a semantic version
/// carrying no pre-release and no build.
pub fn release_of_tag(tag: &str) -> Option<Version> {
  let version = Version::parse(tag.trim().strip_prefix(VERSIONS.tag_prefix.as_str())?).ok()?;

  (version.pre.is_empty() && version.build.is_empty()).then_some(version)
}

/// The newest release the tags name, or none where they name no release.
pub fn last_release<'a>(tags: impl IntoIterator<Item = &'a str>) -> Option<Version> {
  tags.into_iter().filter_map(release_of_tag).max()
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

/// The nightly a day's deployment after a release is named for: the minor above the release, then the
/// policy's word and the day, as `1.1.0-nightly.20261124` after `1.0.0`. A version carrying a pre-release of
/// its own counts from its major and minor alone.
pub fn nightly(released: &Version, day: NaiveDate) -> Version {
  let pre = format!("{}.{}", VERSIONS.nightly, day.format("%Y%m%d"));

  Version {
    major: released.major,
    minor: released.minor + 1,
    patch: 0,
    pre: Prerelease::new(&pre).unwrap_or(Prerelease::EMPTY),
    build: BuildMetadata::EMPTY,
  }
}

/// The version a deployment of a commit names: the declared version where a tag among `tags` names that
/// release, else the day's nightly after it. A commit declaring none names none, and a declared version
/// that is no semantic version is named as it is written.
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

  let cargo = read(&VERSIONS.release.files.cargo_manifest)
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

  read(&VERSIONS.release.files.package_manifest)
    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    .and_then(|manifest| manifest.get("version").and_then(serde_json::Value::as_str).map(str::to_owned))
    .unwrap_or_default()
}
