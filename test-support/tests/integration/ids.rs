//! The fixed ids are one set in two languages (docs/boundary.md, 「The crates」): every id this crate holds
//! is the test kit's own, as its TypeScript spells it, read by the TypeScript parser.

use std::collections::BTreeMap;

use bmo_testbed_test_support::{ACCOUNT, MESSAGE, OWNER, ROOM, UNIT, WORKER};
use oxc_allocator::Allocator;
use oxc_ast::ast::{Declaration, Expression, Statement, UnaryOperator};
use oxc_parser::Parser;
use oxc_span::SourceType;

// A constant's value as the source spells it: a string as it stands, a number as an integer.
fn spelled(value: &Expression) -> Option<String> {
  match value {
    Expression::StringLiteral(text) => Some(text.value.to_string()),
    Expression::NumericLiteral(number) => Some(format!("{}", number.value as i64)),
    Expression::UnaryExpression(negated) if negated.operator == UnaryOperator::UnaryNegation => {
      spelled(&negated.argument).map(|number| format!("-{number}"))
    }
    _ => None,
  }
}

// Every constant the test kit's ids export, by name.
fn test_kit_ids() -> BTreeMap<String, String> {
  let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../testkit/ids.ts");
  let source = std::fs::read_to_string(path).unwrap();
  let allocator = Allocator::default();
  let parsed = Parser::new(&allocator, &source, SourceType::ts()).parse();

  assert!(!parsed.fatal_error, "{path} does not parse");

  let mut ids = BTreeMap::new();

  for statement in &parsed.program.body {
    if let Statement::ExportDeclaration(export) = statement
      && let Declaration::VariableDeclaration(constants) = &export.declaration
    {
      for constant in &constants.declarations {
        let name = constant.id.get_binding_identifier().unwrap().name.to_string();
        let value = constant.init.as_ref().and_then(spelled).unwrap();

        ids.insert(name, value);
      }
    }
  }

  ids
}

#[test]
fn every_id_is_the_test_kits_own() {
  let ours: BTreeMap<String, String> = [
    ("OWNER_ID", OWNER.to_string()),
    ("ROOM_ID", ROOM.to_string()),
    ("MESSAGE_ID", MESSAGE.to_string()),
    ("ACCOUNT_ID", ACCOUNT.to_owned()),
    ("WORKER", WORKER.to_owned()),
    ("UNIT", UNIT.to_owned()),
  ]
  .into_iter()
  .map(|(name, value)| (name.to_owned(), value))
  .collect();

  assert_eq!(test_kit_ids(), ours);
}
