//! `imports` (docs/checks.md, 「Imports a move left behind」): the relative imports a move left broken, found
//! and spelled again. Without `--fix` it only says; with it, it writes every repair or none. The reading and
//! the repairs are the suite crate's; this is the verb's side of them.

use std::path::{Component, Path, PathBuf};

use bmo_testbed_core::answer::{Cut, cap_items, count_text};
use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_core::verb::REFUSED_EXIT_CODE;
use bmo_testbed_suite::moved_imports::{
  ImportRepair, Outcome, apply_repairs, code_files, import_repairs, repository_files,
};
use bmo_testbed_suite::workspace::workspace_declarations;

use crate::said::Said;

// A path with `.` and `..` read off where it stands, without asking the disk.
fn lexical(path: &Path) -> PathBuf {
  let mut clean = PathBuf::new();

  for part in path.components() {
    match part {
      Component::CurDir => {}
      Component::ParentDir => {
        clean.pop();
      }
      other => clean.push(other),
    }
  }

  clean
}

// A path named on the line as the checkout's own, from its root: read from `base` where it is not absolute.
fn from_root(root: &Path, base: &Path, named: &str) -> Option<String> {
  let at = lexical(&base.join(named));
  let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
  let inside = real(&at).strip_prefix(real(root)).ok()?.to_string_lossy().into_owned();

  (!inside.is_empty()).then_some(inside)
}

fn line(repair: &ImportRepair) -> String {
  let said = &VERBS.imports;
  let at = repair.site.line.to_string();
  let place = [
    ("file", repair.file.as_str()),
    ("line", at.as_str()),
    ("spec", repair.site.spec.as_str()),
  ];

  match &repair.outcome {
    Outcome::To(to) => fill_marks(&said.to, &[&place[..], &[("to", to.as_str())]].concat()),
    Outcome::Missing { tail } => {
      fill_marks(&said.missing, &[&place[..], &[("tail", tail.as_str())]].concat())
    }
    Outcome::Ambiguous { tail, candidates } => {
      let count = count_text(candidates.len());
      let named = candidates.join(", ");

      fill_marks(
        &said.ambiguous,
        &[&place[..], &[("tail", tail.as_str()), ("count", &count), ("candidates", &named)]].concat(),
      )
    }
  }
}

/// The verb over the repository, each path named read from `base` where it is not absolute.
pub fn imports_in(named: &[String], fix: bool, repo: &Repository, base: &Path) -> Said {
  let said = &VERBS.imports;
  let root = repo.root.as_path();
  let mut answer = Said::default();
  let all = match repository_files(repo) {
    Ok(all) => all,
    Err(why) => {
      answer
        .err
        .push(fill_marks(&said.not_checkout, &[("root", &root.to_string_lossy()), ("why", &why)]));
      answer.code = REFUSED_EXIT_CODE;
      return answer;
    }
  };
  let mut files = Vec::new();
  let mut missing = Vec::new();

  for word in named {
    match from_root(root, base, word).filter(|path| all.contains(path)) {
      Some(path) => files.push(path),
      None => missing.push(word.as_str()),
    }
  }

  if !missing.is_empty() {
    answer.err.push(fill_marks(&said.not_files, &[("paths", &missing.join(", "))]));
    answer.code = REFUSED_EXIT_CODE;
    return answer;
  }

  let files = if files.is_empty() { code_files(&all) } else { files };
  let repairs = import_repairs(root, &files, &all, &workspace_declarations(root));

  if repairs.is_empty() {
    answer.out.push(said.clean.clone());
    return answer;
  }

  let render = |kept: &[ImportRepair]| kept.iter().map(line).collect::<Vec<_>>().join("\n");
  let capped = cap_items(&repairs, render, Cut::new(&said.unit, &said.how));

  answer.out.push(render(capped.kept));
  answer.out.extend(capped.note);

  let refused = repairs.iter().filter(|repair| repair.refused()).count();
  let mut touched: Vec<&str> = repairs.iter().map(|repair| repair.file.as_str()).collect();

  touched.sort_unstable();
  touched.dedup();

  if refused > 0 {
    answer.err.push(fill_marks(
      &said.refused,
      &[
        ("count", &count_text(refused)),
        ("nothing", if fix { &said.nothing_written } else { "" }),
      ],
    ));
    answer.code = 1;
    return answer;
  }

  let counts = |files: usize| [("count", count_text(repairs.len())), ("files", count_text(files))];

  if !fix {
    let counts = counts(touched.len());

    answer.err.push(fill_marks(
      &said.to_fix,
      &counts.each_ref().map(|(name, value)| (*name, value.as_str())),
    ));
    answer.code = 1;
    return answer;
  }

  match apply_repairs(root, &repairs) {
    Ok(written) => {
      let counts = counts(written.len());

      answer
        .out
        .push(fill_marks(&said.fixed, &counts.each_ref().map(|(name, value)| (*name, value.as_str()))));
    }
    Err(why) => {
      answer.err.push(why);
      answer.code = 1;
    }
  }

  answer
}
