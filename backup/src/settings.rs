//! The settings file (docs/backup.md, 「The settings」): what the machine backs up, where, and the numbers it
//! rotates by, read into a plan with the policy's defaults where the file names none.

use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::fill_shown;
use glob::Pattern;
use serde::Deserialize;

use crate::destination::{Rclone, S3, Store};
use crate::policy::BACKUP;

/// The sets a destination may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Set {
  Hourly,
  Daily,
  Media,
  Releases,
}

/// How a destination is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
  S3,
  Rclone,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSettings {
  pub name: String,
  pub folders: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSettings {
  #[serde(default)]
  pub folders: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationSettings {
  pub name: String,
  pub kind: Kind,
  pub sets: Option<Vec<Set>>,
  pub room: Option<String>,
  pub command: Option<String>,
  pub bucket: Option<String>,
  pub profile: Option<String>,
  pub endpoint: Option<String>,
  pub remote: Option<String>,
}

/// The file as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
  pub root: String,
  pub folders: Vec<String>,
  #[serde(default)]
  pub exclude: Vec<String>,
  pub large_file_bytes: Option<u64>,
  pub databases: Vec<String>,
  #[serde(default)]
  pub exclude_databases: Vec<String>,
  pub server: Option<String>,
  #[serde(default)]
  pub address_files: std::collections::BTreeMap<String, String>,
  pub staging: Option<String>,
  pub prefix: Option<String>,
  pub hourly_slots: Option<u64>,
  pub daily_days: Option<u64>,
  pub releases_kept: Option<usize>,
  pub drill_max_age_hours: Option<u64>,
  pub room_floor_bytes: Option<u64>,
  #[serde(default)]
  pub media: Vec<MediaSettings>,
  #[serde(default)]
  pub releases: ReleaseSettings,
  #[serde(default)]
  pub destination: Vec<DestinationSettings>,
}

/// A media set: its name and its folders under the root.
#[derive(Debug, Clone)]
pub struct MediaSet {
  pub name: String,
  pub folders: Vec<PathBuf>,
}

/// One destination: its name, the sets it takes, the disk its store writes to, and its store.
pub struct Destination {
  pub name: String,
  pub sets: Vec<Set>,
  pub room: Option<PathBuf>,
  pub store: Box<dyn Store>,
}

impl Destination {
  pub fn takes(&self, set: Set) -> bool {
    self.sets.contains(&set)
  }
}

/// The settings with every default filled in, every path made absolute and every glob read.
pub struct Plan {
  pub root: PathBuf,
  pub folders: Vec<PathBuf>,
  pub exclude: Vec<Pattern>,
  pub large_file_bytes: u64,
  pub databases: Vec<Pattern>,
  pub exclude_databases: Vec<Pattern>,
  pub server: String,
  pub address_files: std::collections::BTreeMap<String, PathBuf>,
  pub staging: PathBuf,
  pub prefix: String,
  pub hourly_slots: u64,
  pub daily_days: u64,
  pub releases_kept: usize,
  pub drill_max_age_hours: u64,
  pub room_floor_bytes: u64,
  pub media: Vec<MediaSet>,
  pub release_folders: Vec<String>,
  pub destinations: Vec<Destination>,
}

/// Where the settings file stands under a data root.
pub fn settings_path(data_root: &Path) -> PathBuf {
  data_root.join("env").join(&BACKUP.names.settings_file)
}

/// A path as the settings write it: `~` the home directory.
pub fn expanded(path: &str, home: &Path) -> PathBuf {
  match path.strip_prefix('~') {
    Some(rest) if rest.is_empty() || rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
    _ => PathBuf::from(path),
  }
}

fn wrong(why: &dyn std::fmt::Display) -> String {
  fill_shown(&BACKUP.words.settings_wrong, &[("why", why)])
}

fn patterns(globs: &[String]) -> Result<Vec<Pattern>, String> {
  globs
    .iter()
    .map(|one| Pattern::new(one).map_err(|why| wrong(&format!("{one}: {why}"))))
    .collect()
}

// A name that stands as one part of a key: not empty, no separator, no dot ahead.
fn one_part(name: &str) -> bool {
  !name.is_empty() && !name.contains('/') && !name.starts_with('.')
}

impl DestinationSettings {
  fn store(&self) -> Result<Box<dyn Store>, String> {
    let missing = |key: &str| wrong(&format!("destination {}: {key} is required for its kind", self.name));

    Ok(match self.kind {
      Kind::S3 => Box::new(S3 {
        command: self.command.clone().unwrap_or_else(|| BACKUP.defaults.s3_command.clone()),
        bucket: self.bucket.clone().ok_or_else(|| missing("bucket"))?,
        profile: self.profile.clone(),
        endpoint: self.endpoint.clone(),
      }),
      Kind::Rclone => Box::new(Rclone {
        command: self.command.clone().unwrap_or_else(|| BACKUP.defaults.rclone_command.clone()),
        remote: self.remote.clone().ok_or_else(|| missing("remote"))?,
      }),
    })
  }
}

impl Plan {
  /// The settings file read and filled in; why it does not read, or reads wrong, where it does not.
  pub fn read(path: &Path, home: &Path) -> Result<Self, String> {
    let text = std::fs::read_to_string(path).map_err(|why| {
      fill_shown(&BACKUP.words.settings_unread, &[("path", &path.display()), ("why", &why)])
    })?;
    let settings: Settings = toml::from_str(&text).map_err(|why| {
      fill_shown(&BACKUP.words.settings_unread, &[("path", &path.display()), ("why", &why.message())])
    })?;

    Self::of(settings, home)
  }

  /// The plan of settings already read.
  pub fn of(settings: Settings, home: &Path) -> Result<Self, String> {
    let defaults = &BACKUP.defaults;
    let root = expanded(&settings.root, home);
    let under = |paths: &[String]| paths.iter().map(|one| root.join(one)).collect::<Vec<_>>();

    if settings.destination.is_empty() {
      return Err(wrong(&"no [[destination]] is named"));
    }

    if settings.hourly_slots == Some(0) {
      return Err(wrong(&"hourly_slots is 0"));
    }

    let mut destinations: Vec<Destination> = Vec::new();

    for one in &settings.destination {
      if !one_part(&one.name) || destinations.iter().any(|other| other.name == one.name) {
        return Err(wrong(&format!(
          "destination name {:?} is empty, holds a slash or is named twice",
          one.name
        )));
      }

      destinations.push(Destination {
        name: one.name.clone(),
        sets: one
          .sets
          .clone()
          .unwrap_or_else(|| vec![Set::Hourly, Set::Daily, Set::Media, Set::Releases]),
        room: one.room.as_deref().map(|room| expanded(room, home)),
        store: one.store()?,
      });
    }

    let mut media: Vec<MediaSet> = Vec::new();

    for set in &settings.media {
      if !one_part(&set.name)
        || set.name == BACKUP.names.large_files
        || media.iter().any(|other| other.name == set.name)
      {
        return Err(wrong(&format!(
          "media name {:?} is empty, holds a slash, is taken or is named twice",
          set.name
        )));
      }

      media.push(MediaSet {
        name: set.name.clone(),
        folders: under(&set.folders),
      });
    }

    let prefix = settings.prefix.unwrap_or_else(|| defaults.prefix.clone());
    let prefix = prefix.trim_matches('/').to_owned();

    if prefix.is_empty() {
      return Err(wrong(&"prefix is empty, and nothing at a destination may be rotated outside one"));
    }

    Ok(Self {
      folders: under(&settings.folders),
      exclude: patterns(&settings.exclude)?,
      large_file_bytes: settings.large_file_bytes.unwrap_or(defaults.large_file_bytes),
      databases: patterns(&settings.databases)?,
      exclude_databases: patterns(&settings.exclude_databases)?,
      server: settings.server.unwrap_or_else(|| defaults.server.clone()),
      address_files: settings
        .address_files
        .iter()
        .map(|(database, file)| (database.clone(), expanded(file, home)))
        .collect(),
      staging: expanded(settings.staging.as_deref().unwrap_or(&defaults.staging), home),
      prefix,
      hourly_slots: settings.hourly_slots.unwrap_or(defaults.hourly_slots),
      daily_days: settings.daily_days.unwrap_or(defaults.daily_days),
      releases_kept: settings.releases_kept.unwrap_or(defaults.releases_kept),
      drill_max_age_hours: settings.drill_max_age_hours.unwrap_or(defaults.drill_max_age_hours),
      room_floor_bytes: settings.room_floor_bytes.unwrap_or(defaults.room_floor_bytes),
      media,
      release_folders: settings
        .releases
        .folders
        .iter()
        .map(|one| expanded(one, home).to_string_lossy().into_owned())
        .collect(),
      destinations,
      root,
    })
  }

  /// The key of a part under the prefix.
  pub fn key(&self, parts: &[&str]) -> String {
    std::iter::once(self.prefix.as_str())
      .chain(parts.iter().copied())
      .collect::<Vec<_>>()
      .join("/")
  }

  /// The address a database is dumped through: what its address file holds, or the server's address with
  /// its name in place of the listing database's.
  pub fn address_of(&self, database: &str) -> Result<String, String> {
    match self.address_files.get(database) {
      Some(file) => std::fs::read_to_string(file)
        .map(|text| text.trim().to_owned())
        .map_err(|why| format!("{}: {why}", file.display())),
      None => crate::dump::address_in(&self.server, database),
    }
  }

  /// Whether a database is one to dump: a `databases` glob matches it, no `exclude_databases` glob does,
  /// and it is none of the drill's scratch databases.
  pub fn dumps(&self, database: &str) -> bool {
    self.databases.iter().any(|one| one.matches(database))
      && !self.exclude_databases.iter().any(|one| one.matches(database))
      && !database.starts_with(&BACKUP.names.drill_database)
  }
}
