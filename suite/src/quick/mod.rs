//! The quick checks of the static check (docs/checks.md, 「The quick checks」): the steps a change forgets
//! rather than the behaviour it breaks, read off the tree of a checkout without running any of it. Each
//! runs where the repository's declaration enables it and reads the paths the declaration names; each
//! answers with what it read and a line per fault, which `report` writes the way the static check prints
//! it.

use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::{QuickCheck, Repository};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::quick_checks::QUICK_CHECKS;
use bmo_testbed_core::turn::Env;

pub mod cargo_profile;
pub mod dashes;
pub mod dependencies;
pub mod files;
pub mod language;
pub mod layout;
pub mod shared_versions;
pub mod source;
pub mod wording;

/// What a check read and the faults it found, a line each.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Findings {
  pub paths: usize,
  pub findings: Vec<String>,
}

/// One check's lines as the static check prints them: a line naming the check and what it found, then a
/// line for each fault under it.
pub fn report(name: &str, faults: &[String], clean: &str) -> Vec<String> {
  let said = if faults.is_empty() {
    clean.to_owned()
  } else {
    fill_marks(&QUICK_CHECKS.to_fix, &[("count", &faults.len().to_string())])
  };
  let head = fill_marks(&QUICK_CHECKS.head, &[("name", name), ("said", &said)]);

  std::iter::once(head).chain(faults.iter().map(|fault| format!("  {fault}"))).collect()
}

/// One check run over a checkout: its lines as the static check prints them, and whether it found a
/// fault.
#[derive(Debug, PartialEq, Eq)]
pub struct Checked {
  pub lines: Vec<String>,
  pub faulted: bool,
}

impl Checked {
  fn of(name: &str, faults: &[String], clean: &str) -> Self {
    Self {
      lines: report(name, faults, clean),
      faulted: !faults.is_empty(),
    }
  }
}

fn counted(line: &str, count: usize) -> String {
  fill_marks(line, &[("count", &count.to_string())])
}

// The input a check reads, as the declaration names it, where it stands and the check's name with it;
// a check whose input the declaration does not name is the fault it answers with.
fn declared_input(check: QuickCheck, repo: &Repository, env: &Env) -> Result<(PathBuf, String), Checked> {
  match check.input(&repo.declaration.check) {
    Some((_, Some(declared))) => {
      let named = declared.to_string_lossy();
      let name = fill_marks(&QUICK_CHECKS.named, &[("name", check.name()), ("input", &named)]);

      Ok((repo.path(declared, env), name))
    }
    Some((input, None)) => {
      let fault = fill_marks(&QUICK_CHECKS.no_input, &[("input", input)]);

      Err(Checked::of(check.name(), &[fault], ""))
    }
    None => Ok((repo.root.clone(), check.name().to_owned())),
  }
}

/// The wording check over the checkout, held to the declared document.
pub fn wording_check(repo: &Repository, env: &Env) -> Checked {
  let (document, name) = match declared_input(QuickCheck::Wording, repo, env) {
    Ok(input) => input,
    Err(checked) => return checked,
  };
  let named = repo.declaration.check.wording_document.as_deref().unwrap_or(Path::new(""));
  let files = repo.declaration.check.wording_files.as_deref();
  let (read, faults) =
    match wording::checkout_wording(&repo.root, &document, &named.to_string_lossy(), files) {
      Ok(found) => (found.paths, found.findings),
      Err(why) => (0, vec![why]),
    };

  Checked::of(&name, &faults, &counted(&QUICK_CHECKS.wording.clean, read))
}

/// The dash check over the checkout's declared people's text.
pub fn dashes_check(repo: &Repository) -> Checked {
  let found = dashes::checkout_written_dashes(&repo.root, &repo.declaration.check.people_text);

  Checked::of(
    QuickCheck::Dashes.name(),
    &found.findings,
    &counted(&QUICK_CHECKS.dashes.clean, found.paths),
  )
}

/// The lines of the checkout's code that are not English, held to the declared ledger.
pub fn language_check(repo: &Repository, env: &Env) -> Checked {
  let words = &QUICK_CHECKS.language;
  let (ledger, name) = match declared_input(QuickCheck::Language, repo, env) {
    Ok(input) => input,
    Err(checked) => return checked,
  };
  let named = repo.declaration.check.language_ledger.as_deref().unwrap_or(Path::new(""));
  let (faults, clean) =
    match language::checkout_language_in_code(&repo.root, &ledger, &named.to_string_lossy()) {
      Ok(found) => {
        let clean = if found.below == 0 {
          counted(&words.clean, found.files)
        } else {
          fill_marks(
            &words.below,
            &[("count", &found.files.to_string()), ("below", &found.below.to_string())],
          )
        };

        (found.findings, clean)
      }
      Err(why) => (vec![why], String::new()),
    };

  Checked::of(&name, &faults, &clean)
}

/// The layout check over the checkout.
pub fn layout_check(repo: &Repository) -> Checked {
  let found = layout::checkout_layout(repo);

  Checked::of(
    QuickCheck::Layout.name(),
    &found.findings,
    &counted(&QUICK_CHECKS.layout.clean, found.paths),
  )
}

/// The Cargo profile check over the checkout's workspace manifest.
pub fn cargo_profile_check(repo: &Repository) -> Checked {
  let found = cargo_profile::checkout_cargo_profile(&repo.root);

  Checked::of(
    QuickCheck::CargoProfile.name(),
    &found.findings,
    &counted(&QUICK_CHECKS.cargo_profile.clean, found.paths),
  )
}

/// The shared versions check over the checkout's manifests and locks, against the shared versions its
/// declaration names.
pub fn shared_versions_check(repo: &Repository, env: &Env) -> Checked {
  let found = shared_versions::checkout_shared_versions(repo, env);

  Checked::of(
    QuickCheck::SharedVersions.name(),
    &found.findings,
    &counted(&QUICK_CHECKS.shared_versions.clean, found.paths),
  )
}

/// The dependency check over the checkout's manifests, against the declared base libraries.
pub fn dependencies_check(repo: &Repository) -> Checked {
  let found = dependencies::checkout_dependencies(&repo.root, &repo.declaration.check.base_libraries);

  Checked::of(
    QuickCheck::Dependencies.name(),
    &found.findings,
    &counted(&QUICK_CHECKS.dependencies.clean, found.dependencies),
  )
}

/// One quick check over the checkout, whether or not the declaration enables it.
pub fn quick_check(check: QuickCheck, repo: &Repository, env: &Env) -> Checked {
  match check {
    QuickCheck::Wording => wording_check(repo, env),
    QuickCheck::Dashes => dashes_check(repo),
    QuickCheck::Language => language_check(repo, env),
    QuickCheck::Layout => layout_check(repo),
    QuickCheck::Dependencies => dependencies_check(repo),
    QuickCheck::CargoProfile => cargo_profile_check(repo),
    QuickCheck::SharedVersions => shared_versions_check(repo, env),
  }
}

/// Every quick check the declaration enables, in the order it names them.
pub fn enabled_checks(repo: &Repository, env: &Env) -> Vec<Checked> {
  repo
    .declaration
    .check
    .quick
    .iter()
    .map(|check| quick_check(*check, repo, env))
    .collect()
}
