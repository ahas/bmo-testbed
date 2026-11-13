//! The rotation (docs/backup.md, 「What is removed」): which keys at a destination a run removes, read off
//! their shapes under the prefix alone, and the removal itself, or what it would remove.

use std::collections::BTreeMap;

use bmo_testbed_core::policy::fill_shown;
use chrono::NaiveDate;

use crate::destination::Object;
use crate::outcome::Outcome;
use crate::policy::BACKUP;
use crate::settings::Plan;
use crate::snapshot::Say;

/// What the rotation keeps.
pub struct Rotation<'a> {
  pub prefix: &'a str,
  pub hourly_slots: u64,
  pub today: NaiveDate,
  pub daily_days: u64,
  pub releases_kept: usize,
  /// The media archives' names, the large files' among them.
  pub media: &'a [String],
}

impl Rotation<'_> {
  /// The rotation a plan keeps, as of a day.
  pub fn of<'a>(plan: &'a Plan, today: NaiveDate, media: &'a [String]) -> Rotation<'a> {
    Rotation {
      prefix: &plan.prefix,
      hourly_slots: plan.hourly_slots,
      today,
      daily_days: plan.daily_days,
      releases_kept: plan.releases_kept,
      media,
    }
  }

  /// The keys among the objects the rotation removes. A key outside the prefix, or of a shape the rotation
  /// does not name, is never among them.
  pub fn doomed(&self, objects: &[Object]) -> Vec<String> {
    let names = &BACKUP.names;
    let mut doomed = Vec::new();
    let mut releases: BTreeMap<&str, Vec<&str>> = BTreeMap::new();

    for object in objects {
      let Some(relative) = object.key.strip_prefix(self.prefix).and_then(|rest| rest.strip_prefix('/'))
      else {
        continue;
      };
      let parts: Vec<&str> = relative.split('/').collect();

      match parts.as_slice() {
        [set, slot, _, ..] if *set == names.hourly => {
          if slot.parse::<u64>().is_ok_and(|slot| slot >= self.hourly_slots) {
            doomed.push(object.key.clone());
          }
        }
        [set, day, _, ..] if *set == names.daily => {
          let old = NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok_and(|day| {
            (self.today - day).num_days() >= i64::try_from(self.daily_days).unwrap_or(i64::MAX)
          });

          if old {
            doomed.push(object.key.clone());
          }
        }
        [set, folder, archive] if *set == names.releases && archive.ends_with(&names.archive_suffix) => {
          releases.entry(folder).or_default().push(&object.key);
        }
        [set, archive] if *set == names.media => {
          let undeclared = archive
            .strip_suffix(&names.archive_suffix)
            .is_some_and(|name| !self.media.iter().any(|declared| declared == name));

          if undeclared {
            doomed.push(object.key.clone());
          }
        }
        _ => {}
      }
    }

    for (_, mut keys) in releases {
      // An archive's name carries the time it was taken, so the names sort as their times do.
      keys.sort_unstable_by(|one, other| other.cmp(one));
      doomed.extend(keys.into_iter().skip(self.releases_kept).map(str::to_owned));
    }

    doomed.sort();
    doomed
  }
}

/// The rotation at every destination: what it removes, or with `dry_run` what it would, each said.
pub fn rotate(plan: &Plan, rotation: &Rotation<'_>, dry_run: bool, say: Say<'_>) -> Outcome {
  let words = &BACKUP.words;
  let mut outcome = Outcome::default();
  let mut any = false;

  for destination in &plan.destinations {
    let objects = match destination.store.list(&format!("{}/", plan.prefix)) {
      Ok(objects) => objects,
      Err(why) => {
        say.failure(&why);
        outcome.fail("list", &destination.name, why);
        continue;
      }
    };

    for key in rotation.doomed(&objects) {
      any = true;

      let marks: [(&str, &dyn std::fmt::Display); 2] = [("destination", &destination.name), ("key", &key)];

      if dry_run {
        say.line(&fill_shown(&words.would_remove, &marks));
        continue;
      }

      match destination.store.remove(&key) {
        Ok(()) => say.line(&fill_shown(&words.removed, &marks)),
        Err(why) => {
          say.failure(&why);
          outcome.fail("remove", &format!("{}: {key}", destination.name), why);
        }
      }
    }
  }

  if !any {
    say.line(&words.nothing_to_remove);
  }

  outcome
}
