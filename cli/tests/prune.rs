//! The `prune` verb (docs/deploy.md, 「The Cargo build directory」): its spans read in days and hours, a clear
//! taking none, and a span too short to tell refused before anything is read; and the deployments' build trees
//! found through the registry of checkouts, whose target directories go once they outlived their span; and the
//! filesystem holding the build directory trimmed after a prune that is not a dry run.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use bmo_testbed_cli::cargo::{Deployments, prune_in};
use bmo_testbed_cli::words::{PruneArgs, Verb, read};
use bmo_testbed_core::events::Hook;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use serde_json::Value;

use crate::stand::install;

fn prune_words(given: &[&str]) -> Result<PruneArgs, clap::Error> {
  let words = ["bmo-testbed", "prune"].iter().chain(given).map(OsString::from);

  read(words).map(|words| match words.verb {
    Verb::Prune(args) => args,
    other => panic!("read as {other:?}"),
  })
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
      prune_in(&args, &dir.path().join("nowhere"), &Deployments::default(), &[], &no_hook()).code,
      2,
      "{given:?}"
    );
  }

  let gone = prune_in(
    &prune_words(&[]).unwrap(),
    &dir.path().join("nowhere"),
    &Deployments::default(),
    &[],
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
  let dry = prune_in(&prune_words(&["--dry-run"]).unwrap(), &nowhere, &deployments, &[], &no_hook());

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

  let removed = prune_in(&prune_words(&[]).unwrap(), &nowhere, &deployments, &[], &no_hook());

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
    &no_hook(),
  );

  assert_eq!(dry.code, 0);
  assert!(!dir.path().join("trimmed").exists(), "{:?}", dry.out);

  let cleared =
    prune_in(&prune_words(&["--all"]).unwrap(), &root, &Deployments::default(), &trim, &no_hook());
  let shown = root.to_string_lossy();

  assert_eq!(cleared.code, 0);
  assert_eq!(std::fs::read_to_string(dir.path().join("trimmed")).unwrap(), format!("{shown}\n"));
  assert_eq!(
    cleared.out.last().unwrap(),
    &fill_marks(&VERBS.prune.trimmed, &[("path", &shown), ("said", "1 GiB trimmed")])
  );
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
  let pruned = prune_in(&prune_words(&["--all"]).unwrap(), &root, &Deployments::default(), &trim, &hook);
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
