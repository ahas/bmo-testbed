//! A template's `{{name}}` marks filled (docs/install.md, 「Values」). The marks are the install declaration's own
//! syntax, not a general format: a mark opens with two braces and closes with two, and holds a name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Why a template could not be filled.
#[derive(Debug, PartialEq, Eq)]
pub enum Unfilled {
  /// A mark names what no declaration of the install declares.
  Unknown(String),
  /// A mark names a file that does not stand yet.
  Waiting(String),
  /// A mark opens and never closes.
  Open,
}

/// Where a value of the install comes from.
#[derive(Clone, Debug)]
pub enum Source {
  /// Fixed text, a port's number or a place.
  Text(String),
  /// A file's content: a token's or an input's, with what stands where the file does not.
  File { path: PathBuf, absent: Absent },
  /// Another template, a declared value.
  Template(String),
}

/// What a file-held value stands for while its file does not stand.
#[derive(Clone, Debug)]
pub enum Absent {
  /// Its default, a template, or an empty optional input.
  Text(String),
  /// The name itself, in a dry run, where the file would be made before anything reads it.
  Planned(String),
  /// Nothing: the template cannot be filled.
  Missing,
}

/// The values a template reaches, by the name its marks call them.
#[derive(Clone, Debug, Default)]
pub struct Values {
  pub named: BTreeMap<String, Source>,
  /// Where a `file.` mark's path is read under, its `~/` read under `home`.
  pub data_root: PathBuf,
  pub home: PathBuf,
  /// Where a path beginning `prefix:` is read under: the install prefix.
  pub prefix: PathBuf,
}

impl Values {
  /// A path as a declaration names it: under the install prefix where it begins `prefix:`, under the home directory
  /// where it begins `~/`, else under the data root.
  pub fn path(&self, named: &str) -> PathBuf {
    match named.strip_prefix("prefix:") {
      Some(under_prefix) => self.prefix.join(under_prefix.trim_start_matches('/')),
      None => resolve(named, &self.data_root, &self.home),
    }
  }

  /// The template with every mark filled.
  pub fn fill(&self, template: &str) -> Result<String, Unfilled> {
    self.fill_within(template, 0)
  }

  fn fill_within(&self, template: &str, depth: usize) -> Result<String, Unfilled> {
    let mut out = String::new();
    let mut rest = template;

    while let Some(open) = rest.find("{{") {
      out.push_str(&rest[..open]);

      let after = &rest[open + 2..];
      let close = after.find("}}").ok_or(Unfilled::Open)?;
      let name = after[..close].trim();

      out.push_str(&self.value(name, depth)?);
      rest = &after[close + 2..];
    }

    out.push_str(rest);
    Ok(out)
  }

  fn value(&self, name: &str, depth: usize) -> Result<String, Unfilled> {
    if let Some(path) = name.strip_prefix("file.") {
      let path = self.path(path);

      return read_value(&path).ok_or_else(|| Unfilled::Waiting(path.display().to_string()));
    }

    match self.named.get(name) {
      Some(Source::Text(text)) => Ok(text.clone()),
      Some(Source::File { path, absent }) => match read_value(path) {
        Some(text) => Ok(text),
        None => match absent {
          // A default is a template of its own, so it may name a port the install chose.
          Absent::Text(text) if depth < 16 => self.fill_within(text, depth + 1),
          Absent::Text(_) => Err(Unfilled::Unknown(name.to_owned())),
          Absent::Planned(name) => Ok(format!("<{name}>")),
          Absent::Missing => Err(Unfilled::Unknown(name.to_owned())),
        },
      },
      // A value naming itself, or a ring of values, is no value: the depth bounds the reading, not any work.
      Some(Source::Template(template)) if depth < 16 => self.fill_within(template, depth + 1),
      _ => Err(Unfilled::Unknown(name.to_owned())),
    }
  }
}

/// A path as a declaration names it: under the data root, or under the home directory where it begins `~/`; an
/// absolute path as it stands.
pub fn resolve(named: &str, data_root: &Path, home: &Path) -> PathBuf {
  match named.strip_prefix("~/") {
    Some(under_home) => home.join(under_home),
    None => data_root.join(named),
  }
}

// A file's content, its last line ending taken off; none where it does not stand or holds nothing.
fn read_value(path: &Path) -> Option<String> {
  let text = std::fs::read_to_string(path).ok()?;
  let text = text.strip_suffix('\n').unwrap_or(&text);
  let text = text.strip_suffix('\r').unwrap_or(text);

  (!text.is_empty()).then(|| text.to_owned())
}
