//! `cargo` and `prune` (docs/deploy.md, 「The Cargo build directory」): Cargo run over the shared build
//! directory the way every verb of the testbed runs it, and what no build reads any more removed from that
//! directory, holding Cargo's own build lock in every profile directory, at the front of each lock's line and
//! never in the testbed's; then the target directories of the deployments' build trees that outlived their
//! span. The invocation, the plan, the removal and the locks are the suite crate's; this is
//! the verbs' side of them.

use std::path::Path;

use bmo_testbed_core::declaration::{Repository, root_of};
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::languages::cargo_placeable;
use bmo_testbed_core::placement::{PlaceAsk, runs_here, wide_lane_full};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::now_ms;
use bmo_testbed_core::turn::{Env, Lane, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_deploy::deployment::DeployPlaces;
use bmo_testbed_deploy::expansion::{idle_build_target, remove_build_target};
use bmo_testbed_suite::cargo::run_cargo;
use bmo_testbed_suite::crates::cargo_target_root;
use bmo_testbed_suite::dependents::{read_registry, registry_path};
use bmo_testbed_suite::prune::{
  DAY_MS, HOUR_MS, PrunePlan, ceiling, directory_bytes, disk_free_bytes, keeps_access_times, plan_clear,
  plan_prune, take_prune_locks,
};
use serde_json::json;

use crate::placement_of;
use crate::said::Said;
use crate::words::{BINARY, PruneArgs};

/// `cargo`: Cargo's own code, 75 where the turn did not come. A command that tests, checks or lints, started
/// while every seat of this machine's wide lane is taken, is placed first where the machine's placement sends it
/// elsewhere (docs/testbed.md, 「A run on another machine」), and runs there as this same verb of that machine's
/// testbed.
pub fn cargo(words: &[String]) -> u8 {
  let here = std::env::current_dir().unwrap_or_default();
  let root = root_of(&here).unwrap_or(here);
  let env = process_env();

  if cargo_placeable(words) && !runs_here(&env) && wide_lane_full(&env) {
    let command: Vec<String> = [BINARY.to_owned(), "cargo".to_owned()]
      .into_iter()
      .chain(words.iter().cloned())
      .collect();
    let ask = PlaceAsk {
      command: &command,
      adapter: None,
      lane: Lane::Wide,
      cwd: root.clone(),
      env: &env,
      to_free_seat: true,
    };

    if let Some(code) = placement_of(&env, &root).place(&ask) {
      return code;
    }
  }

  run_cargo(&root, words)
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
        ("stray", &plan.stray_targets.len().to_string()),
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
        ("shared", &gigabytes(PRUNE.shared_ceiling_bytes)),
        ("each", &gigabytes(PRUNE.checkout_ceiling_bytes)),
        ("checkouts", &checkouts),
      ],
    ),
    detail: json!({ "bytes": held, "ceiling_bytes": ceiling.bytes, "checkouts": ceiling.checkouts }),
  });
}

// The shared build directory pruned at `root`: what it says, a refusal where its disk keeps no access times.
fn prune_shared(args: &PruneArgs, root: &Path, days: f64, hours: f64, hook: &Hook) -> Said {
  let said = &VERBS.prune;
  let shown = root.to_string_lossy();

  if !root.exists() {
    return Said {
      out: vec![fill_marks(&said.gone, &[("root", &shown)])],
      ..Said::default()
    };
  }

  if !args.all && !keeps_access_times(root) {
    return Said {
      err: vec![fill_marks(&said.noatime, &[("root", &shown)])],
      code: REFUSED_EXIT_CODE,
      ..Said::default()
    };
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

  let locks = take_prune_locks(root, LANES.numbers.notice_seconds as f64, &|profile| {
    eprintln!("{}", fill_marks(&said.locked, &[("profile", &profile.to_string_lossy())]));
  });
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

/// The deployments whose build trees the prune reads (docs/deploy.md, 「The Cargo build directory」): the
/// places of every checkout of the machine's registry whose declaration deploys, a deploy root several
/// checkouts name read once, and the line saying why none were read where the registry does not read.
#[derive(Debug, Default)]
pub struct Deployments {
  pub places: Vec<DeployPlaces>,
  pub unread: Option<String>,
}

impl Deployments {
  /// The deployments the registry at `registry` names.
  pub fn of_registry(registry: &Path, env: &Env) -> Self {
    let checkouts = match read_registry(registry, env) {
      Ok(checkouts) => checkouts,
      Err(fault) => {
        return Self {
          places: Vec::new(),
          unread: Some(fill_marks(&VERBS.prune.registry_unread, &[("why", &fault.to_string())])),
        };
      }
    };
    let mut places: Vec<DeployPlaces> = Vec::new();

    for checkout in checkouts {
      let Some(found) = Repository::read(&checkout).ok().and_then(|repo| DeployPlaces::of(&repo, env).ok())
      else {
        continue;
      };

      if !places.iter().any(|held| held.deploy_root == found.deploy_root) {
        places.push(found);
      }
    }

    Self {
      places,
      unread: None,
    }
  }
}

// The deployments' build trees' target directories that outlived their span, removed or, in a dry run, named.
fn prune_build_targets(args: &PruneArgs, deployments: &Deployments, now: i64) -> Vec<String> {
  let said = &VERBS.prune;
  let days = PRUNE.deploy_build_days;
  let verb = if args.dry_run { &said.dry } else { &said.removed };
  let mut lines = vec![fill_marks(&said.build_heading, &[("days", &days.to_string()), ("verb", verb)])];

  if let Some(unread) = &deployments.unread {
    lines.push(unread.clone());
  }

  let mut names = Vec::new();
  let mut bytes = 0;
  let mut failed = Vec::new();

  for places in &deployments.places {
    let Some(target) = idle_build_target(places, now, days * DAY_MS) else { continue };
    let held = directory_bytes(&target);
    let removed = if args.dry_run { Ok(()) } else { remove_build_target(places, &target) };

    match removed {
      Ok(()) => {
        names.push(
          places
            .deploy_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        );
        bytes += held;
      }
      Err(why) => failed.push(fill_marks(
        &said.build_not_removed,
        &[("target", &target.to_string_lossy()), ("why", &why)],
      )),
    }
  }

  let named = if names.is_empty() {
    String::new()
  } else {
    fill_marks(&said.named, &[("names", &names.join(", "))])
  };

  lines.push(fill_marks(
    &said.build_counts,
    &[("count", &names.len().to_string()), ("named", &named), ("size", &gigabytes(bytes))],
  ));
  lines.extend(failed);
  lines
}

/// `prune` over the build directory at `root` and the build trees of `deployments`: what it says and the code
/// it ends with.
pub fn prune_in(args: &PruneArgs, root: &Path, deployments: &Deployments, hook: &Hook) -> Said {
  let said = &VERBS.prune;
  let refused = |line: String| Said {
    err: vec![line],
    code: REFUSED_EXIT_CODE,
    ..Said::default()
  };
  let days = args.older_than.unwrap_or(PRUNE.older_than_days as f64);
  let hours = args.incremental_older_than.unwrap_or(PRUNE.incremental_hours as f64);

  if !hours.is_finite() || hours <= 0.0 {
    return refused(said.no_hours.clone());
  }

  if !days.is_finite() || days <= PRUNE.relatime_days as f64 {
    return refused(fill_marks(&said.too_short, &[("days", &PRUNE.relatime_days.to_string())]));
  }

  let mut answer = prune_shared(args, root, days, hours, hook);

  if answer.code == REFUSED_EXIT_CODE {
    return answer;
  }

  answer.out.extend(prune_build_targets(args, deployments, now_ms()));
  answer
}

/// `prune` over the shared build directory and the build trees of the deployments the machine's registry
/// names, its events to the hook the environment or the repository it is run in names.
pub fn prune(args: &PruneArgs) -> u8 {
  let env = process_env();
  let repo = Repository::read(&std::env::current_dir().unwrap_or_default()).ok();
  let deployments = Deployments::of_registry(&registry_path(&data_root(), &env), &env);

  prune_in(args, &cargo_target_root(), &deployments, &Hook::of(repo.as_ref(), &env)).print()
}
