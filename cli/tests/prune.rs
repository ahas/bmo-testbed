//! The `prune` verb (docs/deploy.md, 「The Cargo build directory」): its spans read in days and hours, a clear
//! taking none, and a span too short to tell refused before anything is read; and the deployments' build trees
//! found through the registry of checkouts, whose target directories go once they outlived their span.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use bmo_testbed_cli::cargo::{Deployments, prune_in};
use bmo_testbed_cli::words::{PruneArgs, Verb, read};
use bmo_testbed_core::events::Hook;

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
      prune_in(&args, &dir.path().join("nowhere"), &Deployments::default(), &no_hook()).code,
      2,
      "{given:?}"
    );
  }

  let gone = prune_in(
    &prune_words(&[]).unwrap(),
    &dir.path().join("nowhere"),
    &Deployments::default(),
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
  let dry = prune_in(&prune_words(&["--dry-run"]).unwrap(), &nowhere, &deployments, &no_hook());

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

  let removed = prune_in(&prune_words(&[]).unwrap(), &nowhere, &deployments, &no_hook());

  assert_eq!(removed.code, 0);
  assert!(!old.exists(), "{:?}", removed.out);
  assert!(old.parent().unwrap().is_dir());
  assert!(fresh.is_dir());

  write(&root.join("broken.toml"), "checkouts = 3\n");

  let unread = Deployments::of_registry(&root.join("broken.toml"), &env);

  assert!(unread.places.is_empty());
  assert!(unread.unread.unwrap().contains("does not read"));
}
