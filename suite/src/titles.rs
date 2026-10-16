//! The titles a TypeScript test file gives its tests, read off the source by oxc (docs/checks.md, 「The test
//! count」): every call of `test` or `it` — `test.skip(…)` and `test.if(…)(…)` among them — whose first
//! argument the source spells out whole, in the order the source writes them.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Argument, CallExpression, Expression};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::imports::spelled;

/// The names a test is called by.
pub const TEST_CALLS: [&str; 2] = ["test", "it"];

// The name a call is made under: `test("…")`, `test.skip("…")` and `test.if(…)("…")` are all `test`.
fn callee_name<'a>(callee: &Expression<'a>) -> Option<String> {
  match callee {
    Expression::Identifier(name) => Some(name.name.to_string()),
    Expression::StaticMemberExpression(member) => callee_name(&member.object),
    Expression::ComputedMemberExpression(member) => callee_name(&member.object),
    Expression::CallExpression(call) => callee_name(&call.callee),
    _ => None,
  }
}

struct Titles<'n> {
  names: &'n [&'n str],
  found: Vec<String>,
}

impl<'a> Visit<'a> for Titles<'_> {
  fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
    if callee_name(&it.callee).is_some_and(|name| self.names.contains(&name.as_str()))
      && let Some(title) = it.arguments.first().and_then(Argument::as_expression).and_then(spelled)
    {
      self.found.push(title);
    }

    walk::walk_call_expression(self, it);
  }
}

/// The titles of every call under one of `names` a source makes, in the order it writes them; refused where
/// the source does not parse, since a count that guessed would lose tests.
pub fn called_titles(source: &str, names: &[&str]) -> Result<Vec<String>, String> {
  let allocator = Allocator::default();
  let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();

  if parsed.fatal_error {
    return Err(parsed.diagnostics.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "));
  }

  let mut titles = Titles {
    names,
    found: Vec::new(),
  };

  titles.visit_program(&parsed.program);
  Ok(titles.found)
}

/// The tests one source holds, by title.
pub fn test_titles(source: &str) -> Result<Vec<String>, String> {
  called_titles(source, &TEST_CALLS)
}
