//! `cargo` and `prune` (docs/deploy.md, 「The Cargo build directory」): Cargo run over the shared build
//! directory the way every verb of the testbed runs it, and what no build reads any more removed from that
//! directory, holding Cargo's own build lock in every profile directory, at the front of each lock's line and
//! never in the testbed's, and its ceiling enforced; then the target directories of the deployments' build trees
//! that outlived their span; then the filesystem holding the build directory trimmed where its trim is due, so its
//! freed blocks go back to the drive it stands on. Its timer runs it every few minutes, and it prunes only where
//! the directory stands over its ceiling or its last prune is old enough. The invocation, the plan, the removal,
//! the enforcement and the locks are the suite crate's; this is the verbs' side of them.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::{Repository, root_of};
use bmo_testbed_core::events::{Event, Hook};
use bmo_testbed_core::languages::cargo_placeable;
use bmo_testbed_core::placement::{PlaceAsk, runs_here};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::lanes::LANES;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::now_ms;
use bmo_testbed_core::turn::{Env, Lane, process_env};
use bmo_testbed_core::verb::{REFUSED_EXIT_CODE, data_root};
use bmo_testbed_deploy::deployment::DeployPlaces;
use bmo_testbed_deploy::expansion::{
  deployment_running, expansion_tree, idle_build_target, remove_build_target,
};
use bmo_testbed_suite::cargo::run_cargo;
use bmo_testbed_suite::ceiling::{Ceiling, CeilingPlan, CeilingSetting, Enforcement, Keepers};
use bmo_testbed_suite::crates::cargo_target_root;
use bmo_testbed_suite::dependents::{read_registry, registry_path};
use bmo_testbed_suite::prune::{
  DAY_MS, HOUR_MS, MINUTE_MS, OVER_SENT_MARK_NAME, PLACED_LOW_SENT_MARK_NAME, PRUNED_MARK_NAME, PrunePlan,
  TRIMMED_MARK_NAME, directory_bytes, disk_free_bytes, keeps_access_times, mark, marked_at, plan_clear,
  plan_prune, take_prune_locks,
};
use serde_json::json;

use crate::placement::{FarDisk, MachinePlacement};
use crate::said::Said;
use crate::words::{BINARY, PruneArgs};
use crate::{home_of, placement_of};

/// `cargo`: Cargo's own code, 75 where the turn did not come. A command that tests, checks or lints is placed
/// first where the machine's placement sends it elsewhere (docs/testbed.md, 「A run on another machine」), and
/// runs there as this same verb of that machine's testbed.
pub fn cargo(words: &[String]) -> u8 {
  let here = std::env::current_dir().unwrap_or_default();
  let root = root_of(&here).unwrap_or(here);
  let env = process_env();

  if cargo_placeable(words) && !runs_here(&env) {
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

// What the holders of what the enforcement left say, each with about how much disk it holds.
fn holders_said(enforcement: &Enforcement) -> String {
  let said = &VERBS.prune;

  if enforcement.holders.is_empty() {
    return said.no_holder.clone();
  }

  enforcement
    .holders
    .iter()
    .map(|holder| {
      fill_marks(
        &said.holder,
        &[
          ("path", &holder.path.to_string_lossy()),
          ("by", holder.by.words()),
          ("size", &gigabytes(holder.bytes)),
        ],
      )
    })
    .collect::<Vec<_>>()
    .join("; ")
}

// What the enforcement of the ceiling says: the ceiling and how it was computed, what the directory held
// against it, and where it was over, what each rule removed, or would, and what holds what is left.
fn said_of_ceiling(enforcement: &Enforcement, verb: &str) -> Vec<String> {
  let said = &VERBS.prune;
  let ceiling = &enforcement.ceiling;
  let shown = gigabytes(ceiling.bytes);
  let set = fill_marks(
    if ceiling.setting.named { &said.ceiling_named } else { &said.ceiling_policy },
    &[("set", &gigabytes(ceiling.setting.bytes))],
  );
  let fits = fill_marks(
    if ceiling.setting.bytes <= ceiling.fits {
      &said.ceiling_within
    } else {
      &said.ceiling_lowered
    },
    &[("fits", &gigabytes(ceiling.fits)), ("room", &gigabytes(ceiling.room))],
  );
  let mut lines = vec![
    fill_marks(&said.ceiling, &[("ceiling", &shown), ("set", &set), ("fits", &fits)]),
    fill_marks(&said.held, &[("held", &gigabytes(enforcement.before)), ("ceiling", &shown)]),
  ];

  if enforcement.rules.is_empty() {
    return lines;
  }

  lines.push(fill_marks(
    &said.enforce_heading,
    &[("over", &gigabytes(enforcement.before.saturating_sub(ceiling.bytes))), ("verb", verb)],
  ));

  for rule in &enforcement.rules {
    let named = if rule.names.is_empty() {
      String::new()
    } else {
      fill_marks(&said.named, &[("names", &rule.names.join(", "))])
    };

    lines.push(fill_marks(
      &said.enforced,
      &[
        ("count", &rule.count.to_string()),
        ("rule", rule.words),
        ("named", &named),
        ("size", &gigabytes(rule.bytes)),
      ],
    ));
  }

  lines.push(fill_marks(
    &said.enforced_held,
    &[
      ("held", &gigabytes(enforcement.before)),
      ("after", &gigabytes(enforcement.after)),
      ("ceiling", &shown),
    ],
  ));

  if enforcement.over() {
    lines.push(fill_marks(
      &said.still_over,
      &[
        ("over", &gigabytes(enforcement.after - ceiling.bytes)),
        ("holders", &holders_said(enforcement)),
      ],
    ));
  }

  lines
}

// The mark at `marked` of the last time a warning went out taken away, the state it warns of gone, so the warning
// goes out at once the next time that state comes.
fn forget_sent(marked: &Path) {
  // A mark that stands nowhere is the state this asks for.
  let _ = std::fs::remove_file(marked);
}

// Whether a warning whose last sending the mark at `marked` records goes out at `now` for a state that stands: where
// no mark says it went out since the state last went, or the mark is the prune policy's due span old or more. The
// mark is written where it goes out.
fn sent_due(marked: &Path, now: i64) -> bool {
  if marked_at(marked).is_some_and(|sent| now - sent < PRUNE.due_hours * HOUR_MS) {
    return false;
  }

  mark(marked);
  true
}

// `cargo-target-over-ceiling` sent to the hook where the enforcement left the directory at `root` over its
// ceiling and the event is due, naming what holds what is left; its mark taken away where the enforcement left
// the directory within its ceiling.
fn send_over(root: &Path, enforcement: &Enforcement, hook: &Hook) {
  let marked = root.join(OVER_SENT_MARK_NAME);

  if !enforcement.over() {
    forget_sent(&marked);
    return;
  }

  if !sent_due(&marked, now_ms()) {
    return;
  }

  let ceiling = enforcement.ceiling.bytes;

  hook.send(&Event {
    name: "cargo-target-over-ceiling",
    commit: None,
    summary: fill_marks(
      &VERBS.prune.over,
      &[
        ("root", &root.to_string_lossy()),
        ("held", &gigabytes(enforcement.after)),
        ("ceiling", &gigabytes(ceiling)),
        ("holders", &holders_said(enforcement)),
      ],
    ),
    detail: json!({
      "bytes": enforcement.after,
      "ceiling_bytes": ceiling,
      "holders": enforcement
        .holders
        .iter()
        .map(|holder| json!({ "path": holder.path, "by": holder.by.code(), "bytes": holder.bytes }))
        .collect::<Vec<_>>(),
    }),
  });
}

/// The ceiling a prune holds the shared build directory to: the size it is set to, and how the free space of the
/// filesystem holding the directory is read, since the ceiling is never more than what leaves the policy's room
/// free there (docs/deploy.md, 「The Cargo build directory」).
#[derive(Clone, Copy)]
pub struct CeilingAsk<'a> {
  pub setting: CeilingSetting,
  /// What the filesystem holding a path has free for this account, in bytes.
  pub free: &'a dyn Fn(&Path) -> u64,
}

impl CeilingAsk<'static> {
  /// The ceiling the environment sets, against the free space the disk itself answers.
  pub fn of(env: &Env) -> Self {
    Self {
      setting: CeilingSetting::of(env),
      free: &disk_free_bytes,
    }
  }
}

// The shared build directory pruned at `root` and, unless it is cleared whole, its ceiling enforced: what it
// says, a refusal where its disk keeps no access times.
fn prune_shared(
  args: &PruneArgs,
  root: &Path,
  (days, hours): (f64, f64),
  ceiling: CeilingAsk<'_>,
  deployments: &Deployments,
  hook: &Hook,
) -> Said {
  let CeilingAsk {
    setting,
    free: free_of,
  } = ceiling;
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

  // What the prune removes at `now` while the deployments `building` names run, which are read once the prune
  // holds the build locks, so a deployment that began while it waited on them is among them.
  let plan_at = |now: i64, building: &[PathBuf]| {
    if args.all {
      plan_clear(root)
    } else {
      plan_prune(root, (days * DAY_MS as f64) as i64, now, (hours * HOUR_MS as f64) as i64, building)
    }
  };

  // What the enforcement of the ceiling may remove once the plan made at `now` is carried out, read before anything
  // is: the directory's holding then is what it holds now less what the plan takes, and its filesystem's room the
  // same.
  let ceiling_plan = |planned: &PrunePlan, now: i64| {
    let held = directory_bytes(root).saturating_sub(planned.bytes);
    let free = free_of(root).saturating_add(planned.bytes);

    CeilingPlan::of(root, &planned.paths(), Ceiling::of(setting, held, free), held, now)
  };

  if args.dry_run {
    let now = now_ms();
    let building = deployments.building();
    let planned = plan_at(now, &building);
    let mut out = said_of(root, args, days, hours, &planned, &said.dry);

    // The enforcement plans as though it held every build lock the prune would wait for.
    if !args.all {
      let keepers = Keepers::read(root, None, &building);

      out.extend(said_of_ceiling(&ceiling_plan(&planned, now).enforce(&keepers, false), &said.dry));
    }

    return Said {
      out,
      ..Said::default()
    };
  }

  let locks = take_prune_locks(root, LANES.numbers.notice_seconds as f64, &|profile| {
    eprintln!("{}", fill_marks(&said.locked, &[("profile", &profile.to_string_lossy())]));
  });
  let before = free_of(root);
  let now = now_ms();
  let building = deployments.building();
  let planned = plan_at(now, &building);
  let enforcing = (!args.all).then(|| ceiling_plan(&planned, now));

  planned.apply();

  let enforcement =
    enforcing.map(|enforcing| enforcing.enforce(&Keepers::read(root, Some(&locks), &building), true));

  drop(locks);

  let mut answer = Said {
    out: said_of(root, args, days, hours, &planned, &said.removed),
    ..Said::default()
  };

  answer.out.push(fill_marks(
    &said.free,
    &[("before", &gigabytes(before)), ("after", &gigabytes(free_of(root)))],
  ));

  if let Some(enforcement) = enforcement {
    answer.out.extend(said_of_ceiling(&enforcement, &said.removed));
    send_over(root, &enforcement, hook);
  }

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

  /// The checkouts and build trees the deployments that are running build in.
  pub fn building(&self) -> Vec<PathBuf> {
    self
      .places
      .iter()
      .filter(|places| deployment_running(&places.progress))
      .flat_map(|places| [places.source.clone(), expansion_tree(&places.deploy_dir)])
      .collect()
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

/// The filesystem holding `root` trimmed with the command `trim`, `root` handed as its last word (docs/deploy.md,
/// 「The Cargo build directory」): the line saying what the command said, and, where it did not run or failed, a
/// `disk-trim-failed` event to the hook; nothing where `trim` names no command.
pub fn trim_filesystem(root: &Path, trim: &[String], hook: &Hook) -> Option<String> {
  let said = &VERBS.prune;
  let shown = root.to_string_lossy();
  let (program, words) = trim.split_first()?;
  let ran = Command::new(program).args(words).arg(root).stdin(Stdio::null()).output();
  let failure = match &ran {
    Ok(done) if done.status.success() => {
      let told = String::from_utf8_lossy(&done.stdout).trim().to_owned();

      return Some(fill_marks(&said.trimmed, &[("path", &shown), ("said", &told)]));
    }
    Ok(done) => {
      let told = String::from_utf8_lossy(&done.stderr).trim().to_owned();

      if told.is_empty() { done.status.to_string() } else { told }
    }
    Err(why) => why.to_string(),
  };

  hook.send(&Event {
    name: "disk-trim-failed",
    commit: None,
    summary: fill_marks(&said.trim_failed_summary, &[("path", &shown), ("said", &failure)]),
    detail: json!({ "path": shown, "said": failure }),
  });

  Some(fill_marks(&said.trim_failed, &[("path", &shown), ("said", &failure)]))
}

// The filesystem holding `root` trimmed with the command `trim` where its last trim began the prune policy's span
// ago or more, none is recorded or `args` clears the whole directory, its mark written as the trim begins; the line
// saying when it was last trimmed otherwise.
fn trim_when_due(args: &PruneArgs, root: &Path, trim: &[String], hook: &Hook) -> Option<String> {
  let marked = root.join(TRIMMED_MARK_NAME);
  let since = marked_at(&marked).map(|began| now_ms() - began);

  if !args.all
    && let Some(since) = since
    && since < PRUNE.trim_hours * HOUR_MS
  {
    return Some(fill_marks(
      &VERBS.prune.trim_waits,
      &[
        ("path", &root.to_string_lossy()),
        ("minutes", &(since / MINUTE_MS).to_string()),
        ("hours", &PRUNE.trim_hours.to_string()),
      ],
    ));
  }

  mark(&marked);
  trim_filesystem(root, trim, hook)
}

/// Where `args` asks for a prune only where one is due and none is at `now` (docs/deploy.md, 「The Cargo build
/// directory」): the line saying what the directory at `root` holds of its ceiling under `ceiling` and how long ago
/// it was last pruned. None where the prune is to run: it was not asked to wait for its due, the directory does not
/// exist or holds no mark of a prune, its last prune ended the prune policy's span ago or more, or the directory or
/// its deployments' checkouts hold more than their ceilings.
pub fn not_due(args: &PruneArgs, root: &Path, ceiling: CeilingAsk<'_>, now: i64) -> Option<String> {
  let CeilingAsk { setting, free } = ceiling;

  if !args.when_due || !root.exists() {
    return None;
  }

  let since = now - marked_at(&root.join(PRUNED_MARK_NAME))?;

  if since >= PRUNE.due_hours * HOUR_MS {
    return None;
  }

  let held = directory_bytes(root);
  let ceiling = Ceiling::of(setting, held, free(root));
  let shown = gigabytes(ceiling.bytes);
  // The deployments' checkouts are read only where the machine sets them a ceiling of their own.
  let over = held > ceiling.bytes
    || (setting.deployments.is_some() && CeilingPlan::of(root, &[], ceiling, held, now).is_over());

  (!over).then(|| {
    fill_marks(
      &VERBS.prune.not_due,
      &[
        ("root", &root.to_string_lossy()),
        ("held", &gigabytes(held)),
        ("ceiling", &shown),
        ("minutes", &(since / MINUTE_MS).to_string()),
      ],
    )
  })
}

/// `prune` over the build directory at `root`, its ceiling set to `ceiling`, and the build trees of `deployments`,
/// then, unless it was a dry run, its mark written and the filesystem holding `root` trimmed with the command `trim`
/// where its trim is due: what it says and the code it ends with.
pub fn prune_in(
  args: &PruneArgs,
  root: &Path,
  deployments: &Deployments,
  trim: &[String],
  ceiling: CeilingAsk<'_>,
  hook: &Hook,
) -> Said {
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

  let mut answer = prune_shared(args, root, (days, hours), ceiling, deployments, hook);

  if answer.code == REFUSED_EXIT_CODE {
    return answer;
  }

  answer.out.extend(prune_build_targets(args, deployments, now_ms()));

  if !args.dry_run && root.exists() {
    mark(&root.join(PRUNED_MARK_NAME));
    answer.out.extend(trim_when_due(args, root, trim, hook));
  }

  answer
}

/// The placed machine's free disk read against its floor (docs/deploy.md, 「The Cargo build directory」): what
/// the prune says of it, and, where `send` holds, a `placed-disk-low` event to the hook naming each disk there
/// below the floor where the event is due by its mark at the build root `root`, the mark taken away where every
/// disk there read and none stands below.
pub fn placed_disk(machine: &MachinePlacement, root: &Path, send: bool, hook: &Hook) -> Vec<String> {
  let said = &VERBS.prune;
  let label = machine.settings.label();
  let floor = machine.settings.disk_floor_bytes();
  let mut lines =
    vec![fill_marks(&said.placed_heading, &[("machine", label), ("floor", &gigabytes(floor))])];
  let readings = match machine.disk_free() {
    Ok(readings) => readings,
    Err(why) => {
      lines.push(fill_marks(&said.placed_unreached, &[("said", &why)]));
      return lines;
    }
  };
  let mut low: Vec<(String, FarDisk)> = Vec::new();
  let mut every_read = true;

  for (path, disk) in readings {
    let Some(disk) = disk else {
      lines.push(fill_marks(&said.placed_unread, &[("path", &path)]));
      every_read = false;
      continue;
    };
    let shown = [
      ("path", path.as_str()),
      ("source", disk.source.as_str()),
      ("free", &gigabytes(disk.free_bytes)),
    ];

    if disk.free_bytes < floor {
      lines.push(fill_marks(&said.placed_low, &shown));
      low.push((path, disk));
    } else {
      lines.push(fill_marks(&said.placed_free, &shown));
    }
  }

  let marked = root.join(PLACED_LOW_SENT_MARK_NAME);

  if send && low.is_empty() && every_read {
    forget_sent(&marked);
  }

  if send && !low.is_empty() && sent_due(&marked, now_ms()) {
    let disks = low
      .iter()
      .map(|(path, disk)| {
        fill_marks(
          &said.placed_disk,
          &[("path", path), ("source", &disk.source), ("free", &gigabytes(disk.free_bytes))],
        )
      })
      .collect::<Vec<_>>()
      .join(", ");

    hook.send(&Event {
      name: "placed-disk-low",
      commit: None,
      summary: fill_marks(
        &said.placed_low_summary,
        &[("machine", label), ("floor", &gigabytes(floor)), ("disks", &disks)],
      ),
      detail: json!({
        "machine": label,
        "floor_bytes": floor,
        "disks": low
          .iter()
          .map(|(path, disk)| json!({ "path": path, "source": disk.source, "free_bytes": disk.free_bytes }))
          .collect::<Vec<_>>(),
      }),
    });
  }

  lines
}

/// `prune` over the shared build directory and the build trees of the deployments the machine's registry
/// names where it is due, the filesystem holding the build directory trimmed with the prune policy's command
/// where its trim is due, then the placed machine's free disk read where the settings name one, its events to the
/// hook the environment or the repository it is run in names.
pub fn prune(args: &PruneArgs) -> u8 {
  let env = process_env();
  let root = cargo_target_root();
  let ceiling = CeilingAsk::of(&env);

  if let Some(line) = not_due(args, &root, ceiling, now_ms()) {
    forget_sent(&root.join(OVER_SENT_MARK_NAME));

    return Said {
      out: vec![line],
      ..Said::default()
    }
    .print();
  }

  let repo = Repository::read(&std::env::current_dir().unwrap_or_default()).ok();
  let deployments = Deployments::of_registry(&registry_path(&data_root(), &env), &env);
  let hook = Hook::of(repo.as_ref(), &env);
  let mut answer = prune_in(args, &root, &deployments, &PRUNE.trim_command, ceiling, &hook);

  if !args.all
    && let Some(machine) = MachinePlacement::named(&data_root(), &home_of(&env))
  {
    answer.out.extend(placed_disk(&machine, &root, !args.dry_run, &hook));
  }

  answer.print()
}
