//! The machine's checkouts and which of them build on a repository (docs/suite.md, 「The dependents of a
//! major change」): a registry of checkout paths, read by path, and the consumers whose own declaration names
//! the upstream. The upstream names none of them, so this reading lives here and not in the declaration.

use std::path::{Path, PathBuf};

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::dependents::DEPENDENTS;
use bmo_testbed_core::turn::Env;
use serde::Deserialize;

/// The variable naming the registry file over its place under the data root.
pub const CHECKOUTS_VARIABLE: &str = "BMO_TESTBED_CHECKOUTS";

// One checkout the registry names: its path, or a table holding it.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Checkout {
  Path(PathBuf),
  Table { path: PathBuf },
}

impl Checkout {
  fn into_path(self) -> PathBuf {
    match self {
      Self::Path(path) | Self::Table { path } => path,
    }
  }
}

/// The registry file: its `checkouts` list, or its `[[checkout]]` tables, each naming a checkout.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
  #[serde(default)]
  checkouts: Vec<Checkout>,
  #[serde(default)]
  checkout: Vec<Checkout>,
}

/// Which reading a fault came from: the registry itself or one checkout's declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
  Registry,
  Declaration,
}

/// A registry or a checkout that could not be read: the path and what it came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
  pub kind: FaultKind,
  pub path: PathBuf,
  pub why: String,
}

impl std::fmt::Display for Fault {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(formatter, "{}: {}", self.path.display(), self.why)
  }
}

// A path made whole by its words alone: `.` and `..` resolved as a reader would resolve them.
fn normalized(path: PathBuf) -> PathBuf {
  let mut out = PathBuf::new();

  for part in path.components() {
    match part {
      std::path::Component::ParentDir => {
        out.pop();
      }
      std::path::Component::CurDir => {}
      other => out.push(other.as_os_str()),
    }
  }

  out
}

// A path as the registry holds it: under `HOME` where it begins with `~/`, under `base` where it is relative,
// and as written where it is absolute.
fn expand(path: &Path, base: &Path, env: &Env) -> PathBuf {
  let whole = match path.strip_prefix("~") {
    Ok(under_home) => PathBuf::from(env.get("HOME").map_or("/", String::as_str)).join(under_home),
    Err(_) if path.is_absolute() => path.to_path_buf(),
    Err(_) => base.join(path),
  };

  normalized(whole)
}

/// The registry's path: the one `BMO_TESTBED_CHECKOUTS` names, or `checkouts.toml` under the data root's
/// `testbed/`.
pub fn registry_path(data_root: &Path, env: &Env) -> PathBuf {
  match env.get(CHECKOUTS_VARIABLE).filter(|path| !path.is_empty()) {
    Some(path) => expand(Path::new(path), Path::new("."), env),
    None => data_root.join("testbed").join(&DEPENDENTS.registry),
  }
}

/// The checkouts the registry at `path` names, every path made whole against the registry's folder. A
/// registry that does not stand names none; one that does not read is a fault.
pub fn read_registry(path: &Path, env: &Env) -> Result<Vec<PathBuf>, Fault> {
  let text = match std::fs::read_to_string(path) {
    Ok(text) => text,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
    Err(error) => {
      return Err(Fault {
        kind: FaultKind::Registry,
        path: path.to_path_buf(),
        why: error.to_string(),
      });
    }
  };
  let registry: Registry = toml::from_str(&text).map_err(|error| Fault {
    kind: FaultKind::Registry,
    path: path.to_path_buf(),
    why: error.to_string(),
  })?;
  let base = path.parent().unwrap_or(Path::new("."));
  let paths = registry.checkouts.into_iter().chain(registry.checkout);

  Ok(paths.map(|checkout| expand(&checkout.into_path(), base, env)).collect())
}

/// Every checkout in the registry that names `upstream`'s repository in its own `upstreams`, by its declared
/// name and in that name's order: the direct consumers, and no consumer of a consumer.
pub fn consumers(upstream: &Repository, registry: &Path, env: &Env) -> Result<Vec<Repository>, Fault> {
  let name = upstream.name();
  let here = std::fs::canonicalize(&upstream.root).unwrap_or_else(|_| upstream.root.clone());
  let mut found: Vec<Repository> = Vec::new();

  for checkout in read_registry(registry, env)? {
    let consumer = Repository::read(&checkout).map_err(|refusal| Fault {
      kind: FaultKind::Declaration,
      path: checkout.clone(),
      why: refusal.to_string(),
    })?;
    let there = std::fs::canonicalize(&consumer.root).unwrap_or_else(|_| consumer.root.clone());

    let declared = consumer.declaration.upstreams.iter().chain(&consumer.declaration.check.upstreams);

    if there == here || !declared.into_iter().any(|upstream| *upstream == name) {
      continue;
    }

    if !found.iter().any(|one| one.root == consumer.root) {
      found.push(consumer);
    }
  }

  found.sort_by(|left, right| left.name().cmp(&right.name()).then_with(|| left.root.cmp(&right.root)));
  Ok(found)
}
