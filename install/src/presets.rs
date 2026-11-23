//! The presets file (docs/install.md, 「The presets file」): every repository of the set with the folder it is
//! cloned into, and the presets, each naming what it checks out and what it installs, in order.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

/// The file's name at the index checkout's root.
pub const FILE: &str = "presets.toml";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetsFile {
  pub presets: u64,
  #[serde(default)]
  pub repository: Vec<RepositoryEntry>,
  #[serde(default)]
  pub preset: BTreeMap<String, Preset>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryEntry {
  pub name: String,
  pub folder: String,
  pub url: String,
  #[serde(default)]
  pub features: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
  pub summary: String,
  pub extends: Option<String>,
  #[serde(default)]
  pub clone: Vec<String>,
  #[serde(default)]
  pub units: Vec<String>,
  #[serde(default)]
  pub choose: Vec<Choice>,
  #[serde(default)]
  pub features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Choice {
  pub what: String,
  pub units: Vec<String>,
}

/// Why the file or a preset of it cannot be used.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
  Unread(String),
  NoPreset { preset: String, presets: Vec<String> },
  NoRepository(String),
  ExtendsRing(String),
  ChoiceMany { what: String, units: Vec<String> },
}

/// A preset resolved through what it extends.
#[derive(Debug, PartialEq, Eq)]
pub struct Resolved {
  pub preset: String,
  /// The repositories checked out and not installed.
  pub clone: Vec<String>,
  /// The repositories installed, in the order they are deployed.
  pub units: Vec<String>,
  /// The choices `--with` did not settle.
  pub open: Vec<Choice>,
  /// The console features the preset and its repositories turn on.
  pub features: BTreeSet<String>,
}

impl PresetsFile {
  /// The file read from the index checkout, every repository a preset names held to stand among its entries.
  pub fn read(index: &Path) -> Result<Self, Refused> {
    let text = std::fs::read_to_string(index.join(FILE)).map_err(|why| Refused::Unread(why.to_string()))?;

    Self::parse(&text)
  }

  pub fn parse(text: &str) -> Result<Self, Refused> {
    let file: Self = toml::from_str(text).map_err(|why| Refused::Unread(why.to_string()))?;

    for preset in file.preset.values() {
      let named = preset
        .clone
        .iter()
        .chain(&preset.units)
        .chain(preset.choose.iter().flat_map(|choice| &choice.units));

      for name in named {
        if file.repository(name).is_none() {
          return Err(Refused::NoRepository(name.clone()));
        }
      }
    }

    Ok(file)
  }

  pub fn repository(&self, name: &str) -> Option<&RepositoryEntry> {
    self.repository.iter().find(|entry| entry.name == name)
  }

  /// Every console feature any preset or repository of the file names.
  pub fn every_feature(&self) -> BTreeSet<String> {
    self
      .preset
      .values()
      .flat_map(|preset| preset.features.iter())
      .chain(self.repository.iter().flat_map(|entry| entry.features.iter()))
      .cloned()
      .collect()
  }

  // The preset and every preset it extends, the farthest first.
  fn chain(&self, name: &str) -> Result<Vec<(&str, &Preset)>, Refused> {
    let mut chain: Vec<(&str, &Preset)> = Vec::new();
    let mut at = Some(name.to_owned());

    while let Some(named) = at {
      if chain.iter().any(|(seen, _)| *seen == named) {
        return Err(Refused::ExtendsRing(name.to_owned()));
      }

      let (key, preset) = self.preset.get_key_value(&named).ok_or_else(|| Refused::NoPreset {
        preset: named.clone(),
        presets: self.preset.keys().cloned().collect(),
      })?;

      chain.push((key.as_str(), preset));
      at = preset.extends.clone();
    }

    chain.reverse();
    Ok(chain)
  }

  /// The preset resolved: the lists of the presets it extends before its own, each repository once at its first
  /// place, and each choice settled by `with`, a choice whose every repository a later preset installs anyway
  /// being settled already.
  pub fn resolve(&self, name: &str, with: &[String]) -> Result<Resolved, Refused> {
    let chain = self.chain(name)?;
    let mut clone: Vec<String> = Vec::new();
    let mut units: Vec<String> = Vec::new();
    let mut features: BTreeSet<String> = BTreeSet::new();
    let mut dropped: BTreeSet<String> = BTreeSet::new();
    let mut open = Vec::new();

    for (_, preset) in &chain {
      for repository in &preset.clone {
        if !clone.contains(repository) {
          clone.push(repository.clone());
        }
      }

      for repository in &preset.units {
        if !units.contains(repository) {
          units.push(repository.clone());
        }
      }

      features.extend(preset.features.iter().cloned());
    }

    for (at, (_, preset)) in chain.iter().enumerate() {
      for choice in &preset.choose {
        let later: BTreeSet<&String> =
          chain[at + 1..].iter().flat_map(|(_, later)| later.units.iter()).collect();

        if choice.units.iter().all(|unit| later.contains(unit)) {
          continue;
        }

        let chosen: Vec<&String> = choice.units.iter().filter(|unit| with.contains(unit)).collect();

        match chosen.len() {
          0 => open.push(choice.clone()),
          1 => dropped.extend(
            choice.units.iter().filter(|unit| *unit != chosen[0] && !later.contains(unit)).cloned(),
          ),
          _ => {
            return Err(Refused::ChoiceMany {
              what: choice.what.clone(),
              units: chosen.into_iter().cloned().collect(),
            });
          }
        }
      }
    }

    units.retain(|unit| !dropped.contains(unit));

    if !open.is_empty() {
      let undecided: BTreeSet<&String> =
        open.iter().flat_map(|choice: &Choice| choice.units.iter()).collect();

      units.retain(|unit| !undecided.contains(unit));
    }

    for unit in &units {
      if let Some(entry) = self.repository(unit) {
        features.extend(entry.features.iter().cloned());
      }
    }

    clone.retain(|repository| !units.contains(repository));

    Ok(Resolved {
      preset: name.to_owned(),
      clone,
      units,
      open,
      features,
    })
  }
}
