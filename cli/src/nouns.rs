//! The nouns this binary declares to `bmo` (bmo-cli's `docs/api.md`, 「The unit command」;
//! docs/boundary.md, 「The nouns `bmo` serves」): the platform's names for the verbs, each a line as
//! `nouns` prints it.

/// The nouns `bmo` dispatches to this binary, one a line, as `nouns` prints them.
pub const NOUNS: [&str; 17] = [
  "test suite",
  "test count",
  "test log",
  "test template",
  "test full",
  "check",
  "imports",
  "cargo",
  "cargo prune",
  "deploy commit",
  "deploy backups",
  "backup snapshot",
  "backup media",
  "backup drill",
  "backup prune",
  "install",
  "upgrade",
];

/// What `nouns` prints.
pub fn lines() -> impl Iterator<Item = &'static str> {
  NOUNS.into_iter()
}
