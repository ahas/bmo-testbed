//! What one language answers to run in the testbed (docs/testbed.md, 「Languages」). Each language is a
//! module of its own here, a member of [`Language`], named once in [`LANGUAGES`]; the run, its result and
//! the harness guard all ask that list.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use regex::Regex;

mod browser;
mod bun;
mod cargo;
mod node;

pub use browser::Browser;
pub use bun::Bun;
pub use cargo::{Cargo, compiles as cargo_compiles, placeable as cargo_placeable};
pub use node::Node;

/// What a run's output adds up to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
  pub pass: u64,
  pub fail: u64,
  pub skip: u64,
}

/// A command as it runs holding the machine alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Widened {
  pub command: Vec<String>,
  /// Laid over the caller's environment for the command.
  pub env: BTreeMap<String, String>,
}

/// What a line of a runner's output says of one test while the tests run (docs/suite.md, 「How long a unit
/// and a test may run」).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestProgress {
  /// The test, by its name, is still running and has run for at least these seconds.
  Running { test: String, seconds: u64 },
  /// The test, by its name, has its result.
  Ended { test: String },
}

/// One language the testbed runs.
pub trait Language: Sync {
  /// The name the result line shows and `--adapter` forces.
  fn name(&self) -> &'static str;
  /// Whether this language runs the command, read from its program's name and the words after it as the
  /// shell split them. Only a heavy command is claimed: a claimed one is refused outside the testbed and
  /// queued behind every run on the machine.
  fn claims(&self, program: &str, args: &[String]) -> bool;
  /// The command handed the machine's width through the runner's own handle; a number the command already
  /// names is left as it stands.
  fn widen(&self, command: &[String], width: usize) -> Widened;
  /// The runner's own summary lines in a log, summed over every block it printed; none where none stands
  /// in it.
  fn tally(&self, log: &str) -> Option<Tally>;
  /// What failed, as short lines out of the log, in the order written.
  fn failures(&self, log: &str) -> Vec<String>;
  /// Its own naming of a test file, as git's pathspecs; none for a language that runs no test files.
  fn test_naming(&self) -> &'static [&'static str] {
    &[]
  }
  /// The name of the configuration file its runner reads from the folder it starts in, which decides the
  /// folder its test files run from; none for a language whose runner reads none.
  fn configuration_file(&self) -> Option<&'static str> {
    None
  }
  /// The command that runs these test files, each a path from the folder it runs in, handed the caller's
  /// words, with no per-test limit of the runner's own standing; none for a language that runs no test files.
  fn test_command(&self, _files: &[String], _words: &[String]) -> Option<Vec<String>> {
    None
  }
  /// The words that hand its runner a per-test limit of `seconds`, laid after the caller's `words`; none where
  /// the runner takes no such limit or the caller's words already name one.
  fn per_test_limit(&self, _words: &[String], _seconds: u64) -> Vec<String> {
    Vec::new()
  }
  /// One line of its runner's output read as a test still running or a test ended; none for any other line,
  /// and for a language whose runner says neither.
  fn test_progress(&self, _line: &str) -> Option<TestProgress> {
    None
  }
  /// The command that runs a project's package script; none for a language that runs no scripts.
  fn script_command(&self, _script: &str) -> Option<Vec<String>> {
    None
  }
  /// The files at the repository's root that every package of its workspace stands on, so a change to one
  /// reaches them all; none for a language with no package workspace.
  fn root_configuration(&self) -> &'static [&'static str] {
    &[]
  }
  /// The name of its lockfile, which beside a package manifest marks a folder as a project of its own; none
  /// for a language with no package workspace.
  fn lockfile(&self) -> Option<&'static str> {
    None
  }
}

/// Whether the words already name a flag, alone or with its value after `=`.
pub fn names_flag(words: &[String], flag: &str) -> bool {
  words.iter().any(|word| word == flag || word.starts_with(&format!("{flag}=")))
}

/// The folder a test file runs from, as a path from the repository's root: the nearest folder at or above
/// the file, inside the root, that holds the language's own configuration file; the root itself, an empty
/// path, where none does, where the language reads none, or where the file's path leaves the root.
pub fn run_folder(language: &dyn Language, root: &Path, file: &str) -> PathBuf {
  let file = Path::new(file);
  let inside = file.components().all(|part| matches!(part, Component::Normal(_) | Component::CurDir));

  language
    .configuration_file()
    .filter(|_| inside)
    .and_then(|name| file.ancestors().skip(1).find(|folder| root.join(folder).join(name).is_file()))
    .map_or_else(PathBuf::new, Path::to_path_buf)
}

/// Every language there is, in the order of their names.
pub const LANGUAGES: [&dyn Language; 4] = [&Browser, &Bun, &Cargo, &Node];

/// A command no language claims, started through the testbed by hand: run as it stands, read by its exit
/// code, with the last lines of its output standing for what failed.
pub struct Plain;

// How many of a log's last lines stand for what failed where no language named anything.
const PLAIN_LINES: usize = 40;

impl Language for Plain {
  fn name(&self) -> &'static str {
    "plain"
  }

  fn claims(&self, _: &str, _: &[String]) -> bool {
    false
  }

  fn widen(&self, command: &[String], _: usize) -> Widened {
    Widened {
      command: command.to_vec(),
      env: BTreeMap::new(),
    }
  }

  fn tally(&self, _: &str) -> Option<Tally> {
    None
  }

  fn failures(&self, log: &str) -> Vec<String> {
    let lines: Vec<&str> = log.trim_end().split('\n').collect();

    lines[lines.len().saturating_sub(PLAIN_LINES)..]
      .iter()
      .map(|line| (*line).to_owned())
      .collect()
  }
}

/// The plain language.
pub const PLAIN: &dyn Language = &Plain;

/// A command's program by its name, and the words after it, past leading assignments and `env`.
pub fn program_words(command: &[String]) -> Option<(String, Vec<String>)> {
  let base = |word: &str| {
    Path::new(word)
      .file_name()
      .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
  };
  let assignment = |word: &str| {
    word.split_once('=').is_some_and(|(name, _)| {
      let mut chars = name.chars();

      chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|one| one.is_ascii_alphanumeric() || one == '_')
    })
  };
  let at = command.iter().position(|word| !assignment(word) && base(word) != "env")?;

  Some((base(&command[at]), command[at + 1..].to_vec()))
}

/// The language a command is run by: the one named, else the first that claims it, else the plain one. A
/// name no language answers to is refused with the names there are.
pub fn language_for(
  languages: &[&'static dyn Language],
  command: &[String],
  named: Option<&str>,
) -> Result<&'static dyn Language, Vec<&'static str>> {
  let all = || languages.iter().copied().chain(std::iter::once(PLAIN));

  if let Some(named) = named {
    return all()
      .find(|one| one.name() == named)
      .ok_or_else(|| all().map(|one| one.name()).collect());
  }

  Ok(program_words(command).map_or(PLAIN, |(program, args)| {
    languages.iter().copied().find(|one| one.claims(&program, &args)).unwrap_or(PLAIN)
  }))
}

/// What a log adds up to, read by every language at once: a package script's run prints whatever its
/// runners print, Cargo's results beside bun's, so no one language's reading is the whole of it. None
/// where no language found a summary.
pub fn tally_of_log(languages: &[&dyn Language], log: &str) -> Option<Tally> {
  languages.iter().filter_map(|one| one.tally(log)).reduce(|sum, one| Tally {
    pass: sum.pass + one.pass,
    fail: sum.fail + one.fail,
    skip: sum.skip + one.skip,
  })
}

/// What failed, as every language read it, each line once.
pub fn failures_of_log(languages: &[&dyn Language], log: &str) -> Vec<String> {
  let mut seen: Vec<String> = Vec::new();

  for line in languages.iter().flat_map(|one| one.failures(log)) {
    if !seen.contains(&line) {
      seen.push(line);
    }
  }

  seen
}

/// Whether an option of the command's own already names a value, in either of its two spellings.
pub(crate) fn names_option(command: &[String], names: &[&str]) -> bool {
  command
    .iter()
    .any(|word| names.iter().any(|name| word == name || word.starts_with(&format!("{name}="))))
}

/// Every number a pattern's first group finds on a line of the log, summed; none where it finds none.
/// Thousands come with commas in some runners' summaries.
pub(crate) fn sum_of(log: &str, pattern: &Regex) -> Option<u64> {
  log.split('\n').fold(None, |sum, line| match pattern.captures(line) {
    Some(found) => Some(sum.unwrap_or(0) + found[1].replace(',', "").parse::<u64>().unwrap_or(0)),
    None => sum,
  })
}

/// A pattern of a language's own, compiled once.
pub(crate) fn pattern(text: &str) -> Regex {
  Regex::new(text).expect("a language's own pattern")
}
