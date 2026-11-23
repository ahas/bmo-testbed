//! A repository's install declaration (docs/install.md, 「The install declaration」): `install.toml` at its root,
//! naming the programs, ports, tokens, inputs, values, databases, settings, lines, after-steps and verifications
//! installing it needs.

use std::path::Path;

use serde::Deserialize;

/// The file's name at a repository's root.
pub const FILE: &str = "install.toml";

/// The schema's major this testbed reads.
pub const MAJOR: u64 = 1;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallDeclaration {
  pub install: u64,
  pub restart: Option<String>,
  pub report: Option<String>,
  #[serde(default)]
  pub needs: Vec<Need>,
  #[serde(default)]
  pub port: Vec<Port>,
  #[serde(default)]
  pub token: Vec<Token>,
  #[serde(default)]
  pub input: Vec<Input>,
  #[serde(default)]
  pub value: Vec<Value>,
  #[serde(default)]
  pub database: Vec<Database>,
  #[serde(default)]
  pub settings: Vec<Settings>,
  #[serde(default)]
  pub line: Vec<EnvLine>,
  #[serde(default)]
  pub after: Vec<After>,
  #[serde(default)]
  pub verify: Vec<Verify>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Need {
  pub program: Option<String>,
  pub check: Option<String>,
  pub what: String,
  pub source: Option<NeedSource>,
}

/// Where a missing program is built from (docs/install.md, 「A program built from its source」).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeedSource {
  /// The folder the repository is cloned into under the data root's `install/sources/`.
  pub name: String,
  pub repository: String,
  /// The tag or commit checked out.
  pub rev: String,
  /// The command, run from the clone, that builds the program and places it where the need asks for it.
  pub build: String,
}

impl Need {
  /// What the need is called on its line: its program, else its check.
  pub fn name(&self) -> String {
    self.program.clone().or_else(|| self.check.clone()).unwrap_or_default()
  }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
  pub name: String,
  pub number: u16,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
  #[default]
  Hex,
  Base64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Token {
  pub name: String,
  pub file: String,
  pub bytes: Option<usize>,
  #[serde(default)]
  pub encoding: Encoding,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
  pub name: String,
  pub what: String,
  pub file: String,
  #[serde(default)]
  pub secret: bool,
  #[serde(default)]
  pub optional: bool,
  pub default: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Value {
  pub name: String,
  pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
  pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
  pub file: String,
  pub text: String,
  pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvLine {
  pub file: String,
  pub key: String,
  pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct After {
  pub id: String,
  pub run: String,
  pub done: Option<String>,
  #[serde(default)]
  pub inputs: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verify {
  pub id: String,
  pub run: String,
  pub what: String,
}

/// How reading a checkout's declaration ends.
#[derive(Debug)]
pub enum Read {
  Declared(Box<InstallDeclaration>),
  Missing,
  Unread(String),
}

impl InstallDeclaration {
  pub fn parse(text: &str) -> Result<Self, String> {
    let declaration: Self = toml::from_str(text).map_err(|why| why.to_string())?;

    if declaration.install != MAJOR {
      return Err(format!(
        "install = {} is not the major this testbed reads, {MAJOR}",
        declaration.install
      ));
    }

    Ok(declaration)
  }

  /// The declaration a checkout holds at its root.
  pub fn read(checkout: &Path) -> Read {
    match std::fs::read_to_string(checkout.join(FILE)) {
      Ok(text) => match Self::parse(&text) {
        Ok(declaration) => Read::Declared(Box::new(declaration)),
        Err(why) => Read::Unread(why),
      },
      Err(why) if why.kind() == std::io::ErrorKind::NotFound => Read::Missing,
      Err(why) => Read::Unread(why.to_string()),
    }
  }

  /// Every name the declaration declares, with its kind: the names a template reaches.
  pub fn names(&self) -> Vec<String> {
    let token = self.token.iter().map(|token| format!("token.{}", token.name));
    let port = self.port.iter().map(|port| format!("port.{}", port.name));
    let input = self.input.iter().map(|input| format!("input.{}", input.name));
    let value = self.value.iter().map(|value| format!("value.{}", value.name));

    token.chain(port).chain(input).chain(value).collect()
  }
}
