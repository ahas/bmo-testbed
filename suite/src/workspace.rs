//! The workspace packages a checkout declares, read from its own package files: the root `package.json`'s
//! `workspaces`, and each package's `package.json` for its name, the workspace packages it depends on and
//! its `bmo` section; and the projects of their own beside them, found by their own files or named by the
//! declaration (docs/checks.md, 「Which tests a change runs」). Nothing here keeps a list of packages of
//! its own.

use std::path::Path;
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::Repository;
use serde_json::Value;

use crate::files::checkout_files;
use crate::selection::glob_matches;

/// A workspace package as its own `package.json` declares it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageDeclaration {
  /// The package's directory from the checkout's root.
  pub dir: String,
  pub name: String,
  /// The workspace packages its dependencies and devDependencies name, by name.
  pub dependencies: Vec<String>,
  /// Globs for paths outside every package that its tests read.
  pub reads: Vec<String>,
  /// Files of other packages it launches as a process.
  pub runs: Vec<String>,
  /// Whether it is selected on every change.
  pub always: bool,
  /// The command checking a part of the package its runner does not cover, run inside its directory.
  pub check: Option<String>,
}

/// A project of its own: a directory carrying its own `package.json` and a declared language's lockfile that
/// the root's workspaces do not list, or one the declaration names (docs/checks.md, 「Which tests a change
/// runs」).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnProject {
  pub dir: String,
  /// Whether its `package.json` names the declared check script that is its static check and its tests.
  pub check: bool,
  /// The workspace packages standing inside it, by directory: their paths are theirs, not the project's.
  pub members: Vec<String>,
  /// The globs of its browser test files, from its own folder, as its manifest's `bmo` section names them.
  pub browser: Vec<String>,
}

fn read_json(path: &Path) -> Option<Value> {
  serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn subdirectories(root: &Path, dir: &str) -> Vec<String> {
  let mut names: Vec<String> = std::fs::read_dir(root.join(dir))
    .map(|entries| {
      entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
    })
    .unwrap_or_default();

  names.sort();
  names
}

fn strings(value: &Value) -> Vec<String> {
  value
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|one| one.as_str().map(str::to_owned))
    .collect()
}

fn joined(dir: &str, name: &str) -> String {
  match dir.trim_end_matches('/') {
    "" | "." => name.to_owned(),
    dir => format!("{dir}/{name}"),
  }
}

fn declared(dir: String, own: &Value) -> PackageDeclaration {
  let bmo = &own["bmo"];
  let mut dependencies: Vec<String> = Vec::new();

  for listed in [&own["dependencies"], &own["devDependencies"]] {
    for (name, version) in listed.as_object().into_iter().flatten() {
      if version.as_str().is_some_and(|version| version.starts_with("workspace:"))
        && !dependencies.contains(name)
      {
        dependencies.push(name.clone());
      }
    }
  }

  PackageDeclaration {
    dir,
    name: own["name"].as_str().unwrap_or_default().to_owned(),
    dependencies,
    reads: strings(&bmo["reads"]),
    runs: strings(&bmo["runs"]),
    always: bmo["always"].as_bool() == Some(true),
    check: bmo["check"].as_str().map(str::to_owned),
  }
}

/// Every package the root lists. A directory entry is one package and an entry ending in `/*` is every
/// directory under it holding a `package.json`; a directory whose `package.json` gives no name is no
/// package. A checkout with no workspaces declares none.
pub fn workspace_declarations(root: &Path) -> Vec<PackageDeclaration> {
  let manifest = read_json(&root.join("package.json")).unwrap_or(Value::Null);
  let listed = if manifest["workspaces"].is_array() {
    strings(&manifest["workspaces"])
  } else {
    strings(&manifest["workspaces"]["packages"])
  };
  let mut packages = Vec::new();

  for entry in listed {
    let dirs = match entry.strip_suffix("/*") {
      Some(parent) => subdirectories(root, parent).into_iter().map(|name| joined(parent, &name)).collect(),
      None => vec![entry.trim_end_matches('/').to_owned()],
    };

    for dir in dirs {
      if let Some(own) = read_json(&root.join(&dir).join("package.json"))
        && own["name"].is_string()
      {
        packages.push(declared(dir, &own));
      }
    }
  }

  // Only a dependency on another package of this workspace is one the selection follows.
  let names: Vec<String> = packages.iter().map(|one: &PackageDeclaration| one.name.clone()).collect();

  for one in &mut packages {
    one.dependencies.retain(|name| names.contains(name));
  }

  packages
}

// The directories of the checkout holding the lockfile a declared language names, from the root, the root's
// own left out, each once.
fn lockfile_dirs(repo: &Repository) -> Vec<String> {
  let lockfiles: Vec<String> = repo
    .declaration
    .test
    .languages()
    .into_iter()
    .filter_map(|language| language.lockfile())
    .map(|name| format!("/{name}"))
    .collect();

  if lockfiles.is_empty() {
    return Vec::new();
  }

  let mut dirs: Vec<String> = checkout_files(&repo.root)
    .into_iter()
    .filter(|path| !path.starts_with("node_modules/") && !path.contains("/node_modules/"))
    .filter_map(|path| {
      lockfiles.iter().find_map(|name| path.strip_suffix(name.as_str()).map(str::to_owned))
    })
    .collect();

  dirs.sort();
  dirs.dedup();
  dirs
}

/// Every project of its own the checkout holds, by directory: where the declaration leaves discovery on,
/// each directory carrying its own `package.json` and the lockfile a declared language names that the root's
/// workspaces do not list, one standing inside another included; then each directory the declaration names
/// outright.
pub fn own_projects(repo: &Repository) -> Vec<OwnProject> {
  let root = repo.root.as_path();
  let declared = &repo.declaration.projects;
  let listed: Vec<String> = workspace_declarations(root).into_iter().map(|one| one.dir).collect();
  let mut dirs: Vec<String> = Vec::new();

  if declared.discover {
    dirs = lockfile_dirs(repo)
      .into_iter()
      .filter(|dir| !listed.contains(dir) && root.join(dir).join("package.json").is_file())
      .collect();
  }

  for path in &declared.paths {
    let dir = path.to_string_lossy().trim_end_matches('/').to_owned();

    if !dirs.contains(&dir) {
      dirs.push(dir);
    }
  }

  dirs
    .into_iter()
    .filter_map(|dir| {
      let own = read_json(&root.join(&dir).join("package.json"))?;
      let mut members: Vec<String> =
        listed.iter().filter(|member| member.starts_with(&format!("{dir}/"))).cloned().collect();

      members.sort();

      let browser = own["bmo"]["browser"]
        .as_array()
        .map(|globs| globs.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();

      Some(OwnProject {
        check: own["scripts"][&declared.check_script].is_string(),
        members,
        browser,
        dir,
      })
    })
    .collect()
}

/// The browser test files a project of its own declares, each a path from the repository's root: every file
/// git tracks or holds new and not ignored inside the project that one of its globs reaches from its folder,
/// none of a project of `projects` standing inside it among them.
pub fn project_browser_files(root: &Path, project: &OwnProject, projects: &[OwnProject]) -> Vec<String> {
  if project.browser.is_empty() {
    return Vec::new();
  }

  let globs: Vec<String> = project.browser.iter().map(|glob| format!("{}/{glob}", project.dir)).collect();
  let mut files: Vec<String> =
    git_lines(root, &["ls-files", "--cached", "--others", "--exclude-standard", "--", &project.dir])
      .unwrap_or_default()
      .into_iter()
      .filter(|file| !nested_project_holds(projects, project, file))
      .filter(|file| globs.iter().any(|glob| glob_matches(glob, file)))
      .collect();

  files.sort();
  files.dedup();
  files
}

/// Every directory a project of its own installs in, from the root: each one inside it that holds the
/// lockfile a declared language names and no project standing inside it holds, the project's own directory
/// first (docs/checks.md, 「Which tests a change runs」).
pub fn own_project_install_dirs(repo: &Repository) -> Vec<String> {
  let lockfiles = lockfile_dirs(repo);
  let projects = own_projects(repo);

  projects
    .iter()
    .flat_map(|one| {
      let mut dirs = vec![one.dir.clone()];
      let below = lockfiles.iter().filter(|dir| dir.starts_with(&format!("{}/", one.dir)));

      for nested in below.filter(|dir| !nested_project_holds(&projects, one, dir)) {
        if !dirs.contains(nested) {
          dirs.push(nested.clone());
        }
      }

      dirs
    })
    .collect()
}

// Whether a project of `projects` standing inside `project` holds `path`, its own directory counting: such a
// path is that project's, not the one holding it.
fn nested_project_holds(projects: &[OwnProject], project: &OwnProject, path: &str) -> bool {
  let inside = |dir: &str, above: &str| dir.starts_with(&format!("{above}/"));

  projects
    .iter()
    .any(|other| inside(&other.dir, &project.dir) && (path == other.dir || inside(path, &other.dir)))
}

/// The project of its own a path lies in, or none: the innermost one whose directory holds it, its own path
/// counting, which a submodule's moved pin is; and a path inside a workspace package standing in that project
/// is the package's (docs/checks.md, 「Which tests a change runs」).
pub fn own_project_of<'a>(projects: &'a [OwnProject], path: &str) -> Option<&'a OwnProject> {
  let inside = |dir: &str| path == dir || path.starts_with(&format!("{dir}/"));

  projects
    .iter()
    .filter(|one| inside(&one.dir))
    .max_by_key(|one| one.dir.len())
    .filter(|one| !one.members.iter().any(|member| inside(member)))
}

/// The package a path belongs to: the deepest one whose directory holds it, or none.
pub fn owning_package<'a>(
  packages: &'a [PackageDeclaration],
  path: &str,
) -> Option<&'a PackageDeclaration> {
  packages
    .iter()
    .filter(|one| path.starts_with(&format!("{}/", one.dir)))
    .max_by_key(|one| one.dir.len())
}

/// The paths that are this repository's own files: every one inside a project of its own left out.
pub fn repository_paths(repo: &Repository, paths: Vec<String>) -> Vec<String> {
  let projects = own_projects(repo);

  paths.into_iter().filter(|path| own_project_of(&projects, path).is_none()).collect()
}

/// Lines git printed, each trimmed, the empty ones left out; none where git failed.
pub fn git_lines(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
  let done = Command::new("git")
    .args(args)
    .current_dir(root)
    .stdin(Stdio::null())
    .output()
    .map_err(|why| format!("git {}: {why}", args.join(" ")))?;

  if !done.status.success() {
    return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&done.stderr).trim()));
  }

  Ok(
    String::from_utf8_lossy(&done.stdout)
      .lines()
      .map(str::trim)
      .filter(|line| !line.is_empty())
      .map(str::to_owned)
      .collect(),
  )
}
