//! The testbed's own fixed numbers and words, one TOML file to a part under the repository's `policy/`,
//! compiled in here. Each part is read once, on first use, into its type.

use serde::de::DeserializeOwned;

pub mod answer;
pub mod backups;
pub mod close_line;
pub mod count;
pub mod daily;
pub mod declaration;
pub mod dependents;
pub mod deploy;
pub mod deploy_steps;
pub mod deploy_tool;
pub mod events;
pub mod lanes;
pub mod main_branch;
pub mod plugin;
pub mod prune;
pub mod quick_checks;
pub mod record;
pub mod service;
pub mod static_check;
pub mod suite;
pub mod verbs;
pub mod versions;

/// A line of the policy with each `{name}` it holds filled in, every one at once so a value that itself
/// holds a mark is left as it is; a mark nothing names stays.
pub fn fill_marks(line: &str, values: &[(&str, &str)]) -> String {
  let mut out = String::new();
  let mut rest = line;

  while let Some(open) = rest.find('{') {
    out.push_str(&rest[..open]);

    let after = &rest[open + 1..];
    let name_end = after
      .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
      .unwrap_or(after.len());
    let value = (name_end > 0 && after[name_end..].starts_with('}'))
      .then(|| values.iter().find(|(name, _)| *name == &after[..name_end]))
      .flatten();

    match value {
      Some((_, value)) => {
        out.push_str(value);
        rest = &after[name_end + 1..];
      }
      None => {
        out.push('{');
        rest = after;
      }
    }
  }

  out + rest
}

/// A line of the policy filled as `fill_marks` fills it, each value written out as it displays.
pub fn fill_shown(line: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
  let shown: Vec<(&str, String)> = values.iter().map(|(name, value)| (*name, value.to_string())).collect();
  let pairs: Vec<(&str, &str)> = shown.iter().map(|(name, value)| (*name, value.as_str())).collect();

  fill_marks(line, &pairs)
}

/// One part read from its file's text. A file that does not read as its type is a fault of the
/// build, which every part's own test meets before anything runs on it.
pub fn read<T: DeserializeOwned>(file: &str, text: &str) -> T {
  toml::from_str(text).unwrap_or_else(|error| panic!("the testbed's policy {file} does not read: {error}"))
}

/// A part of the policy, compiled in from its file under the repository's `policy/` and read once on
/// first use.
macro_rules! part {
  ($vis:vis $name:ident: $type:ty = $file:literal) => {
    $vis static $name: std::sync::LazyLock<$type> = std::sync::LazyLock::new(|| {
      $crate::policy::read($file, include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../policy/", $file)))
    });
  };
}

use part;
