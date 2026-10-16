//! A source file of the checkout as the quick checks read it (docs/checks.md, 「The quick checks」): its
//! comments, its regular expressions and its test modules, which hold no words for a person, and the pieces of
//! text its code writes, each at the line it opens on. Each language is read by its own parser, so a comment, a
//! pattern and a test are never mistaken for text: TypeScript by oxc, Rust by the tree-sitter Rust grammar, and a
//! screen by the tree-sitter HTML grammar with oxc for its scripts and the expressions of its template.

use std::ops::Range;

use oxc_allocator::Allocator;
use oxc_ast::ast::{JSXText, RegExpLiteral, StringLiteral, TemplateElement, TemplateLiteral};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;
use tree_sitter::{Node, Tree};

/// A stretch of a text by byte offset, its end excluded.
pub type Span = Range<usize>;

/// A piece of text the code writes: a literal, a template read whole, a screen's text or attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
  /// The line the piece opens on, counted from one.
  pub line: usize,
  pub text: String,
}

/// What a source holds: the stretches that are no words of the code's own, and the pieces of text it writes.
#[derive(Debug, Default)]
pub struct Reading {
  pub comments: Vec<Span>,
  /// Regular expressions, which are patterns rather than words.
  pub patterns: Vec<Span>,
  /// Stretches whose code is test input whatever the file is: a crate's test module.
  pub tests: Vec<Span>,
  pub pieces: Vec<Piece>,
}

impl Reading {
  // The reading of a text standing inside a larger one, from `offset` on and opening on line `line`.
  fn moved(mut self, offset: usize, line: usize) -> Self {
    for span in self.comments.iter_mut().chain(&mut self.patterns).chain(&mut self.tests) {
      *span = span.start + offset..span.end + offset;
    }

    for piece in &mut self.pieces {
      piece.line += line - 1;
    }

    self
  }
}

/// A language a parser here reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
  Script { tsx: bool },
  Screen,
  Rust,
}

impl Language {
  /// The language a path is written in, by its extension, where a parser here reads it.
  pub fn of(path: &str) -> Option<Self> {
    match path.rsplit_once('.').map(|(_, extension)| extension)? {
      "ts" | "mts" | "cts" | "js" | "mjs" | "cjs" => Some(Self::Script { tsx: false }),
      "tsx" | "jsx" => Some(Self::Script { tsx: true }),
      "vue" => Some(Self::Screen),
      "rs" => Some(Self::Rust),
      _ => None,
    }
  }

  /// A text written in this language, read.
  pub fn read(self, text: &str) -> Reading {
    match self {
      Self::Script { tsx } => script_reading(text, tsx),
      Self::Screen => screen_reading(text),
      Self::Rust => rust_reading(text),
    }
  }
}

/// The line a byte offset of a text stands on, counted from one.
pub fn line_at(text: &str, offset: usize) -> usize {
  1 + text.as_bytes()[..offset.min(text.len())].iter().filter(|byte| **byte == b'\n').count()
}

/// Whether an offset lies in any of the spans.
pub fn within(spans: &[Span], at: usize) -> bool {
  spans.iter().any(|span| span.contains(&at))
}

/// Whether a text holds a dash between words: an em dash, a horizontal bar, or an en dash with a space either
/// side, an en dash touching its words being a range.
pub fn holds_dash(text: &str) -> bool {
  text.contains(['\u{2014}', '\u{2015}']) || text.contains(" \u{2013} ")
}

/// Whether a text is nothing but one dash, the mark for an empty value in a table or a figure.
pub fn is_placeholder(text: &str) -> bool {
  text == "\u{2014}" || text == "\u{2015}"
}

fn cooked(element: &TemplateElement) -> String {
  element.value.cooked.as_ref().unwrap_or(&element.value.raw).to_string()
}

// The literals inside a template, its own parts and those in its holes, each at the offset it stands at.
#[derive(Default)]
struct TemplateParts {
  found: Vec<(u32, String)>,
}

impl<'a> Visit<'a> for TemplateParts {
  fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
    self.found.push((it.span.start, it.value.to_string()));
  }

  fn visit_template_element(&mut self, it: &TemplateElement<'a>) {
    self.found.push((it.span.start, cooked(it)));
  }

  fn visit_jsx_text(&mut self, it: &JSXText<'a>) {
    self.found.push((it.span.start, it.value.to_string()));
  }
}

// The literals and patterns of a script, each piece at the offset it opens at.
#[derive(Default)]
struct ScriptPieces {
  pieces: Vec<(u32, String)>,
  patterns: Vec<Span>,
}

impl<'a> Visit<'a> for ScriptPieces {
  fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
    self.pieces.push((it.span.start, it.value.to_string()));
  }

  fn visit_jsx_text(&mut self, it: &JSXText<'a>) {
    self.pieces.push((it.span.start, it.value.to_string()));
  }

  fn visit_reg_exp_literal(&mut self, it: &RegExpLiteral<'a>) {
    self.patterns.push(it.span.start as usize..it.span.end as usize);
  }

  // A template is read whole, with every literal inside its holes, so the Korean on one side of a hole and the
  // dash joining a value on the other, or in a template nested in it, are one piece. Its line is that of the
  // first part holding a dash.
  fn visit_template_literal(&mut self, it: &TemplateLiteral<'a>) {
    if it.expressions.is_empty() {
      if let Some(only) = it.quasis.first() {
        self.pieces.push((it.span.start, cooked(only)));
      }

      return;
    }

    let mut parts = TemplateParts::default();

    walk::walk_template_literal(&mut parts, it);
    parts.found.sort_by_key(|(at, _)| *at);
    parts.found.retain(|(_, text)| !text.is_empty() && !is_placeholder(text));

    let at = parts
      .found
      .iter()
      .find(|(_, text)| holds_dash(text))
      .map_or(it.span.start, |(at, _)| *at);
    let text: Vec<&str> = parts.found.iter().map(|(_, text)| text.as_str()).collect();

    self.pieces.push((at, text.join("\u{2026}")));

    // A pattern in a hole is a pattern still.
    let mut holes = ScriptPieces::default();

    for expression in &it.expressions {
      holes.visit_expression(expression);
    }

    self.patterns.extend(holes.patterns);
  }
}

// A TypeScript text: every comment, every regular expression and every literal. A text that does not parse
// whole gives what its parsed part holds.
fn script_reading(text: &str, tsx: bool) -> Reading {
  let allocator = Allocator::default();
  let kind = if tsx { SourceType::tsx() } else { SourceType::ts() };
  let parsed = Parser::new(&allocator, text, kind).parse();
  let mut found = ScriptPieces::default();

  found.visit_program(&parsed.program);

  let comments = parsed.program.comments.iter();

  Reading {
    comments: comments.map(|comment| comment.span.start as usize..comment.span.end as usize).collect(),
    patterns: found.patterns,
    tests: Vec::new(),
    pieces: found
      .pieces
      .into_iter()
      .map(|(at, piece)| Piece {
        line: line_at(text, at as usize),
        text: piece,
      })
      .collect(),
  }
}

// A text parsed by a tree-sitter grammar; none where the parser gives up.
fn parse(language: tree_sitter::Language, text: &str) -> Option<Tree> {
  let mut parser = tree_sitter::Parser::new();

  parser.set_language(&language).ok()?;
  parser.parse(text, None)
}

// Every node of a tree, each before the nodes inside it.
fn every_node(tree: &Tree) -> Vec<Node<'_>> {
  let mut nodes = Vec::new();
  let mut left = vec![tree.root_node()];

  while let Some(node) = left.pop() {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    left.extend(children.into_iter().rev());
    nodes.push(node);
  }

  nodes
}

fn node_text<'t>(node: Node, text: &'t str) -> &'t str {
  text.get(node.byte_range()).unwrap_or_default()
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
  let mut cursor = node.walk();

  node.children(&mut cursor).collect()
}

// A Rust text: its comments, its test modules, and its string literals outside them, a macro's included.
fn rust_reading(text: &str) -> Reading {
  let Some(tree) = parse(tree_sitter_rust::LANGUAGE.into(), text) else {
    return Reading::default();
  };
  let mut reading = Reading::default();
  let mut strings = Vec::new();

  for node in every_node(&tree) {
    match node.kind() {
      "line_comment" | "block_comment" => reading.comments.push(node.byte_range()),
      "mod_item"
        if node
          .child_by_field_name("name")
          .is_some_and(|name| matches!(node_text(name, text), "tests" | "test")) =>
      {
        reading.tests.push(node.byte_range());
      }
      "string_literal" | "raw_string_literal" => strings.push(node),
      _ => {}
    }
  }

  reading.pieces = strings
    .into_iter()
    .filter(|node| !within(&reading.tests, node.start_byte()))
    .map(|node| Piece {
      line: node.start_position().row + 1,
      text: node_text(node, text).to_owned(),
    })
    .collect();
  reading
}

// The name of an element's tag, read off its opening tag.
fn tag_name<'t>(element: Node, text: &'t str) -> Option<&'t str> {
  children(element)
    .into_iter()
    .find(|child| matches!(child.kind(), "start_tag" | "self_closing_tag"))
    .and_then(|tag| children(tag).into_iter().find(|child| child.kind() == "tag_name"))
    .map(|name| node_text(name, text))
}

// An attribute's name and its value's node, where it has one.
fn attribute_parts<'t>(attribute: Node<'t>, text: &'t str) -> (&'t str, Option<Node<'t>>) {
  let mut name = "";
  let mut value = None;

  for child in children(attribute) {
    match child.kind() {
      "attribute_name" => name = node_text(child, text),
      "attribute_value" => value = Some(child),
      "quoted_attribute_value" => {
        value = Some(
          children(child)
            .into_iter()
            .find(|inner| inner.kind() == "attribute_value")
            .unwrap_or(child),
        );
      }
      _ => {}
    }
  }

  (name, value)
}

// The literals of one of the template's expressions, standing in the file from `offset` on.
fn expression_pieces(source: &str, offset: usize, text: &str) -> Vec<Piece> {
  script_reading(&format!("({source})"), false).moved(0, line_at(text, offset)).pieces
}

// The pieces between a template's tags: its words, cut at Vue's interpolations, whose expressions are read by
// oxc. The HTML grammar knows no interpolation, so the stretch between two tags is cut at Vue's own delimiters.
fn gap_pieces(text: &str, from: usize, to: usize, pieces: &mut Vec<Piece>) {
  let words = |pieces: &mut Vec<Piece>, at: usize, until: usize| {
    let stretch = &text[at..until];
    let trimmed = stretch.trim();

    if !trimmed.is_empty() {
      let lead = stretch.len() - stretch.trim_start().len();

      pieces.push(Piece {
        line: line_at(text, at + lead),
        text: trimmed.to_owned(),
      });
    }
  };
  let mut at = from;

  while let Some(open) = text[at..to].find("{{").map(|open| at + open) {
    let Some(close) = text[open + 2..to].find("}}").map(|close| open + 2 + close) else {
      break;
    };

    words(pieces, at, open);

    let source = &text[open + 2..close];
    let lead = source.len() - source.trim_start().len();

    pieces.extend(expression_pieces(source.trim(), open + 2 + lead, text));
    at = close + 2;
  }

  words(pieces, at, to);
}

// Every piece of an element of the template: its attributes where `own_tag` says they are read, the words
// between its tags and the elements inside it.
fn template_pieces(element: Node, text: &str, own_tag: bool, pieces: &mut Vec<Piece>) {
  let mut gap: Option<(usize, usize)> = None;

  for child in children(element) {
    if matches!(child.kind(), "text" | "entity" | "erroneous_end_tag" | "ERROR") {
      gap = Some((gap.map_or(child.start_byte(), |(from, _)| from), child.end_byte()));
      continue;
    }

    if let Some((from, to)) = gap.take() {
      gap_pieces(text, from, to, pieces);
    }

    match child.kind() {
      "start_tag" | "self_closing_tag" if own_tag => {
        for attribute in children(child).into_iter().filter(|one| one.kind() == "attribute") {
          let (name, value) = attribute_parts(attribute, text);
          let directive = ["v-", ":", "@", "#", "."].iter().any(|mark| name.starts_with(mark));

          match value {
            Some(value) if directive => {
              pieces.extend(expression_pieces(node_text(value, text), value.start_byte(), text));
            }
            None if directive => {}
            value => pieces.push(Piece {
              line: attribute.start_position().row + 1,
              text: value.map_or("", |value| node_text(value, text)).to_owned(),
            }),
          }
        }
      }
      "element" => template_pieces(child, text, true, pieces),
      _ => {}
    }
  }

  if let Some((from, to)) = gap {
    gap_pieces(text, from, to, pieces);
  }
}

// A screen: its scripts read as TypeScript, every comment of its template, its style blocks counted as comments
// since a stylesheet carries no words of its own, and in its template every text, every fixed attribute and the
// literals of every expression.
fn screen_reading(text: &str) -> Reading {
  let Some(tree) = parse(tree_sitter_html::LANGUAGE.into(), text) else {
    return Reading::default();
  };
  let mut reading = Reading::default();

  for node in every_node(&tree).into_iter().filter(|node| node.kind() == "comment") {
    reading.comments.push(node.byte_range());
  }

  for block in children(tree.root_node()) {
    match block.kind() {
      "script_element" => {
        let tsx = children(block)
          .into_iter()
          .filter(|child| child.kind() == "start_tag")
          .flat_map(children)
          .filter(|child| child.kind() == "attribute")
          .any(|attribute| {
            let (name, value) = attribute_parts(attribute, text);

            name == "lang" && value.is_some_and(|value| node_text(value, text) == "tsx")
          });

        if let Some(body) = children(block).into_iter().find(|child| child.kind() == "raw_text") {
          let script = script_reading(node_text(body, text), tsx)
            .moved(body.start_byte(), body.start_position().row + 1);

          reading.comments.extend(script.comments);
          reading.patterns.extend(script.patterns);
          reading.pieces.extend(script.pieces);
        }
      }
      "style_element" => reading.comments.push(block.byte_range()),
      "element" if tag_name(block, text) == Some("template") => {
        template_pieces(block, text, false, &mut reading.pieces);
      }
      _ => {}
    }
  }

  reading
}
