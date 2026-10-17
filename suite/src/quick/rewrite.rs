//! A markdown file's dashes between words rewritten the way a person types the same sentence (docs/checks.md,
//! 「The quick checks」): a name and its explanation get a colon, an aside between two dashes of one sentence
//! gets commas, or brackets where it has commas of its own, every further dash in the sentence a comma, and a
//! dash between two digits is a range. The file is read by the markdown parser, and only its paragraphs,
//! headings and table cells are prose; code, quotations, links and web addresses keep their dashes, and so do
//! words in double quotes and a dash standing alone at either end of a piece.
//!
//! The rule over a piece of prose is this repository's own reading of a sentence, not a general format, so
//! it reads the characters themselves.

use std::sync::LazyLock;

use markdown::mdast::Node;
use markdown::{ParseOptions, to_mdast};
use regex::Regex;

// What may follow a sentence's closing mark: any footnote mark, emphasis, quote or bracket closing on it,
// then a space or the text's end.
static AFTER_SENTENCE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"^(?:\[[^\]\s]{1,8}\])*[*_"'\u{201d}\u{2019})\]\u{300d}\u{300f}]*(?:\s|$)"#)
    .expect("a sentence's end")
});
// A line's opening list mark, which is not a word the dash could stand after.
static LIST_MARK: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^\s*(?:[-*+\u{2022}]|[0-9]{1,3}[.)])$").expect("a list mark"));
// A label opening a line, such as "TITLE: ": its colon names the line, and the sentence after it may still
// take one. The space after the colon is matched and left out of the label.
static LABEL: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^\s*(?:[-*+\u{2022}]\s+|[0-9]{1,3}[.)]\s+)?[^\s:.,;!?]+(?: [^\s:.,;!?]+){0,2}:\s")
    .expect("a label")
});
static QUOTED: LazyLock<Regex> =
  LazyLock::new(|| Regex::new("\"[^\"]*\"|\u{201c}[^\u{201d}]*\u{201d}").expect("a quotation"));

// Marks after which the dash only doubled a pause the sentence already has.
fn pause(character: Option<&char>) -> bool {
  matches!(character, Some('.' | ',' | ':' | ';' | '!' | '?' | '\u{3002}' | '\u{3001}'))
}

fn opener(character: Option<&char>) -> bool {
  matches!(character, Some('(' | '[' | '{' | '\u{201c}' | '\u{2018}' | '\u{300c}' | '\u{300e}'))
}

fn closer(character: Option<&char>) -> bool {
  matches!(character, Some(')' | ']' | '}' | '\u{201d}' | '\u{2019}' | '\u{300d}' | '\u{300f}'))
}

// Marks inside an aside that commas around it would run together with.
fn aside_pause(character: &char) -> bool {
  matches!(character, ',' | ';' | ':')
}

fn is_space(character: Option<&char>) -> bool {
  matches!(character, Some(' ' | '\t' | '\n'))
}

fn is_digit(character: Option<&char>) -> bool {
  character.is_some_and(char::is_ascii_digit)
}

fn is_dash(character: char) -> bool {
  matches!(character, '\u{2014}' | '\u{2015}')
}

fn text(characters: &[char]) -> String {
  characters.iter().collect()
}

// The last line break before `before`.
fn line_break_before(piece: &[char], before: usize) -> Option<usize> {
  piece[..before.min(piece.len())].iter().rposition(|one| *one == '\n')
}

/// Whether a text holds a dash that may stand between words: an em dash or a horizontal bar, or an en dash
/// with a space either side; an en dash touching its words is a range and stays.
pub fn any_dash(text: &str) -> bool {
  text.chars().any(is_dash) || text.contains(" \u{2013} ")
}

// Each dash of a piece: a run of em dashes or bars, or one spaced en dash.
fn dash_runs(piece: &[char]) -> Vec<(usize, usize)> {
  let mut runs = vec![];
  let mut at = 0;

  while at < piece.len() {
    if is_dash(piece[at]) {
      let end = at + piece[at..].iter().take_while(|one| is_dash(**one)).count();

      runs.push((at, end));
      at = end;
    } else if piece[at] == '\u{2013}' && at > 0 && piece[at - 1] == ' ' && piece.get(at + 1) == Some(&' ') {
      runs.push((at, at + 1));
      at += 1;
    } else {
      at += 1;
    }
  }

  runs
}

struct Dash {
  start: usize,
  end: usize,
  // The nearest character that is not white space on each side.
  left: usize,
  right: usize,
  sentence: usize,
}

struct Swap {
  from: usize,
  to: usize,
  text: String,
}

// The stretches of a piece that are not the writer's prose: the ones the parser found, and words in double
// quotes, each quote paired within its line.
fn kept_spans(piece: &[char], found: &[(usize, usize)]) -> Vec<(usize, usize)> {
  let mut spans = found.to_vec();
  let mut offset = 0;

  for line in text(piece).split('\n') {
    for quoted in QUOTED.find_iter(line) {
      let start = offset + line[..quoted.start()].chars().count();

      spans.push((start, start + quoted.as_str().chars().count()));
    }

    offset += line.chars().count() + 1;
  }

  spans
}

// What takes a dash's place with the white space around it: a mark that holds on to the word before, a mark
// that holds on to the word after, and between them a space, or the line break and indent of the line the
// text goes on in where the dash stood at a wrap.
fn swap_at(piece: &[char], dash: &Dash, before: &str, after: &str) -> Swap {
  let ahead = &piece[dash.left + 1..dash.start];
  let behind = &piece[dash.end..dash.right];
  let from_break = |side: &[char]| side.iter().position(|one| *one == '\n').map(|at| text(&side[at..]));
  let wrap = from_break(ahead).or_else(|| from_break(behind)).unwrap_or_else(|| " ".to_owned());

  Swap {
    from: dash.left + 1,
    to: dash.right,
    text: format!("{before}{wrap}{after}"),
  }
}

// Whether every bracket in a stretch closes inside it.
fn balanced(stretch: &[char]) -> bool {
  let mut depth = 0i64;

  for character in stretch {
    if opener(Some(character)) {
      depth += 1;
    } else if closer(Some(character)) {
      depth -= 1;

      if depth < 0 {
        return false;
      }
    }
  }

  depth == 0
}

/// One piece of prose with its dashes between words rewritten, `found` the stretches of it, by character,
/// that are not prose.
pub fn piece_without_dashes(text_of: &str, found: &[(usize, usize)]) -> String {
  if !any_dash(text_of) {
    return text_of.to_owned();
  }

  let piece: Vec<char> = text_of.chars().collect();
  let kept = kept_spans(&piece, found);
  let in_kept = |at: usize| kept.iter().any(|(start, end)| at >= *start && at < *end);
  let ends: Vec<usize> = (0..piece.len())
    .filter(|at| matches!(piece[*at], '.' | '!' | '?' | '\u{3002}'))
    .filter(|at| AFTER_SENTENCE.is_match(&text(&piece[at + 1..])))
    .filter(|at| !in_kept(*at))
    .collect();
  let mut dashes = vec![];

  for (start, end) in dash_runs(&piece) {
    if in_kept(start) {
      continue;
    }

    let mut left = start as i64 - 1;
    let mut right = end;

    while left >= 0 && is_space(piece.get(left as usize)) {
      left -= 1;
    }

    while right < piece.len() && is_space(piece.get(right)) {
      right += 1;
    }

    if left < 0 || right >= piece.len() {
      continue;
    }

    let left = left as usize;

    if piece[left] == '|' || piece[right] == '|' {
      continue;
    }

    // A dash a bracket or a quote holds is the character named, not a pause.
    if (opener(piece.get(left)) || piece[left] == '"') && (closer(piece.get(right)) || piece[right] == '"')
    {
      continue;
    }

    let line_start = line_break_before(&piece, left).map_or(0, |at| at + 1);

    if LIST_MARK.is_match(&text(&piece[line_start..=left])) {
      continue;
    }

    dashes.push(Dash {
      start,
      end,
      left,
      right,
      sentence: ends.iter().filter(|at| **at < start).count(),
    });
  }

  let mut swaps = vec![];
  let mut prose = vec![];

  for dash in dashes {
    let before = piece.get(dash.left);

    if dash.left + 1 == dash.start
      && dash.right == dash.end
      && is_digit(before)
      && is_digit(piece.get(dash.right))
    {
      swaps.push(Swap {
        from: dash.start,
        to: dash.end,
        text: "~".to_owned(),
      });
    } else if pause(before) {
      swaps.push(swap_at(&piece, &dash, "", ""));
    } else if opener(before) {
      swaps.push(Swap {
        from: dash.left + 1,
        to: dash.right,
        text: String::new(),
      });
    } else {
      prose.push(dash);
    }
  }

  let mut at = 0;

  while at < prose.len() {
    let dash = &prose[at];

    // Two dashes set off an aside only where no bracket opens or closes between them without the other.
    if let Some(next) = prose.get(at + 1)
      && next.sentence == dash.sentence
      && balanced(&piece[dash.right..=next.left])
    {
      let bracketed = piece[dash.right..=next.left].iter().any(aside_pause);
      // Where the sentence already pauses right after the aside, its own mark closes it.
      let closing = if pause(piece.get(next.right)) {
        Swap {
          from: next.left + 1,
          to: next.right,
          text: if bracketed { ")" } else { "" }.to_owned(),
        }
      } else {
        swap_at(&piece, next, if bracketed { ")" } else { "," }, "")
      };

      swaps.push(if bracketed { swap_at(&piece, dash, "", "(") } else { swap_at(&piece, dash, ",", "") });
      swaps.push(closing);
      at += 2;
      continue;
    }

    let line_start = line_break_before(&piece, dash.start).map_or(0, |at| at + 1);
    let label = LABEL.find(&text(&piece[line_start..])).map(|found| found.as_str().chars().count() - 1);
    let opening = (if dash.sentence == 0 { 0 } else { ends[dash.sentence - 1] + 1 })
      .max(label.map_or(0, |label| line_start + label));
    let colon_before = (opening..dash.start).any(|at| {
      piece[at] == ':'
        && !in_kept(at)
        && !(at > 0 && is_digit(piece.get(at - 1)) && is_digit(piece.get(at + 1)))
    }) || swaps
      .iter()
      .any(|swap| swap.text.starts_with(':') && swap.from >= opening && swap.from < dash.start);

    swaps.push(swap_at(&piece, dash, if colon_before { "," } else { ":" }, ""));
    at += 1;
  }

  swaps.sort_by_key(|swap| swap.from);

  let mut written = String::new();
  let mut from = 0;

  for swap in swaps {
    written += &text(&piece[from.min(swap.from)..swap.from]);
    written += &swap.text;
    from = swap.to;
  }

  written + &text(&piece[from.min(piece.len())..])
}

// A node's place in the text, by byte.
fn span(node: &Node) -> Option<(usize, usize)> {
  node.position().map(|at| (at.start.offset, at.end.offset))
}

// Whether a node within prose is not the writer's prose: code, markup, a link or a web address.
fn not_prose(node: &Node) -> bool {
  matches!(
    node,
    Node::InlineCode(_)
      | Node::InlineMath(_)
      | Node::Html(_)
      | Node::Link(_)
      | Node::LinkReference(_)
      | Node::Image(_)
  )
}

// Every stretch under a node that is not prose, by byte.
fn kept_under(node: &Node, found: &mut Vec<(usize, usize)>) {
  for child in node.children().into_iter().flatten() {
    if not_prose(child) {
      found.extend(span(child));
    } else {
      kept_under(child, found);
    }
  }
}

// A piece of prose: its place in the text, and the stretches in it that are not prose, by byte.
type Piece = ((usize, usize), Vec<(usize, usize)>);

// The pieces of prose under a node, each its place and the stretches in it that are not prose; nothing under
// a quotation or code.
fn pieces(node: &Node, found: &mut Vec<Piece>) {
  match node {
    Node::Blockquote(_) | Node::Code(_) | Node::Math(_) | Node::Html(_) => {}
    Node::Paragraph(_) | Node::Heading(_) | Node::TableCell(_) => {
      if let Some(place) = span(node) {
        let mut kept = Vec::new();

        kept_under(node, &mut kept);
        found.push((place, kept));
      }
    }
    _ => {
      for child in node.children().into_iter().flatten() {
        pieces(child, found);
      }
    }
  }
}

/// A markdown text with every dash left between words in its prose rewritten. A text the parser cannot read
/// stands as it is.
pub fn without_dashes(markdown_text: &str) -> String {
  if !any_dash(markdown_text) {
    return markdown_text.to_owned();
  }

  let Ok(tree) = to_mdast(markdown_text, &ParseOptions::gfm()) else {
    return markdown_text.to_owned();
  };
  let mut found = Vec::new();

  pieces(&tree, &mut found);
  found.sort_by_key(|((start, _), _)| *start);

  let mut written = String::new();
  let mut from = 0;

  for ((start, end), kept) in found {
    if start < from || end > markdown_text.len() {
      continue;
    }

    let piece = &markdown_text[start..end];
    let by_char = |byte: usize| piece[..byte.saturating_sub(start).min(piece.len())].chars().count();
    let kept: Vec<(usize, usize)> = kept.iter().map(|(from, to)| (by_char(*from), by_char(*to))).collect();

    written += &markdown_text[from..start];
    written += &piece_without_dashes(piece, &kept);
    from = end;
  }

  written + &markdown_text[from..]
}
