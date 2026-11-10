//! The Cargo profile check (docs/checks.md, 「The quick checks」): the repository's workspace manifest holds
//! every profile setting the policy says keeps each build's output small. A repository with no workspace
//! manifest builds nothing with Cargo and has nothing to hold.

use std::path::Path;

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;

use super::Findings;

// A setting's value as the manifest writes it: a string quoted, a boolean or a number bare.
fn shown(value: &toml::Value) -> String {
  serde_json::to_string(value).unwrap_or_default()
}

/// The manifest at the root of the checkout at `root` held to the wanted settings: a line for each setting
/// it leaves out or sets otherwise, and how many settings it read.
pub fn checkout_cargo_profile(root: &Path) -> Findings {
  let words = &QUICK_CHECKS.cargo_profile;
  let Ok(text) = std::fs::read_to_string(root.join(&words.manifest)) else {
    return Findings::default();
  };
  let manifest = match toml::from_str::<toml::Table>(&text) {
    Ok(manifest) => manifest,
    Err(why) => {
      return Findings {
        paths: 0,
        findings: vec![fill_marks(
          &words.not_manifest,
          &[("manifest", &words.manifest), ("why", &why.to_string())],
        )],
      };
    }
  };
  let mut findings = Vec::new();
  let mut read = 0;

  for (profile, settings) in &words.wanted {
    let standing = manifest.get("profile").and_then(|profiles| profiles.get(profile));

    for (key, wanted) in settings {
      read += 1;

      let wanted_shown = shown(wanted);
      let at = [
        ("manifest", words.manifest.as_str()),
        ("profile", profile),
        ("key", key),
        ("wanted", &wanted_shown),
      ];

      // A key with dots in it is a path through the profile's tables, `package.*.debug` among them.
      match standing.and_then(|one| key.split('.').try_fold(one, |at, part| at.get(part))) {
        None => findings.push(fill_marks(&words.missing, &at)),
        Some(found) if found != wanted => {
          let found = shown(found);

          findings.push(fill_marks(&words.differs, &[at.as_slice(), &[("found", &found)]].concat()));
        }
        Some(_) => {}
      }
    }
  }

  Findings {
    paths: read,
    findings,
  }
}
