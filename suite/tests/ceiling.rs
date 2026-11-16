// The ceiling of the shared Cargo build directory and its enforcement (docs/deploy.md, 「The Cargo build
// directory」), over a build directory shaped as Cargo leaves it and dated by hand: shared units at the build root,
// two live checkouts' directories with units of their own, incremental state, a binary handed back and Cargo's
// lock files.

use std::collections::HashMap;
use std::fs::{File, FileTimes};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_suite::ceiling::{
  CEILING_VARIABLE, Ceiling, CeilingPlan, CeilingSetting, DEPLOYMENT_CEILING_VARIABLE, Enforcement, HeldBy,
  Holder, Keepers,
};
use bmo_testbed_suite::crates::{CHECKOUT_RECORD_NAME, DEPLOYMENT_MARK_NAME};
use bmo_testbed_suite::prune::{HOUR_MS, directory_bytes, disk_free_bytes, take_prune_locks};
use bmo_testbed_suite::superseded::CARGO_BUILD_LOCK_NAME;

const GIGABYTE: u64 = 1_000_000_000;

fn now() -> i64 {
  SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

// The file or directory at `path` read and changed at `at`.
fn date(path: &Path, at: i64) {
  let at = UNIX_EPOCH + Duration::from_millis(at as u64);

  File::open(path)
    .unwrap()
    .set_times(FileTimes::new().set_accessed(at).set_modified(at))
    .unwrap();
}

// A file of `size` bytes at `path`, read and changed at `at`.
fn write(path: &Path, size: usize, at: i64) {
  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, vec![b'u'; size]).unwrap();
  date(path, at);
}

// A unit in the profile directory read last at `at`: its fingerprint directory, dated with it, and its library.
fn unit(profile: &Path, index: u64, at: i64) -> Vec<PathBuf> {
  let hash = format!("{index:016x}");
  let fingerprint = profile.join(".fingerprint").join(format!("unit{index}-{hash}"));
  let library = profile.join("deps").join(format!("libunit{index}-{hash}.rlib"));

  write(&fingerprint.join(format!("lib-unit{index}")), 16, at);
  date(&fingerprint, at);
  write(&library, 64 * 1024, at);

  vec![fingerprint, library]
}

// A unit in the profile directory that is a library alone, read last at `at`.
fn library_alone(profile: &Path, index: u64, at: i64) -> Vec<PathBuf> {
  let library = profile.join("deps").join(format!("libunit{index}-{index:016x}.rlib"));

  write(&library, 64 * 1024, at);
  vec![library]
}

fn bytes_of(paths: &[PathBuf]) -> u64 {
  paths.iter().map(|path| directory_bytes(path)).sum()
}

fn all_exist(paths: &[PathBuf]) -> bool {
  paths.iter().all(|path| path.exists())
}

fn none_exist(paths: &[PathBuf]) -> bool {
  paths.iter().all(|path| !path.exists())
}

// A build directory: shared units read long ago and lately, one a live checkout holds a link to; checkout `a`
// with a unit read before every shared one, checkout `b` with one read lately; in each checkout's profile
// directory its incremental state, a binary handed back and Cargo's lock; and the build root's own lock.
struct Built {
  root: PathBuf,
  shared_old: Vec<PathBuf>,
  shared_new: Vec<PathBuf>,
  shared_linked: Vec<PathBuf>,
  a_unit: Vec<PathBuf>,
  b_unit: Vec<PathBuf>,
  a_rest: Vec<PathBuf>,
  b_rest: Vec<PathBuf>,
  locks: Vec<PathBuf>,
  records: Vec<PathBuf>,
}

fn built(scratch: &Path) -> Built {
  let now = now();
  let root = scratch.join("build");
  let shared = root.join("debug");
  let a = root.join("checkouts").join("a-0123456789ab");
  let b = root.join("checkouts").join("b-0123456789ab");
  let shared_linked = library_alone(&shared, 1, now - 90 * HOUR_MS);
  let shared_old = unit(&shared, 2, now - 60 * HOUR_MS);
  let shared_new = unit(&shared, 3, now - 2 * HOUR_MS);
  let a_unit = unit(&a.join("debug"), 4, now - 80 * HOUR_MS);
  let b_unit = unit(&b.join("debug"), 5, now - HOUR_MS);
  let rest = |checkout: &Path, at: i64| {
    let profile = checkout.join("debug");
    let incremental = profile.join("incremental").join("crate-0abc");

    write(&incremental.join("s-one").join("dep-graph.bin"), 4096, at);
    write(&profile.join("app"), 4096, at);
    vec![profile.join("incremental"), profile.join("app")]
  };
  let mut a_rest = rest(&a, now - 70 * HOUR_MS);
  let b_rest = rest(&b, now - 3 * HOUR_MS);
  let linked_in_a = a.join("debug").join("libunit1.rlib");

  std::fs::hard_link(&shared_linked[0], &linked_in_a).unwrap();
  date(&linked_in_a, now - 90 * HOUR_MS);
  a_rest.push(linked_in_a);

  let mut locks = Vec::new();
  let mut records = Vec::new();

  for profile in [&shared, &a.join("debug"), &b.join("debug")] {
    write(&profile.join(CARGO_BUILD_LOCK_NAME), 0, now);
    std::fs::create_dir_all(profile.join(format!("{CARGO_BUILD_LOCK_NAME}.queue"))).unwrap();
    locks.push(profile.join(CARGO_BUILD_LOCK_NAME));
    locks.push(profile.join(format!("{CARGO_BUILD_LOCK_NAME}.queue")));
  }

  for checkout in [&a, &b] {
    write(&checkout.join(CHECKOUT_RECORD_NAME), 8, now);
    records.push(checkout.join(CHECKOUT_RECORD_NAME));
  }

  // The directories the files were made in are dated with what they hold, as a build leaves them.
  for (dir, at) in [
    (a.join("debug").join("deps"), now - 80 * HOUR_MS),
    (a.join("debug").join(".fingerprint"), now - 80 * HOUR_MS),
    (a.join("debug").join("incremental").join("crate-0abc").join("s-one"), now - 70 * HOUR_MS),
    (a.join("debug").join("incremental").join("crate-0abc"), now - 70 * HOUR_MS),
    (a.join("debug").join("incremental"), now - 70 * HOUR_MS),
    (b.join("debug").join("deps"), now - HOUR_MS),
    (b.join("debug").join(".fingerprint"), now - HOUR_MS),
  ] {
    date(&dir, at);
  }

  Built {
    root,
    shared_old,
    shared_new,
    shared_linked,
    a_unit,
    b_unit,
    a_rest,
    b_rest,
    locks,
    records,
  }
}

// A ceiling of `bytes` set by the environment, with the deployments' own of `deployments` where it is given, on a
// filesystem with room enough for it.
fn ceiling_of(bytes: u64, deployments: Option<u64>, held: u64) -> Ceiling {
  Ceiling::of(
    CeilingSetting {
      bytes,
      named: true,
      deployments,
    },
    held,
    bytes.saturating_add(PRUNE.ceiling_room_gigabytes * GIGABYTE),
  )
}

// The enforcement over the build directory with nothing gone before it and the prune holding every lock, the
// deployments' checkouts kept whole.
fn enforce(root: &Path, ceiling: u64, keepers: &Keepers, remove: bool) -> Enforcement {
  enforce_with(root, ceiling, None, keepers, remove)
}

// The same with the deployments' own ceiling `deployments`.
fn enforce_with(
  root: &Path,
  ceiling: u64,
  deployments: Option<u64>,
  keepers: &Keepers,
  remove: bool,
) -> Enforcement {
  let held = directory_bytes(root);

  CeilingPlan::of(root, &[], ceiling_of(ceiling, deployments, held), held).enforce(keepers, remove)
}

// "Its ceiling is a setting of the machine: BMO_CARGO_TARGET_CEILING_GB ... and 200 where the environment names
// no positive whole number"; "never more than the filesystem can hold ... the lesser of the setting and what the
// filesystem holding the build root leaves the directory with the prune policy's room ... still free"
#[test]
fn the_ceiling_is_its_setting_and_never_more_than_its_filesystem_leaves_with_the_room_free() {
  let named =
    |given: &str| CeilingSetting::of(&HashMap::from([(CEILING_VARIABLE.to_owned(), given.to_owned())]));
  let policy = CeilingSetting {
    bytes: PRUNE.ceiling_gigabytes * GIGABYTE,
    named: false,
    deployments: None,
  };
  let room = PRUNE.ceiling_room_gigabytes * GIGABYTE;

  assert_eq!(
    named("300"),
    CeilingSetting {
      bytes: 300 * GIGABYTE,
      named: true,
      deployments: None,
    }
  );

  for unread in ["0", "-5", "none", ""] {
    assert_eq!(named(unread), policy, "{unread:?}");
  }

  assert_eq!(CeilingSetting::of(&HashMap::new()), policy);

  let roomy = Ceiling::of(policy, 100 * GIGABYTE, 1000 * GIGABYTE);

  assert_eq!((roomy.bytes, roomy.fits, roomy.room), (policy.bytes, 1100 * GIGABYTE - room, room));
  assert_eq!(Ceiling::of(policy, 100 * GIGABYTE, 100 * GIGABYTE).bytes, 200 * GIGABYTE - room);
  assert_eq!(Ceiling::of(policy, 10 * GIGABYTE, 10 * GIGABYTE).bytes, 0);

  // On the filesystem the test stands on, however large the setting.
  let scratch = tempfile::tempdir().unwrap();
  let held = directory_bytes(scratch.path());
  let free = disk_free_bytes(scratch.path());
  let any = CeilingSetting {
    bytes: u64::MAX,
    named: true,
    deployments: None,
  };

  assert!(Ceiling::of(any, held, free).bytes <= (held + free).saturating_sub(room));
}

// Under its ceiling the prune removes no more than its rules do.
#[test]
fn under_its_ceiling_the_enforcement_reads_and_removes_nothing() {
  let scratch = tempfile::tempdir().unwrap();
  let built = built(scratch.path());
  let held = directory_bytes(&built.root);
  let enforcement = enforce(&built.root, held, &Keepers::default(), true);

  assert!(enforcement.rules.is_empty());
  assert_eq!((enforcement.before, enforcement.after), (held, held));
  assert!(!enforcement.over());
  assert_eq!(directory_bytes(&built.root), held);
}

// "shared units ... then live checkouts' own units ... then live checkouts' target directories ... each taking its
// own in order of least recent read"; "It stops as soon as the directory holds no more than its ceiling"; "A
// removal that frees nothing is passed over"
#[test]
fn over_its_ceiling_shared_units_go_first_then_checkouts_units_each_by_least_recent_read_until_it_is_under()
{
  let scratch = tempfile::tempdir().unwrap();
  let built = built(scratch.path());
  let held = directory_bytes(&built.root);
  let shared = bytes_of(&built.shared_old) + bytes_of(&built.shared_new);
  // Over by one byte once both shared units are gone: checkout `a`'s unit, read before either shared one, is the
  // next to go, and the last.
  let ceiling = held - shared - 1;
  let enforcement = enforce(&built.root, ceiling, &Keepers::default(), true);

  assert_eq!(enforcement.rules.len(), 2, "{enforcement:?}");
  assert_eq!((enforcement.rules[0].count, enforcement.rules[0].bytes), (2, shared));
  assert_eq!(enforcement.rules[1].count, 1);
  assert!(!enforcement.over());
  assert!(none_exist(&built.shared_old) && none_exist(&built.shared_new) && none_exist(&built.a_unit));

  // The oldest shared unit is held by checkout `a`'s link, so removing it frees nothing and it stays.
  assert!(all_exist(&built.shared_linked));
  assert!(all_exist(&built.b_unit) && all_exist(&built.a_rest) && all_exist(&built.b_rest));
  assert_eq!(directory_bytes(&built.root), enforcement.after);
}

// "live checkouts' target directories: whatever else a live checkout's profile directories hold"; "It never
// removes Cargo's lock files and their queues ... and nothing outside the profile directories"
#[test]
fn last_the_checkouts_profile_directories_are_emptied_oldest_first_but_their_locks_and_records_stay() {
  let scratch = tempfile::tempdir().unwrap();
  let built = built(scratch.path());
  let enforcement = enforce(&built.root, 0, &Keepers::default(), true);
  let last = &enforcement.rules[2];

  assert_eq!(enforcement.rules.len(), 3);
  assert_eq!(last.names, ["a-0123456789ab", "b-0123456789ab"]);
  assert!(none_exist(&built.a_unit) && none_exist(&built.b_unit));
  assert!(none_exist(&built.a_rest) && none_exist(&built.b_rest));
  assert!(all_exist(&built.locks) && all_exist(&built.records));

  // Checkout `a`'s link went with its directory, so the shared unit it held now frees its disk on a later run;
  // nothing holds what is left, which is that unit alone.
  assert!(all_exist(&built.shared_linked));
  assert!(enforcement.over() && enforcement.holders.is_empty());
}

// A live checkout's directory under the build root holding `count` units from `first` on, read at `at`, carrying a
// deployment's mark where `deployment` says so; its units.
fn checkout_of(
  root: &Path,
  name: &str,
  first: u64,
  count: u64,
  at: i64,
  deployment: bool,
) -> Vec<Vec<PathBuf>> {
  let dir = root.join("checkouts").join(name);

  write(&dir.join(CHECKOUT_RECORD_NAME), 8, at);

  if deployment {
    write(&dir.join(DEPLOYMENT_MARK_NAME), 0, at);
  }

  (first..first + count).map(|index| unit(&dir.join("debug"), index, at)).collect()
}

// "neither the prune's rules nor its ceiling remove anything in it"; "the enforcement removes every working
// checkout's rebuildable output and stops there, the directory standing over its ceiling ... which its line and
// its event name": a deployment's checkout read long before the working one keeps every unit, and is named as
// what holds the rest
#[test]
fn a_deployment_s_checkout_keeps_everything_and_holds_the_rest_once_the_working_ones_are_emptied() {
  let scratch = tempfile::tempdir().unwrap();
  let root = scratch.path().join("build");
  let now = now();
  let deployment = checkout_of(&root, "deployed-0123456789ab", 1, 2, now - 50 * HOUR_MS, true);
  let working = checkout_of(&root, "working-0123456789ab", 3, 2, now - HOUR_MS, false);
  let enforcement = enforce(&root, 0, &Keepers::default(), true);

  assert!(enforcement.over(), "{enforcement:?}");
  assert!(deployment.iter().all(|one| all_exist(one)));
  assert!(working.iter().all(|one| none_exist(one)));
  assert_eq!(
    enforcement
      .holders
      .iter()
      .map(|holder| (holder.path.clone(), holder.by))
      .collect::<Vec<_>>(),
    [(root.join("checkouts").join("deployed-0123456789ab"), HeldBy::DeploymentCheckout)]
  );
}

// "BMO_CARGO_TARGET_DEPLOYMENT_CEILING_GB ... Where it names no positive whole number, the default, the
// deployments' checkouts are kept whole"
#[test]
fn the_deployments_ceiling_is_named_in_gigabytes_and_none_where_nothing_positive_is_named() {
  let named = |given: &str| {
    CeilingSetting::of(&HashMap::from([(DEPLOYMENT_CEILING_VARIABLE.to_owned(), given.to_owned())]))
      .deployments
  };

  assert_eq!(named("50"), Some(50 * GIGABYTE));

  for unread in ["0", "-5", "none", ""] {
    assert_eq!(named(unread), None, "{unread:?}");
  }

  assert_eq!(CeilingSetting::of(&HashMap::new()).deployments, None);
}

// "Where it names one, the enforcement holds them to it ... while they hold more, it removes their rebuildable
// output by two rules of their own ... in the same order of least recent read": the directory under its own
// ceiling, the deployments' checkouts one unit over theirs lose the unit read longest ago, and nothing else goes
#[test]
fn a_deployments_ceiling_takes_their_unit_read_longest_ago_and_nothing_of_the_working_checkouts() {
  let scratch = tempfile::tempdir().unwrap();
  let root = scratch.path().join("build");
  let now = now();
  let older = checkout_of(&root, "older-0123456789ab", 1, 2, now - 50 * HOUR_MS, true);
  let newer = checkout_of(&root, "newer-0123456789ab", 3, 1, now - 2 * HOUR_MS, true);
  let working = checkout_of(&root, "working-0123456789ab", 4, 2, now - HOUR_MS, false);
  let unit_bytes = bytes_of(&newer[0]);
  let held = directory_bytes(&root);
  let enforcement = enforce_with(&root, held, Some(2 * unit_bytes), &Keepers::default(), true);

  assert!(!enforcement.over(), "{enforcement:?}");
  assert_eq!(older.iter().filter(|one| none_exist(one)).count(), 1);
  assert!(all_exist(&newer[0]) && working.iter().all(|one| all_exist(one)));
  assert_eq!(
    enforcement.rules.iter().map(|rule| (rule.words, rule.count)).collect::<Vec<_>>(),
    [(VERBS.prune.deployment_units.as_str(), 1)]
  );
}

// "It never removes ... anything in a profile directory whose build lock the prune does not hold"; the event
// names "a profile directory a build's lock holds"
#[test]
fn a_unit_in_a_profile_directory_whose_lock_the_prune_does_not_hold_stays_and_is_named_as_held() {
  let scratch = tempfile::tempdir().unwrap();
  let built = built(scratch.path());
  let held_profile = built.root.join("checkouts").join("b-0123456789ab").join("debug");
  let locks = take_prune_locks(&built.root, 0.01, &|_| {});
  let mut keepers = Keepers::read(&built.root, Some(&locks), &[]);

  // A build holds checkout `b`'s lock: the prune took every other.
  assert!(keepers.locked.as_mut().unwrap().remove(&held_profile));

  let enforcement = enforce(&built.root, 0, &keepers, true);

  assert!(all_exist(&built.b_unit) && all_exist(&built.b_rest));
  assert!(none_exist(&built.a_unit) && none_exist(&built.a_rest));
  assert!(enforcement.over());
  assert_eq!(
    enforcement.holders,
    [Holder {
      path: held_profile,
      by: HeldBy::BuildLock,
      bytes: bytes_of(&built.b_unit) + bytes_of(&built.b_rest),
    }]
  );
}

// "--dry-run says what it would remove ... what enforcing the ceiling would take"; "A dry run therefore orders
// the removals as the run would"
#[test]
fn a_dry_run_removes_nothing_and_counts_what_the_run_then_removes() {
  let scratch = tempfile::tempdir().unwrap();
  let built = built(scratch.path());
  let held = directory_bytes(&built.root);
  let ceiling = held - bytes_of(&built.shared_old) - bytes_of(&built.shared_new) - 1;
  let dry = enforce(&built.root, ceiling, &Keepers::default(), false);

  assert_eq!(directory_bytes(&built.root), held);
  assert!(all_exist(&built.shared_old) && all_exist(&built.a_unit));

  let run = enforce(&built.root, ceiling, &Keepers::default(), true);

  assert_eq!(dry, run);
}
