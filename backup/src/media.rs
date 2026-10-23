//! The weekly media (docs/backup.md, 「How the media run runs」): each media set, and the large files the
//! hourly archive leaves out, packed as one archive under a name that stays the same, sent to every
//! destination that takes media, and removed locally once sent.

use std::path::PathBuf;
use std::time::Instant;

use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_deploy::remove::remove_tree;

use crate::outcome::Outcome;
use crate::pack::{Walk, digest, pack};
use crate::policy::BACKUP;
use crate::settings::{Plan, Set};
use crate::snapshot::{Say, fresh, has_room, seconds_since};

/// Each media archive's name and the paths under the root it holds: the declared sets in order, then the
/// large files of the hourly folders.
pub fn media_sets(plan: &Plan) -> Vec<(String, Vec<PathBuf>)> {
  let mut sets: Vec<(String, Vec<PathBuf>)> = plan
    .media
    .iter()
    .map(|set| {
      let walked = Walk {
        root: &plan.root,
        exclude: &plan.exclude,
        apart: &[],
        large_over: None,
      }
      .over(&set.folders);

      (set.name.clone(), walked.held)
    })
    .collect();
  let apart: Vec<PathBuf> = plan.media.iter().flat_map(|set| set.folders.clone()).collect();
  let large = Walk {
    root: &plan.root,
    exclude: &plan.exclude,
    apart: &apart,
    large_over: Some(plan.large_file_bytes),
  }
  .over(&plan.folders)
  .large;

  sets.push((BACKUP.names.large_files.clone(), large));
  sets
}

/// The media packed and sent, each archive removed locally after.
pub fn media(plan: &Plan, say: Say<'_>) -> Outcome {
  let names = &BACKUP.names;
  let started = Instant::now();
  let mut outcome = Outcome::default();
  let dir = plan.staging.join(&names.media);

  if let Err(why) = fresh(plan, &dir) {
    say(&why);
    outcome.fail("stage", &dir.display().to_string(), why);
    return outcome;
  }

  for (name, paths) in media_sets(plan) {
    if paths.is_empty() {
      continue;
    }

    let file = format!("{name}{}", names.archive_suffix);
    let archive = dir.join(&file);
    let packed = Instant::now();
    let read = pack(&plan.root, &paths, &archive).and_then(|()| digest(&archive));
    let (size, sha256) = match read {
      Ok(read) => read,
      Err(why) => {
        say(&why);
        outcome.fail("pack", &name, why);
        continue;
      }
    };

    say(&fill_shown(
      &BACKUP.words.packed_media,
      &[("name", &name), ("bytes", &size), ("seconds", &seconds_since(packed))],
    ));

    let key = plan.key(&[&names.media, &file]);
    let folder = format!("{}/", plan.key(&[&names.media]));

    for destination in plan.destinations.iter().filter(|one| one.takes(Set::Media)) {
      if !has_room(plan, destination, size, say, &mut outcome) {
        continue;
      }

      let sending = Instant::now();
      let sent = destination.store.put(&archive, &key, &sha256).and_then(|()| {
        let listed = destination.store.list(&folder)?;

        match listed.iter().find(|object| object.key == key) {
          Some(object) if object.size == size => Ok(()),
          Some(object) => Err(fill_shown(
            &BACKUP.words.size_mismatch,
            &[("key", &key), ("there", &object.size), ("here", &size)],
          )),
          None => Err(fill_shown(&BACKUP.words.not_listed, &[("key", &key)])),
        }
      });

      match sent {
        Ok(()) => say(&fill_shown(
          &BACKUP.words.sent,
          &[
            ("destination", &destination.name),
            ("key", &key),
            ("bytes", &size),
            ("seconds", &seconds_since(sending)),
          ],
        )),
        Err(why) => {
          say(&why);
          outcome.fail("upload", &format!("{}: {key}", destination.name), why);
        }
      }
    }

    if let Err(why) = remove_tree(&archive, &plan.root) {
      outcome.fail("remove", &archive.display().to_string(), why);
    }
  }

  say(&fill_shown(
    &BACKUP.words.media_done,
    &[("failed", &outcome.failed.len()), ("seconds", &seconds_since(started))],
  ));
  outcome
}
