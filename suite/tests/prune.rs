// A build directory shaped as Cargo leaves it, dated by hand: units old and recent, chained through the
// fingerprint values their fingerprints name, some held by a checkout's target directory, beside incremental
// state, loose files and the target directories of checkouts live and gone (docs/deploy.md, 「The Cargo build
// directory」).

use std::fs::{DirEntry, File, FileTimes, Metadata};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bmo_testbed_core::policy::prune::PRUNE;
use bmo_testbed_core::release_target::mark_release_target;
use bmo_testbed_suite::crates::{CHECKOUT_RECORD_NAME, DEPLOYMENT_MARK_NAME, checkout_target_name};
use bmo_testbed_suite::prune::{
  CARGO_LOCK_NAME, DAY_MS, HOUR_MS, PrunePlan, UnitFingerprint, plan_clear, read_fingerprint,
  take_prune_locks,
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

// The prune over the build directory at `root` with the policy's spans, as the verb runs it unless told others.
fn plan_prune(root: &Path, now: i64) -> PrunePlan {
  bmo_testbed_suite::prune::plan_prune(
    root,
    PRUNE.older_than_days * DAY_MS,
    now,
    PRUNE.incremental_hours * HOUR_MS,
  )
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
  let old = now - (PRUNE.older_than_days + 1) * DAY_MS;
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

  // Judged by modification time alone, in hours: a session made an hour past the policy's span and read since
  // goes, the one beside it made an hour ago stays.
  let past_span = now - (PRUNE.incremental_hours + 1) * HOUR_MS;

  write(&profile.join("incremental/mixed-0fed/s-old/dep-graph.bin"), "", past_span);
  date_as(&profile.join("incremental/mixed-0fed/s-old/dep-graph.bin"), recent, past_span);
  date_dirs(&profile.join("incremental/mixed-0fed/s-old"), past_span);
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
  let plan = plan_prune(&root, now);
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
fn a_live_checkouts_own_target_is_a_stray_the_prune_takes_away_whole() {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let old = now - (PRUNE.older_than_days + 1) * DAY_MS;
  let recent = now - HOUR_MS;
  let root = scratch.path().join("hand");
  let checkout = scratch.path().join("hand-checkout");
  let own_target = checkout.join("target");
  let profile = own_target.join("debug");

  std::fs::create_dir_all(root.join("debug").join(".fingerprint")).unwrap();
  std::fs::create_dir_all(&checkout).unwrap();
  write(
    &root.join("checkouts").join(checkout_target_name(&checkout)).join(CHECKOUT_RECORD_NAME),
    &checkout.to_string_lossy(),
    recent,
  );
  // Whatever a build typed outside the Cargo verb left in the checkout's own target/, old or recent: every
  // build belongs to the shared directory, so the whole target/ goes.
  unit(&profile, 12, "hand_old", old, &[]);
  write(&profile.join("service-core"), "", old);
  write(&profile.join("build/script-0000000000000000/output"), "", recent);

  let plan = plan_prune(&root, now);

  assert_eq!(plan.stray_targets, vec![own_target.clone()]);
  assert!(plan.bytes > 0);

  plan.apply();

  assert!(!own_target.exists());
  assert!(checkout.exists());
}

// "nor a deployment's build tree's target/, which carries the release build's mark"; "each asked again just before
// it is removed": a build tree's target/ marked as its release build's own is no stray, and one the plan found
// before a deployment marked it is passed over when the plan is carried out.
#[test]
fn a_target_marked_as_a_deployment_s_release_build_is_no_stray_even_when_marked_after_the_plan() {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let root = scratch.path().join("shared");
  let build_tree = scratch.path().join("deploy-expand");
  let own_target = build_tree.join("target");
  let binary = own_target.join("release").join("app");

  std::fs::create_dir_all(root.join("debug").join(".fingerprint")).unwrap();
  std::fs::create_dir_all(&build_tree).unwrap();
  write(
    &root
      .join("checkouts")
      .join(checkout_target_name(&build_tree))
      .join(CHECKOUT_RECORD_NAME),
    &build_tree.to_string_lossy(),
    now,
  );
  write(&binary, "built", now);

  let found = plan_prune(&root, now);

  assert_eq!(found.stray_targets, vec![own_target.clone()]);

  mark_release_target(&own_target).unwrap();
  found.apply();

  assert!(binary.exists());
  assert!(plan_prune(&root, now).stray_targets.is_empty());

  plan_clear(&root).apply();

  assert!(binary.exists());
}

#[test]
fn reading_a_fingerprint_leaves_its_access_time_where_it_was() {
  let scratch = tempfile::tempdir().unwrap();
  let old = now() - (PRUNE.older_than_days + 1) * DAY_MS;
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

  assert!(plan_prune(&root, now).checkouts.is_empty());

  std::fs::remove_dir_all(&checkout).unwrap();

  assert_eq!(plan_prune(&root, now).checkouts, vec![dir]);
}

#[test]
fn a_checkouts_own_unit_goes_once_no_record_names_it_however_lately_it_was_read_and_a_dependency_keeps_its_span()
 {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let old = now - (PRUNE.older_than_days + 1) * DAY_MS;
  let recent = now - HOUR_MS;
  let before_grace = now - (PRUNE.own_grace_hours + 1) * HOUR_MS;
  let within_grace = now - (PRUNE.own_grace_hours - 1) * HOUR_MS;
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
  // grace, and one a build typed outside the verb handed back into the checkout's own target/. The checkout's
  // own target/ is a stray the prune takes away, so the hard link in it keeps nothing.
  let named = unit(&profile, 22, "own_crate", before_grace, &[&under]);
  let earlier = unit(&profile, 23, "own_crate", before_grace, &[]);

  unit(&profile, 24, "own_crate", within_grace, &[]);

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

  let plan = plan_prune(&root, now);
  let gone: Vec<String> = {
    let mut hashes: Vec<String> = plan.units.iter().map(|one| one.hash.clone()).collect();

    hashes.sort();
    hashes
  };

  // The earlier own unit goes, and the one handed into the checkout's own target/ goes with it: that target/
  // is a stray, so its hard link protects nothing.
  let mut expected = vec![earlier.hash, handed.hash];

  expected.sort();

  assert_eq!(gone, expected);
  assert_eq!(plan.stray_targets, vec![checkout.join("target")]);
}

// "from a profile directory of a live checkout's own directory ... the same": a checkout builds in its own
// directory, where an earlier unit of its own crate goes once no record names it and its grace is past, and a
// dependency nothing stands on goes once unread for the span, while what the record names, what that was
// built from, a unit within the grace and a dependency read lately stay.
#[test]
fn a_live_checkouts_own_directory_keeps_what_its_record_names_and_what_was_read_within_the_span() {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let old = now - (PRUNE.older_than_days + 1) * DAY_MS;
  let recent = now - HOUR_MS;
  let before_grace = now - (PRUNE.own_grace_hours + 1) * HOUR_MS;
  let within_grace = now - (PRUNE.own_grace_hours - 1) * HOUR_MS;
  let root = scratch.path().join("within");
  let checkout = scratch.path().join("within-checkout");
  let dir = root.join("checkouts").join(checkout_target_name(&checkout));
  let profile = dir.join("debug");

  std::fs::create_dir_all(&checkout).unwrap();

  let unread = unit(&profile, 30, "unread_dep", old, &[]);
  let read = unit(&profile, 31, "read_dep", old, &[]);
  let under = unit(&profile, 32, "used_dep", old, &[]);
  let named = unit(&profile, 33, "own_crate", before_grace, &[&under]);
  let earlier = unit(&profile, 34, "own_crate", before_grace, &[]);

  unit(&profile, 35, "own_crate", within_grace, &[]);

  for path in [
    library(&profile, &read),
    profile
      .join(".fingerprint")
      .join(format!("{}-{}", read.name, read.hash))
      .join(format!("lib-{}", read.name)),
  ] {
    let modified = modified_ms(&std::fs::metadata(&path).unwrap());

    date_as(&path, recent, modified);
  }

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

  let plan = plan_prune(&root, now);
  let mut gone: Vec<String> = plan.units.iter().map(|one| one.hash.clone()).collect();
  let mut expected = vec![unread.hash, earlier.hash];

  gone.sort();
  expected.sort();

  assert_eq!(gone, expected);
  assert!(plan.checkouts.is_empty());
}

#[test]
fn the_locks_are_cargos_own_build_lock_in_each_profile_directory_held_until_let_go() {
  let scratch = tempfile::tempdir().unwrap();
  let root = scratch.path().join("locks");

  std::fs::create_dir_all(root.join("debug").join(".fingerprint")).unwrap();
  std::fs::create_dir_all(root.join("release").join(".fingerprint")).unwrap();

  let held = take_prune_locks(&root, 0.01, &|_| {});
  let build_lock = root.join("debug").join(CARGO_BUILD_LOCK_NAME);

  assert_eq!(held.len(), 2);

  // A second taker waits behind the holder until it lets go, told which profile directory it waits on.
  let second = {
    let root = root.clone();

    std::thread::spawn(move || {
      let told = std::sync::Mutex::new(Vec::new());
      let held = take_prune_locks(&root, 0.01, &|profile| told.lock().unwrap().push(profile.to_path_buf()));

      (held.len(), told.into_inner().unwrap())
    })
  };

  std::thread::sleep(std::time::Duration::from_millis(200));

  assert!(!second.is_finished());

  drop(held);

  let (taken, told) = second.join().unwrap();

  assert!(build_lock.exists());
  assert_eq!(taken, 2);
  assert!(told.contains(&root.join("debug")), "{told:?}");
}

// "incremental state unchanged for three hours ... in each live checkout's directory but a deployment's
// checkout's"; "nor anything in a deployment's checkout's directory": two live checkouts with incremental state
// past the span, and only the working one's goes
#[test]
fn a_deployment_s_checkout_keeps_its_stale_incremental_state_while_a_working_one_s_goes() {
  let scratch = tempfile::tempdir().unwrap();
  let now = now();
  let past_span = now - (PRUNE.incremental_hours + 1) * HOUR_MS;
  let root = scratch.path().join("build");
  let session = |checkout: &Path| {
    let dir = root.join("checkouts").join(checkout_target_name(checkout));
    let crate_state = dir.join("debug/incremental/own_crate-0abc");

    std::fs::create_dir_all(checkout).unwrap();
    std::fs::create_dir_all(dir.join("debug/.fingerprint")).unwrap();
    write(&dir.join(CHECKOUT_RECORD_NAME), &checkout.to_string_lossy(), now);
    write(&crate_state.join("s-old/dep-graph.bin"), "", past_span);
    date_dirs(&crate_state, past_span);
    (dir, crate_state)
  };
  let (_, working) = session(&scratch.path().join("working"));
  let (deployment_dir, deployment) = session(&scratch.path().join("deployed"));

  write(&deployment_dir.join(DEPLOYMENT_MARK_NAME), "", now);

  let plan = plan_prune(&root, now);

  assert!(plan.incremental.iter().any(|path| path.starts_with(&working)), "{:?}", plan.incremental);
  assert!(!plan.paths().iter().any(|path| path.starts_with(&deployment_dir)), "{:?}", plan.paths());
  assert!(deployment.exists());
}
