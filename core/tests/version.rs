//! The set's version names (docs/deploy.md, 「The deployed version」): the release a tag names, the platform's day
//! and the nightly between releases.

use chrono::NaiveDate;
use semver::Version;

use bmo_testbed_core::version::{
  deployed_version, leads_to, nightly, platform_day, release_of_tag, release_tag,
};

fn day(year: i32, month: u32, date: u32) -> NaiveDate {
  NaiveDate::from_ymd_opt(year, month, date).unwrap()
}

fn tags(names: &[&str]) -> Vec<String> {
  names.iter().map(|one| (*one).to_owned()).collect()
}

// "A commit a release tag names is that release": the newest release tag pointing at it, whatever its kind
#[test]
fn a_commit_a_release_tag_names_is_that_release() {
  let reached = tags(&["v1.0.0-rc.1"]);

  assert_eq!(deployed_version(&tags(&["v1.0.0"]), &reached, day(2026, 11, 24)), "1.0.0");
  assert_eq!(deployed_version(&tags(&["other", "v1.0.0"]), &[], day(2026, 11, 24)), "1.0.0");
  assert_eq!(deployed_version(&tags(&["v1.1.0-beta.2"]), &[], day(2026, 11, 24)), "1.1.0-beta.2");
  assert_eq!(release_tag("2.3.0"), "v2.3.0");
  assert_eq!(release_tag("1.1.0-rc.1"), "v1.1.0-rc.1");
}

// "Any other commit is the day's nightly" of the version main leads to from the newest release tag it reaches:
// the minor above a stable release, a beta's or a release candidate's, the alpha's own version, and the first
// release before any
#[test]
fn an_untagged_commit_is_the_days_nightly_of_the_version_main_leads_to() {
  let at_day = |reached: &[&str], on: NaiveDate| deployed_version(&[], &tags(reached), on);

  assert_eq!(at_day(&["v1.0.0"], day(2026, 11, 24)), "1.1.0-0.nightly.20261124");
  assert_eq!(at_day(&["v1.0.0-beta.1", "v1.0.0"], day(2026, 11, 24)), "1.1.0-0.nightly.20261124");
  assert_eq!(at_day(&["v2.4.1"], day(2027, 1, 2)), "2.5.0-0.nightly.20270102");
  assert_eq!(at_day(&["v1.1.0-alpha.2"], day(2026, 11, 24)), "1.1.0-0.nightly.20261124");
  assert_eq!(at_day(&["v1.1.0-beta.1"], day(2026, 12, 1)), "1.2.0-0.nightly.20261201");
  assert_eq!(at_day(&["v1.1.0-rc.3"], day(2026, 12, 1)), "1.2.0-0.nightly.20261201");
  assert_eq!(at_day(&[], day(2026, 11, 23)), "1.0.0-0.nightly.20261123");
}

// "any other tag, an upstream's kept in a fork among them, counts for nothing"
#[test]
fn a_tag_that_names_no_release_of_the_set_counts_for_nothing() {
  let upstream = ["v0.27.5-beta.3", "v2026.11.23-canary.455", "v0.10.0", "next", "v1.0.0+build"];

  for tag in upstream {
    assert_eq!(release_of_tag(tag), None, "{tag}");
  }

  assert_eq!(
    deployed_version(&tags(&upstream), &tags(&upstream), day(2026, 11, 25)),
    "1.0.0-0.nightly.20261125"
  );
  assert_eq!(release_of_tag("v1.0.0-beta.1"), Some(Version::parse("1.0.0-beta.1").unwrap()));
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

// bmo-chat's 「Versions」: for one version, nightly < alpha < beta < rc < stable, each kind counted by its N
#[test]
fn the_five_kinds_of_one_version_sort_nightly_alpha_beta_rc_stable() {
  let ordered = [
    "1.1.0-0.nightly.20261124",
    "1.1.0-0.nightly.20261124.2",
    "1.1.0-0.nightly.20261125",
    "1.1.0-alpha.1",
    "1.1.0-alpha.2",
    "1.1.0-alpha.10",
    "1.1.0-beta.1",
    "1.1.0-beta.2",
    "1.1.0-rc.1",
    "1.1.0-rc.2",
    "1.1.0",
    "1.1.1",
    "1.2.0-0.nightly.20261126",
  ];
  let versions: Vec<Version> = ordered.iter().map(|one| Version::parse(one).unwrap()).collect();

  for pair in versions.windows(2) {
    assert!(pair[0] < pair[1], "{} < {}", pair[0], pair[1]);
  }

  assert_eq!(nightly(&Version::parse("1.0.0").unwrap(), day(2026, 11, 24)), versions[0]);
}

// "A nightly names the next unreleased version": the minor above a stable release, X.Y.Z itself while an alpha
// of X.Y.Z is main's last release, the minor above once a beta of X.Y.Z has cut its release branch, and 1.0.0
// before the first release
#[test]
fn main_leads_to_the_alphas_version_or_the_minor_above() {
  let led = |declared: &str| leads_to(&Version::parse(declared).unwrap()).to_string();

  assert_eq!(led("1.0.0"), "1.1.0");
  assert_eq!(led("1.0.3"), "1.1.0");
  assert_eq!(led("1.1.0-alpha.3"), "1.1.0");
  assert_eq!(led("2.0.0-alpha.1"), "2.0.0");
  assert_eq!(led("1.1.0-beta.1"), "1.2.0");
  assert_eq!(led("1.1.0-rc.2"), "1.2.0");
  assert_eq!(led("0.1.0-rev.9"), "1.0.0");
  assert_eq!(led("0.1.0"), "1.0.0");
}

// "the day the deployment picked its commit in the platform's time zone, Asia/Seoul"
#[test]
fn the_day_is_counted_in_the_platforms_time_zone() {
  // 2026-11-23 14:59:59 and 15:00:00 in UTC, the last second of one day in Seoul and the first of the next.
  assert_eq!(platform_day(1_795_445_999_000), day(2026, 11, 23));
  assert_eq!(platform_day(1_795_446_000_000), day(2026, 11, 24));
}
