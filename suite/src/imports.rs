//! What a module of the checkout loads, read off its source by oxc (docs/checks.md, 「Which tests a change
//! runs」): every module it names by a quoted path — an `import` or `export … from`, a dynamic `import()`, a
//! `require()` and a test's `mock.module()` — and the checkout file each one names. A single-file component's
//! scripts are read out of it by an HTML reader first.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
  Argument, CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression, ImportDeclaration,
  ImportExpression,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// The extensions a code file of the checkout is read under.
pub const CODE_EXTENSIONS: [&str; 6] = [".ts", ".tsx", ".mts", ".js", ".mjs", ".vue"];

// The extensions an import may leave off, in the order a bare path is tried against them.
const LEFT_OFF: [&str; 5] = [".ts", ".tsx", ".mts", ".js", ".mjs"];

/// A string a source spells out whole: a literal, or a template with nothing put into it.
pub fn spelled<'a>(expression: &Expression<'a>) -> Option<String> {
  match expression {
    Expression::StringLiteral(text) => Some(text.value.to_string()),
    Expression::TemplateLiteral(template) if template.expressions.is_empty() => template
      .quasis
      .first()
      .and_then(|quasi| quasi.value.cooked.as_ref())
      .map(ToString::to_string),
    _ => None,
  }
}

#[derive(Default)]
struct Specifiers {
  found: Vec<String>,
  // Imports and re-exports of types alone left out, as the runtime strips them before it loads anything.
  values_only: bool,
}

impl Specifiers {
  fn add(&mut self, spec: String) {
    if !self.found.contains(&spec) {
      self.found.push(spec);
    }
  }
}

impl<'a> Visit<'a> for Specifiers {
  fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
    if !(self.values_only && it.import_kind.is_type()) {
      self.add(it.source.value.to_string());
    }
  }

  fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
    if !(self.values_only && it.export_kind.is_type()) {
      self.add(it.source.value.to_string());
    }
  }

  fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
    if !(self.values_only && it.export_kind.is_type()) {
      self.add(it.source.value.to_string());
    }
  }

  fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
    if let Some(spec) = spelled(&it.source) {
      self.add(spec);
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

    if loads && let Some(spec) = it.arguments.first().and_then(Argument::as_expression).and_then(spelled) {
      self.add(spec);
    }

    walk::walk_call_expression(self, it);
  }
}

fn script_specifiers(name: &str, text: &str, found: &mut Specifiers) {
  let Ok(kind) = SourceType::from_path(name) else { return };
  let allocator = Allocator::default();
  let parsed = Parser::new(&allocator, text, kind).parse();

  found.visit_program(&parsed.program);
}

// The scripts of a single-file component, each with the language it is written in.
pub(crate) fn component_scripts(text: &str) -> Vec<(String, String)> {
  let page = scraper::Html::parse_document(text);
  let Ok(scripts) = scraper::Selector::parse("script") else { return Vec::new() };

  page
    .select(&scripts)
    .map(|script| {
      let lang = script.value().attr("lang").unwrap_or("js");

      (format!("component.{lang}"), script.text().collect::<String>())
    })
    .collect()
}

/// Every module a file's code names, relative or not, in the order it names them; none twice. A file that
/// does not parse names what its parsed part names.
pub fn module_specifiers(file: &str, text: &str) -> Vec<String> {
  let mut found = Specifiers::default();

  if file.ends_with(".vue") {
    for (name, script) in component_scripts(text) {
      script_specifiers(&name, &script, &mut found);
    }
  } else {
    script_specifiers(file, text, &mut found);
  }

  found.found
}

/// The modules a file loads when it runs, in the order it names them, none twice: what `module_specifiers`
/// finds less the imports and re-exports of types alone, which the runtime strips before it loads anything.
/// None where the file does not parse, since what it would load is then unknown.
pub fn loaded_specifiers(file: &str, text: &str) -> Option<Vec<String>> {
  let kind = SourceType::from_path(file).ok()?;
  let allocator = Allocator::default();
  let parsed = Parser::new(&allocator, text, kind).parse();

  if parsed.fatal_error || parsed.diagnostics.has_errors() {
    return None;
  }

  let mut found = Specifiers {
    values_only: true,
    ..Specifiers::default()
  };

  found.visit_program(&parsed.program);

  Some(found.found)
}
/// A workspace package as an import names it: its directory from the root, and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePackage {
  pub dir: String,
  pub name: String,
}

// A path with `.` and `..` read off, as a POSIX join leaves it.
fn normalized(path: &str) -> String {
  let mut parts: Vec<&str> = Vec::new();

  for part in path.split('/') {
    match part {
      "" | "." => {}
      ".." => {
        parts.pop();
      }
      part => parts.push(part),
    }
  }

  parts.join("/")
}

fn parent_of(file: &str) -> &str {
  file.rfind('/').map_or("", |at| &file[..at])
}

// The file a path names the way the runtime finds it: itself, with an extension it left off, as a
// directory's index, or a `.js` spelling of a TypeScript file.
fn on_disk(root: &Path, path: &str) -> Option<String> {
  let mut tries = vec![path.to_owned()];

  tries.extend(LEFT_OFF.iter().map(|extension| format!("{path}{extension}")));
  tries.extend(LEFT_OFF.iter().map(|extension| normalized(&format!("{path}/index{extension}"))));

  if let Some(stem) = path.strip_suffix(".js") {
    tries.extend([format!("{stem}.ts"), format!("{stem}.tsx")]);
  }

  tries.into_iter().find(|one| root.join(one).is_file())
}

/// The checkout file an import names, from the checkout's root, or none where it names none this reading
/// can find: a relative path, or a workspace package's name followed by the module's path inside it. A
/// package on the registry and an alias a configuration defines name nothing here.
pub fn resolve_import(
  root: &Path,
  file: &str,
  spec: &str,
  packages: &[WorkspacePackage],
) -> Option<String> {
  if spec.starts_with("./") || spec.starts_with("../") {
    return on_disk(root, &normalized(&format!("{}/{spec}", parent_of(file))));
  }

  let named = packages
    .iter()
    .filter(|one| spec.starts_with(&format!("{}/", one.name)))
    .max_by_key(|one| one.name.len())?;

  on_disk(root, &normalized(&format!("{}/{}", named.dir, &spec[named.name.len() + 1..])))
}
