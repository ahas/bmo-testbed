//! A shell line read the way the shell splits it, for the `classify` verb (docs/testbed.md, 「A shell hook
//! sends a harness's heavy command to it」). Each simple command comes out as its words with their quoting kept
//! as written and the words its redirects name apart from them; the text of every command and process
//! substitution comes out beside them, since the shell runs it as commands of its own; and every heredoc's
//! body is kept out, since a body is text a command carries rather than commands. What a script file hides
//! is out of its sight.
//!
//! The reading is brush-parser's tokenizer and word parser, which read the bash grammar a harness's line is
//! written in almost always. A line they cannot read, a quote left open or a construct of zsh's alone,
//! reads as no command at all: the shell refuses the first, and the second is out of this reading's sight
//! as a script file is.

use std::collections::HashMap;

use brush_parser::word::{self, Parameter, ParameterExpr, WordPiece, WordPieceWithSource};
use brush_parser::{ParserOptions, Token};

/// The shells a command line can hand text to run.
pub const SHELLS: [&str; 4] = ["sh", "bash", "zsh", "dash"];

/// One simple command as the shell splits it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
  /// Its words as written, quoting kept.
  pub words: Vec<String>,
  /// The files its redirects name, as written; none of them is an argument of the program.
  pub redirects: Vec<String>,
}

/// The text a command substitution or a process substitution runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Substitution {
  pub text: String,
}

/// A command line read: its simple commands in order, and the substitutions standing in their words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
  pub commands: Vec<Command>,
  pub inner: Vec<Substitution>,
}

// What the next word is, after a redirect operator.
enum Pending {
  File,
  // A descriptor a `>&` or `<&` duplicates, which names no file.
  Duplicate,
}

const OUTPUTS: [&str; 6] = [">", ">>", ">|", "&>", "&>>", ">&"];
const INPUTS: [&str; 3] = ["<", "<&", "<>"];

// Byte offsets of a text's characters, so a token's place, which brush counts in characters, slices it.
fn offsets(text: &str) -> Vec<usize> {
  text.char_indices().map(|(at, _)| at).chain([text.len()]).collect()
}

fn span_of(token: &Token) -> (usize, usize) {
  match token {
    Token::Word(_, span) | Token::Operator(_, span) => (span.start.index, span.end.index),
  }
}

fn is_operator(token: Option<&Token>, wanted: &str) -> bool {
  matches!(token, Some(Token::Operator(op, _)) if op == wanted)
}

// One simple command ended: kept where it holds anything.
fn end_command(current: &mut Command, read: &mut Split) {
  let done = std::mem::take(current);

  if !done.words.is_empty() || !done.redirects.is_empty() {
    read.commands.push(done);
  }
}

/// The command line read, or nothing read where the tokenizer cannot read it.
pub fn split(text: &str) -> Split {
  let Ok(tokens) = brush_parser::tokenize_str(text) else {
    return Split::default();
  };
  let places = offsets(text);
  let byte = |at: usize| places.get(at).copied().unwrap_or(text.len());
  let mut read = Split::default();
  let mut current = Command::default();
  let mut pending: Option<Pending> = None;
  let mut at = 0;

  while at < tokens.len() {
    match &tokens[at] {
      Token::Word(word, _) => {
        let next = tokens.get(at + 1);
        let adjacent = next.is_some_and(|next| span_of(next).0 == span_of(&tokens[at]).1);
        let redirect_follows = matches!(next, Some(Token::Operator(op, _))
          if OUTPUTS.contains(&op.as_str()) || INPUTS.contains(&op.as_str()) || op.starts_with("<<"));

        // A descriptor standing against its redirect belongs to it, not to the command's words.
        if pending.is_none() && adjacent && redirect_follows && word.chars().all(|ch| ch.is_ascii_digit()) {
          at += 1;
          continue;
        }

        match pending.take() {
          Some(Pending::File) => current.redirects.push(word.clone()),
          Some(Pending::Duplicate) => {}
          None => {
            read.inner.extend(substitutions(word));
            current.words.push(word.clone());
          }
        }

        at += 1;
      }
      Token::Operator(op, span) => {
        let op = op.as_str();
        let process = (op == "<" || op == ">")
          && is_operator(tokens.get(at + 1), "(")
          && tokens.get(at + 1).is_some_and(|open| span_of(open).0 == span.end.index);

        if process {
          let close = closing(&tokens, at + 1);
          let inside_from = byte(span_of(&tokens[at + 1]).1);
          let (inside_to, word_to) = tokens
            .get(close)
            .map_or((text.len(), text.len()), |end| (byte(span_of(end).0), byte(span_of(end).1)));

          read.inner.push(Substitution {
            text: text[inside_from..inside_to].to_owned(),
          });
          current.words.push(text[byte(span.start.index)..word_to].to_owned());
          at = close + 1;
          continue;
        }

        // A heredoc: its tag, its body and its closing delimiter are text the command carries.
        if op == "<<" || op == "<<-" {
          let tag = match tokens.get(at + 1) {
            Some(Token::Word(tag, _)) => tag.clone(),
            _ => String::new(),
          };
          let plain = brush_parser::unquote_str(&tag);
          let mut next = at + 2;

          if matches!(tokens.get(next), Some(Token::Word(text, _)) if *text != plain) {
            next += 1;
          }

          if matches!(tokens.get(next), Some(Token::Word(end, _)) if *end == plain) {
            next += 1;
          }

          at = next;
          continue;
        }

        if op == "<<<" || INPUTS.contains(&op) || OUTPUTS.contains(&op) {
          let duplicates = op == "<&"
            || (op == ">&"
              && matches!(tokens.get(at + 1), Some(Token::Word(fd, _)) if fd == "-" || fd.chars().all(|ch| ch.is_ascii_digit())));

          pending = Some(if duplicates { Pending::Duplicate } else { Pending::File });
        } else {
          end_command(&mut current, &mut read);
          pending = None;
        }

        at += 1;
      }
    }
  }

  end_command(&mut current, &mut read);

  read
}

// The token closing the bracket opened at `open`, counted with nesting, or the end where it never closes.
fn closing(tokens: &[Token], open: usize) -> usize {
  let mut depth = 0;

  for (at, token) in tokens.iter().enumerate().skip(open) {
    match token {
      Token::Operator(op, _) if op == "(" => depth += 1,
      Token::Operator(op, _) if op == ")" => {
        depth -= 1;

        if depth == 0 {
          return at;
        }
      }
      _ => {}
    }
  }

  tokens.len()
}

fn pieces(word: &str) -> Vec<WordPieceWithSource> {
  word::parse(word, &ParserOptions::default()).unwrap_or_default()
}

/// The command substitutions standing in a word, in order.
pub fn substitutions(word: &str) -> Vec<Substitution> {
  fn walk(pieces: &[WordPieceWithSource], found: &mut Vec<Substitution>) {
    for piece in pieces {
      match &piece.piece {
        WordPiece::CommandSubstitution(text) | WordPiece::BackquotedCommandSubstitution(text) => {
          found.push(Substitution { text: text.clone() });
        }
        WordPiece::DoubleQuotedSequence(inner) | WordPiece::GettextDoubleQuotedSequence(inner) => {
          walk(inner, found);
        }
        _ => {}
      }
    }
  }

  let mut found = vec![];

  walk(&pieces(word), &mut found);
  found
}

/// A word with its quoting taken off and its escapes put in, every expansion and substitution left as
/// written.
pub fn value(word: &str) -> String {
  let read = pieces(word);

  // A word the word parser cannot read stands as written.
  if read.is_empty() {
    return word.to_owned();
  }

  let mut text = String::new();

  for piece in read {
    let source = word.get(piece.start_index..piece.end_index).unwrap_or_default();

    match &piece.piece {
      WordPiece::Text(plain) | WordPiece::SingleQuotedText(plain) | WordPiece::AnsiCQuotedText(plain) => {
        text.push_str(plain);
      }
      WordPiece::DoubleQuotedSequence(_) | WordPiece::GettextDoubleQuotedSequence(_) => {
        let inner = source.strip_prefix('$').unwrap_or(source);

        text.push_str(inner.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')).unwrap_or(inner));
      }
      WordPiece::EscapeSequence(escaped) => text.push_str(escaped.strip_prefix('\\').unwrap_or(escaped)),
      _ => text.push_str(source),
    }
  }

  text
}

// The variable a word is a use of and nothing more, and whether the use stood in double quotes.
fn variable_of(word: &str) -> Option<(String, bool)> {
  let named = |piece: &WordPiece| match piece {
    WordPiece::ParameterExpansion(ParameterExpr::Parameter {
      parameter: Parameter::Named(name),
      indirect: false,
    }) => Some(name.clone()),
    _ => None,
  };

  match pieces(word).as_slice() {
    [one] => match &one.piece {
      WordPiece::DoubleQuotedSequence(inner) => match inner.as_slice() {
        [only] => named(&only.piece).map(|name| (name, true)),
        _ => None,
      },
      piece => named(piece).map(|name| (name, false)),
    },
    _ => None,
  }
}

/// The words a command's word stands for once the variables the line itself assigned are put in: a use of
/// one alone is its value, split into words where it stood unquoted; every other word is its value.
pub fn expanded(word: &str, assigned: &HashMap<String, String>) -> Vec<String> {
  match variable_of(word).and_then(|(name, quoted)| assigned.get(&name).map(|value| (value, quoted))) {
    Some((value, true)) => vec![value.clone()],
    Some((value, false)) => value.split_whitespace().map(str::to_owned).collect(),
    None => vec![value(word)],
  }
}

/// The last part of a word's path.
pub fn basename(word: &str) -> String {
  word.rsplit('/').next().unwrap_or(word).to_owned()
}

/// A program a simple command runs, with the words after it and the runners looked past.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
  pub program: String,
  pub args: Vec<String>,
  pub runners: Vec<String>,
}

// What may stand in front of a program without being it: runners, and the words of the shell's own
// grammar that open a command.
const RUNNERS: [&str; 14] = [
  "env", "command", "exec", "nohup", "time", "sudo", "doas", "nice", "ionice", "stdbuf", "setsid",
  "timeout", "builtin", "busybox",
];
const GRAMMAR: [&str; 11] = ["if", "then", "else", "elif", "do", "while", "until", "!", "{", "}", "time"];

/// An assignment standing before a program, `NAME=value` or `NAME+=value`.
pub fn is_assignment(word: &str) -> bool {
  let name_end = word.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')).unwrap_or(word.len());
  let rest = &word[name_end..];

  name_end > 0
    && !word.starts_with(|ch: char| ch.is_ascii_digit())
    && (rest.starts_with('=') || rest.starts_with("+="))
}

/// The program a simple command runs and the words after it, its words already taken out of their quoting,
/// looking past assignments, runners and the grammar that opens a command, with the runners it looked past.
pub fn program_of(words: &[String]) -> Option<Run> {
  let mut passed = vec![];
  let mut at = 0;

  while at < words.len() {
    let word = &words[at];

    if is_assignment(word) || GRAMMAR.contains(&word.as_str()) {
      at += 1;
      continue;
    }

    let name = basename(word);

    if RUNNERS.contains(&name.as_str()) {
      at += 1;

      while at < words.len() && words[at].starts_with('-') {
        at += 1;
      }

      // The duration `timeout` takes before its program.
      if name == "timeout"
        && words.get(at).is_some_and(|word| word.starts_with(|ch: char| ch.is_ascii_digit()))
      {
        at += 1;
      }

      passed.push(name);
      continue;
    }

    return Some(Run {
      program: name,
      args: words[at + 1..].to_vec(),
      runners: passed,
    });
  }

  None
}

/// Whether a word is a shell's `-c` flag, alone or in a cluster.
pub fn is_command_flag(word: &str) -> bool {
  word.len() > 1
    && word.starts_with('-')
    && word[1..].chars().all(|ch| ch.is_ascii_alphabetic())
    && word[1..].contains('c')
}

/// The text a command hands the shell to run as commands: a shell's `-c` script, or what `eval` is given.
pub fn script_of(run: &Run) -> Option<String> {
  if run.program == "eval" {
    return Some(run.args.join(" "));
  }

  if !SHELLS.contains(&run.program.as_str()) {
    return None;
  }

  let at = run.args.iter().position(|arg| is_command_flag(arg))?;

  run.args.get(at + 1).cloned()
}
