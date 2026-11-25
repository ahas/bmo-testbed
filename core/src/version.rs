//! The set's version names (bmo-chat's `docs/conventions.md`, 「Versions」; docs/deploy.md, 「The deployed
//! version」): the tag that names a release, the day in the platform's time zone, and the version a deployment
//! is named for off the tags: the release its commit is tagged with, else the day's nightly.

use chrono::{DateTime, FixedOffset, NaiveDate};
use semver::{BuildMetadata, Prerelease, Version};

use crate::policy::versions::VERSIONS;

/// The version this build of the testbed answers with: the one a deployment hands its build as `BMO_VERSION`, else
/// the manifest's placeholder (bmo-chat's `docs/conventions.md`, 「Versions」).
pub const VERSION: &str = match option_env!("BMO_VERSION") {
  Some(version) => version,
  None => env!("CARGO_PKG_VERSION"),
};

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

/// The release a tag names: the policy's prefix before a semantic version of the first major or above that is a
/// stable release, an alpha, a beta or a release candidate; none for any other tag.
pub fn release_of_tag(tag: &str) -> Option<Version> {
  let version = Version::parse(tag.trim().strip_prefix(VERSIONS.tag_prefix.as_str())?).ok()?;
  let word = version.pre.as_str().split('.').next().unwrap_or_default();
  let release =
    version.pre.is_empty() || word == VERSIONS.alpha || VERSIONS.releases.iter().any(|one| one == word);

  (release && version.build.is_empty() && version.major >= VERSIONS.first_major).then_some(version)
}

/// The version a deployment of a commit names, read off the tags alone: the newest release among `at`, the
/// tags pointing at the commit itself, where one stands; else the day's nightly of the version main leads to
/// from the newest release among `reached`, the tags the commit reaches, the set's first release's before any.
pub fn deployed_version(at: &[String], reached: &[String], day: NaiveDate) -> String {
  let newest = |tags: &[String]| tags.iter().filter_map(|tag| release_of_tag(tag)).max();

  if let Some(release) = newest(at) {
    return release.to_string();
  }

  match newest(reached) {
    Some(last) => nightly(&last, day).to_string(),
    None => nightly(&Version::new(0, 0, 0), day).to_string(),
  }
}
