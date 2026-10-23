//! The restore drill (docs/backup.md, 「The restore drill」): the newest snapshot any destination holds,
//! downloaded and held to its manifest, each dump restored into a scratch database and its rows counted
//! against the manifest's, and the data archive read through and counted.

use std::path::Path;
use std::time::Instant;

use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_deploy::remove::remove_tree;
use chrono::{DateTime, Local};

use crate::dump::{restore_and_count, sweep_scratch};
use crate::manifest::{Manifest, Tables};
use crate::outcome::Outcome;
use crate::pack::{digest, entries};
use crate::policy::BACKUP;
use crate::settings::{Plan, Set};
use crate::snapshot::{Say, fresh, seconds_since};

/// The newest snapshot: the destination holding it, the key it stands under, and its manifest.
pub struct Newest {
  pub destination: usize,
  pub base: String,
  pub manifest: Manifest,
}

/// Every destination's hourly manifests read, the newest kept, the first destination in order winning a
/// tie; each destination that cannot be read is failed and passed over.
pub fn newest(plan: &Plan, dir: &Path, say: Say<'_>, outcome: &mut Outcome) -> Option<Newest> {
  let names = &BACKUP.names;
  let mut found: Option<Newest> = None;

  for (index, destination) in plan.destinations.iter().enumerate() {
    if !destination.takes(Set::Hourly) {
      continue;
    }

    let listed = match destination.store.list(&format!("{}/", plan.key(&[&names.hourly]))) {
      Ok(listed) => listed,
      Err(why) => {
        say(&why);
        outcome.fail("list", &destination.name, why);
        continue;
      }
    };

    for object in listed {
      let Some(base) = object.key.strip_suffix(&format!("/{}", names.manifest)) else {
        continue;
      };
      let local = dir.join(format!("{index}-{}", base.replace('/', "-")));
      let read = destination.store.get(&object.key, &local).and_then(|()| {
        let text = std::fs::read_to_string(&local).map_err(|why| why.to_string())?;

        serde_json::from_str::<Manifest>(&text).map_err(|why| why.to_string())
      });

      match read {
        Ok(manifest)
          if found.as_ref().is_none_or(|newest| manifest.taken_ms > newest.manifest.taken_ms) =>
        {
          found = Some(Newest {
            destination: index,
            base: base.to_owned(),
            manifest,
          });
        }
        Ok(_) => {}
        Err(why) => {
          let line = fill_shown(&BACKUP.words.manifest_unread, &[("key", &object.key), ("why", &why)]);

          say(&line);
          outcome.fail("manifest", &destination.name, line);
        }
      }
    }
  }

  found
}

/// Where the restored rows differ from the manifest's: each table missing, extra or counted otherwise.
pub fn differences(database: &str, expected: &Tables, restored: &Tables) -> Vec<String> {
  let words = &BACKUP.words;
  let mut differ = Vec::new();

  for (table, rows) in expected {
    match restored.get(table) {
      None => differ.push(fill_shown(&words.table_missing, &[("database", &database), ("table", table)])),
      Some(back) if back != rows => differ.push(fill_shown(
        &words.count_mismatch,
        &[("database", &database), ("table", table), ("expected", rows), ("restored", back)],
      )),
      Some(_) => {}
    }
  }

  for table in restored.keys().filter(|table| !expected.contains_key(*table)) {
    differ.push(fill_shown(&words.table_extra, &[("database", &database), ("table", table)]));
  }

  differ
}

/// The drill, run once: 0 failures where everything matched.
pub fn drill(plan: &Plan, now: DateTime<Local>, say: Say<'_>) -> Outcome {
  let words = &BACKUP.words;
  let started = Instant::now();
  let mut outcome = Outcome::default();
  let dir = plan.staging.join(&BACKUP.names.drill);

  if let Err(why) = fresh(plan, &dir) {
    say(&why);
    outcome.fail("stage", &dir.display().to_string(), why);
    return outcome;
  }

  let Some(newest) = newest(plan, &dir, say, &mut outcome) else {
    say(&words.no_snapshot);
    outcome.fail("find", "", words.no_snapshot.clone());
    let _ = remove_tree(&dir, &plan.root);
    return outcome;
  };
  let destination = &plan.destinations[newest.destination];
  let manifest = &newest.manifest;
  let age_ms = now.timestamp_millis() - manifest.taken_ms;
  let most_ms = i64::try_from(plan.drill_max_age_hours.saturating_mul(3_600_000)).unwrap_or(i64::MAX);

  say(&fill_shown(
    &words.drill_from,
    &[
      ("destination", &destination.name),
      ("key", &newest.base),
      ("taken_at", &manifest.taken_at),
    ],
  ));

  if age_ms > most_ms {
    let line = fill_shown(
      &words.too_old,
      &[("taken_at", &manifest.taken_at), ("hours", &plan.drill_max_age_hours)],
    );

    say(&line);
    outcome.fail("age", &newest.base, line);
  }

  let mut whole = Vec::new();

  for file in &manifest.files {
    let local = dir.join(&file.name);
    let read = destination
      .store
      .get(&format!("{}/{}", newest.base, file.name), &local)
      .and_then(|()| digest(&local));

    match read {
      Ok((size, sha256)) if size == file.size && sha256 == file.sha256 => whole.push(file.name.clone()),
      Ok(_) => {
        let line = fill_shown(&words.sha_mismatch, &[("file", &file.name)]);

        say(&line);
        outcome.fail("download", &file.name, line);
      }
      Err(why) => {
        say(&why);
        outcome.fail("download", &file.name, why);
      }
    }
  }

  let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();

  if let Ok(runtime) = &runtime {
    match runtime.block_on(sweep_scratch(&plan.server)) {
      Ok(left) => {
        for name in left {
          let line = fill_shown(&words.not_dropped, &[("database", &name), ("why", &"it still stands")]);

          say(&line);
          outcome.fail("drop", &name, line);
        }
      }
      Err(why) => {
        say(&why);
        outcome.fail("sweep", &plan.server, why);
      }
    }
  }

  for (index, database) in manifest.databases.iter().enumerate() {
    if !whole.contains(&database.file) {
      continue;
    }

    let Ok(runtime) = &runtime else {
      outcome.fail("restore", &database.name, "no runtime");
      continue;
    };
    let restoring = Instant::now();
    let scratch = format!("{}{}_{index}", BACKUP.names.drill_database, std::process::id());

    match runtime.block_on(restore_and_count(&plan.server, &scratch, &dir.join(&database.file))) {
      Ok(restored) => {
        if let Some(left) = restored.left {
          say(&left);
          outcome.fail("drop", &scratch, left);
        }

        if let Some(complaint) = restored.complaint {
          say(&complaint);
          outcome.fail("restore", &database.name, complaint);
        }

        for line in differences(&database.name, &database.tables, &restored.tables) {
          say(&line);
          outcome.fail("count", &database.name, line);
        }

        say(&fill_shown(
          &words.drill_database,
          &[
            ("database", &database.name),
            ("tables", &restored.tables.len()),
            ("rows", &restored.tables.values().sum::<i64>()),
            ("seconds", &seconds_since(restoring)),
          ],
        ));
      }
      Err(why) => {
        say(&why);
        outcome.fail("restore", &database.name, why);
      }
    }
  }

  if let Some(data) = manifest.data.as_ref().filter(|data| whole.contains(&data.file)) {
    match entries(&dir.join(&data.file)) {
      Ok(count) if count == data.entries => {
        say(&fill_shown(&words.drill_data, &[("file", &data.file), ("entries", &count)]));
      }
      Ok(count) => {
        let line = fill_shown(
          &words.entries_mismatch,
          &[("file", &data.file), ("expected", &data.entries), ("read", &count)],
        );

        say(&line);
        outcome.fail("data", &data.file, line);
      }
      Err(why) => {
        say(&why);
        outcome.fail("data", &data.file, why);
      }
    }
  }

  if let Err(why) = remove_tree(&dir, &plan.root) {
    outcome.fail("remove", &dir.display().to_string(), why);
  }

  let verdict = if outcome.passed() { &words.drill_passed } else { &words.drill_failed };

  say(&fill_shown(
    verdict,
    &[("failed", &outcome.failed.len()), ("seconds", &seconds_since(started))],
  ));
  outcome
}
