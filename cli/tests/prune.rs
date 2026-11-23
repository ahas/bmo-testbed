//! The `prune` verb (docs/deploy.md, 「The Cargo build directory」): its spans read in days and hours, a clear
//! taking none, and a span too short to tell refused before anything is read; and the deployments' build trees
//! found through the registry of checkouts, whose target directories go once they outlived their span; and the
//! filesystem holding the build directory trimmed after a prune that is not a dry run where its trim is due; a
//! timed prune run only where one is due; and its ceiling enforced, what a running deployment builds in kept and
//! named in the event a directory left over sends.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use bmo_testbed_cli::cargo::{Deployments, not_due, prune_in};
use bmo_testbed_cli::words::{PruneArgs, Verb, read};
use bmo_testbed_core::events::Hook;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::queue::now_ms;
use bmo_testbed_core::release_target::mark_release_target;
use bmo_testbed_deploy::progress::{DeployStart, Outcome, begin_deploy};
use bmo_testbed_suite::ceiling::CeilingSetting;
use bmo_testbed_suite::crates::{CHECKOUT_RECORD_NAME, checkout_target_name};
use bmo_testbed_suite::prune::HOUR_MS;
use bmo_testbed_suite::superseded::CARGO_BUILD_LOCK_NAME;
use serde_json::Value;

use crate::stand::install;

fn prune_words(given: &[&str]) -> Result<PruneArgs, clap::Error> {
  let words = ["bmo-testbed", "prune"].iter().chain(given).map(OsString::from);

  read(words).map(|words| match words.verb {
    Verb::Prune(args) => args,
    other => panic!("read as {other:?}"),
  })
}

// The ceiling the policy sets, which no directory a test makes reaches.
fn policy_ceiling() -> CeilingSetting {
  CeilingSetting::of(&HashMap::new())
}

// A ceiling of nothing, which every directory a test makes stands over.
fn no_room() -> CeilingSetting {
  CeilingSetting {
    bytes: 0,
    named: true,
    deployments: None,
  }
}

fn no_hook() -> Hook {
  Hook {
    command: None,
    time: std::time::Duration::from_secs(60),
    repository: None,
    root: None,
  }
}

#[test]
fn a_clear_takes_no_span_and_a_prune_reads_its_spans_in_days_and_hours() {
  let spans = prune_words(&["--older-than", "3", "--incremental-older-than", "1.5", "--dry-run"]).unwrap();

  assert_eq!(
    (spans.older_than, spans.incremental_older_than, spans.dry_run),
    (Some(3.0), Some(1.5), true)
  );
  assert!(prune_words(&["--all", "--older-than", "3"]).is_err());
  assert!(prune_words(&["--all", "--incremental-older-than", "3"]).is_err());
  assert!(prune_words(&["stray"]).is_err());
}

#[test]
fn a_span_of_a_day_or_less_and_no_incremental_span_are_refused_before_anything_is_read() {
  let dir = tempfile::tempdir().unwrap();

  for given in [&["--older-than", "1"][..], &["--incremental-older-than", "0"]] {
    let args = prune_words(given).unwrap();

    assert_eq!(
      prune_in(
        &args,
        &dir.path().join("nowhere"),
        &Deployments::default(),
        &[],
        policy_ceiling(),
        &no_hook()
      )
      .code,
      2,
      "{given:?}"
    );
  }

  let gone = prune_in(
    &prune_words(&[]).unwrap(),
    &dir.path().join("nowhere"),
    &Deployments::default(),
    &[],
    policy_ceiling(),
    &no_hook(),
  );

  assert_eq!(gone.code, 0);
  assert!(gone.out[0].contains("nothing to remove"), "{:?}", gone.out);
}

fn write(path: &Path, text: &str) {
  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
}

// A checkout named `name` under `root` whose declaration deploys to `root/<name>-deployed`, last deployed at
// `at`, its build tree's target directory built.
fn deployed_checkout(root: &Path, name: &str, at: &str) {
  let deploy_root = root.join(format!("{name}-deployed"));

  write(
    &root.join(name).join("testbed.toml"),
    &format!(
      "testbed = 0\nname = \"{name}\"\n\n[deploy]\nroot = \"{}\"\ndata_root = \"{}\"\n",
      deploy_root.display(),
      root.join(format!("{name}-data")).display()
    ),
  );
  write(
    &deploy_root.join("deployed.json"),
    &format!(r#"{{ "commit": "c0ffee", "subject": "a change", "at": "{at}", "backup": "" }}"#),
  );
  write(&deploy_root.join("deploy-expand/target/release/app"), "built");
}

// "The same prune takes the deployments' build trees' target directories that have outlived their span. It finds
// the deploy roots through the machine's registry of checkouts ... a deploy root several checkouts name is read
// once. A registry that does not read is said and passed over"
#[test]
fn the_build_trees_of_the_registry_s_deployments_go_once_their_last_deployment_is_old_enough() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path();
  let env = HashMap::new();

  deployed_checkout(root, "old", "2020-01-01T00:00:00.000Z");
  deployed_checkout(root, "fresh", &chrono::Utc::now().to_rfc3339());
  write(&root.join("plain/testbed.toml"), "testbed = 0\nname = \"plain\"\n");
  write(
    &root.join("checkouts.toml"),
    "checkouts = [\"old\", \"old\", \"fresh\", \"plain\", \"missing\"]\n",
  );

  let deployments = Deployments::of_registry(&root.join("checkouts.toml"), &env);
  let roots: Vec<_> = deployments.places.iter().map(|places| places.deploy_root.clone()).collect();

  assert_eq!(roots, [root.join("old-deployed"), root.join("fresh-deployed")]);
  assert_eq!(deployments.unread, None);

  let old = root.join("old-deployed/deploy-expand/target");
  let fresh = root.join("fresh-deployed/deploy-expand/target");
  let nowhere = root.join("nowhere");
  let dry = prune_in(
    &prune_words(&["--dry-run"]).unwrap(),
    &nowhere,
    &deployments,
    &[],
    policy_ceiling(),
    &no_hook(),
  );

  assert_eq!(dry.code, 0);
  assert!(
    dry
      .out
      .iter()
      .any(|line| line.contains("deploy build trees") && line.contains("(nothing removed)")),
    "{:?}",
    dry.out
  );
  assert!(
    dry.out.iter().any(|line| line.contains("1 target directories (old-deployed)")),
    "{:?}",
    dry.out
  );
  assert!(old.is_dir() && fresh.is_dir());

  let removed =
    prune_in(&prune_words(&[]).unwrap(), &nowhere, &deployments, &[], policy_ceiling(), &no_hook());

  assert_eq!(removed.code, 0);
  assert!(!old.exists(), "{:?}", removed.out);
  assert!(old.parent().unwrap().is_dir());
  assert!(fresh.is_dir());

  write(&root.join("broken.toml"), "checkouts = 3\n");

  let unread = Deployments::of_registry(&root.join("broken.toml"), &env);

  assert!(unread.places.is_empty());
  assert!(unread.unread.unwrap().contains("does not read"));
}

// A command at `at` standing for the trim: it writes the path it is handed into `trimmed` beside it, then says
// `said` and ends with `code`, its words on standard output where it ends 0 and on standard error otherwise.
fn trim_stand_in(at: &Path, said: &str, code: u8) -> Vec<String> {
  let stream = if code == 0 { "" } else { " >&2" };

  install(
    &format!(
      "#!/bin/sh\necho \"$1\" >> '{trimmed}'\necho '{said}'{stream}\nexit {code}\n",
      trimmed = at.with_file_name("trimmed").display(),
    ),
    at,
  );

  vec![at.to_string_lossy().into_owned()]
}

// "The filesystem is trimmed after every prune that removes ... the prune runs the prune policy's trim command
// ... on the build root"; "--dry-run ... trims nothing; --all ... trims after it"
#[test]
fn a_prune_trims_the_filesystem_holding_its_build_root_and_a_dry_run_trims_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path().join("target");
  let trim = trim_stand_in(&dir.path().join("trim"), "1 GiB trimmed", 0);

  write(&root.join("debug/deps/unit-0123456789abcdef.rlib"), "built");

  let dry = prune_in(
    &prune_words(&["--all", "--dry-run"]).unwrap(),
    &root,
    &Deployments::default(),
    &trim,
    policy_ceiling(),
    &no_hook(),
  );

  assert_eq!(dry.code, 0);
  assert!(!dir.path().join("trimmed").exists(), "{:?}", dry.out);

  let cleared = prune_in(
    &prune_words(&["--all"]).unwrap(),
    &root,
    &Deployments::default(),
    &trim,
    policy_ceiling(),
    &no_hook(),
  );
  let shown = root.to_string_lossy();

  assert_eq!(cleared.code, 0);
  assert_eq!(std::fs::read_to_string(dir.path().join("trimmed")).unwrap(), format!("{shown}\n"));
  assert_eq!(
    cleared.out.last().unwrap(),
    &fill_marks(&VERBS.prune.trimmed, &[("path", &shown), ("said", "1 GiB trimmed")])
  );

  // "where the prune policy's trim span ... has passed since the last trim began, or no trim is recorded, and
  // after every --all. Otherwise the prune says on one line when the filesystem was last trimmed."
  let soon = prune_in(
    &prune_words(&[]).unwrap(),
    &root,
    &Deployments::default(),
    &trim,
    policy_ceiling(),
    &no_hook(),
  );

  assert_eq!(soon.code, 0);
  assert_eq!(std::fs::read_to_string(dir.path().join("trimmed")).unwrap(), format!("{shown}\n"));
  assert_eq!(
    soon.out.last().unwrap(),
    &fill_marks(
      &VERBS.prune.trim_waits,
      &[("path", &shown), ("minutes", "0"), ("hours", &PRUNE.trim_hours.to_string())]
    )
  );

  prune_in(
    &prune_words(&["--all"]).unwrap(),
    &root,
    &Deployments::default(),
    &trim,
    policy_ceiling(),
    &no_hook(),
  );

  assert_eq!(
    std::fs::read_to_string(dir.path().join("trimmed")).unwrap(),
    format!("{shown}\n{shown}\n")
  );
}

// "The timer runs prune --when-due ... prunes only where the directory holds more than its ceiling ... or the
// last prune ended at least the prune policy's span ago ... A build root holding no mark is due."
#[test]
fn a_prune_waiting_for_its_due_runs_only_over_its_ceiling_or_once_its_span_has_passed() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path().join("target");
  let when_due = prune_words(&["--when-due"]).unwrap();

  std::fs::create_dir_all(&root).unwrap();

  assert_eq!(not_due(&when_due, &root, policy_ceiling(), now_ms()), None, "no mark of a prune is due");

  prune_in(&when_due, &root, &Deployments::default(), &[], policy_ceiling(), &no_hook());

  let now = now_ms();
  let shown = root.to_string_lossy();
  let waits =
    not_due(&when_due, &root, policy_ceiling(), now).expect("an empty directory just pruned waits");

  assert!(waits.starts_with(&format!("prune: {shown} holds 0.0 GB of its ceiling ")), "{waits}");
  assert_eq!(not_due(&prune_words(&[]).unwrap(), &root, policy_ceiling(), now), None, "run by hand");
  assert_eq!(
    not_due(&when_due, &root, policy_ceiling(), now + PRUNE.due_hours * HOUR_MS),
    None,
    "span passed"
  );

  write(&root.join("debug/deps/unit-0123456789abcdef.rlib"), "built");

  assert_eq!(not_due(&when_due, &root, no_room(), now), None, "over its ceiling");
  assert!(prune_words(&["--when-due", "--all"]).is_err());
}

// "Where sudo refuses, or the disk takes no trim, the prune says what was said on a line and sends a
// disk-trim-failed event naming the build root; the prune's outcome does not change."
#[test]
fn a_trim_that_is_refused_is_said_and_sends_disk_trim_failed_and_the_prune_still_succeeds() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path().join("target");
  let refused = "sudo: a password is required";
  let trim = trim_stand_in(&dir.path().join("trim"), refused, 1);
  let hook_script = dir.path().join("hook");

  std::fs::create_dir_all(&root).unwrap();
  install(
    &format!(
      "#!/bin/sh\necho \"$1\" >> '{names}'\ncat >> '{payloads}'\n",
      names = dir.path().join("events").display(),
      payloads = dir.path().join("payloads").display(),
    ),
    &hook_script,
  );

  let hook = Hook {
    command: Some(hook_script.to_string_lossy().into_owned()),
    ..no_hook()
  };
  let pruned = prune_in(
    &prune_words(&["--all"]).unwrap(),
    &root,
    &Deployments::default(),
    &trim,
    policy_ceiling(),
    &hook,
  );
  let shown = root.to_string_lossy();
  let payload: Value =
    serde_json::from_str(&std::fs::read_to_string(dir.path().join("payloads")).unwrap()).unwrap();

  assert_eq!(pruned.code, 0);
  assert_eq!(
    pruned.out.last().unwrap(),
    &fill_marks(&VERBS.prune.trim_failed, &[("path", &shown), ("said", refused)])
  );
  assert_eq!(std::fs::read_to_string(dir.path().join("events")).unwrap(), "disk-trim-failed\n");
  assert_eq!(payload["detail"]["path"], shown.as_ref());
  assert_eq!(payload["detail"]["said"], refused);
}

// A hook at `dir/hook` writing each event's name into `dir/events` and its payload into `dir/payloads`.
fn recording_hook(dir: &Path) -> Hook {
  let script = dir.join("hook");

  install(
    &format!(
      "#!/bin/sh\necho \"$1\" >> '{names}'\ncat >> '{payloads}'\n",
      names = dir.join("events").display(),
      payloads = dir.join("payloads").display(),
    ),
    &script,
  );

  Hook {
    command: Some(script.to_string_lossy().into_owned()),
    ..no_hook()
  }
}

// A unit Cargo names by its hash in the profile directory, with the profile's build lock beside it.
fn built_unit(profile: &Path, name: &str) -> std::path::PathBuf {
  let library = profile.join("deps").join(format!("lib{name}-0123456789abcdef.rlib"));

  write(&library, &"built ".repeat(1000));
  write(&profile.join(".fingerprint").join(format!("{name}-0123456789abcdef")).join("lib"), "0");
  write(&profile.join(CARGO_BUILD_LOCK_NAME), "");
  library
}

// "the directory of the checkout or the build tree of a deployment of the machine's registry that is running" is
// never removed; "A timed prune that leaves the directory over its ceiling sends a cargo-target-over-ceiling
// event ... naming what holds what is left"; "A dry run sends nothing."
#[test]
fn over_its_ceiling_a_prune_keeps_what_a_running_deployment_builds_in_and_names_it_in_the_event() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path();
  let target = root.join("target");
  let env = HashMap::new();

  deployed_checkout(root, "app", &chrono::Utc::now().to_rfc3339());
  write(&root.join("checkouts.toml"), "checkouts = [\"app\"]\n");

  let deployments = Deployments::of_registry(&root.join("checkouts.toml"), &env);
  let shared = built_unit(&target.join("debug"), "shared");
  let building = target.join("checkouts").join(checkout_target_name(&root.join("app")));
  let deploying = built_unit(&building.join("debug"), "app");
  let hook = recording_hook(root);
  let running = DeployStart {
    unit: None,
    pid: i64::from(std::process::id()),
    commit: None,
    plan: Vec::new(),
    now: 0,
    repository: None,
    record: None,
  };
  let _run = begin_deploy(&deployments.places[0].progress, &running, || 0);
  let dry = prune_in(&prune_words(&["--dry-run"]).unwrap(), &target, &deployments, &[], no_room(), &hook);

  assert_eq!(dry.code, 0);
  assert!(shared.exists() && deploying.exists());
  assert!(!root.join("events").exists());
  assert!(
    dry
      .out
      .iter()
      .any(|line| line.contains("over its ceiling") && line.contains("(nothing removed)")),
    "{:?}",
    dry.out
  );
  assert!(dry.out.iter().any(|line| line.contains("1 shared units")), "{:?}", dry.out);

  let pruned = prune_in(&prune_words(&[]).unwrap(), &target, &deployments, &[], no_room(), &hook);
  let payload: Value =
    serde_json::from_str(&std::fs::read_to_string(root.join("payloads")).unwrap()).unwrap();

  assert_eq!(pruned.code, 0);
  assert!(!shared.exists() && deploying.exists(), "{:?}", pruned.out);
  assert!(building.join("debug").join(CARGO_BUILD_LOCK_NAME).exists());
  assert_eq!(std::fs::read_to_string(root.join("events")).unwrap(), "cargo-target-over-ceiling\n");
  assert_eq!(payload["detail"]["ceiling_bytes"], 0);
  assert_eq!(payload["detail"]["holders"][0]["path"], building.to_string_lossy().as_ref());
  assert_eq!(payload["detail"]["holders"][0]["by"], "deployment");
  assert!(payload["detail"]["holders"][0]["bytes"].as_u64().unwrap() > 0);
}

// A checkout at `checkout` the Cargo verb has built in under the build root at `target`, as its record there names
// it, its own `target/` holding the binary given.
fn recorded_checkout(target: &Path, checkout: &Path, binary: &str) -> std::path::PathBuf {
  let built = checkout.join("target/release").join(binary);

  write(&built, "built");
  write(
    &target.join("checkouts").join(checkout_target_name(checkout)).join(CHECKOUT_RECORD_NAME),
    &checkout.to_string_lossy(),
  );
  built
}

// "The build tree's target directory carries the release build's mark ... so only the mark keeps that window: from
// the start of the build until the binaries are placed, and past it until the span above, no rule of the prune
// takes the target directory"; "nor a deployment's build tree's target/, which carries the release build's mark":
// a deployment between its release build's end and its binaries' placement, its declared steps running and no
// Cargo process in the build tree, keeps the binary it is about to place through a prune over its ceiling, while
// an unmarked checkout's own target/ beside it goes as the stray it is.
#[test]
fn over_its_ceiling_a_prune_keeps_the_release_build_a_running_deployment_has_yet_to_place() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path();
  let target = root.join("target");
  let env = HashMap::new();

  deployed_checkout(root, "app", &chrono::Utc::now().to_rfc3339());
  write(&root.join("checkouts.toml"), "checkouts = [\"app\"]\n");

  let deployments = Deployments::of_registry(&root.join("checkouts.toml"), &env);
  let build_tree = root.join("app-deployed/deploy-expand");
  let release_build = recorded_checkout(&target, &build_tree, "app");
  let stray = recorded_checkout(&target, &root.join("hand"), "hand");
  let shared = built_unit(&target.join("debug"), "shared");
  let running = DeployStart {
    unit: None,
    pid: i64::from(std::process::id()),
    commit: None,
    plan: Vec::new(),
    now: 0,
    repository: None,
    record: None,
  };

  mark_release_target(&build_tree.join("target")).unwrap();

  let run = begin_deploy(&deployments.places[0].progress, &running, || 0);
  let pruned = prune_in(&prune_words(&[]).unwrap(), &target, &deployments, &[], no_room(), &no_hook());

  assert_eq!(pruned.code, 0);
  assert!(!shared.exists() && !stray.exists(), "{:?}", pruned.out);
  assert_eq!(std::fs::read_to_string(&release_build).unwrap(), "built", "{:?}", pruned.out);

  // Once the deployment has ended the mark still keeps it, the build tree's own span the one that takes it.
  run.end(Outcome::Live, None);

  let after = prune_in(&prune_words(&[]).unwrap(), &target, &deployments, &[], no_room(), &no_hook());

  assert_eq!(after.code, 0);
  assert!(release_build.exists(), "{:?}", after.out);
}
