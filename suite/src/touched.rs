//! What a change touches, for the change check (docs/checks.md, 「The change check」): the crates it
//! reaches, the test files it reaches, and the projects of their own, the tools checked by their manifest
//! and the packages declaring a check it reaches.
//! The rule is stated here once; a finer selection by a change's impact replaces this one function.

use std::collections::{HashMap, HashSet};

use bmo_testbed_core::declaration::Repository;

use crate::crates::workspace_crates;
use crate::run::test_files;
use crate::selection::{Layout, MarkReason, glob_matches, import_closure, select_crates};
use crate::test_targets::{TargetTests, owed_tests};
use crate::tool_checks::{CheckedTool, package_check};
use crate::workspace::{OwnProject, own_project_of};

/// What a change touches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Touched {
  /// The crates owed a `cargo check`: those it reaches and every one depending on them.
  pub checked_crates: Vec<String>,
  /// The crates whose tests it owes: those a changed path belongs to or declares it reads.
  pub tested_crates: Vec<String>,
  /// The tests of the crates among them whose only changed paths are test files, each target those reach,
  /// whole or by its modules' filters; a tested crate not here owes its tests whole.
  pub targets: Vec<TargetTests>,
  /// The TypeScript test files it owes: each changed itself, or loading a changed file.
  pub tests: Vec<String>,
  /// The projects of their own whose check it owes, each holding a changed path.
  pub projects: Vec<OwnProject>,
  /// The tools checked by their manifest whose check it owes, each holding a changed path, and the check of each
  /// package declaring one that a changed path belongs to or matches a `reads` glob of.
  pub tools: Vec<CheckedTool>,
}

// Whether a crate was marked for a path of its own rather than through another crate or the workspace's
// root, which every crate's check answers for and no crate's tests do.
fn touched_itself(reasons: &[MarkReason]) -> bool {
  reasons
    .iter()
    .any(|reason| matches!(reason, MarkReason::Owner { .. } | MarkReason::Reads { .. }))
}

/// What a change of these paths touches in the repository's checkout.
pub fn touched(repo: &Repository, changed: &[String]) -> Touched {
  let root = repo.root.as_path();
  let layout = Layout::of_parts(repo, workspace_crates(root).unwrap_or_default());
  let marked = select_crates(&layout, changed);
  let tools = layout
    .tools
    .iter()
    .filter(|one| changed.iter().any(|path| one.holds(path)))
    .cloned()
    .chain(
      layout
        .packages
        .iter()
        .filter(|one| {
          changed.iter().any(|path| {
            layout.owner_of(path).is_some_and(|owner| owner.name == one.name)
              || one.reads.iter().any(|glob| glob_matches(glob, path))
          })
        })
        .filter_map(package_check),
    )
    .collect();
  let projects = layout
    .projects
    .iter()
    .filter(|one| {
      changed
        .iter()
        .any(|path| own_project_of(&layout.projects, path).is_some_and(|owner| owner.dir == one.dir))
    })
    .cloned()
    .collect();
  let changed_set: HashSet<&String> = changed.iter().collect();
  let mut cache = HashMap::new();
  let tests = test_files(repo)
    .into_iter()
    .filter(|file| {
      changed_set.contains(file)
        || import_closure(root, file, &layout, &mut cache)
          .iter()
          .any(|loaded| changed_set.contains(loaded))
    })
    .collect();

  let tested: Vec<(&String, &Vec<MarkReason>)> =
    marked.iter().filter(|(_, reasons)| touched_itself(reasons)).collect();
  let targets = tested
    .iter()
    .filter_map(|(name, reasons)| {
      let krate = layout.crates.iter().find(|one| one.name == **name)?;
      let mut paths = Vec::new();

      for reason in reasons.iter() {
        match reason {
          MarkReason::Owner { path } => paths.push(path.clone()),
          MarkReason::Reads { .. } => return None,
          _ => {}
        }
      }

      owed_tests(root, krate, &paths)
    })
    .flatten()
    .collect();

  Touched {
    checked_crates: marked.keys().cloned().collect(),
    tested_crates: tested.iter().map(|(name, _)| (*name).clone()).collect(),
    targets,
    tests,
    projects,
    tools,
  }
}
