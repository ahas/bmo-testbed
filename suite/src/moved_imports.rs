//! Imports a move left behind (docs/checks.md, 「Imports a move left behind」): a relative import naming a file
//! that is no longer where it said, followed to the one file whose path ends the way the import's path ends,
//! and spelled again from where the importing file stands — relative inside one workspace package, by the
//! package's name across two. The imports are read off the source by oxc, so one written inside a string, a
//! template's text or a comment is text about an import and never one.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
  Argument, CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression, ImportDeclaration,
  ImportExpression, StringLiteral,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_core::policy::verbs::VERBS;

use crate::imports::{component_scripts, resolve_import};
use crate::workspace::{PackageDeclaration, git_lines, owning_package, repository_paths};

// The extensions an import may leave off, which is what a file is called by where an import spells none.
const LEFT_OFF: [&str; 5] = [".ts", ".tsx", ".mts", ".js", ".mjs"];

// What a code file of the checkout may be named with, which is what a run naming no file reads.
const CODE_FILE: [&str; 4] = [".ts", ".tsx", ".mts", ".vue"];

// An extension an import spelled out, which it keeps.
const SPELLED_EXTENSIONS: [&str; 14] = [
  ".ts", ".tsx", ".mts", ".js", ".mjs", ".json", ".vue", ".css", ".md", ".yaml", ".yml", ".txt", ".html",
  ".svg",
];

// The directories no import of ours is written in.
const OUTSIDE_CODE: [&str; 3] = ["node_modules", ".nuxt", ".output"];

/// One relative import as it stands in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportSite {
  /// Where the quoted path starts and ends in the file's text, in bytes, quotes excluded.
  pub start: usize,
  pub end: usize,
  pub line: usize,
  pub spec: String,
}

/// What a broken import becomes, or why it becomes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
  /// The import spelled again, naming the one file its path's tail fits.
  To(String),
  /// More than one file ends the way the import's path ends.
  Ambiguous {
    tail: String,
    candidates: Vec<String>,
  },
  /// No file does.
  Missing { tail: String },
}

/// One broken relative import and what it becomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRepair {
  pub file: String,
  pub site: ImportSite,
  pub outcome: Outcome,
}

impl ImportRepair {
  /// Whether this import could not be chosen for.
  pub fn refused(&self) -> bool {
    !matches!(self.outcome, Outcome::To(_))
  }
}

#[derive(Default)]
struct Sites {
  // Each quoted path's span in the script, quotes included, and its value.
  found: Vec<(u32, u32, String)>,
}

impl Sites {
  fn add(&mut self, literal: &StringLiteral<'_>) {
    let spec = literal.value.as_str();

    if spec.starts_with("./") || spec.starts_with("../") {
      self.found.push((literal.span.start, literal.span.end, spec.to_owned()));
    }
  }
}

impl<'a> Visit<'a> for Sites {
  fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
    self.add(&it.source);
  }

  fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
    self.add(&it.source);
  }

  fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
    self.add(&it.source);
  }

  fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
    if let Expression::StringLiteral(literal) = &it.source {
      self.add(literal);
    }

    walk::walk_import_expression(self, it);
  }

  fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
    let loads = match &it.callee {
      Expression::Identifier(name) => name.name == "require",
      Expression::StaticMemberExpression(member) => {
        member.property.name == "module"
          && matches!(&member.object, Expression::Identifier(name) if name.name == "mock")
      }
      _ => false,
    };

    if loads
      && let Some(Expression::StringLiteral(literal)) =
        it.arguments.first().and_then(Argument::as_expression)
    {
      self.add(literal);
    }

    walk::walk_call_expression(self, it);
  }
}

// The relative imports of one script, its positions moved by where the script stands in its file.
fn script_sites(name: &str, script: &str, offset: usize, text: &str, sites: &mut Vec<ImportSite>) {
  let Ok(kind) = SourceType::from_path(name) else { return };
  let allocator = Allocator::default();
  let parsed = Parser::new(&allocator, script, kind).parse();
  let mut found = Sites::default();

  found.visit_program(&parsed.program);

  for (start, end, spec) in found.found {
    let start = offset + start as usize + 1;

    sites.push(ImportSite {
      start,
      end: offset + end as usize - 1,
      line: text[..start].matches('\n').count() + 1,
      spec,
    });
  }
}

/// Every relative import in a file's text, with where its path stands, in the order the file names them. A
/// single-file component's are read off its scripts; a file that does not parse names what its parsed part names.
pub fn relative_imports(file: &str, text: &str) -> Vec<ImportSite> {
  let mut sites = Vec::new();

  if !file.ends_with(".vue") {
    script_sites(file, text, 0, text, &mut sites);
    return sites;
  }

  // The HTML reader hands a script's text over without where it stood, and a script's text is the file's own
  // characters, so each is found in the file after the one before it.
  let mut from = 0;

  for (name, script) in component_scripts(text) {
    if let Some(at) = text[from..].find(&script) {
      script_sites(&name, &script, from + at, text, &mut sites);
      from += at + script.len();
    }
  }

  sites
}

fn outside_code(path: &str) -> bool {
  path
    .rsplit_once('/')
    .is_some_and(|(dirs, _)| dirs.split('/').any(|dir| OUTSIDE_CODE.contains(&dir)))
}

/// Every file of the repository its checkout tracks or holds untracked, from its root, that is still on disk:
/// installed packages, a nuxt build's output and every project of its own left out.
pub fn repository_files(repo: &Repository) -> Result<Vec<String>, String> {
  let root = repo.root.as_path();
  let mut files: Vec<String> = Vec::new();

  for path in git_lines(root, &["ls-files", "--cached", "--others", "--exclude-standard"])? {
    if !outside_code(&path) && root.join(&path).exists() && !files.contains(&path) {
      files.push(path);
    }
  }

  Ok(repository_paths(repo, files))
}

/// The code files among `files`, which is what a run naming no file reads.
pub fn code_files(files: &[String]) -> Vec<String> {
  files
    .iter()
    .filter(|path| CODE_FILE.iter().any(|extension| path.ends_with(extension)) && !path.ends_with(".d.ts"))
    .cloned()
    .collect()
}

fn spells_extension(spec: &str) -> bool {
  SPELLED_EXTENSIONS.iter().any(|extension| spec.ends_with(extension))
}

fn left_off(path: &str) -> Option<&'static str> {
  LEFT_OFF.iter().copied().find(|extension| path.ends_with(extension))
}

// What an import that spells no extension could call a file by: its path without the extension, and its
// directory where the file is that directory's index.
fn bare_names(path: &str) -> Vec<String> {
  let Some(extension) = left_off(path) else { return Vec::new() };

  if path.ends_with(".d.ts") {
    return Vec::new();
  }

  let bare = &path[..path.len() - extension.len()];

  match bare.strip_suffix("/index") {
    Some(dir) => vec![bare.to_owned(), dir.to_owned()],
    None => vec![bare.to_owned()],
  }
}

// The files a moved import could mean, with the tail that found them: those whose path ends the way the import's
// path ends, with its `./` and `../` steps taken off. The whole tail is tried first and then one leading directory
// shorter at a time; the first length that matches anything decides.
fn moved_to(spec: &str, file: &str, all: &[String]) -> (String, Vec<String>) {
  let parts: Vec<&str> = spec.split('/').filter(|part| !matches!(*part, "." | ".." | "")).collect();
  let spelled = spells_extension(spec);
  let named: Vec<(&String, Vec<String>)> = all
    .iter()
    .filter(|path| *path != file)
    .map(|path| (path, if spelled { vec![path.clone()] } else { bare_names(path) }))
    .collect();

  for from in 0..parts.len() {
    let tail = parts[from..].join("/");
    let candidates: Vec<String> = named
      .iter()
      .filter(|(_, names)| names.iter().any(|name| *name == tail || name.ends_with(&format!("/{tail}"))))
      .map(|(path, _)| (*path).clone())
      .collect();

    if !candidates.is_empty() {
      return (tail, candidates);
    }
  }

  (parts.last().map_or_else(|| spec.to_owned(), |last| (*last).to_owned()), Vec::new())
}

fn parent_of(file: &str) -> &str {
  file.rfind('/').map_or("", |at| &file[..at])
}

// The path `to` from the directory `from`, both from the checkout's root, as a POSIX path reads it.
fn relative(from: &str, to: &str) -> String {
  let from: Vec<&str> = from.split('/').filter(|part| !part.is_empty()).collect();
  let to: Vec<&str> = to.split('/').filter(|part| !part.is_empty()).collect();
  let shared = from.iter().zip(&to).take_while(|(one, other)| one == other).count();
  let mut steps: Vec<&str> = vec![".."; from.len() - shared];

  steps.extend(&to[shared..]);
  steps.join("/")
}

/// The import spelled from where the importing file stands: relative inside one package, the package's name and the
/// module's path inside it across two. An extension the import spelled is kept, and one it left off stays off.
pub fn spelling(file: &str, spec: &str, target: &str, packages: &[PackageDeclaration]) -> String {
  let extension = left_off(target);
  let bare = extension.map_or(target, |extension| &target[..target.len() - extension.len()]);
  let shown = if !spells_extension(spec) {
    match bare.strip_suffix("/index") {
      Some(dir) if !spec.ends_with("/index") => dir.to_owned(),
      _ => bare.to_owned(),
    }
  } else if spec.ends_with(".js") && extension.is_some_and(|extension| extension != ".js") {
    format!("{bare}.js")
  } else {
    target.to_owned()
  };
  let from = owning_package(packages, file).map_or("", |one| one.dir.as_str());

  if let Some(to) = owning_package(packages, target)
    && to.dir != from
  {
    return format!("{}/{}", to.name, relative(&to.dir, &shown));
  }

  let path = relative(parent_of(file), &shown);

  if path.starts_with("../") { path } else { format!("./{path}") }
}

/// What every broken relative import in `files` becomes, or why it becomes nothing. `all` is every file of the
/// checkout, which is where a moved file is looked for; `packages` are the workspace packages it declares.
pub fn import_repairs(
  root: &Path,
  files: &[String],
  all: &[String],
  packages: &[PackageDeclaration],
) -> Vec<ImportRepair> {
  let mut repairs = Vec::new();

  for file in files {
    let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();

    for site in relative_imports(file, &text) {
      if resolve_import(root, file, &site.spec, &[]).is_some() {
        continue;
      }

      let (tail, mut candidates) = moved_to(&site.spec, file, all);
      let outcome = match candidates.len() {
        0 => Outcome::Missing { tail },
        1 => Outcome::To(spelling(file, &site.spec, &candidates.remove(0), packages)),
        _ => Outcome::Ambiguous { tail, candidates },
      };

      repairs.push(ImportRepair {
        file: file.clone(),
        site,
        outcome,
      });
    }
  }

  repairs
}

/// The repairs written into their files, every one of them or none: a run holding any refusal writes nothing.
/// Answers how many imports each file had rewritten, in the order the files first come.
pub fn apply_repairs(root: &Path, repairs: &[ImportRepair]) -> Result<Vec<(String, usize)>, String> {
  if repairs.iter().any(ImportRepair::refused) {
    return Err(VERBS.imports.none_written.clone());
  }

  let mut files: Vec<&str> = Vec::new();

  for repair in repairs {
    if !files.contains(&repair.file.as_str()) {
      files.push(&repair.file);
    }
  }

  let mut counts = Vec::new();

  for file in files {
    let mut own: Vec<&ImportRepair> = repairs.iter().filter(|repair| repair.file == file).collect();
    let path = root.join(file);
    let mut text = std::fs::read_to_string(&path).map_err(|why| format!("{file}: {why}"))?;

    own.sort_by_key(|repair| std::cmp::Reverse(repair.site.start));

    for repair in &own {
      if let Outcome::To(to) = &repair.outcome {
        text.replace_range(repair.site.start..repair.site.end, to);
      }
    }

    std::fs::write(&path, text).map_err(|why| format!("{file}: {why}"))?;
    counts.push((file.to_owned(), own.len()));
  }

  Ok(counts)
}
