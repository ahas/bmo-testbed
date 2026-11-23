//! The install's run (docs/install.md, 「The order of a run」): each stage in order, every step read from the machine
//! first so a run that stopped is taken up again, and every step said as one line.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use base64::Engine;
use bmo_testbed_core::declaration::{Declaration, Repository};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::install::INSTALL;
use serde::{Deserialize, Serialize};

use crate::declaration::{Encoding, InstallDeclaration, NeedSource, Read};
use crate::line::{Line, Status};
use crate::machine::{Machine, Ran};
use crate::presets::{FILE as PRESETS_FILE, PresetsFile, Refused, Resolved};
use crate::template::{Absent, Source, Unfilled, Values, resolve};

/// The variable naming the index checkout where `--index` is not given.
pub const INDEX_VARIABLE: &str = "BMO_INDEX";
/// The console features file, under the data root (docs/install.md, 「The console features」).
pub const FEATURES_FILE: &str = "env/console-features.toml";
/// The install's own state, under the data root (docs/install.md, 「The install's own state」).
pub const STATE_FILE: &str = "install/preset.json";
/// The ports the install chose, by name, under the data root (docs/install.md, 「Ports」).
pub const PORTS_FILE: &str = "install/ports.json";

/// Where a missing program's source is cloned, under the data root (docs/install.md, 「A program built from its
/// source」).
pub const SOURCES_DIR: &str = "install/sources";
/// How many ports past its declared number a port is looked for at, where the declared one is taken.
const PORT_TRIES: u16 = 200;

/// What the verb is asked to do.
#[derive(Clone, Debug)]
pub enum Mode {
  Preset { name: String, with: Vec<String> },
  Add(String),
  Status,
  Set(String),
}

/// The verb's words and the places it runs against.
#[derive(Clone, Debug)]
pub struct Ask {
  pub mode: Mode,
  pub index: Option<PathBuf>,
  pub data_root: PathBuf,
  pub home: PathBuf,
  pub user: String,
  pub dry_run: bool,
  /// The directory the verb runs in, whose checkout is the index where nothing else names one.
  pub cwd: PathBuf,
  /// The install prefix, under which every repository's deploy root and data root stand.
  pub prefix: PathBuf,
  /// Where the services reach PostgreSQL and Valkey: a system's own or ones a packager bundles.
  pub services: Services,
}

/// The servers the installed services reach, as the install's values name them (docs/install.md, 「Values」).
#[derive(Clone, Debug)]
pub struct Services {
  /// PostgreSQL's host: a socket folder or a host name.
  pub postgres_host: String,
  pub postgres_port: String,
  /// Valkey's address, with no database number.
  pub valkey: String,
}

impl Default for Services {
  fn default() -> Self {
    Self {
      postgres_host: "/var/run/postgresql".to_owned(),
      postgres_port: "5432".to_owned(),
      valkey: "redis://127.0.0.1:6379".to_owned(),
    }
  }
}

/// How a run ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
  Done,
  Waiting,
  Failed,
  Refused(String),
}

/// The exit code of an outcome (docs/install.md, 「What it prints」).
pub fn exit_code(outcome: &Outcome) -> u8 {
  match outcome {
    Outcome::Done => 0,
    Outcome::Failed => 1,
    Outcome::Refused(_) => 2,
    Outcome::Waiting => 3,
  }
}

/// The state the install keeps: the preset, the repositories chosen and those added.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
  pub preset: String,
  #[serde(default)]
  pub with: Vec<String>,
  #[serde(default)]
  pub added: Vec<String>,
}

// One repository the install installs.
struct Unit {
  name: String,
  checkout: PathBuf,
  declaration: InstallDeclaration,
  testbed: Option<Declaration>,
}

impl Unit {
  // The deploy tree's link, where the repository deploys anything.
  fn deploy_tree(&self, ask: &Ask) -> Option<PathBuf> {
    let deploy = &self.testbed.as_ref()?.deploy;
    let root = deploy.root.as_ref()?;

    Some(resolve(&root.to_string_lossy(), &self.checkout, &ask.home).join(&deploy.link))
  }

  fn deploy_root(&self, ask: &Ask) -> Option<PathBuf> {
    self.deploy_tree(ask).and_then(|tree| tree.parent().map(Path::to_path_buf))
  }

  fn deployed(&self, ask: &Ask) -> bool {
    self.deploy_tree(ask).is_some_and(|tree| tree.exists())
  }
}

// What writing a settings file or a line came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wrote {
  /// Written now.
  Now,
  /// Standing already, planned in a dry run, or failed: nothing more to do in this run.
  Settled,
  /// Put off until a file it names stands.
  Waiting,
}

// A settings file or a line put off until a file it names stands.
enum Deferred {
  Settings { unit: usize, at: usize },
  Line { unit: usize, at: usize },
}

/// One run of the verb.
pub struct Install<'a> {
  ask: Ask,
  machine: &'a dyn Machine,
  say: &'a mut dyn FnMut(&Line),
  failed: Option<String>,
  waiting: usize,
}

fn mark(line: &str, values: &[(&str, &str)]) -> String {
  fill_marks(line, values)
}

fn quoted(said: &str) -> String {
  let chars = INSTALL.numbers.quoted_chars;
  let count = said.chars().count();

  if count <= chars {
    said.to_owned()
  } else {
    said.chars().skip(count - chars).collect()
  }
}

/// A file written whole: beside its place first, then renamed over it, readable as its mode says.
pub fn write_whole(path: &Path, text: &str, mode: u32) -> Result<(), String> {
  let parent = path.parent().ok_or_else(|| "no folder".to_owned())?;

  std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;

  let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
  let beside = parent.join(format!(".{name}.install"));
  let mut file = std::fs::File::create(&beside).map_err(|why| why.to_string())?;

  std::fs::set_permissions(&beside, std::fs::Permissions::from_mode(mode))
    .map_err(|why| why.to_string())?;
  file.write_all(text.as_bytes()).map_err(|why| why.to_string())?;
  file.sync_all().map_err(|why| why.to_string())?;
  std::fs::rename(&beside, path).map_err(|why| why.to_string())
}

// A list as a line names it: its names, or `none`.
fn listed(names: &[String]) -> String {
  if names.is_empty() { "none".to_owned() } else { names.join(", ") }
}

fn mode_of(named: Option<&str>) -> u32 {
  named.and_then(|mode| u32::from_str_radix(mode, 8).ok()).unwrap_or(0o600)
}

// Whether an env file holds a line of the key.
fn holds_key(path: &Path, key: &str) -> bool {
  std::fs::read_to_string(path)
    .map(|text| {
      text
        .lines()
        .any(|line| line.trim_start().strip_prefix(key).is_some_and(|rest| rest.starts_with('=')))
    })
    .unwrap_or(false)
}

// The variable an input's file is handed to an after-step under.
fn input_variable(name: &str) -> String {
  let upper: String = name
    .chars()
    .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
    .collect();

  format!("BMO_INPUT_{upper}")
}

/// The index checkout: the one named, else the variable's, else the git top level of the directory the verb runs
/// in where that holds the presets file.
pub fn find_index(ask: &Ask) -> Option<PathBuf> {
  if let Some(index) = &ask.index {
    return Some(index.clone());
  }

  if let Some(named) = std::env::var_os(INDEX_VARIABLE).filter(|named| !named.is_empty()) {
    return Some(PathBuf::from(named));
  }

  let top = std::process::Command::new("git")
    .arg("-C")
    .arg(&ask.cwd)
    .args(["rev-parse", "--show-toplevel"])
    .output()
    .ok()
    .filter(|output| output.status.success())
    .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
    .unwrap_or_else(|| ask.cwd.clone());

  top.join(PRESETS_FILE).is_file().then_some(top)
}

impl<'a> Install<'a> {
  pub fn new(ask: Ask, machine: &'a dyn Machine, say: &'a mut dyn FnMut(&Line)) -> Self {
    Self {
      ask,
      machine,
      say,
      failed: None,
      waiting: 0,
    }
  }

  fn line(
    &mut self,
    step: impl Into<String>,
    status: Status,
    detail: impl Into<String>,
    next: impl Into<String>,
  ) {
    let line = Line::new(step, status, detail, next);

    match status {
      Status::Failed => {
        if self.failed.is_none() {
          self.failed = Some(line.step.clone());
        }
      }
      Status::NeedsInput => self.waiting += 1,
      Status::Ok | Status::Planned => {}
    }

    (self.say)(&line);
  }

  // Whether the run stops at the end of a stage: a real run stops where a step failed or waits for input; a dry
  // run goes on, so it says the whole plan.
  fn stops(&self) -> bool {
    !self.ask.dry_run && (self.failed.is_some() || self.waiting > 0)
  }

  fn outcome(&self) -> Outcome {
    if self.failed.is_some() {
      Outcome::Failed
    } else if self.waiting > 0 {
      Outcome::Waiting
    } else {
      Outcome::Done
    }
  }

  fn refused(line: &str) -> Outcome {
    Outcome::Refused(line.to_owned())
  }

  fn refusal(why: Refused, file: &Path) -> Outcome {
    let said = &INSTALL.refusal;
    let file = file.display().to_string();

    Self::refused(&match why {
      Refused::Unread(why) => mark(&said.presets_unread, &[("file", &file), ("why", &why)]),
      Refused::NoPreset { preset, presets } => mark(
        &said.no_preset,
        &[("preset", &preset), ("file", &file), ("presets", &presets.join(", "))],
      ),
      Refused::NoRepository(name) => mark(&said.no_repository, &[("name", &name), ("file", &file)]),
      Refused::ExtendsRing(preset) => mark(&said.extends_ring, &[("preset", &preset)]),
      Refused::ChoiceMany { what, units } => {
        mark(&said.choice_many, &[("what", &what), ("units", &units.join(", "))])
      }
    })
  }

  fn state_path(&self) -> PathBuf {
    self.ask.data_root.join(STATE_FILE)
  }

  fn read_state(&self) -> Option<State> {
    serde_json::from_str(&std::fs::read_to_string(self.state_path()).ok()?).ok()
  }

  /// The run, as its words ask.
  pub fn run(mut self) -> Outcome {
    let Some(index) = find_index(&self.ask) else {
      return Self::refused(&mark(&INSTALL.refusal.no_index, &[("file", PRESETS_FILE)]));
    };
    let presets = match PresetsFile::read(&index) {
      Ok(presets) => presets,
      Err(why) => return Self::refusal(why, &index.join(PRESETS_FILE)),
    };

    match self.ask.mode.clone() {
      Mode::Set(name) => self.set(&index, &presets, &name),
      Mode::Status => self.status(&index, &presets),
      Mode::Preset { name, with } => {
        let state = State {
          preset: name,
          with,
          added: Vec::new(),
        };

        self.install(&index, &presets, state)
      }
      Mode::Add(repository) => {
        let Some(mut state) = self.read_state() else {
          return Self::refused(&mark(
            &INSTALL.refusal.no_install,
            &[("dir", &self.ask.data_root.display().to_string())],
          ));
        };

        if presets.repository(&repository).is_none() {
          return Self::refusal(Refused::NoRepository(repository), &index.join(PRESETS_FILE));
        }

        if !state.added.contains(&repository) {
          state.added.push(repository);
        }

        self.install(&index, &presets, state)
      }
    }
  }

  // The preset resolved with what was added, or the refusal.
  fn resolved(presets: &PresetsFile, state: &State) -> Result<Resolved, Refused> {
    let mut resolved = presets.resolve(&state.preset, &state.with)?;

    for added in &state.added {
      if !resolved.units.contains(added) {
        resolved.units.push(added.clone());
      }

      resolved.clone.retain(|repository| repository != added);

      if let Some(entry) = presets.repository(added) {
        resolved.features.extend(entry.features.iter().cloned());
      }
    }

    Ok(resolved)
  }

  fn checkout_of(index: &Path, presets: &PresetsFile, name: &str) -> PathBuf {
    presets
      .repository(name)
      .map_or_else(|| index.join(name), |entry| index.join(&entry.folder))
  }

  fn install(&mut self, index: &Path, presets: &PresetsFile, state: State) -> Outcome {
    let said = &INSTALL.detail;
    let next = &INSTALL.next;
    let resolved = match Self::resolved(presets, &state) {
      Ok(resolved) => resolved,
      Err(why) => return Self::refusal(why, &index.join(PRESETS_FILE)),
    };

    // 1. The preset.
    for choice in &resolved.open {
      let units = choice.units.join(", ");

      self.line(
        "preset",
        Status::NeedsInput,
        mark(&said.choice_open, &[("what", &choice.what), ("units", &units)]),
        mark(&next.choose, &[("preset", &state.preset), ("units", &units)]),
      );
    }

    if !resolved.open.is_empty() {
      return self.finish(index, None, &[]);
    }

    self.line(
      "preset",
      Status::Ok,
      mark(
        &said.preset,
        &[
          ("preset", &resolved.preset),
          ("units", &resolved.units.join(", ")),
          ("clone", &listed(&resolved.clone)),
        ],
      ),
      "",
    );

    if !self.ask.dry_run {
      let text = serde_json::to_string_pretty(&state).unwrap_or_default();

      if let Err(why) = write_whole(&self.state_path(), &text, 0o600) {
        self.line("preset", Status::Failed, why, &next.mend_and_rerun);
        return self.finish(index, None, &[]);
      }
    }

    // 2. The checkouts.
    let mut standing: BTreeSet<String> = BTreeSet::new();

    for name in resolved.clone.iter().chain(&resolved.units) {
      let Some(entry) = presets.repository(name) else { continue };
      let path = index.join(&entry.folder);
      let shown = path.display().to_string();
      let step = format!("clone:{name}");
      let values = [("name", name.as_str()), ("url", entry.url.as_str()), ("path", shown.as_str())];

      if path.join(".git").exists() {
        if !self.ask.dry_run {
          let _ = self.machine.fetch(&path);
        }

        standing.insert(name.clone());
        self.line(step, Status::Ok, mark(&said.clone_stands, &values), "");
      } else if self.ask.dry_run {
        self.line(step, Status::Planned, mark(&said.clone_planned, &values), "");
      } else {
        let cloned = self.machine.clone_repository(&entry.url, &path);

        if cloned.ok() {
          standing.insert(name.clone());
          self.line(step, Status::Ok, mark(&said.clone_made, &values), "");
        } else {
          let said_by = quoted(&cloned.said);

          self.line(
            step,
            Status::Failed,
            mark(&said.clone_failed, &[("name", name), ("url", &entry.url), ("said", &said_by)]),
            &next.mend_and_rerun,
          );
        }
      }
    }

    if self.stops() {
      return self.finish(index, None, &[]);
    }

    // 3. The declarations.
    let mut units: Vec<Unit> = Vec::new();

    for name in &resolved.units {
      let checkout = Self::checkout_of(index, presets, name);
      let step = format!("declaration:{name}");

      if !standing.contains(name) {
        // A dry run's repository not cloned yet has nothing to read until it is.
        continue;
      }

      let declaration = match InstallDeclaration::read(&checkout) {
        Read::Declared(declaration) => *declaration,
        Read::Missing => {
          let file = checkout.join(crate::declaration::FILE).display().to_string();

          self.line(
            step,
            Status::Failed,
            mark(&said.declaration_missing, &[("name", name)]),
            mark(&next.read_declaration, &[("file", &file)]),
          );
          continue;
        }
        Read::Unread(why) => {
          self.line(
            step,
            Status::Failed,
            mark(&said.declaration_unread, &[("name", name), ("why", &why)]),
            &next.mend_and_rerun,
          );
          continue;
        }
      };
      let testbed = if checkout.join("testbed.toml").exists() {
        match Repository::read(&checkout) {
          Ok(repository) => Some(repository.declaration),
          Err(why) => {
            self.line(
              step,
              Status::Failed,
              mark(&said.testbed_unread, &[("name", name), ("why", &why.0)]),
              &next.mend_and_rerun,
            );
            continue;
          }
        }
      } else {
        None
      };

      self.line(
        step,
        Status::Ok,
        mark(
          &said.declaration,
          &[
            ("name", name),
            ("settings", &declaration.settings.len().to_string()),
            ("lines", &declaration.line.len().to_string()),
            ("inputs", &declaration.input.len().to_string()),
            ("after", &declaration.after.len().to_string()),
          ],
        ),
        "",
      );
      units.push(Unit {
        name: name.clone(),
        checkout,
        declaration,
        testbed,
      });
    }

    let values = self.values(index, presets, &units);

    if self.stops() {
      return self.finish(index, Some(&values), &units);
    }

    // 4. The programs.
    let mut asked: BTreeSet<String> = BTreeSet::new();

    for unit in &units {
      for need in &unit.declaration.needs {
        let name = need.name();

        if !asked.insert(name.clone()) {
          continue;
        }

        let env = self.environment(Some(index), unit, &[]);
        let stands = |machine: &dyn Machine| match (&need.program, &need.check) {
          (Some(program), _) => machine.has_program(program),
          (None, Some(check)) => machine.shell(check, &unit.checkout, &env).ok(),
          (None, None) => true,
        };
        let step = format!("needs:{name}");

        if stands(self.machine) {
          self.line(step, Status::Ok, mark(&said.need_stands, &[("program", &name)]), "");
          continue;
        }

        let Some(source) = &need.source else {
          self.line(
            step,
            Status::Failed,
            mark(&said.need_missing, &[("program", &name), ("what", &need.what)]),
            mark(&next.install_program, &[("program", &name)]),
          );
          continue;
        };
        let shown = [
          ("program", name.as_str()),
          ("repository", source.repository.as_str()),
          ("rev", source.rev.as_str()),
        ];

        if self.ask.dry_run {
          self.line(step, Status::Planned, mark(&said.need_build_planned, &shown), "");
          continue;
        }

        let built = self.build_source(source, &env);

        if built.ok() && stands(self.machine) {
          self.line(step, Status::Ok, mark(&said.need_built, &shown), "");
        } else {
          let why = if built.ok() { quoted(&need.what) } else { quoted(&built.said) };

          self.line(
            step,
            Status::Failed,
            mark(&said.need_build_failed, &[shown[0], shown[1], shown[2], ("said", &why)]),
            &next.mend_and_rerun,
          );
        }
      }
    }

    if self.stops() {
      return self.finish(index, Some(&values), &units);
    }

    // 5. The places and the ports.
    self.places(&units);

    let mut values = values;
    let chosen = self.ports(&units);

    for (name, number) in &chosen {
      values.named.insert(format!("port.{name}"), Source::Text(number.to_string()));
    }

    // 6. The inputs.
    for unit in &units {
      for input in &unit.declaration.input {
        let path = values.path(&input.file);
        let step = format!("input:{}", input.name);
        let given = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0);

        if given {
          self.line(step, Status::Ok, mark(&said.input_given, &[("name", &input.name)]), "");
        } else if input.default.is_some() {
          self.line(step, Status::Ok, mark(&said.input_default, &[("name", &input.name)]), "");
        } else if input.optional {
          self.line(step, Status::Ok, mark(&said.input_optional, &[("name", &input.name)]), "");
        } else {
          self.line(
            step,
            Status::NeedsInput,
            mark(&said.input_needed, &[("what", &input.what)]),
            mark(&next.set, &[("name", &input.name)]),
          );
        }
      }
    }

    if self.stops() {
      return self.finish(index, Some(&values), &units);
    }

    // 7. The tokens.
    for unit in &units {
      for token in &unit.declaration.token {
        let path = values.path(&token.file);
        let file = path.display().to_string();
        let step = format!("token:{}", token.name);
        let shown = [("name", token.name.as_str()), ("file", file.as_str())];

        if path.exists() {
          self.line(step, Status::Ok, mark(&said.token_stands, &shown), "");
        } else if self.ask.dry_run {
          self.line(step, Status::Planned, mark(&said.token_planned, &shown), "");
        } else {
          let made =
            self
              .machine
              .random(token.bytes.unwrap_or(INSTALL.numbers.token_bytes))
              .and_then(|bytes| {
                let text = match token.encoding {
                  Encoding::Hex => bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
                  Encoding::Base64 => base64::engine::general_purpose::STANDARD.encode(&bytes),
                };

                write_whole(&path, &format!("{text}\n"), 0o600)
              });

          match made {
            Ok(()) => self.line(step, Status::Ok, mark(&said.token_made, &shown), ""),
            Err(why) => self.line(
              step,
              Status::Failed,
              mark(&said.token_failed, &[("name", &token.name), ("file", &file), ("said", &why)]),
              &next.mend_and_rerun,
            ),
          }
        }
      }
    }

    // 8. The databases.
    let mut made: BTreeSet<String> = BTreeSet::new();

    for unit in &units {
      for database in &unit.declaration.database {
        if !made.insert(database.name.clone()) {
          continue;
        }

        let step = format!("database:{}", database.name);
        let name = [("name", database.name.as_str())];

        match self.machine.database_exists(&database.name) {
          Ok(true) => self.line(step, Status::Ok, mark(&said.database_stands, &name), ""),
          Ok(false) if self.ask.dry_run => {
            self.line(step, Status::Planned, mark(&said.database_planned, &name), "")
          }
          Ok(false) => {
            let created = self.machine.create_database(&database.name);

            if created.ok() {
              self.line(step, Status::Ok, mark(&said.database_made, &name), "");
            } else {
              let why = quoted(&created.said);

              self.line(
                step,
                Status::Failed,
                mark(&said.database_failed, &[("name", &database.name), ("said", &why)]),
                &next.mend_and_rerun,
              );
            }
          }
          Err(why) => {
            let why = quoted(&why);

            self.line(
              step,
              Status::Failed,
              mark(&said.database_failed, &[("name", &database.name), ("said", &why)]),
              &next.mend_and_rerun,
            );
          }
        }
      }
    }

    if self.stops() {
      return self.finish(index, Some(&values), &units);
    }

    // 9. The settings and the lines.
    let mut deferred: Vec<Deferred> = Vec::new();
    let mut owners: BTreeMap<PathBuf, usize> = BTreeMap::new();

    for (at_unit, unit) in units.iter().enumerate() {
      for settings in &unit.declaration.settings {
        owners.insert(values.path(&settings.file), at_unit);
      }
    }

    let mut changed: BTreeMap<usize, PathBuf> = BTreeMap::new();

    for (at_unit, unit) in units.iter().enumerate() {
      for at in 0..unit.declaration.settings.len() {
        match self.write_settings(&values, unit, at) {
          Wrote::Waiting => deferred.push(Deferred::Settings { unit: at_unit, at }),
          Wrote::Now => {
            changed.insert(at_unit, values.path(&unit.declaration.settings[at].file));
          }
          Wrote::Settled => {}
        }
      }
    }

    for (at_unit, unit) in units.iter().enumerate() {
      for at in 0..unit.declaration.line.len() {
        match self.write_line(&values, unit, at) {
          Wrote::Waiting => deferred.push(Deferred::Line { unit: at_unit, at }),
          Wrote::Now => {
            let file = values.path(&unit.declaration.line[at].file);

            changed.insert(owners.get(&file).copied().unwrap_or(at_unit), file);
          }
          Wrote::Settled => {}
        }
      }
    }

    // A repository live already whose settings were written now is restarted onto them.
    for (owner, file) in changed {
      if units[owner].deployed(&self.ask) {
        self.restart(&units[owner], &file);
      }
    }

    // 10. The console features.
    self.features(presets, &resolved);

    if self.stops() {
      return self.finish(index, Some(&values), &units);
    }

    // 11. Each repository's first release and its after-steps.
    for at_unit in 0..units.len() {
      deferred = self.settle(&values, &units, &owners, deferred);

      let unit = &units[at_unit];

      if !self.deploy(unit) {
        return self.finish(index, Some(&values), &units);
      }

      if !self.after(&values, index, unit) {
        return self.finish(index, Some(&values), &units);
      }
    }

    deferred = self.settle(&values, &units, &owners, deferred);

    if !self.ask.dry_run {
      for left in &deferred {
        let (unit, file) = match left {
          Deferred::Settings { unit, at } => (unit, &units[*unit].declaration.settings[*at].file),
          Deferred::Line { unit, at } => (unit, &units[*unit].declaration.line[*at].file),
        };
        let shown = values.path(file).display().to_string();

        self.line(
          format!("settings:{shown}"),
          Status::Failed,
          mark(&said.settings_waiting, &[("file", &shown), ("waiting", &units[*unit].name)]),
          &next.mend_and_rerun,
        );
      }
    }

    // 12. The verifications.
    self.verify(&units);
    self.finish(index, Some(&values), &units)
  }

  // Every repository's deploy root and data root under the install prefix, so a packager that names one finds the
  // whole install there; one that stands elsewhere is said, never moved.
  fn places(&mut self, units: &[Unit]) {
    let said = &INSTALL.detail;
    let prefix = self.ask.prefix.display().to_string();

    for unit in units {
      let Some(deploy) = unit.testbed.as_ref().map(|testbed| &testbed.deploy) else {
        continue;
      };
      let named = [deploy.root.as_ref(), deploy.data_root.as_ref()];

      for place in named.into_iter().flatten() {
        let path = resolve(&place.to_string_lossy(), &unit.checkout, &self.ask.home);

        if !path.starts_with(&self.ask.prefix) && !path.starts_with(&self.ask.data_root) {
          let shown = path.display().to_string();

          self.line(
            format!("place:{}", unit.name),
            Status::Failed,
            mark(&said.place_outside, &[("name", &unit.name), ("place", &shown), ("prefix", &prefix)]),
            &INSTALL.next.mend_and_rerun,
          );
        }
      }
    }
  }

  // Each declared port, or where it is taken and its repository is not live, the next free one; the choices kept
  // under the data root, so every run and every packager reads the same numbers.
  fn ports(&mut self, units: &[Unit]) -> BTreeMap<String, u16> {
    let said = &INSTALL.detail;
    let path = self.ask.data_root.join(PORTS_FILE);
    let mut recorded: BTreeMap<String, u16> = std::fs::read_to_string(&path)
      .ok()
      .and_then(|text| serde_json::from_str(&text).ok())
      .unwrap_or_default();
    let mut taken: BTreeMap<u16, String> =
      recorded.iter().map(|(name, number)| (*number, name.clone())).collect();
    let mut changed = false;

    for unit in units {
      for port in &unit.declaration.port {
        let step = format!("port:{}", port.name);

        if let Some(number) = recorded.get(&port.name) {
          let number = number.to_string();

          self.line(
            step,
            Status::Ok,
            mark(&said.port_kept, &[("name", &port.name), ("number", &number)]),
            "",
          );
          continue;
        }

        let live = unit.deployed(&self.ask);
        let free = |number: u16, taken: &BTreeMap<u16, String>, machine: &dyn Machine| {
          !taken.contains_key(&number) && (live || machine.port_free(number))
        };
        let found = (0..PORT_TRIES)
          .filter_map(|step| port.number.checked_add(step))
          .find(|number| free(*number, &taken, self.machine));

        let Some(number) = found else {
          let number = port.number.to_string();

          self.line(
            step,
            Status::Failed,
            mark(&said.port_taken, &[("name", &port.name), ("number", &number)]),
            &INSTALL.next.mend_and_rerun,
          );
          continue;
        };
        let shown = number.to_string();
        let detail = if number == port.number {
          mark(&said.port_free, &[("name", &port.name), ("number", &shown)])
        } else {
          mark(
            &said.port_moved,
            &[("name", &port.name), ("number", &shown), ("declared", &port.number.to_string())],
          )
        };

        taken.insert(number, port.name.clone());
        recorded.insert(port.name.clone(), number);
        changed = true;
        self.line(step, if self.ask.dry_run { Status::Planned } else { Status::Ok }, detail, "");
      }
    }

    if changed && !self.ask.dry_run {
      let text = serde_json::to_string_pretty(&recorded).unwrap_or_default();

      if let Err(why) = write_whole(&path, &text, 0o644) {
        let shown = path.display().to_string();

        self.line(
          "ports",
          Status::Failed,
          mark(&said.settings_failed, &[("file", &shown), ("said", &why)]),
          &INSTALL.next.mend_and_rerun,
        );
      }
    }

    recorded
  }

  // The values every template of the install reaches; a name declared twice is a failed step.
  fn values(&mut self, index: &Path, presets: &PresetsFile, units: &[Unit]) -> Values {
    let mut values = Values {
      data_root: self.ask.data_root.clone(),
      home: self.ask.home.clone(),
      ..Values::default()
    };
    let mut declared_by: BTreeMap<String, String> = BTreeMap::new();
    let planned =
      |name: &str, dry: bool| if dry { Absent::Planned(name.to_owned()) } else { Absent::Missing };

    for (name, text) in [
      ("data_root", self.ask.data_root.display().to_string()),
      ("home", self.ask.home.display().to_string()),
      ("index", index.display().to_string()),
      ("user", self.ask.user.clone()),
      ("prefix", self.ask.prefix.display().to_string()),
      ("postgres.host", self.ask.services.postgres_host.clone()),
      ("postgres.port", self.ask.services.postgres_port.clone()),
      ("valkey.url", self.ask.services.valkey.clone()),
    ] {
      values.named.insert(name.to_owned(), Source::Text(text));
    }

    for entry in &presets.repository {
      values.named.insert(
        format!("checkout.{}", entry.name),
        Source::Text(index.join(&entry.folder).display().to_string()),
      );
    }

    for unit in units {
      let declared = &unit.declaration;
      let mut add = |name: String, source: Source, this: &mut Self| {
        if let Some(first) = declared_by.get(&name) {
          let first = first.clone();

          this.line(
            format!("declaration:{}", unit.name),
            Status::Failed,
            mark(
              &INSTALL.detail.name_twice,
              &[("name", &name), ("first", &first), ("second", &unit.name)],
            ),
            &INSTALL.next.mend_and_rerun,
          );
          return;
        }

        declared_by.insert(name.clone(), unit.name.clone());
        values.named.insert(name, source);
      };

      for token in &declared.token {
        let path = resolve(&token.file, &self.ask.data_root, &self.ask.home);

        add(
          format!("token.{}", token.name),
          Source::File {
            path,
            absent: planned(&token.name, self.ask.dry_run),
          },
          self,
        );
      }

      for port in &declared.port {
        add(format!("port.{}", port.name), Source::Text(port.number.to_string()), self);
      }

      for input in &declared.input {
        let path = resolve(&input.file, &self.ask.data_root, &self.ask.home);
        let absent = match (&input.default, input.optional) {
          (Some(default), _) => Absent::Text(default.clone()),
          (None, true) => Absent::Text(String::new()),
          (None, false) => planned(&input.name, self.ask.dry_run),
        };

        add(format!("input.{}", input.name), Source::File { path, absent }, self);
      }

      for value in &declared.value {
        add(format!("value.{}", value.name), Source::Template(value.value.clone()), self);
      }
    }

    values
  }

  // A settings file written where it does not stand.
  fn write_settings(&mut self, values: &Values, unit: &Unit, at: usize) -> Wrote {
    let said = &INSTALL.detail;
    let settings = &unit.declaration.settings[at];
    let path = values.path(&settings.file);
    let shown = path.display().to_string();
    let step = format!("settings:{shown}");

    if path.exists() {
      self.line(step, Status::Ok, mark(&said.settings_stands, &[("file", &shown)]), "");
      return Wrote::Settled;
    }

    match values.fill(&settings.text) {
      Ok(_) if self.ask.dry_run => {
        self.line(step, Status::Planned, mark(&said.settings_planned, &[("file", &shown)]), "")
      }
      Ok(text) => match write_whole(&path, &text, mode_of(settings.mode.as_deref())) {
        Ok(()) => {
          self.line(step, Status::Ok, mark(&said.settings_written, &[("file", &shown)]), "");
          return Wrote::Now;
        }
        Err(why) => self.line(
          step,
          Status::Failed,
          mark(&said.settings_failed, &[("file", &shown), ("said", &why)]),
          &INSTALL.next.mend_and_rerun,
        ),
      },
      Err(Unfilled::Waiting(waiting)) => {
        self.line(
          step,
          Status::Planned,
          mark(&said.settings_waiting, &[("file", &shown), ("waiting", &waiting)]),
          "",
        );
        return Wrote::Waiting;
      }
      Err(Unfilled::Unknown(name)) => self.line(
        step,
        Status::Failed,
        mark(&said.settings_unknown, &[("file", &shown), ("name", &name)]),
        &INSTALL.next.mend_and_rerun,
      ),
      Err(Unfilled::Open) => self.line(
        step,
        Status::Failed,
        mark(&said.settings_unknown, &[("file", &shown), ("name", "{{")]),
        &INSTALL.next.mend_and_rerun,
      ),
    }

    Wrote::Settled
  }

  // A line added to its file where the file holds no line of its key. It waits for its file, which its owner's
  // settings write, or for a file a service makes.
  fn write_line(&mut self, values: &Values, unit: &Unit, at: usize) -> Wrote {
    let said = &INSTALL.detail;
    let line = &unit.declaration.line[at];
    let path = values.path(&line.file);
    let shown = path.display().to_string();
    let step = format!("line:{shown}:{}", line.key);
    let named = [("key", line.key.as_str()), ("file", shown.as_str())];

    if holds_key(&path, &line.key) {
      self.line(step, Status::Ok, mark(&said.line_stands, &named), "");
      return Wrote::Settled;
    }

    let filled = values.fill(&line.value);

    if self.ask.dry_run {
      return match filled {
        Err(Unfilled::Unknown(name)) => {
          self.line(
            step,
            Status::Failed,
            mark(&said.settings_unknown, &[("file", &shown), ("name", &name)]),
            &INSTALL.next.mend_and_rerun,
          );
          Wrote::Settled
        }
        _ => {
          self.line(step, Status::Planned, mark(&said.line_planned, &named), "");
          Wrote::Settled
        }
      };
    }

    if !path.exists() {
      self.line(
        step,
        Status::Planned,
        mark(&said.line_waiting, &[("key", &line.key), ("file", &shown), ("waiting", &shown)]),
        "",
      );
      return Wrote::Waiting;
    }

    match filled {
      Ok(value) => {
        let appended = std::fs::read_to_string(&path).map_err(|why| why.to_string()).and_then(|before| {
          let joiner = if before.is_empty() || before.ends_with('\n') { "" } else { "\n" };
          let mode =
            std::fs::metadata(&path).map(|meta| meta.permissions().mode() & 0o777).unwrap_or(0o600);

          write_whole(&path, &format!("{before}{joiner}{}={value}\n", line.key), mode)
        });

        match appended {
          Ok(()) => {
            self.line(step, Status::Ok, mark(&said.line_added, &named), "");
            Wrote::Now
          }
          Err(why) => {
            self.line(
              step,
              Status::Failed,
              mark(&said.settings_failed, &[("file", &shown), ("said", &why)]),
              &INSTALL.next.mend_and_rerun,
            );
            Wrote::Settled
          }
        }
      }
      Err(Unfilled::Waiting(waiting)) => {
        self.line(
          step,
          Status::Planned,
          mark(&said.line_waiting, &[("key", &line.key), ("file", &shown), ("waiting", &waiting)]),
          "",
        );
        Wrote::Waiting
      }
      Err(Unfilled::Unknown(name)) => {
        self.line(
          step,
          Status::Failed,
          mark(&said.settings_unknown, &[("file", &shown), ("name", &name)]),
          &INSTALL.next.mend_and_rerun,
        );
        Wrote::Settled
      }
      Err(Unfilled::Open) => {
        self.line(
          step,
          Status::Failed,
          mark(&said.settings_unknown, &[("file", &shown), ("name", "{{")]),
          &INSTALL.next.mend_and_rerun,
        );
        Wrote::Settled
      }
    }
  }

  // What was put off, written where it now can be; a repository live already whose file changed is restarted.
  // What still waits is handed back.
  fn settle(
    &mut self,
    values: &Values,
    units: &[Unit],
    owners: &BTreeMap<PathBuf, usize>,
    deferred: Vec<Deferred>,
  ) -> Vec<Deferred> {
    if self.ask.dry_run {
      return deferred;
    }

    let mut left = Vec::new();
    let mut restart: BTreeMap<usize, PathBuf> = BTreeMap::new();

    for item in deferred {
      let (unit, file, written) = match &item {
        Deferred::Settings { unit, at } => {
          let file = values.path(&units[*unit].declaration.settings[*at].file);
          let stood = file.exists();

          (*unit, file, !stood && self.settle_settings(values, &units[*unit], *at) == Wrote::Now)
        }
        Deferred::Line { unit, at } => {
          let file = values.path(&units[*unit].declaration.line[*at].file);

          (*unit, file, self.settle_line(values, &units[*unit], *at) == Wrote::Now)
        }
      };

      if !written {
        // An item that settled without being written, a failure said on its own line, is not kept waiting.
        let still = match &item {
          Deferred::Settings { unit, at } => {
            matches!(values.fill(&units[*unit].declaration.settings[*at].text), Err(Unfilled::Waiting(_)))
          }
          Deferred::Line { unit, at } => {
            let line = &units[*unit].declaration.line[*at];

            !values.path(&line.file).exists()
              || matches!(values.fill(&line.value), Err(Unfilled::Waiting(_)))
          }
        };

        if still {
          left.push(item);
        }

        continue;
      }

      let owner = owners.get(&file).copied().unwrap_or(unit);

      if units[owner].deployed(&self.ask) {
        restart.insert(owner, file);
      }
    }

    for (owner, file) in restart {
      self.restart(&units[owner], &file);
    }

    left
  }

  // A deferred settings file written once what it waits for stands; false while it still waits.
  fn settle_settings(&mut self, values: &Values, unit: &Unit, at: usize) -> Wrote {
    let settings = &unit.declaration.settings[at];

    if matches!(values.fill(&settings.text), Err(Unfilled::Waiting(_))) {
      return Wrote::Waiting;
    }

    self.write_settings(values, unit, at)
  }

  fn settle_line(&mut self, values: &Values, unit: &Unit, at: usize) -> Wrote {
    let line = &unit.declaration.line[at];

    if !values.path(&line.file).exists() || matches!(values.fill(&line.value), Err(Unfilled::Waiting(_))) {
      return Wrote::Waiting;
    }

    self.write_line(values, unit, at)
  }

  fn restart(&mut self, unit: &Unit, file: &Path) {
    let said = &INSTALL.detail;
    let shown = file.display().to_string();
    let step = format!("restart:{}", unit.name);
    let named = [("name", unit.name.as_str()), ("file", shown.as_str())];

    let Some(command) = &unit.declaration.restart else {
      self.line(step, Status::Failed, mark(&said.restart_none, &named), &INSTALL.next.mend_and_rerun);
      return;
    };
    let ran = self.machine.shell(command, &unit.checkout, &self.environment(None, unit, &[]));

    if ran.ok() {
      self.line(step, Status::Ok, mark(&said.restart_done, &named), "");
    } else {
      let why = quoted(&ran.said);

      self.line(
        step,
        Status::Failed,
        mark(&said.restart_failed, &[("name", &unit.name), ("file", &shown), ("said", &why)]),
        &INSTALL.next.mend_and_rerun,
      );
    }
  }

  fn features(&mut self, presets: &PresetsFile, resolved: &Resolved) {
    let said = &INSTALL.detail;
    let path = self.ask.data_root.join(FEATURES_FILE);
    let shown = path.display().to_string();
    let every = presets.every_feature();
    let mut text =
      String::from("# The console's own features this install shows, written by the install verb.\n");
    let mut listed = Vec::new();

    for feature in &every {
      let on = resolved.features.contains(feature);

      text.push_str(&format!("{feature} = {on}\n"));
      listed.push(format!("{feature} {}", if on { "on" } else { "off" }));
    }

    let listed = listed.join(", ");
    let step = "console-features";

    if std::fs::read_to_string(&path).is_ok_and(|before| before == text) {
      self.line(step, Status::Ok, mark(&said.features_stands, &[("file", &shown)]), "");
    } else if self.ask.dry_run {
      self.line(
        step,
        Status::Planned,
        mark(&said.features_planned, &[("file", &shown), ("features", &listed)]),
        "",
      );
    } else {
      match write_whole(&path, &text, 0o644) {
        Ok(()) => self.line(
          step,
          Status::Ok,
          mark(&said.features_written, &[("file", &shown), ("features", &listed)]),
          "",
        ),
        Err(why) => self.line(
          step,
          Status::Failed,
          mark(&said.settings_failed, &[("file", &shown), ("said", &why)]),
          &INSTALL.next.mend_and_rerun,
        ),
      }
    }
  }

  // The repository's first release; false where the run stops here.
  fn deploy(&mut self, unit: &Unit) -> bool {
    let said = &INSTALL.detail;
    let step = format!("deploy:{}", unit.name);
    let name = [("name", unit.name.as_str())];

    if unit.deploy_tree(&self.ask).is_none() {
      self.line(step, Status::Ok, mark(&said.deploy_none, &name), "");
      return true;
    }

    if unit.deployed(&self.ask) {
      self.line(step, Status::Ok, mark(&said.deploy_live, &name), "");
      return true;
    }

    let main = unit
      .testbed
      .as_ref()
      .map_or_else(|| "main".to_owned(), |testbed| testbed.check.main.clone());

    if self.ask.dry_run {
      self.line(
        step,
        Status::Planned,
        mark(&said.deploy_planned, &[("name", &unit.name), ("main", &main)]),
        "",
      );
      return true;
    }

    let Some(commit) = self.machine.main_head(&unit.checkout, &main) else {
      let why = format!("origin/{main} does not resolve");

      self.line(
        step,
        Status::Failed,
        mark(&said.deploy_failed, &[("name", &unit.name), ("said", &why)]),
        &INSTALL.next.mend_and_rerun,
      );
      return false;
    };
    let message = format!("The first install of {}", unit.name);
    let (ran, deployment) = self.machine.deploy(&unit.checkout, &commit, &message);
    let recorded = unit
      .deploy_root(&self.ask)
      .and_then(|root| std::fs::read_to_string(root.join("deployed.json")).ok())
      .and_then(|text| {
        serde_json::from_str::<serde_json::Value>(&text)
          .ok()?
          .get("commit")?
          .as_str()
          .map(str::to_owned)
      });
    let next = deployment.as_ref().map_or_else(
      || INSTALL.next.mend_and_rerun.clone(),
      |unit| mark(&INSTALL.next.journal, &[("unit", unit)]),
    );

    if ran.ok() && recorded.as_deref() == Some(commit.as_str()) && unit.deployed(&self.ask) {
      self.line(
        step,
        Status::Ok,
        mark(&said.deploy_done, &[("name", &unit.name), ("commit", &commit)]),
        "",
      );
      return true;
    }

    let why = if ran.ok() {
      recorded.map_or_else(
        || "no deployment record names it".to_owned(),
        |live| format!("the record names {live}"),
      )
    } else {
      quoted(&ran.said)
    };

    self.line(
      step,
      Status::Failed,
      mark(&said.deploy_failed, &[("name", &unit.name), ("said", &why)]),
      next,
    );
    false
  }

  /// A missing program's source cloned, or fetched where its clone stands, moved onto its revision and built; what
  /// the first step that did not succeed said.
  fn build_source(&self, source: &NeedSource, env: &[(String, String)]) -> Ran {
    let clone = self.ask.data_root.join(SOURCES_DIR).join(&source.name);
    let reached = if clone.join(".git").exists() {
      self.machine.fetch(&clone)
    } else {
      self.machine.clone_repository(&source.repository, &clone)
    };

    if !reached.ok() {
      return reached;
    }

    let moved = self.machine.check_out(&clone, &source.rev);

    if !moved.ok() {
      return moved;
    }

    self.machine.shell(&source.build, &clone, env)
  }

  // The environment a need's check, a source's build, an after-step, a restart or a verification is run with.
  fn environment(
    &self,
    index: Option<&Path>,
    _unit: &Unit,
    inputs: &[(String, PathBuf)],
  ) -> Vec<(String, String)> {
    let mut env = vec![
      ("BMO_DATA_DIR".to_owned(), self.ask.data_root.display().to_string()),
      ("BMO_INSTALL_PREFIX".to_owned(), self.ask.prefix.display().to_string()),
    ];

    if let Some(index) = index {
      env.push(("BMO_INSTALL_INDEX".to_owned(), index.display().to_string()));
    }

    for (name, path) in inputs {
      env.push((input_variable(name), path.display().to_string()));
    }

    env
  }

  // The repository's after-steps; false where one failed and the run stops.
  fn after(&mut self, values: &Values, index: &Path, unit: &Unit) -> bool {
    let said = &INSTALL.detail;

    for after in &unit.declaration.after {
      let step = format!("after:{}:{}", unit.name, after.id);
      let id = [("id", after.id.as_str())];
      let inputs: Vec<(String, PathBuf)> = after
        .inputs
        .iter()
        .map(|name| {
          let path = match values.named.get(&format!("input.{name}")) {
            Some(Source::File { path, .. }) => path.clone(),
            _ => PathBuf::new(),
          };

          (name.clone(), path)
        })
        .collect();
      let env = self.environment(Some(index), unit, &inputs);

      // A dry run asks no step's check either, since a check reaches the services it would set up.
      if self.ask.dry_run {
        self.line(step, Status::Planned, mark(&said.after_planned, &id), "");
        continue;
      }

      if let Some(done) = &after.done
        && self.machine.shell(done, &unit.checkout, &env).ok()
      {
        self.line(step, Status::Ok, mark(&said.after_done, &id), "");
        continue;
      }

      let ran: Ran = self.machine.shell(&after.run, &unit.checkout, &env);

      if ran.ok() {
        self.line(step, Status::Ok, mark(&said.after_ran, &id), "");
      } else {
        let why = quoted(&ran.said);

        self.line(
          step,
          Status::Failed,
          mark(
            &said.after_failed,
            &[("id", &after.id), ("code", &ran.code.to_string()), ("said", &why)],
          ),
          &INSTALL.next.mend_and_rerun,
        );
        return false;
      }
    }

    true
  }

  fn verify(&mut self, units: &[Unit]) {
    let said = &INSTALL.detail;

    for unit in units {
      let step = format!("verify:{}", unit.name);
      let name = [("name", unit.name.as_str())];

      if let Some(tree) = unit.deploy_tree(&self.ask) {
        let health = unit.testbed.as_ref().and_then(|testbed| testbed.deploy.health.clone());

        match health {
          _ if self.ask.dry_run => {}
          None => self.line(step.clone(), Status::Ok, mark(&said.health_none, &name), ""),
          Some(health) => {
            let dir = if tree.exists() { tree.clone() } else { unit.checkout.clone() };
            let ran = self.machine.shell(&health, &dir, &self.environment(None, unit, &[]));

            if ran.ok() {
              self.line(step.clone(), Status::Ok, mark(&said.health_ok, &name), "");
            } else {
              let why = quoted(&ran.said);

              self.line(
                step.clone(),
                Status::Failed,
                mark(&said.health_failed, &[("name", &unit.name), ("said", &why)]),
                &INSTALL.next.mend_and_rerun,
              );
            }
          }
        }
      }

      for verify in &unit.declaration.verify {
        let step = format!("verify:{}:{}", unit.name, verify.id);

        if self.ask.dry_run {
          self.line(
            step,
            Status::Planned,
            mark(&said.verify_planned, &[("id", &verify.id), ("what", &verify.what)]),
            "",
          );
          continue;
        }

        let ran = self.machine.shell(&verify.run, &unit.checkout, &self.environment(None, unit, &[]));

        if ran.ok() {
          self.line(
            step,
            Status::Ok,
            mark(&said.verify_ok, &[("id", &verify.id), ("what", &verify.what)]),
            "",
          );
        } else {
          let why = quoted(&ran.said);

          self.line(
            step,
            Status::Failed,
            mark(&said.verify_failed, &[("id", &verify.id), ("what", &verify.what), ("said", &why)]),
            &INSTALL.next.mend_and_rerun,
          );
        }
      }
    }
  }

  // The last line, and the run's outcome.
  fn finish(&mut self, _index: &Path, values: Option<&Values>, units: &[Unit]) -> Outcome {
    let said = &INSTALL.detail;
    let count = units.len().to_string();
    let address = values.and_then(|values| {
      units
        .iter()
        .find_map(|unit| unit.declaration.report.as_ref())
        .and_then(|report| values.fill(report).ok())
    });

    if let Some(step) = self.failed.clone() {
      (self.say)(&Line::new(
        "done",
        Status::Failed,
        mark(&said.done_failed, &[("step", &step)]),
        &INSTALL.next.mend_and_rerun,
      ));
    } else if self.waiting > 0 {
      let waiting = self.waiting.to_string();

      (self.say)(&Line::new(
        "done",
        Status::NeedsInput,
        mark(&said.done_waiting, &[("count", &waiting)]),
        "",
      ));
    } else if self.ask.dry_run {
      (self.say)(&Line::new("done", Status::Planned, mark(&said.done_planned, &[("count", &count)]), ""));
    } else {
      let next =
        address.map_or_else(String::new, |address| mark(&INSTALL.next.open, &[("address", &address)]));

      (self.say)(&Line::new("done", Status::Ok, mark(&said.done, &[("count", &count)]), next));
    }

    self.outcome()
  }

  fn status(&mut self, index: &Path, presets: &PresetsFile) -> Outcome {
    let said = &INSTALL.detail;
    let Some(state) = self.read_state() else {
      return Self::refused(&mark(
        &INSTALL.refusal.no_install,
        &[("dir", &self.ask.data_root.display().to_string())],
      ));
    };
    let resolved = match Self::resolved(presets, &state) {
      Ok(resolved) => resolved,
      Err(why) => return Self::refusal(why, &index.join(PRESETS_FILE)),
    };

    for name in &resolved.units {
      let checkout = Self::checkout_of(index, presets, name);
      let cloned = checkout.join(".git").exists();
      let testbed = cloned
        .then(|| Repository::read(&checkout).ok())
        .flatten()
        .map(|repository| repository.declaration);
      let unit = Unit {
        name: name.clone(),
        checkout,
        declaration: InstallDeclaration::default(),
        testbed,
      };
      let deployed = unit.deployed(&self.ask);
      let commit = unit
        .deploy_root(&self.ask)
        .and_then(|root| std::fs::read_to_string(root.join("deployed.json")).ok())
        .and_then(|text| {
          serde_json::from_str::<serde_json::Value>(&text)
            .ok()?
            .get("commit")?
            .as_str()
            .map(str::to_owned)
        });
      let shown_deployed = match (&commit, deployed) {
        (Some(commit), true) => format!("{} {commit}", said.status_deployed),
        _ => said.status_undeployed.clone(),
      };
      let detail = mark(
        &said.status,
        &[
          ("name", name),
          ("checkout", if cloned { &said.status_cloned } else { &said.status_absent }),
          ("deployed", &shown_deployed),
        ],
      );
      let status = if cloned && (deployed || unit.deploy_tree(&self.ask).is_none()) {
        Status::Ok
      } else {
        Status::Failed
      };

      self.line(format!("status:{name}"), status, detail, "");
    }

    // The ports the install chose, each on a line of its own, so a screen reads every service's port from here.
    let ports: BTreeMap<String, u16> = std::fs::read_to_string(self.ask.data_root.join(PORTS_FILE))
      .ok()
      .and_then(|text| serde_json::from_str(&text).ok())
      .unwrap_or_default();

    for (name, number) in &ports {
      let number = number.to_string();

      self.line(
        format!("port:{name}"),
        Status::Ok,
        mark(&said.port_kept, &[("name", name), ("number", &number)]),
        "",
      );
    }

    let rerun = mark(&INSTALL.next.rerun, &[("preset", &state.preset)]);

    if self.failed.is_some() {
      (self.say)(&Line::new(
        "done",
        Status::Failed,
        mark(&said.done_failed, &[("step", self.failed.as_deref().unwrap_or_default())]),
        rerun,
      ));
    } else {
      (self.say)(&Line::new(
        "done",
        Status::Ok,
        mark(&said.done, &[("count", &resolved.units.len().to_string())]),
        "",
      ));
    }

    self.outcome()
  }

  fn set(&mut self, index: &Path, presets: &PresetsFile, name: &str) -> Outcome {
    let found = presets.repository.iter().find_map(|entry| {
      match InstallDeclaration::read(&index.join(&entry.folder)) {
        Read::Declared(declaration) => declaration.input.into_iter().find(|input| input.name == name),
        _ => None,
      }
    });
    let Some(input) = found else {
      return Self::refused(&mark(
        &INSTALL.refusal.no_input,
        &[("index", &index.display().to_string()), ("name", name)],
      ));
    };
    let mut value = String::new();

    if let Err(why) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut value) {
      return Self::refused(&mark(
        &INSTALL.refusal.set_failed,
        &[("file", &input.file), ("said", &why.to_string())],
      ));
    }

    let value = value.strip_suffix('\n').unwrap_or(&value);
    let value = value.strip_suffix('\r').unwrap_or(value);
    let path = resolve(&input.file, &self.ask.data_root, &self.ask.home);
    let shown = path.display().to_string();

    if let Err(why) = write_whole(&path, &format!("{value}\n"), 0o600) {
      return Self::refused(&mark(&INSTALL.refusal.set_failed, &[("file", &shown), ("said", &why)]));
    }

    self.line(
      format!("set:{name}"),
      Status::Ok,
      mark(&INSTALL.detail.set, &[("name", name), ("file", &shown)]),
      "",
    );
    Outcome::Done
  }
}
