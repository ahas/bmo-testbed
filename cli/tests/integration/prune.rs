//! The `prune` verb (docs/deploy.md, 「The Cargo build directory」): its spans read in days and hours, a clear
//! taking none, and a span too short to tell refused before anything is read.

use std::ffi::OsString;
use std::time::Duration;

use bmo_testbed_cli::cargo::prune_in;
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
    timeout: Duration::from_secs(1),
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

    assert_eq!(prune_in(&args, &dir.path().join("nowhere"), &no_hook()).code, 2, "{given:?}");
  }

  let gone = prune_in(&prune_words(&[]).unwrap(), &dir.path().join("nowhere"), &no_hook());

  assert_eq!(gone.code, 0);
  assert!(gone.out[0].contains("nothing to remove"), "{:?}", gone.out);
}
