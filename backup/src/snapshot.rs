//! The hourly snapshot (docs/backup.md, 「How a snapshot runs」): the databases dumped and the data folders
//! packed into the staging folder, put in place of the newest, sent to each destination's slot and day,
//! the release archives sent beside them, and the rotation after.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_deploy::remove::remove_tree;
use chrono::{DateTime, Local};

use crate::destination::free_bytes;
use crate::dump::{databases, dump};
use crate::manifest::{DataEntry, DatabaseEntry, FileEntry, Manifest};
use crate::outcome::Outcome;
use crate::pack::{Walk, digest, entries, pack};
use crate::policy::BACKUP;
use crate::rotation::{Rotation, rotate};
use crate::settings::{Destination, Plan, Set};

/// What a run says, line by line.
pub type Say<'a> = &'a mut dyn FnMut(&str);

/// Seconds since a moment, to a tenth.
pub fn seconds_since(started: Instant) -> String {
  format!("{:.1}", started.elapsed().as_secs_f64())
}

/// A folder of the staging area made afresh, readable by the user alone, since it holds env files: what a
/// run cut off left there is taken away first.
pub fn fresh(plan: &Plan, dir: &Path) -> Result<(), String> {
  use std::os::unix::fs::PermissionsExt as _;

  remove_tree(dir, &plan.root)?;
  std::fs::create_dir_all(dir).map_err(|why| format!("{}: {why}", dir.display()))?;

  for one in [plan.staging.as_path(), dir] {
    std::fs::set_permissions(one, std::fs::Permissions::from_mode(0o700))
      .map_err(|why| format!("{}: {why}", one.display()))?;
  }

  Ok(())
}

/// The slot an hour falls in: the hours since the epoch modulo the slots.
pub fn slot_of(now: DateTime<Local>, slots: u64) -> u64 {
  u64::try_from(now.timestamp() / 3600).unwrap_or_default() % slots.max(1)
}

/// Whether the destination's disk keeps the floor free once the bytes land; said and failed where not.
pub fn has_room(
  plan: &Plan,
  destination: &Destination,
  bytes: u64,
  say: Say<'_>,
  outcome: &mut Outcome,
) -> bool {
  let Some(room) = &destination.room else {
    return true;
  };
  let free = free_bytes(room).unwrap_or_default();

  if free >= bytes.saturating_add(plan.room_floor_bytes) {
    return true;
  }

  let line = fill_shown(
    &BACKUP.words.no_room,
    &[
      ("destination", &destination.name),
      ("free", &free),
      ("bytes", &bytes),
      ("floor", &plan.room_floor_bytes),
    ],
  );

  say(&line);
  outcome.fail("room", &destination.name, line);
  false
}

// The files of a snapshot sent under a base key, the manifest last, and whatever else the base held taken
// away; whether the whole of it stands there.
fn send_snapshot(
  destination: &Destination,
  dir: &Path,
  manifest: &Manifest,
  base: &str,
  say: Say<'_>,
  outcome: &mut Outcome,
) -> bool {
  let names = &BACKUP.names;
  let started = Instant::now();
  let store = &destination.store;
  let target = format!("{}: {base}", destination.name);
  let manifest_key = format!("{base}/{}", names.manifest);
  let mut failed = |step: &str, why: String, say: Say<'_>| {
    say(&why);
    outcome.fail(step, &target, why);
    false
  };
  let before = match store.list(&format!("{base}/")) {
    Ok(objects) => objects,
    Err(why) => return failed("list", why, say),
  };

  // A slot whose manifest is gone is never taken for a whole one while it is written.
  if before.iter().any(|object| object.key == manifest_key)
    && let Err(why) = store.remove(&manifest_key)
  {
    return failed("remove", why, say);
  }

  for file in &manifest.files {
    if let Err(why) = store.put(&dir.join(&file.name), &format!("{base}/{}", file.name), &file.sha256) {
      return failed("upload", why, say);
    }
  }

  let after = match store.list(&format!("{base}/")) {
    Ok(objects) => objects,
    Err(why) => return failed("list", why, say),
  };

  for file in &manifest.files {
    let key = format!("{base}/{}", file.name);

    match after.iter().find(|object| object.key == key) {
      None => return failed("read-back", fill_shown(&BACKUP.words.not_listed, &[("key", &key)]), say),
      Some(object) if object.size != file.size => {
        let line = fill_shown(
          &BACKUP.words.size_mismatch,
          &[("key", &key), ("there", &object.size), ("here", &file.size)],
        );

        return failed("read-back", line, say);
      }
      Some(_) => {}
    }
  }

  if let Err(why) = store.put(&dir.join(&names.manifest), &manifest_key, "") {
    return failed("upload", why, say);
  }

  for object in &after {
    let kept = object.key == manifest_key
      || manifest.files.iter().any(|file| object.key == format!("{base}/{}", file.name));

    if !kept && let Err(why) = store.remove(&object.key) {
      return failed("remove", why, say);
    }
  }

  say(&fill_shown(
    &BACKUP.words.sent,
    &[
      ("destination", &destination.name),
      ("key", &base),
      ("bytes", &manifest.bytes()),
      ("seconds", &seconds_since(started)),
    ],
  ));
  true
}

// Each declared release folder's newest archives, by name, which carries the time each was taken: the key
// of the folder, the name of the folder above it, and the archives.
fn release_archives(plan: &Plan) -> Vec<(String, Vec<PathBuf>)> {
  let suffix = &BACKUP.names.archive_suffix;
  let mut found = Vec::new();

  for pattern in &plan.release_folders {
    for dir in glob::glob(pattern).into_iter().flatten().flatten().filter(|dir| dir.is_dir()) {
      let Some(name) =
        dir.parent().and_then(Path::file_name).map(|name| name.to_string_lossy().into_owned())
      else {
        continue;
      };
      let mut archives: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.to_string_lossy().ends_with(suffix.as_str()))
        .collect();

      archives.sort_unstable_by(|one, other| other.cmp(one));
      archives.truncate(plan.releases_kept);
      found.push((name, archives));
    }
  }

  found
}

// The release archives a destination does not yet hold whole, sent.
fn send_releases(plan: &Plan, destination: &Destination, say: Say<'_>, outcome: &mut Outcome) {
  for (folder, archives) in release_archives(plan) {
    let base = plan.key(&[&BACKUP.names.releases, &folder]);
    let standing = match destination.store.list(&format!("{base}/")) {
      Ok(objects) => objects,
      Err(why) => {
        say(&why);
        outcome.fail("list", &destination.name, why);
        continue;
      }
    };

    for archive in archives {
      let Some(name) = archive.file_name().map(|name| name.to_string_lossy().into_owned()) else {
        continue;
      };
      let key = format!("{base}/{name}");
      let size = std::fs::metadata(&archive).map(|meta| meta.len()).unwrap_or_default();

      if standing.iter().any(|object| object.key == key && object.size == size)
        || !has_room(plan, destination, size, say, outcome)
      {
        continue;
      }

      let started = Instant::now();
      let sent = digest(&archive).and_then(|(_, sha256)| destination.store.put(&archive, &key, &sha256));

      match sent {
        Ok(()) => say(&fill_shown(
          &BACKUP.words.release_sent,
          &[
            ("destination", &destination.name),
            ("key", &key),
            ("bytes", &size),
            ("seconds", &seconds_since(started)),
          ],
        )),
        Err(why) => {
          say(&why);
          outcome.fail("upload", &format!("{}: {key}", destination.name), why);
        }
      }
    }
  }
}

// The databases dumped into the folder, each said; what was dumped.
fn dump_databases(plan: &Plan, dir: &Path, say: Say<'_>, outcome: &mut Outcome) -> Vec<DatabaseEntry> {
  let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(runtime) => runtime,
    Err(why) => {
      outcome.fail("dump", "", why.to_string());
      return vec![];
    }
  };
  let listed = match runtime.block_on(databases(&plan.server)) {
    Ok(listed) => listed,
    Err(why) => {
      say(&why);
      outcome.fail("list-databases", &plan.server, why);
      return vec![];
    }
  };
  let mut dumped = Vec::new();

  for database in listed.into_iter().filter(|one| plan.dumps(one)) {
    let started = Instant::now();
    let file = format!("{database}{}", BACKUP.names.dump_suffix);

    let dumped_now = plan
      .address_of(&database)
      .map(|address| runtime.block_on(dump(&address, &dir.join(&file))));

    match dumped_now.and_then(|dumped| dumped) {
      Ok(tables) => {
        let size = std::fs::metadata(dir.join(&file)).map(|meta| meta.len()).unwrap_or_default();

        say(&fill_shown(
          &BACKUP.words.dumped,
          &[
            ("database", &database),
            ("bytes", &size),
            ("tables", &tables.len()),
            ("rows", &tables.values().sum::<i64>()),
            ("seconds", &seconds_since(started)),
          ],
        ));
        dumped.push(DatabaseEntry {
          name: database,
          file,
          tables,
        });
      }
      Err(why) => {
        say(&why);
        outcome.fail("dump", &database, why);
      }
    }
  }

  dumped
}

// The data folders packed into the folder, said; the archive's entry where it was packed.
fn pack_data(plan: &Plan, dir: &Path, say: Say<'_>, outcome: &mut Outcome) -> Option<DataEntry> {
  let words = &BACKUP.words;
  let started = Instant::now();
  let apart: Vec<PathBuf> = plan.media.iter().flat_map(|set| set.folders.clone()).collect();
  let walked = Walk {
    root: &plan.root,
    exclude: &plan.exclude,
    apart: &apart,
    large_over: Some(plan.large_file_bytes),
  }
  .over(&plan.folders);

  for missing in &walked.missing {
    say(&fill_shown(&words.missing_folder, &[("path", &missing.display())]));
  }

  let file = BACKUP.names.data_archive.clone();
  let archive = dir.join(&file);
  let counted = pack(&plan.root, &walked.held, &archive).and_then(|()| entries(&archive));

  match counted {
    Ok(count) => {
      let size = std::fs::metadata(&archive).map(|meta| meta.len()).unwrap_or_default();

      say(&fill_shown(
        &words.packed,
        &[("entries", &count), ("bytes", &size), ("seconds", &seconds_since(started))],
      ));

      if !walked.large.is_empty() {
        say(&fill_shown(
          &words.left_out,
          &[
            ("count", &walked.large.len()),
            ("bytes", &walked.large_bytes),
            ("over", &plan.large_file_bytes),
          ],
        ));
      }

      Some(DataEntry {
        file,
        entries: count,
      })
    }
    Err(why) => {
      say(&why);
      outcome.fail("pack", &plan.root.display().to_string(), why);
      None
    }
  }
}

/// The snapshot made, put in place of the newest, sent and rotated.
pub fn snapshot(plan: &Plan, now: DateTime<Local>, say: Say<'_>) -> Outcome {
  let names = &BACKUP.names;
  let started = Instant::now();
  let mut outcome = Outcome::default();
  let next = plan.staging.join(&names.next);
  let latest = plan.staging.join(&names.latest);

  if let Err(why) = fresh(plan, &next) {
    say(&why);
    outcome.fail("stage", &next.display().to_string(), why);
    return outcome;
  }

  let slot = slot_of(now, plan.hourly_slots);
  let databases = dump_databases(plan, &next, say, &mut outcome);
  let data = pack_data(plan, &next, say, &mut outcome);
  let mut files = Vec::new();

  for name in databases.iter().map(|one| &one.file).chain(data.iter().map(|one| &one.file)) {
    match digest(&next.join(name)) {
      Ok((size, sha256)) => files.push(FileEntry {
        name: name.clone(),
        size,
        sha256,
      }),
      Err(why) => outcome.fail("digest", name, why),
    }
  }

  let manifest = Manifest {
    version: BACKUP.numbers.manifest_version,
    taken_at: now.to_rfc3339(),
    taken_ms: now.timestamp_millis(),
    slot,
    files,
    databases,
    data,
  };
  let written = serde_json::to_string_pretty(&manifest)
    .map_err(|why| why.to_string())
    .and_then(|text| std::fs::write(next.join(&names.manifest), text).map_err(|why| why.to_string()))
    .and_then(|()| remove_tree(&latest, &plan.root))
    .and_then(|()| std::fs::rename(&next, &latest).map_err(|why| why.to_string()));

  if let Err(why) = written {
    say(&why);
    outcome.fail("stage", &latest.display().to_string(), why);
    return outcome;
  }

  let today = now.date_naive();
  let day = today.format("%Y-%m-%d").to_string();

  for destination in &plan.destinations {
    if destination.takes(Set::Hourly) && has_room(plan, destination, manifest.bytes(), say, &mut outcome) {
      let base = plan.key(&[&names.hourly, &slot.to_string()]);

      send_snapshot(destination, &latest, &manifest, &base, say, &mut outcome);
    }

    if destination.takes(Set::Daily) {
      let base = plan.key(&[&names.daily, &day]);
      let standing = destination.store.list(&format!("{base}/"));
      let whole = standing.as_ref().is_ok_and(|objects| {
        objects.iter().any(|object| object.key == format!("{base}/{}", names.manifest))
      });

      if let Err(why) = standing {
        say(&why);
        outcome.fail("list", &destination.name, why);
      } else if !whole && has_room(plan, destination, manifest.bytes(), say, &mut outcome) {
        send_snapshot(destination, &latest, &manifest, &base, say, &mut outcome);
      }
    }

    if destination.takes(Set::Releases) {
      send_releases(plan, destination, say, &mut outcome);
    }
  }

  let media = media_names(plan);
  let rotated = rotate(plan, &Rotation::of(plan, today, &media), false, say);

  outcome.failed.extend(rotated.failed);
  say(&fill_shown(
    &BACKUP.words.snapshot_done,
    &[("slot", &slot), ("failed", &outcome.failed.len()), ("seconds", &seconds_since(started))],
  ));
  outcome
}

/// Every media archive's name: each declared set's, and the large files'.
pub fn media_names(plan: &Plan) -> Vec<String> {
  plan
    .media
    .iter()
    .map(|set| set.name.clone())
    .chain([BACKUP.names.large_files.clone()])
    .collect()
}
