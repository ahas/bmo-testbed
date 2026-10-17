//! The `classify` verb (docs/boundary.md, 「The verbs」): whether a shell line holds a heavy command started
//! outside the testbed, and the line that runs it there. Whether a command is heavy is each language's own
//! answer (docs/testbed.md, 「Languages」); this reads the line the way the shell splits it and asks the
//! testbed's languages.

use std::collections::HashMap;

use bmo_testbed_core::languages::Language;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::verbs::VERBS;
use bmo_testbed_suite::own_binary::running_testbed;
use serde::Serialize;

use crate::shell::{basename, expanded, is_assignment, program_of, script_of, split};

// The testbed's own binary, whose run holds whatever follows it.
const TESTBED_BINARY: &str = "bmo-testbed";
// How deep text run as commands inside text run as commands is read.
const DEPTH_MOST: usize = 3;

/// A command the caller names as one that takes the turn itself: its program and the words it opens with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TakesTurn(Vec<String>);

impl TakesTurn {
  /// The command a shell line names, or none where the line holds no words the shell reads.
  pub fn read(line: &str) -> Option<Self> {
    shlex::split(line).filter(|words| !words.is_empty()).map(Self)
  }

  // Whether a command opens this way: the same program, its first words past its flags the ones named.
  fn opens(&self, program: &str, args: &[String]) -> bool {
    let Some((first, rest)) = self.0.split_first() else { return false };

    basename(first) == program
      && args.iter().filter(|word| !word.starts_with('-')).take(rest.len()).eq(rest.iter())
  }
}

/// A heavy command a line holds: the language that claims it, and its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heavy {
  pub language: &'static str,
  pub command: Vec<String>,
}

// Whether a command is a run of the testbed itself, or of a command that takes the turn itself.
fn through_testbed(program: &str, args: &[String], takes_turn: &[TakesTurn]) -> bool {
  let running = running_testbed();
  let own = running.file_name().and_then(|name| name.to_str());

  program == TESTBED_BINARY || own == Some(program) || takes_turn.iter().any(|one| one.opens(program, args))
}

// The variables a command of assignments alone gives the commands after it, `export` and its kin among them.
fn assign(words: &[String], assigned: &mut HashMap<String, String>) {
  let given = match words.first().map(String::as_str) {
    Some("export" | "local" | "readonly" | "typeset" | "declare") => &words[1..],
    _ if words.iter().all(|word| is_assignment(word)) => words,
    _ => return,
  };

  for word in given.iter().filter(|word| is_assignment(word)) {
    if let Some((name, value)) = word.split_once('=') {
      assigned.insert(name.trim_end_matches('+').to_owned(), value.to_owned());
    }
  }
}

fn heavy_within(
  text: &str,
  languages: &[&dyn Language],
  takes_turn: &[TakesTurn],
  depth: usize,
) -> Option<Heavy> {
  if depth > DEPTH_MOST {
    return None;
  }

  let read = split(text);
  let mut assigned = HashMap::new();

  for command in &read.commands {
    let words: Vec<String> = command.words.iter().flat_map(|word| expanded(word, &assigned)).collect();

    assign(&words, &mut assigned);

    let Some(run) = program_of(&words) else {
      continue;
    };

    // A run of the testbed holds whatever follows it, so nothing after it in the same command is read.
    if through_testbed(&run.program, &run.args, takes_turn) {
      continue;
    }

    if let Some(language) = languages.iter().find(|language| language.claims(&run.program, &run.args)) {
      return Some(Heavy {
        language: language.name(),
        command: [vec![run.program.clone()], run.args.clone()].concat(),
      });
    }

    if let Some(within) =
      script_of(&run).and_then(|script| heavy_within(&script, languages, takes_turn, depth + 1))
    {
      return Some(within);
    }
  }

  read
    .inner
    .iter()
    .find_map(|inner| heavy_within(&inner.text, languages, takes_turn, depth + 1))
}

/// The first heavy command a shell line holds outside the testbed, or none.
pub fn heavy_command(line: &str, languages: &[&dyn Language], takes_turn: &[TakesTurn]) -> Option<Heavy> {
  heavy_within(line, languages, takes_turn, 0)
}

/// The line that runs a heavy command in the testbed: the running binary's `run --` and its words.
pub fn instead(heavy: &Heavy) -> String {
  let quoted =
    |word: &str| shlex::try_quote(word).map_or_else(|_| word.to_owned(), |word| word.into_owned());
  let words: Vec<String> = heavy.command.iter().map(|word| quoted(word)).collect();

  fill_marks(
    &VERBS.classify.instead,
    &[("testbed", &quoted(&running_testbed().to_string_lossy())), ("words", &words.join(" "))],
  )
}

// What `classify --json` prints.
#[derive(Serialize)]
struct Answer {
  heavy: bool,
  language: Option<&'static str>,
  command: Option<Vec<String>>,
  instead: Option<String>,
}

/// What `classify` answers for a line, thin or as JSON, and the code it ends with: 0 for light, 1 for heavy.
pub fn answer(heavy: Option<&Heavy>, json: bool) -> (String, u8) {
  let said = &VERBS.classify;
  let code = u8::from(heavy.is_some());

  if json {
    let answer = Answer {
      heavy: heavy.is_some(),
      language: heavy.map(|heavy| heavy.language),
      command: heavy.map(|heavy| heavy.command.clone()),
      instead: heavy.map(instead),
    };

    return (serde_json::to_string(&answer).unwrap_or_default(), code);
  }

  match heavy {
    None => (said.light.clone(), code),
    Some(heavy) => (
      format!("{}\n{}", fill_marks(&said.heavy, &[("language", heavy.language)]), instead(heavy)),
      code,
    ),
  }
}
