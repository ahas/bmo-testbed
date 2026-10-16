// A build directory shaped as Cargo leaves it, dated by hand: units old and recent, chained through the
// fingerprint values their fingerprints name, some held by a checkout's target directory, beside incremental
// state, loose files and the target directories of checkouts live and gone (docs/deploy.md, 「The Cargo build
// directory」).

use std::fs::{DirEntry, File, FileTimes, Metadata};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bmo_testbed_suite::crates::{CHECKOUT_RECORD_NAME, checkout_target_name};
use bmo_testbed_suite::prune::{
  CARGO_LOCK_NAME, CHECKOUT_CEILING_BYTES, Ceiling, DAY_MS, HOUR_MS, INCREMENTAL_HOURS,
  SHARED_CEILING_BYTES, UnitFingerprint, ceiling, hand_profile_dirs, plan_clear, plan_prune,
  read_fingerprint, take_prune_locks,
};
use bmo_testbed_suite::superseded::{BUILT_UNITS_NAME, CARGO_BUILD_LOCK_NAME};

fn entries_of(dir: &Path) -> Vec<DirEntry> {
  std::fs::read_dir(dir).map(|read| read.flatten().collect()).unwrap_or_default()
}

fn is_dir(entry: &DirEntry) -> bool {
  entry.file_type().is_ok_and(|kind| kind.is_dir())
}

fn name_of(entry: &DirEntry) -> String {
  entry.file_name().to_string_lossy().into_owned()
}

fn modified_ms(held: &Metadata) -> i64 {
  held.mtime() * 1000 + held.mtime_nsec() / 1_000_000
}

fn accessed_ms(held: &Metadata) -> i64 {
  held.atime() * 1000 + held.atime_nsec() / 1_000_000
}

fn now() -> i64 {
  SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

fn time_of(at: i64) -> SystemTime {
  UNIX_EPOCH + Duration::from_millis(at as u64)
}

// The file or directory at `path` read at `accessed` and changed at `modified`.
fn date_as(path: &Path, accessed: i64, modified: i64) {
  let times = FileTimes::new().set_accessed(time_of(accessed)).set_modified(time_of(modified));

  File::open(path).unwrap().set_times(times).unwrap();
}

// The file or directory at `path` dated `at`, read and changed alike.
fn date(path: &Path, at: i64) {
  date_as(path, at, at);
}

// Every directory at and under `path` dated `at`, since making a file in one changes it.
fn date_dirs(path: &Path, at: i64) {
  for entry in entries_of(path) {
    if is_dir(&entry) {
      date_dirs(&entry.path(), at);
    }
  }

  date(path, at);
}

fn write(path: &Path, text: &str, at: i64) {
  std::fs::create_dir_all(path.parent().unwrap()).unwrap();
  std::fs::write(path, text).unwrap();
  date(path, at);
}

// A fingerprint value past what a double holds exactly, one per unit.
fn value_of(index: u64) -> u64 {
  18_000_000_000_000_000_000 + index
}

fn hex_of(value: u64) -> String {
  value.to_le_bytes().iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Unit {
  name: String,
  hash: String,
  value: u64,
}

// A library unit in the profile: its fingerprint directory, naming the units it was built from, and its
// library, all dated `at`.
fn unit(profile: &Path, index: u64, name: &str, at: i64, built_from: &[&Unit]) -> Unit {
  let hash = format!("{index:016x}");
  let fingerprint = profile.join(".fingerprint").join(format!("{name}-{hash}"));
  let deps: Vec<String> = built_from
    .iter()
    .map(|one| format!("[1,\"{}\",false,{}]", one.name, one.value))
    .collect();

  write(&fingerprint.join(format!("lib-{name}")), &hex_of(value_of(index)), at);
  write(
    &fingerprint.join(format!("lib-{name}.json")),
    &format!("{{\"rustc\":1,\"deps\":[{}]}}", deps.join(",")),
    at,
  );
  date(&fingerprint, at);
  write(&profile.join("deps").join(format!("lib{name}-{hash}.rlib")), name, at);

  Unit {
    name: name.to_owned(),
    hash,
    value: value_of(index),
  }
}

fn library(profile: &Path, made: &Unit) -> PathBuf {
  profile.join("deps").join(format!("lib{}-{}.rlib", made.name, made.hash))
}

// A checkout's target directory holding a hard link to a unit's library, with the checkout's path recorded
// where one is given.
fn checkout_dir(
  root: &Path,
  name: &str,
  profile: &Path,
  holds: &Unit,
  recorded: Option<&Path>,
  at: i64,
) -> PathBuf {
  let dir = root.join("checkouts").join(name);
  let link = dir.join("debug").join(format!("lib{}.rlib", holds.name));

  std::fs::create_dir_all(dir.join("debug")).unwrap();
  std::fs::hard_link(library(profile, holds), &link).unwrap();
  date(&link, at);

  if let Some(recorded) = recorded {
    write(&dir.join(CHECKOUT_RECORD_NAME), &recorded.to_string_lossy(), at);
  }

  date_dirs(&dir, at);
  dir
}

fn sorted(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
  paths.sort();
  paths
}

fn names_in(dir: &Path) -> Vec<String> {
  let mut names: Vec<String> = entries_of(dir).iter().map(name_of).collect();

  names.sort();
  names
}

#[test]
fn a_prune_removes_what_nobody_read_within_the_span_and_keeps_what_a_live_checkout_or_a_recent_build_stands_on()
 {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let old = now - 3 * DAY_MS;
  let recent = now - HOUR_MS;
  let root = scratch.path().join("one");
  let profile = root.join("debug");
  let live = scratch.path().join("live-checkout");

  std::fs::create_dir_all(&live).unwrap();

  // Old and alone; old under an old one; old under a recent one; old under what a live checkout holds; old and
  // held only by a gone checkout.
  let alone = unit(&profile, 1, "alone", old, &[]);
  let under_old = unit(&profile, 2, "under_old", old, &[]);
  let old_top = unit(&profile, 3, "old_top", old, &[&under_old]);
  let under_recent = unit(&profile, 4, "under_recent", old, &[]);
  let recent_top = unit(&profile, 5, "recent_top", recent, &[&under_recent]);
  let under_live = unit(&profile, 6, "under_live", old, &[]);
  let live_top = unit(&profile, 7, "live_top", old, &[&under_live]);
  let gone_top = unit(&profile, 8, "gone_top", old, &[]);
  let script = profile.join("build").join(format!("script-{}", alone.hash));

  write(&script.join("out").join("generated.rs"), "", old);
  date_dirs(&script, old);
  write(&profile.join("incremental/alone-0abc/s-old/dep-graph.bin"), "", old);
  date_dirs(&profile.join("incremental/alone-0abc"), old);
  write(&profile.join("incremental/recent_top-0def/s-new/dep-graph.bin"), "", recent);

  // Judged by modification time alone, in hours: a session made four hours ago and read since goes, the one
  // beside it made an hour ago stays.
  let four_hours = now - 4 * HOUR_MS;

  write(&profile.join("incremental/mixed-0fed/s-old/dep-graph.bin"), "", four_hours);
  date_as(&profile.join("incremental/mixed-0fed/s-old/dep-graph.bin"), recent, four_hours);
  date_dirs(&profile.join("incremental/mixed-0fed/s-old"), four_hours);
  write(&profile.join("incremental/mixed-0fed/s-new/dep-graph.bin"), "", recent);
  write(&profile.join("old-bin"), "", old);
  write(&profile.join("new-bin"), "", recent);
  write(&profile.join(CARGO_BUILD_LOCK_NAME), "", old);

  let gone_checkout = scratch.path().join("gone-checkout");
  let live_dir = checkout_dir(&root, &checkout_target_name(&live), &profile, &live_top, Some(&live), old);
  let gone_dir = checkout_dir(
    &root,
    &checkout_target_name(&gone_checkout),
    &profile,
    &gone_top,
    Some(&gone_checkout),
    old,
  );
  let idle_dir = checkout_dir(&root, "idle-0123456789ab", &profile, &alone, None, old);
  let busy_dir = checkout_dir(&root, "busy-0123456789ab", &profile, &under_recent, None, recent);
  let plan = plan_prune(&root, 2 * DAY_MS, now, INCREMENTAL_HOURS * HOUR_MS);
  let mut gone: Vec<&str> = plan.units.iter().map(|one| one.hash.as_str()).collect();
  let mut expected: Vec<&str> =
    [&alone, &under_old, &old_top, &gone_top].iter().map(|one| one.hash.as_str()).collect();

  gone.sort_unstable();
  expected.sort_unstable();

  assert_eq!(gone, expected);
  assert_eq!(
    plan.units.iter().find(|one| one.hash == alone.hash).unwrap().paths,
    vec![
      profile.join(".fingerprint").join(format!("alone-{}", alone.hash)),
      script.clone(),
      library(&profile, &alone),
    ]
  );
  assert_eq!(
    sorted(plan.incremental.clone()),
    vec![profile.join("incremental/alone-0abc"), profile.join("incremental/mixed-0fed/s-old")]
  );
  assert_eq!(plan.loose, vec![profile.join("old-bin")]);
  assert_eq!(sorted(plan.checkouts.clone()), sorted(vec![gone_dir.clone(), idle_dir.clone()]));
  assert!(plan.bytes > 0);

  plan.apply();

  for kept in [&recent_top, &under_recent, &live_top, &under_live] {
    assert!(library(&profile, kept).exists(), "{}", kept.name);
  }

  assert!(!library(&profile, &alone).exists());
  assert!(!script.exists());
  assert!(profile.join("incremental/recent_top-0def").exists());
  assert!(profile.join("incremental/mixed-0fed/s-new").exists());
  assert!(profile.join("new-bin").exists());
  assert!(profile.join(CARGO_BUILD_LOCK_NAME).exists());
  assert!(live_dir.exists() && busy_dir.exists());
  assert!(!gone_dir.exists() && !idle_dir.exists());
}

#[test]
fn a_clear_removes_everything_but_the_profile_directories_locks_live_checkouts_and_recent_units_alike() {
  let scratch = tempfile::tempdir().unwrap();
  let recent = now() - HOUR_MS;
  let root = scratch.path().join("clear");
  let profile = root.join("debug");
  let live = scratch.path().join("clear-live");

  std::fs::create_dir_all(&live).unwrap();

  let recent_top = unit(&profile, 11, "recent_top", recent, &[]);

  write(&profile.join(CARGO_BUILD_LOCK_NAME), "", recent);
  write(&profile.join(CARGO_LOCK_NAME), "", recent);
  std::fs::create_dir_all(profile.join(".cargo-build-lock.queue")).unwrap();
  write(&profile.join("service-core"), "", recent);
  write(&root.join(".rustc_info.json"), "{}", recent);
  checkout_dir(&root, &checkout_target_name(&live), &profile, &recent_top, Some(&live), recent);

  let plan = plan_clear(&root);

  plan.apply();

  assert!(plan.bytes > 0);
  assert_eq!(names_in(&root), ["checkouts", "debug"]);
  assert!(names_in(&root.join("checkouts")).is_empty());
  assert_eq!(names_in(&profile), [CARGO_BUILD_LOCK_NAME, ".cargo-build-lock.queue", CARGO_LOCK_NAME]);
}

#[test]
fn a_live_checkouts_own_target_loses_the_work_a_build_typed_by_hand_left_there_once_unused_its_binaries_kept()
 {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let old = now - 3 * DAY_MS;
  let recent = now - HOUR_MS;
  let root = scratch.path().join("hand");
  let checkout = scratch.path().join("hand-checkout");
  let hand_profile = checkout.join("target").join("debug");

  std::fs::create_dir_all(root.join("debug").join(".fingerprint")).unwrap();
  std::fs::create_dir_all(&checkout).unwrap();
  write(
    &root.join("checkouts").join(checkout_target_name(&checkout)).join(CHECKOUT_RECORD_NAME),
    &checkout.to_string_lossy(),
    recent,
  );
  unit(&hand_profile, 12, "hand_old", old, &[]);
  write(&hand_profile.join("incremental/hand_old-0abc/s-old/dep-graph.bin"), "", old);

  for dir in ["incremental", "deps", ".fingerprint"] {
    date_dirs(&hand_profile.join(dir), old);
  }

  write(&hand_profile.join("service-core"), "", old);
  write(&hand_profile.join(CARGO_LOCK_NAME), "", old);
  write(&hand_profile.join("build/script-0000000000000000/output"), "", recent);

  let plan = plan_prune(&root, 2 * DAY_MS, now, INCREMENTAL_HOURS * HOUR_MS);

  assert_eq!(
    sorted(plan.hand_built.clone()),
    vec![
      hand_profile.join(".fingerprint"),
      hand_profile.join("deps"),
      hand_profile.join("incremental")
    ]
  );
  assert_eq!(hand_profile_dirs(&root), vec![hand_profile.clone()]);

  plan.apply();

  assert_eq!(names_in(&hand_profile), [CARGO_LOCK_NAME, "build", "service-core"]);
}

#[test]
fn reading_a_fingerprint_leaves_its_access_time_where_it_was() {
  let scratch = tempfile::tempdir().unwrap();
  let old = now() - 3 * DAY_MS;
  let profile = scratch.path().join("quiet").join("debug");
  let other = Unit {
    name: "other".to_owned(),
    hash: String::new(),
    value: value_of(10),
  };
  let one = unit(&profile, 9, "quiet", old, &[&other]);
  let dir = profile.join(".fingerprint").join(format!("quiet-{}", one.hash));

  assert_eq!(
    read_fingerprint(&dir),
    UnitFingerprint {
      values: vec![value_of(9).to_string()],
      dependencies: vec![value_of(10).to_string()],
    }
  );

  let json = std::fs::metadata(dir.join("lib-quiet.json")).unwrap();

  assert_eq!(accessed_ms(&json) / 1000, old / 1000);
}

#[test]
fn a_checkout_is_pruned_once_the_directory_its_target_directory_records_is_gone() {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let root = scratch.path().join("recorded");
  let checkout = scratch.path().join("short-lived");
  let dir = root.join("checkouts").join(checkout_target_name(&checkout));

  std::fs::create_dir_all(&checkout).unwrap();
  std::fs::create_dir_all(root.join("debug").join(".fingerprint")).unwrap();
  write(&dir.join(CHECKOUT_RECORD_NAME), &checkout.to_string_lossy(), now);

  assert!(plan_prune(&root, 2 * DAY_MS, now, INCREMENTAL_HOURS * HOUR_MS).checkouts.is_empty());

  std::fs::remove_dir_all(&checkout).unwrap();

  assert_eq!(plan_prune(&root, 2 * DAY_MS, now, INCREMENTAL_HOURS * HOUR_MS).checkouts, vec![dir]);
}

#[test]
fn a_checkouts_own_unit_goes_once_no_record_names_it_however_lately_it_was_read_and_a_dependency_keeps_its_span()
 {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let old = now - 3 * DAY_MS;
  let recent = now - HOUR_MS;
  let before_grace = now - 13 * HOUR_MS;
  let root = scratch.path().join("own");
  let profile = root.join("debug");
  let checkout = scratch.path().join("own-checkout");
  let read_now = |made: &Unit| {
    let fingerprint = profile.join(".fingerprint").join(format!("{}-{}", made.name, made.hash));

    for path in [library(&profile, made), fingerprint.join(format!("lib-{}", made.name))] {
      let modified = modified_ms(&std::fs::metadata(&path).unwrap());

      date_as(&path, recent, modified);
    }
  };

  std::fs::create_dir_all(&checkout).unwrap();

  // A dependency read lately that nothing stands on, and one the recorded unit was built from. The grace and
  // the span are both past for these, so only what each rule reads keeps them.
  let loose = unit(&profile, 20, "loose_dep", old, &[]);
  let under = unit(&profile, 21, "used_dep", old, &[]);

  read_now(&loose);

  // The checkout's own crate: the unit its record names, an earlier one read lately, one built within the
  // grace, and one a build typed by hand handed back into the checkout's own target/.
  let named = unit(&profile, 22, "own_crate", before_grace, &[&under]);
  let earlier = unit(&profile, 23, "own_crate", before_grace, &[]);

  unit(&profile, 24, "own_crate", recent, &[]);

  let handed = unit(&profile, 25, "own_crate", before_grace, &[]);

  read_now(&earlier);

  let dir = root.join("checkouts").join(checkout_target_name(&checkout));
  let record = serde_json::json!({
    "test": {
      "at": now,
      "wrapper": "wrapper",
      "generation": "one",
      "places": { "debug|crates/own|lib|own_crate|plain|build": format!("debug:{}", named.hash) },
    }
  });

  write(&dir.join(CHECKOUT_RECORD_NAME), &checkout.to_string_lossy(), recent);
  write(&dir.join(BUILT_UNITS_NAME), &record.to_string(), recent);
  std::fs::create_dir_all(checkout.join("target").join("debug")).unwrap();
  std::fs::hard_link(library(&profile, &handed), checkout.join("target/debug/libown_crate.rlib")).unwrap();

  let gone: Vec<String> = plan_prune(&root, 2 * DAY_MS, now, INCREMENTAL_HOURS * HOUR_MS)
    .units
    .into_iter()
    .map(|one| one.hash)
    .collect();

  assert_eq!(gone, [earlier.hash]);
}

#[test]
fn the_ceiling_counts_the_live_checkouts_that_record_their_builds_and_no_other() {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let root = scratch.path().join("ceiling");
  let live = scratch.path().join("ceiling-live");
  let unbuilt = scratch.path().join("ceiling-unbuilt");
  let gone = scratch.path().join("ceiling-gone");

  for checkout in [&live, &unbuilt] {
    std::fs::create_dir_all(checkout).unwrap();
  }

  for (checkout, built) in [(&live, true), (&unbuilt, false), (&gone, true)] {
    let dir = root.join("checkouts").join(checkout_target_name(checkout));

    write(&dir.join(CHECKOUT_RECORD_NAME), &checkout.to_string_lossy(), now);

    if built {
      write(&dir.join(BUILT_UNITS_NAME), "{}", now);
    }
  }

  assert_eq!(
    ceiling(&root, now),
    Ceiling {
      bytes: SHARED_CEILING_BYTES + CHECKOUT_CEILING_BYTES,
      checkouts: 1,
    }
  );
}

#[test]
fn the_locks_are_cargos_own_in_each_profile_and_a_hand_builds_in_a_live_checkouts_target_held_until_let_go()
{
  let scratch = tempfile::tempdir().unwrap();
  let root = scratch.path().join("locks");
  let checkout = scratch.path().join("locks-checkout");

  std::fs::create_dir_all(root.join("debug").join(".fingerprint")).unwrap();
  std::fs::create_dir_all(root.join("release").join(".fingerprint")).unwrap();
  std::fs::create_dir_all(checkout.join("target/debug/.fingerprint")).unwrap();
  write(
    &root.join("checkouts").join(checkout_target_name(&checkout)).join(CHECKOUT_RECORD_NAME),
    &checkout.to_string_lossy(),
    now(),
  );

  let held = take_prune_locks(&root, 1.0).unwrap();
  let build_lock = root.join("debug").join(CARGO_BUILD_LOCK_NAME);

  assert_eq!(held.len(), 3);
  assert!(checkout.join("target/debug").join(CARGO_LOCK_NAME).exists());

  // A second taker waits behind the holder and is refused once its wait runs out.
  assert_eq!(take_prune_locks(&root, 0.0).err(), Some(root.join("debug")));

  drop(held);

  assert!(build_lock.exists());
  assert_eq!(take_prune_locks(&root, 1.0).map(|held| held.len()).ok(), Some(3));
}
