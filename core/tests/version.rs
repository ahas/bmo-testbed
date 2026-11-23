//! The set's version names (docs/deploy.md, 「The deployed version」): the declared version a tree carries, the
//! release a tag names, the platform's day and the nightly between releases.

use chrono::NaiveDate;
use semver::Version;

use bmo_testbed_core::version::{
  declared_version, deployed_version, last_release, nightly, platform_day, release_of_tag, release_tag,
};

fn day(year: i32, month: u32, date: u32) -> NaiveDate {
  NaiveDate::from_ymd_opt(year, month, date).unwrap()
}

fn tags(names: &[&str]) -> Vec<String> {
  names.iter().map(|one| (*one).to_owned()).collect()
}

// "A commit the release tag names is that release"
#[test]
fn a_commit_tagged_with_its_declared_release_is_that_release() {
  assert_eq!(deployed_version("1.0.0", &tags(&["v1.0.0"]), day(2026, 11, 24)), "1.0.0");
  assert_eq!(deployed_version("1.0.0\n", &tags(&["other", "v1.0.0"]), day(2026, 11, 24)), "1.0.0");
  assert_eq!(release_tag("2.3.0"), "v2.3.0");
}

// "Any other commit is the day's nightly": the minor above the declared version, then the day, a tag naming
// another version left aside
#[test]
fn an_untagged_commit_is_the_days_nightly_after_its_declared_release() {
  assert_eq!(deployed_version("1.0.0", &[], day(2026, 11, 24)), "1.1.0-nightly.20261124");
  assert_eq!(
    deployed_version("1.0.0", &tags(&["v0.9.0"]), day(2026, 11, 24)),
    "1.1.0-nightly.20261124"
  );
  assert_eq!(deployed_version("2.4.1", &[], day(2027, 1, 2)), "2.5.0-nightly.20270102");
  assert_eq!(deployed_version("0.1.0-rev.77", &[], day(2026, 11, 23)), "0.2.0-nightly.20261123");
}

// "A commit declaring none is deployed all the same, its version left empty"; "A declared version that is no
// semantic version is recorded as it is written"
#[test]
fn no_declared_version_names_none_and_an_unread_one_stands_as_written() {
  assert_eq!(deployed_version("", &[], day(2026, 11, 24)), "");
  assert_eq!(deployed_version("next", &[], day(2026, 11, 24)), "next");
}

// "Semver orders every nightly after the release before it and before the release it leads to"
#[test]
fn every_nightly_sorts_between_the_release_before_it_and_the_one_it_leads_to() {
  let released = Version::parse("1.0.0").unwrap();
  let first = nightly(&released, day(2026, 11, 24));
  let later = nightly(&released, day(2026, 11, 25));

  assert!(released < first);
  assert!(first < later);
  assert!(later < Version::parse("1.1.0").unwrap());
}

// "the day the deployment picked its commit in the platform's time zone, Asia/Seoul"
#[test]
fn the_day_is_counted_in_the_platforms_time_zone() {
  // 2026-11-23 14:59:59 and 15:00:00 in UTC, the last second of one day in Seoul and the first of the next.
  assert_eq!(platform_day(1_795_445_999_000), day(2026, 11, 23));
  assert_eq!(platform_day(1_795_446_000_000), day(2026, 11, 24));
}

// "The last release is the newest tag v<major>.<minor>.<patch> carrying no pre-release"
#[test]
fn the_last_release_is_the_newest_plain_release_tag() {
  assert_eq!(release_of_tag("v1.2.0"), Some(Version::parse("1.2.0").unwrap()));
  assert_eq!(release_of_tag("v1.3.0-nightly.20261124"), None);
  assert_eq!(release_of_tag("v2026.11.10-canary.905"), None);
  assert_eq!(release_of_tag("1.2.0"), None);
  assert_eq!(
    last_release(["v1.0.0", "v1.10.0", "v1.9.0", "v2.0.0-rc.1", "other"]),
    Some(Version::parse("1.10.0").unwrap())
  );
  assert_eq!(last_release(["other"]), None);
}

// "The declared version is the one the commit itself carries": its version file, else the root Cargo
// manifest's workspace or package version, else the root package manifest's
#[test]
fn the_declared_version_is_read_from_the_version_file_then_cargo_then_the_package_manifest() {
  let tree = |files: &'static [(&'static str, &'static str)]| {
    move |path: &str| files.iter().find(|(name, _)| *name == path).map(|(_, text)| (*text).to_owned())
  };

  assert_eq!(
    declared_version(&tree(&[("VERSION", "1.2.0\n"), ("Cargo.toml", "[package]\nversion = \"9.9.9\"\n")])),
    "1.2.0"
  );
  assert_eq!(
    declared_version(&tree(&[("Cargo.toml", "[workspace.package]\nversion = \"1.3.0\"\n")])),
    "1.3.0"
  );
  assert_eq!(
    declared_version(&tree(&[("Cargo.toml", "[package]\nname = \"one\"\nversion = \"1.4.0\"\n")])),
    "1.4.0"
  );
  assert_eq!(
    declared_version(&tree(&[
      ("Cargo.toml", "[workspace]\nmembers = []\n"),
      ("package.json", "{\"version\": \"1.5.0\"}")
    ])),
    "1.5.0"
  );
  assert_eq!(declared_version(&tree(&[])), "");
}
