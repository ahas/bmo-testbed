use std::sync::LazyLock;

use bmo_testbed_core::policy::answer::ANSWER;
use bmo_testbed_core::policy::backups::BACKUPS;
use bmo_testbed_core::policy::close_line::CLOSE_LINE;
use bmo_testbed_core::policy::count::COUNT;
use bmo_testbed_core::policy::daily::DAILY;
use bmo_testbed_core::policy::declaration::DECLARATION;
use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::deploy_steps::{DEPLOY_STEPS, DeployStep};
use bmo_testbed_core::policy::deploy_tool::DEPLOY_TOOL;
use bmo_testbed_core::policy::events::EVENTS;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::plugin::PLUGIN;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use bmo_testbed_core::policy::record::RECORD;
use bmo_testbed_core::policy::service::SERVICE;
use bmo_testbed_core::policy::static_check::STATIC_CHECK;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::policy::{fill_marks, fill_shown};

// "every part of the policy reads as its type": each file met once, so one that does not read fails here
// before anything runs on it.
#[test]
fn every_part_reads_as_its_type() {
  LazyLock::force(&LANES);
  LazyLock::force(&SUITE);
  LazyLock::force(&CLOSE_LINE);
  LazyLock::force(&STATIC_CHECK);
  LazyLock::force(&COUNT);
  LazyLock::force(&DAILY);
  LazyLock::force(&DEPLOY);
  LazyLock::force(&DEPLOY_STEPS);
  LazyLock::force(&DEPLOY_TOOL);
  LazyLock::force(&BACKUPS);
  LazyLock::force(&DECLARATION);
  LazyLock::force(&QUICK_CHECKS);
  LazyLock::force(&VERBS);
  LazyLock::force(&EVENTS);
  LazyLock::force(&PRUNE);
  LazyLock::force(&RECORD);
  LazyLock::force(&SERVICE);
  LazyLock::force(&PLUGIN);

  assert_eq!(ANSWER.ceiling_chars, 30_000);
  assert_eq!(SUITE.lines.mark, "suite:");
  assert_eq!(BACKUPS.upload_usage.len(), 2);
}

// "a line's marks are filled in": every mark a value names, each in its place.
#[test]
fn a_lines_marks_are_filled_in() {
  assert_eq!(
    fill_marks("{count} files in {seconds}s", &[("count", "3"), ("seconds", "12")]),
    "3 files in 12s"
  );
}

// "a mark nothing names stays": the mark is left as written, and the others still filled.
#[test]
fn a_mark_nothing_names_stays() {
  assert_eq!(fill_marks("{known} and {unknown}", &[("known", "one")]), "one and {unknown}");
}

// "a value holding a mark is not filled again": every mark is filled at once, from the line alone.
#[test]
fn a_value_holding_a_mark_is_not_filled_again() {
  assert_eq!(
    fill_marks("{first} then {second}", &[("first", "{second}"), ("second", "two")]),
    "{second} then two"
  );
}

// "a brace with no name stays": an open brace with no name, or a name with no closing brace, is text.
#[test]
fn a_brace_with_no_name_stays() {
  assert_eq!(fill_marks("{} and {x", &[("x", "filled")]), "{} and {x");
  assert_eq!(fill_marks("{ x } and {x}", &[("x", "filled")]), "{ x } and filled");
  assert_eq!(fill_marks("ends with {", &[]), "ends with {");
}

// "a value that is no text is written as it displays": numbers and paths fill the marks as `fill_marks`
// fills them with their text.
#[test]
fn a_value_that_is_no_text_is_written_as_it_displays() {
  let path = std::path::Path::new("/srv/data");

  assert_eq!(
    fill_shown(
      "{count} files under {dir}, {left}",
      &[("count", &3), ("dir", &path.display()), ("left", &"{count}")]
    ),
    "3 files under /srv/data, {count}"
  );
}

// "the policy names every step and no other"
#[test]
fn the_file_names_every_step_and_no_other() {
  assert_eq!(DEPLOY_STEPS.keys().copied().collect::<Vec<_>>(), DeployStep::ALL.to_vec());
}

// "each step has a name of its own, and only the suite is named for tests"
#[test]
fn each_step_has_a_name_of_its_own_and_only_the_suite_is_named_for_tests() {
  for step in DeployStep::ALL {
    assert!(!step.name().is_empty());
    assert_eq!(DeployStep::ALL.iter().filter(|other| other.name() == step.name()).count(), 1);
  }

  assert_eq!(
    DeployStep::ALL
      .iter()
      .filter(|step| step.name().contains(DeployStep::Tests.name()))
      .collect::<Vec<_>>(),
    [&DeployStep::Tests]
  );
}
