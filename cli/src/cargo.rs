//! `cargo` and `prune` (docs/deploy.md, 「The Cargo build directory」): Cargo run over the shared build
//! directory the way every verb of the testbed runs it, and what no build reads any more removed from that
//! directory, holding Cargo's own build lock in every profile directory, at the front of each lock's line and
//! never in the testbed's. The invocation, the plan, the removal and the locks are the suite crate's; this is
//! the verbs' side of them.

use std::path::Path;

use bmo_testbed_core::declaration::{Repository, root_of};
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::now_ms;
use bmo_testbed_core::turn::{NOT_RUN_EXIT_CODE, process_env};
use bmo_testbed_core::verb::REFUSED_EXIT_CODE;
use bmo_testbed_suite::cargo::run_cargo;
use bmo_testbed_suite::crates::cargo_target_root;
use bmo_testbed_suite::prune::{
  CHECKOUT_CEILING_BYTES, DAY_MS, HOUR_MS, INCREMENTAL_HOURS, PrunePlan, RELATIME_DAYS,
  SHARED_CEILING_BYTES, ceiling, directory_bytes, disk_free_bytes, keeps_access_times, plan_clear,
  plan_prune, take_prune_locks,
};
use bmo_testbed_suite::superseded::PRUNE_DAYS;
use serde_json::json;

use crate::said::Said;
use crate::words::PruneArgs;

/// `cargo`: Cargo's own code, 75 where the turn did not come.
pub fn cargo(words: &[String]) -> u8 {
  let here = std::env::current_dir().unwrap_or_default();

  run_cargo(&root_of(&here).unwrap_or(here), words)
}

fn gigabytes(bytes: u64) -> String {
  fill_marks(&VERBS.prune.gigabytes, &[("size", &format!("{:.1}", bytes as f64 / 1e9))])
}

// What a prune says it removed, or would.
fn said_of(
  root: &Path,
  args: &PruneArgs,
  days: f64,
  hours: f64,
  plan: &PrunePlan,
  verb: &str,
) -> Vec<String> {
  let said = &VERBS.prune;
  let checkouts: Vec<String> = plan
    .checkouts
    .iter()
    .map(|dir| dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default())
    .collect();
  let what = if args.all {
    said.everything.clone()
  } else {
    fill_marks(&said.spans, &[("days", &days.to_string()), ("hours", &hours.to_string())])
  };
  let named = if checkouts.is_empty() {
    String::new()
  } else {
    fill_marks(&said.named, &[("names", &checkouts.join(", "))])
  };
  let cleared = if args.all {
    fill_marks(&said.cleared, &[("count", &plan.cleared.len().to_string())])
  } else {
    String::new()
  };

  vec![
    fill_marks(&said.heading, &[("root", &root.to_string_lossy()), ("what", &what), ("verb", verb)]),
    fill_marks(
      &said.counts,
      &[
        ("units", &plan.units.len().to_string()),
        ("incremental", &plan.incremental.len().to_string()),
        ("loose", &plan.loose.len().to_string()),
        ("checkouts", &plan.checkouts.len().to_string()),
        ("named", &named),
        ("cleared", &cleared),
        ("hand_built", &plan.hand_built.len().to_string()),
      ],
    ),
    fill_marks(&said.bytes, &[("size", &gigabytes(plan.bytes))]),
  ]
}

// What the build directory holds after a prune, against its ceiling, sent to the hook where it is over:
// something then builds outside one generation per checkout.
fn hold_to_ceiling(root: &Path, hook: &Hook, answer: &mut Said) {
  let said = &VERBS.prune;
  let held = directory_bytes(root);
  let ceiling = ceiling(root, now_ms());
  let checkouts = ceiling.checkouts.to_string();

  answer.out.push(fill_marks(
    &said.held,
    &[
      ("held", &gigabytes(held)),
      ("ceiling", &gigabytes(ceiling.bytes)),
      ("checkouts", &checkouts),
    ],
  ));

  if held <= ceiling.bytes {
    return;
  }

  hook.send(&Event {
    name: "cargo-target-over-ceiling",
    commit: None,
    summary: fill_marks(
      &said.over,
      &[
        ("root", &root.to_string_lossy()),
        ("held", &gigabytes(held)),
        ("ceiling", &gigabytes(ceiling.bytes)),
        ("shared", &gigabytes(SHARED_CEILING_BYTES)),
        ("each", &gigabytes(CHECKOUT_CEILING_BYTES)),
        ("checkouts", &checkouts),
      ],
    ),
    detail: json!({ "bytes": held, "ceiling_bytes": ceiling.bytes, "checkouts": ceiling.checkouts }),
  });
}

/// `prune` over the build directory at `root`: what it says and the code it ends with.
pub fn prune_in(args: &PruneArgs, root: &Path, hook: &Hook) -> Said {
  let said = &VERBS.prune;
  let refused = |line: String| Said {
    err: vec![line],
    code: REFUSED_EXIT_CODE,
    ..Said::default()
  };
  let days = args.older_than.unwrap_or(PRUNE_DAYS as f64);
  let hours = args.incremental_older_than.unwrap_or(INCREMENTAL_HOURS as f64);
  let shown = root.to_string_lossy();

  if !hours.is_finite() || hours <= 0.0 {
    return refused(said.no_hours.clone());
  }

  if !days.is_finite() || days <= RELATIME_DAYS as f64 {
    return refused(fill_marks(&said.too_short, &[("days", &RELATIME_DAYS.to_string())]));
  }

  if !root.exists() {
    return Said {
      out: vec![fill_marks(&said.gone, &[("root", &shown)])],
      ..Said::default()
    };
  }

  if !args.all && !keeps_access_times(root) {
    return refused(fill_marks(&said.noatime, &[("root", &shown)]));
  }

  let plan_now = || {
    if args.all {
      plan_clear(root)
    } else {
      plan_prune(root, (days * DAY_MS as f64) as i64, now_ms(), (hours * HOUR_MS as f64) as i64)
    }
  };

  if args.dry_run {
    return Said {
      out: said_of(root, args, days, hours, &plan_now(), &said.dry),
      ..Said::default()
    };
  }

  let locks = match take_prune_locks(root, said.lock_wait_seconds as f64) {
    Ok(locks) => locks,
    Err(profile) => {
      return Said {
        err: vec![fill_marks(
          &said.locked,
          &[
            ("profile", &profile.to_string_lossy()),
            ("seconds", &said.lock_wait_seconds.to_string()),
          ],
        )],
        code: NOT_RUN_EXIT_CODE,
        ..Said::default()
      };
    }
  };
  let before = disk_free_bytes(root);
  let planned = plan_now();

  planned.apply();

  let mut answer = Said {
    out: said_of(root, args, days, hours, &planned, &said.removed),
    ..Said::default()
  };

  answer.out.push(fill_marks(
    &said.free,
    &[("before", &gigabytes(before)), ("after", &gigabytes(disk_free_bytes(root)))],
  ));

  if !args.all {
    hold_to_ceiling(root, hook, &mut answer);
  }

  drop(locks);
  answer
}

/// `prune` over the shared build directory, its events to the hook the environment or the repository it is
/// run in names.
pub fn prune(args: &PruneArgs) -> u8 {
  let env = process_env();
  let repo = Repository::read(&std::env::current_dir().unwrap_or_default()).ok();

  prune_in(args, &cargo_target_root(), &Hook::of(repo.as_ref(), &env)).print()
}
