//! Which packages a change reaches, and so which test files a run starts (docs/checks.md, 「Which tests a
//! change runs」). The packages and what each declares are read from the checkout's own package files and its
//! declaration; this module holds the rule and no list of packages.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::suite::SUITE;

use crate::crates::{CrateDeclaration, is_cargo_path, is_workspace_path, owning_crate, workspace_crates};
use crate::files::checkout_files;
use crate::imports::{CODE_EXTENSIONS, WorkspacePackage, module_specifiers, resolve_import};
use crate::run::file_languages;
use crate::tool_checks::{CheckedTool, checked_tool_of, checked_tools};
use crate::workspace::{
  OwnProject, PackageDeclaration, git_lines, own_project_of, own_projects, owning_package,
  workspace_declarations,
};

/// The packages a run chooses among, the projects of their own beside them, the workspace's crates and the
/// tools checked by their manifest.
#[derive(Clone, Debug, Default)]
pub struct Layout {
  pub packages: Vec<PackageDeclaration>,
  pub projects: Vec<OwnProject>,
  pub crates: Vec<CrateDeclaration>,
  pub tools: Vec<CheckedTool>,
  /// The root's own configuration, which every package stands on: the files each declared language names.
  pub root_configuration: Vec<&'static str>,
  /// The folders below the root where a declared language's runner reads its configuration, each the
  /// folder its test files run from.
  pub runner_dirs: Vec<String>,
  /// The globs naming the test files the declared languages run: the declaration's own, or else each
  /// language's own naming.
  pub test_naming: Vec<String>,
  /// The workspace packages each project of its own lists in its own manifest, by their directories from
  /// the root, which an import names as it names one of the root's.
  pub project_packages: Vec<WorkspacePackage>,
}

impl Layout {
  /// The layout the repository's own package files and its declaration declare.
  pub fn of_checkout(repo: &Repository) -> Result<Self, String> {
    Ok(Self::of_parts(repo, workspace_crates(&repo.root)?))
  }

  /// The layout the repository's own package files and its declaration declare beside these crates: the
  /// tools checked by their manifest are those none of them holds.
  pub fn of_parts(repo: &Repository, crates: Vec<CrateDeclaration>) -> Self {
    let packages = workspace_declarations(&repo.root);
    let projects = own_projects(repo);
    let tools = checked_tools(repo, &packages, &projects, &crates);
    let mut root_configuration: Vec<&'static str> = repo
      .declaration
      .test
      .languages()
      .into_iter()
      .flat_map(|language| language.root_configuration().iter().copied())
      .collect();

    root_configuration.sort_unstable();
    root_configuration.dedup();

    let project_packages = projects
      .iter()
      .flat_map(|project| {
        workspace_declarations(&repo.root.join(&project.dir))
          .into_iter()
          .map(|one| WorkspacePackage {
            dir: format!("{}/{}", project.dir, one.dir),
            name: one.name,
          })
      })
      .collect();
    let test = &repo.declaration.test;
    let languages = file_languages(test);
    let configurations: Vec<String> = languages
      .iter()
      .filter_map(|language| language.configuration_file())
      .map(|name| format!("/{name}"))
      .collect();
    let mut runner_dirs: Vec<String> = if configurations.is_empty() {
      Vec::new()
    } else {
      checkout_files(&repo.root)
        .into_iter()
        .filter(|path| !path.starts_with("node_modules/") && !path.contains("/node_modules/"))
        .filter_map(|path| {
          configurations
            .iter()
            .find_map(|name| path.strip_suffix(name.as_str()).map(str::to_owned))
        })
        .collect()
    };

    runner_dirs.sort();
    runner_dirs.dedup();

    let test_naming = test.files.clone().unwrap_or_else(|| {
      languages
        .iter()
        .flat_map(|language| language.test_naming().iter().map(|naming| (*naming).to_owned()))
        .collect()
    });

    Self {
      packages,
      projects,
      crates,
      tools,
      root_configuration,
      runner_dirs,
      test_naming,
      project_packages,
    }
  }

  /// The package a path belongs to.
  pub fn owner_of(&self, path: &str) -> Option<&PackageDeclaration> {
    owning_package(&self.packages, path)
  }

  /// Whether a unit other than a crate holds a path: a project of its own, a workspace package, the root's
  /// configuration a declared language names, a tool checked by its manifest, a folder below the root where
  /// a declared language's runner reads its configuration, or a test file a declared language runs. What
  /// none of them holds, nor another crate's directory, is the crate standing at the workspace's root.
  pub fn held_beside_crates(&self, path: &str) -> bool {
    own_project_of(&self.projects, path).is_some()
      || self.root_configuration.contains(&path)
      || self.owner_of(path).is_some()
      || checked_tool_of(&self.tools, path).is_some()
      || self.runner_dirs.iter().any(|dir| path.starts_with(&format!("{dir}/")))
      || self.test_naming.iter().any(|glob| glob_matches(glob, path))
  }
}

/// Why a package or a crate was marked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkReason {
  Owner { path: String },
  Root { path: String },
  Reads { path: String, glob: String },
  Unclaimed { path: String },
  Runs { launched: String, path: String },
  Dependent { on: String },
  Always,
}

impl MarkReason {
  /// The reason, as a selected run says it.
  pub fn text(&self) -> String {
    let said = &SUITE.reasons;

    match self {
      Self::Owner { path } => fill_marks(&said.owner, &[("path", path)]),
      Self::Root { path } => fill_marks(&said.root, &[("path", path)]),
      Self::Reads { path, glob } => fill_marks(&said.reads, &[("path", path), ("glob", glob)]),
      Self::Unclaimed { path } => fill_marks(&said.unclaimed, &[("path", path)]),
      Self::Runs { launched, path } if launched == path => {
        fill_marks(&said.runs, &[("launched", launched)])
      }
      Self::Runs { launched, path } => {
        fill_marks(&said.runs_loading, &[("launched", launched), ("path", path)])
      }
      Self::Dependent { on } => fill_marks(&said.dependent, &[("on", on)]),
      Self::Always => said.always.clone(),
    }
  }
}

/// What a set of changed paths marks.
#[derive(Clone, Debug, Default)]
pub struct Selection {
  /// Every marked package by name, with each reason it was marked for, the first one first.
  pub marked: BTreeMap<String, Vec<MarkReason>>,
  /// Every marked project of its own by directory, with the first changed path inside it.
  pub projects: BTreeMap<String, String>,
  /// Every marked tool checked by its manifest by directory, with the first changed path inside it.
  pub tools: BTreeMap<String, String>,
  /// Every marked crate by name, with each reason it was marked for, the first one first.
  pub crates: BTreeMap<String, Vec<MarkReason>>,
}

/// Every path a range and the working tree changed: the commits from `since` to HEAD, the tracked files
/// modified since HEAD whether staged or not, and the files not yet tracked. A rename counts as both of its
/// paths, since the package the file left is changed as well as the one it went to.
pub fn changed_paths(root: &Path, since: &str) -> Result<Vec<String>, String> {
  git_lines(root, &["rev-parse", "--verify", "--quiet", &format!("{since}^{{commit}}")])?;

  let mut paths: Vec<String> = git_lines(root, &["diff", "--name-only", "--no-renames", since, "HEAD"])?
    .into_iter()
    .chain(git_lines(root, &["diff", "--name-only", "--no-renames", "HEAD"])?)
    .chain(git_lines(root, &["ls-files", "--others", "--exclude-standard"])?)
    .collect::<HashSet<String>>()
    .into_iter()
    .collect();

  paths.sort();
  Ok(paths)
}

/// Whether a glob a package or a crate declares matches a path: a `*` stays inside one directory and
/// `**` crosses them.
pub fn glob_matches(pattern: &str, path: &str) -> bool {
  glob::Pattern::new(pattern).is_ok_and(|glob| {
    glob.matches_with(
      path,
      glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
      },
    )
  })
}

/// The files one file loads, itself included, followed through every import this reading can resolve: a
/// relative path, or the name and module of a workspace package, the root's or one a project of its own
/// lists. `cache` is shared across the launched files of one selection.
pub fn import_closure(
  root: &Path,
  file: &str,
  layout: &Layout,
  cache: &mut HashMap<String, Vec<String>>,
) -> Vec<String> {
  let packages: Vec<WorkspacePackage> = layout
    .packages
    .iter()
    .map(|one| WorkspacePackage {
      dir: one.dir.clone(),
      name: one.name.clone(),
    })
    .chain(layout.project_packages.iter().cloned())
    .collect();
  let mut seen: Vec<String> = Vec::new();
  let mut waiting = vec![file.to_owned()];

  while let Some(next) = waiting.pop() {
    if seen.contains(&next) {
      continue;
    }
    seen.push(next.clone());

    let named = cache.entry(next.clone()).or_insert_with(|| {
      let read = CODE_EXTENSIONS
        .iter()
        .any(|extension| next.ends_with(extension))
        .then(|| std::fs::read_to_string(root.join(&next)).ok())
        .flatten()
        .unwrap_or_default();

      module_specifiers(&next, &read)
        .into_iter()
        .filter_map(|spec| resolve_import(root, &next, &spec, &packages))
        .collect()
    });

    waiting.extend(named.iter().cloned());
  }

  seen
}

// A mark set down: the reason kept, and the name passed on to dependents where the mark passes.
fn mark(
  marked: &mut BTreeMap<String, Vec<MarkReason>>,
  passing: &mut HashSet<String>,
  name: &str,
  reason: MarkReason,
  passes: bool,
) {
  marked.entry(name.to_owned()).or_default().push(reason);

  if passes {
    passing.insert(name.to_owned());
  }
}

// Dependents marked in turn until nothing more is marked, each from the first dependency passing it a mark.
fn mark_dependents<'a>(
  members: impl Iterator<Item = (&'a str, &'a [String])> + Clone,
  marked: &mut BTreeMap<String, Vec<MarkReason>>,
  passing: &mut HashSet<String>,
) {
  let mut grew = true;

  while grew {
    grew = false;

    for (name, dependencies) in members.clone() {
      if passing.contains(name) {
        continue;
      }

      if let Some(on) = dependencies.iter().find(|dependency| passing.contains(*dependency)) {
        let on = on.clone();

        mark(marked, passing, name, MarkReason::Dependent { on }, true);
        grew = true;
      }
    }
  }
}

/// The crates a set of changed paths marks: a path inside a crate marks that crate wherever the crate
/// stands, the workspace's root files mark every crate, a path no crate holds marks a crate whose `reads`
/// match it and, where nothing else of the layout holds it, the crate standing at the workspace's root, and
/// a crate depending on a crate marked by any rule but `reads` is marked in turn.
pub fn select_crates(layout: &Layout, changed: &[String]) -> BTreeMap<String, Vec<MarkReason>> {
  let crates = layout.crates.as_slice();
  let at_root = crates.iter().find(|one| one.dir.is_empty());
  let mut marked = BTreeMap::new();
  let mut passing = HashSet::new();

  for path in changed {
    if let Some(owner) = owning_crate(crates, path) {
      mark(&mut marked, &mut passing, &owner.name, MarkReason::Owner { path: path.clone() }, true);
      continue;
    }

    if !is_cargo_path(path) {
      for one in crates {
        if let Some(glob) = one.reads.iter().find(|glob| glob_matches(glob, path)) {
          let reason = MarkReason::Reads {
            path: path.clone(),
            glob: glob.clone(),
          };

          mark(&mut marked, &mut passing, &one.name, reason, false);
        }
      }

      if let Some(owner) = at_root.filter(|_| !layout.held_beside_crates(path)) {
        mark(&mut marked, &mut passing, &owner.name, MarkReason::Owner { path: path.clone() }, true);
      }

      continue;
    }

    for one in crates {
      mark(&mut marked, &mut passing, &one.name, MarkReason::Root { path: path.clone() }, true);
    }
  }

  mark_dependents(
    crates.iter().map(|one| (one.name.as_str(), one.dependencies.as_slice())),
    &mut marked,
    &mut passing,
  );

  marked
}

/// The packages a set of changed paths marks, by the rule in the order the rule states it.
pub fn select_packages(root: &Path, layout: &Layout, changed: &[String]) -> Selection {
  let mut selection = Selection::default();
  // The packages whose mark is passed on to their dependents: every mark but one by `reads` alone.
  let mut passing = HashSet::new();
  // The paths of this repository's own files: the Rust workspace's, a crate's folder wherever it stands
  // included, mark crates, and the rule below reads the rest.
  let in_repository: Vec<String> = changed
    .iter()
    .filter(|path| own_project_of(&layout.projects, path).is_none())
    .cloned()
    .collect();

  for path in changed {
    if let Some(project) = own_project_of(&layout.projects, path) {
      selection.projects.entry(project.dir.clone()).or_insert_with(|| path.clone());
    }

    if let Some(tool) = checked_tool_of(&layout.tools, path) {
      selection.tools.entry(tool.dir.clone()).or_insert_with(|| path.clone());
    }
  }

  selection.crates = select_crates(layout, &in_repository);

  for path in in_repository.iter().filter(|path| !is_workspace_path(&layout.crates, path)) {
    if let Some(owner) = layout.owner_of(path) {
      mark(
        &mut selection.marked,
        &mut passing,
        &owner.name,
        MarkReason::Owner { path: path.clone() },
        true,
      );
      continue;
    }

    if layout.root_configuration.contains(&path.as_str()) {
      for one in &layout.packages {
        mark(
          &mut selection.marked,
          &mut passing,
          &one.name,
          MarkReason::Root { path: path.clone() },
          true,
        );
      }
      continue;
    }

    let readers: Vec<(&PackageDeclaration, &String)> = layout
      .packages
      .iter()
      .filter_map(|one| one.reads.iter().find(|glob| glob_matches(glob, path)).map(|glob| (one, glob)))
      .collect();

    // A path a tool checked by its manifest holds is claimed by that tool's check.
    if readers.is_empty() && checked_tool_of(&layout.tools, path).is_some() {
      continue;
    }

    if readers.is_empty() {
      for one in &layout.packages {
        mark(
          &mut selection.marked,
          &mut passing,
          &one.name,
          MarkReason::Unclaimed { path: path.clone() },
          true,
        );
      }
      continue;
    }

    for (one, glob) in readers {
      let reason = MarkReason::Reads {
        path: path.clone(),
        glob: glob.clone(),
      };

      mark(&mut selection.marked, &mut passing, &one.name, reason, false);
    }
  }

  // A launched file is reached by a change to itself or to anything it loads, and a launched crate by a
  // change anywhere inside its directory.
  let mut cache = HashMap::new();
  let touched: HashSet<&String> = in_repository.iter().collect();

  for one in &layout.packages {
    for launched in &one.runs {
      let crate_dir = launched.trim_end_matches('/');
      let reached = match layout.crates.iter().find(|candidate| candidate.dir == crate_dir) {
        Some(found) => {
          in_repository.iter().find(|path| path.starts_with(&format!("{}/", found.dir))).cloned()
        }
        None => import_closure(root, launched, layout, &mut cache)
          .into_iter()
          .find(|path| touched.contains(path)),
      };

      if let Some(path) = reached {
        let reason = MarkReason::Runs {
          launched: launched.clone(),
          path,
        };

        mark(&mut selection.marked, &mut passing, &one.name, reason, true);
      }
    }
  }

  mark_dependents(
    layout.packages.iter().map(|one| (one.name.as_str(), one.dependencies.as_slice())),
    &mut selection.marked,
    &mut passing,
  );

  for one in layout.packages.iter().filter(|one| one.always) {
    mark(&mut selection.marked, &mut passing, &one.name, MarkReason::Always, false);
  }

  selection
}

/// The test files a selection runs: those inside a marked package, and those inside no package and no
/// project of its own, which no rule could have left out.
pub fn selected_tests(layout: &Layout, selection: &Selection, tests: &[String]) -> Vec<String> {
  tests
    .iter()
    .filter(|file| own_project_of(&layout.projects, file).is_none())
    .filter(|file| layout.owner_of(file).is_none_or(|owner| selection.marked.contains_key(&owner.name)))
    .cloned()
    .collect()
}

/// What a selected run says first: every package, marked or not, the marked ones with the reason they were
/// marked and how many more there were; then the projects of their own, the tools checked by their manifest and
/// the crates.
pub fn selection_lines(layout: &Layout, selection: &Selection) -> Vec<String> {
  let said = &SUITE.selection;
  let mut lines = vec![fill_marks(
    &said.heading,
    &[
      ("marked", &selection.marked.len().to_string()),
      ("packages", &layout.packages.len().to_string()),
    ],
  )];

  for one in &layout.packages {
    lines.push(match selection.marked.get(&one.name) {
      None => fill_marks(&said.unmarked, &[("name", &one.name)]),
      Some(reasons) => {
        let more = if reasons.len() > 1 {
          fill_marks(&said.more, &[("count", &(reasons.len() - 1).to_string())])
        } else {
          String::new()
        };

        fill_marks(&said.marked, &[("name", &one.name), ("reason", &reasons[0].text()), ("more", &more)])
      }
    });
  }

  for one in &layout.projects {
    lines.push(match selection.projects.get(&one.dir) {
      None => fill_marks(&said.project_unmarked, &[("dir", &one.dir)]),
      Some(path) => fill_marks(&said.project_marked, &[("dir", &one.dir), ("path", path)]),
    });
  }

  for one in &layout.tools {
    lines.push(match selection.tools.get(&one.dir) {
      None => fill_marks(&said.tool_unmarked, &[("dir", &one.dir)]),
      Some(path) => fill_marks(&said.tool_marked, &[("dir", &one.dir), ("path", path)]),
    });
  }

  for one in &layout.crates {
    lines.push(match selection.crates.get(&one.name) {
      None => fill_marks(&said.crate_unmarked, &[("dir", &one.dir)]),
      Some(reasons) => fill_marks(&said.crate_marked, &[("dir", &one.dir), ("reason", &reasons[0].text())]),
    });
  }

  lines
}
