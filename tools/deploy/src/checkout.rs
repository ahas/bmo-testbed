//! The checkout a repository is deployed from: the one in the machine's registry whose declaration names it
//! (docs/suite.md, 「The dependents of a major change」; docs/deploy.md, 「The hosted deploy tool」).

use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::deploy_tool::DEPLOY_TOOL;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::turn::Env;
use bmo_testbed_suite::dependents::read_registry;

/// What a repository's name may begin with and be named without.
const NAME_PREFIX: &str = "bmo-";

/// Whether two repository names are one: the same once a leading `bmo-` is taken off each.
pub fn same_name(declared: &str, asked: &str) -> bool {
  let bare = |name: &str| name.trim().strip_prefix(NAME_PREFIX).unwrap_or(name.trim()).to_owned();

  bare(declared) == bare(asked)
}

/// The one checkout in the registry at `registry` whose declaration names `repository`, or why there is
/// none: a registry that does not stand or does not read, no checkout of that name, or more than one.
pub fn find_checkout(repository: &str, registry: &Path, env: &Env) -> Result<Repository, String> {
  let said = &DEPLOY_TOOL.lines;
  let path = registry.to_string_lossy();

  if !registry.exists() {
    return Err(fill_marks(&said.no_registry, &[("path", &path)]));
  }

  let checkouts = read_registry(registry, env)
    .map_err(|fault| fill_marks(&said.registry, &[("path", &path), ("why", &fault.why)]))?;
  let mut names: Vec<String> = Vec::new();
  let mut unread: Vec<String> = Vec::new();
  let mut found: Vec<(PathBuf, Repository)> = Vec::new();

  for checkout in checkouts {
    let Ok(read) = Repository::read(&checkout) else {
      unread.push(checkout.to_string_lossy().into_owned());
      continue;
    };
    let name = read.name();

    if !same_name(&name, repository) {
      names.push(name);
      continue;
    }

    let real = std::fs::canonicalize(&read.root).unwrap_or_else(|_| read.root.clone());

    if !found.iter().any(|(there, _)| *there == real) {
      found.push((real, read));
    }
  }

  match found.len() {
    1 => Ok(found.remove(0).1),
    0 => {
      names.sort();
      names.dedup();

      let shown = if names.is_empty() { said.no_names.clone() } else { names.join(", ") };
      let mut line =
        fill_marks(&said.unknown, &[("path", &path), ("repository", repository.trim()), ("names", &shown)]);

      if !unread.is_empty() {
        line.push_str("; ");
        line.push_str(&fill_marks(&said.unread, &[("checkouts", &unread.join(", "))]));
      }

      Err(line)
    }
    _ => {
      let roots: Vec<String> =
        found.iter().map(|(_, read)| read.root.to_string_lossy().into_owned()).collect();

      Err(fill_marks(
        &said.ambiguous,
        &[("repository", repository.trim()), ("path", &path), ("roots", &roots.join(", "))],
      ))
    }
  }
}
