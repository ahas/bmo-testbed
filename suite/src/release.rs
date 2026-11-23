//! The set's release (docs/deploy.md, 「The set's release」): the repositories standing in the set's folder, each
//! read as its remote's main holds it; every value that declares a unit's own version, worked out file by file
//! and changed where it stands; the release made in git, commit, push and tag; and the step the commits since
//! the last release propose. It knows no repository by name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::declaration::DECLARATION;
use bmo_testbed_core::policy::versions::{FileWords, VERSIONS};
use bmo_testbed_core::version::{last_release, release_of_tag, release_tag};
use regex::Regex;
use semver::{BuildMetadata, Prerelease, Version};
use toml_edit::{DocumentMut, Item, TableLike};

use crate::selection::glob_matches;

/// A git command run to its end in a directory: its standard output where it ended 0, else what it said.
pub type Git<'a> = &'a dyn Fn(&[&str], &Path) -> Result<String, String>;

/// Git on this machine, its standard input closed.
pub fn machine_git(args: &[&str], cwd: &Path) -> Result<String, String> {
  let ran = Command::new("git")
    .args(args)
    .current_dir(cwd)
    .stdin(Stdio::null())
    .output()
    .map_err(|why| why.to_string())?;

  if ran.status.success() {
    Ok(String::from_utf8_lossy(&ran.stdout).into_owned())
  } else {
    Err(String::from_utf8_lossy(&ran.stderr).trim().to_owned())
  }
}

/// One repository of the set: its name, the remote's; its folder from the set's; the clone it is read
/// through; its remote; and the main branch its declaration names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
  pub name: String,
  pub folder: String,
  pub checkout: PathBuf,
  pub remote: String,
  pub main: String,
}

/// A folder holding a repository the release leaves out, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassedOver {
  pub folder: String,
  pub why: String,
}

/// The set as its folder holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetRead {
  pub members: Vec<Member>,
  pub passed_over: Vec<PassedOver>,
}

// The folders directly inside one, by name, links and hidden folders left out.
fn folders_in(dir: &Path) -> Vec<PathBuf> {
  let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
    .map(|entry| entry.path())
    .collect();

  found.sort();
  found
}

// The name a remote's address ends with, its `.git` left off.
fn remote_name(remote: &str) -> String {
  let last = remote.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or(remote);

  last.strip_suffix(".git").unwrap_or(last).to_owned()
}

// The declaration's table at a dotted path, read loosely: a release reads only what it needs of a declaration,
// whatever major wrote it.
fn declared<'a>(declaration: &'a toml::Table, path: &[&str]) -> Option<&'a toml::Value> {
  let (first, rest) = path.split_first()?;

  rest.iter().try_fold(declaration.get(*first)?, |value, key| value.get(*key))
}

fn read_declaration(text: &str) -> toml::Table {
  text.parse::<toml::Table>().unwrap_or_default()
}

/// The set standing in `set`: its own repository and every repository whose root stands directly in a folder
/// directly inside it, each through its primary checkout, once per remote.
pub fn read_set(set: &Path, git: Git) -> SetRead {
  let said = &VERSIONS.release;
  let candidates = std::iter::once(set.to_path_buf())
    .chain(folders_in(set).into_iter().flat_map(|group| folders_in(&group)))
    .filter(|dir| dir.join(".git").is_dir());
  let mut read = SetRead::default();
  let mut by_remote: BTreeMap<String, Vec<Member>> = BTreeMap::new();

  for checkout in candidates {
    let folder = checkout
      .strip_prefix(set)
      .ok()
      .map(|path| path.to_string_lossy().into_owned())
      .filter(|path| !path.is_empty())
      .unwrap_or_else(|| ".".to_owned());
    let Ok(remote) = git(&["config", "--get", "remote.origin.url"], &checkout) else {
      read.passed_over.push(PassedOver {
        folder,
        why: said.no_remote.clone(),
      });
      continue;
    };
    let remote = remote.trim().to_owned();
    let declaration =
      read_declaration(&std::fs::read_to_string(checkout.join(&DECLARATION.file)).unwrap_or_default());
    let main = declared(&declaration, &["check", "main"])
      .and_then(toml::Value::as_str)
      .unwrap_or(&DECLARATION.defaults.check.main)
      .to_owned();

    by_remote.entry(remote.clone()).or_default().push(Member {
      name: remote_name(&remote),
      folder,
      checkout,
      remote,
      main,
    });
  }

  for (remote, mut clones) in by_remote {
    // The clone whose folder's name ends the repository's name is the repository's own; any other is a copy.
    let own = clones
      .iter()
      .position(|clone| {
        clone
          .checkout
          .file_name()
          .is_some_and(|name| clone.name.ends_with(name.to_string_lossy().as_ref()))
      })
      .unwrap_or(0);
    let member = clones.remove(own);

    for other in clones {
      read.passed_over.push(PassedOver {
        folder: other.folder,
        why: fill_marks(&said.second_clone, &[("remote", &remote)]),
      });
    }

    read.members.push(member);
  }

  read.members.sort_by(|left, right| left.folder.cmp(&right.folder));
  read.passed_over.sort_by(|left, right| left.folder.cmp(&right.folder));
  read
}

/// Where the set stands: before its first release, or at the last release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
  Before,
  At(Version),
}

fn read_words() -> &'static FileWords {
  &VERSIONS.release.files
}

// The base every unit stood at before the set's first release.
fn base() -> Version {
  Version::parse(&read_words().base).unwrap_or_else(|_| Version::new(0, 1, 0))
}

impl Standing {
  /// The standing the release tags of the set's repositories name, each repository's tags listed apart: the
  /// newest release more than half of them hold, so an upstream's own tags kept in a fork never count.
  pub fn of(tags: &[Vec<String>]) -> Self {
    let mut held: BTreeMap<Version, usize> = BTreeMap::new();

    for listed in tags {
      let mut releases: Vec<Version> = listed.iter().filter_map(|tag| release_of_tag(tag)).collect();

      releases.sort();
      releases.dedup();

      for release in releases {
        *held.entry(release).or_default() += 1;
      }
    }

    held
      .into_iter()
      .filter(|(_, count)| count * 2 > tags.len())
      .map(|(release, _)| release)
      .max()
      .map_or(Self::Before, Self::At)
  }

  /// The version a release must stand above.
  pub fn version(&self) -> Version {
    match self {
      Self::Before => base(),
      Self::At(version) => version.clone(),
    }
  }

  /// Whether a value is a unit's own version: the last release's, or before the first release the base or
  /// one of its revisions. A build suffix is no part of it.
  pub fn owns(&self, value: &Version) -> bool {
    let bare = Version {
      build: BuildMetadata::EMPTY,
      ..value.clone()
    };

    match self {
      Self::At(version) => bare == *version,
      Self::Before => {
        let revision = bare
          .pre
          .as_str()
          .strip_prefix(&format!("{}.", read_words().revision))
          .is_some_and(|count| !count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit()));

        let base = base();

        (bare.major, bare.minor, bare.patch) == (base.major, base.minor, base.patch)
          && (bare.pre.is_empty() || revision)
      }
    }
  }

  // Whether a text holds a value that can be read as a unit's own version, read by the same pattern the
  // candidates are found by.
  fn owns_text(&self, text: &str) -> Option<Version> {
    Version::parse(text.trim()).ok().filter(|version| self.owns(version))
  }

  // The pattern the files that may declare an own version are found by.
  fn candidates(&self) -> String {
    match self {
      Self::Before => regex::escape(&base().to_string()),
      Self::At(version) => regex::escape(&version.to_string()),
    }
  }

  // The pattern a mention that can be nothing but an own version answers: a revision before the first
  // release, the release's version after it.
  fn mention(&self) -> Regex {
    let pattern = match self {
      Self::Before => {
        format!(r"\b{}-{}\.[0-9]+\b", regex::escape(&base().to_string()), regex::escape(&read_words().revision))
      }
      Self::At(version) => format!(r"(^|[^0-9.]){}($|[^0-9])", regex::escape(&version.to_string())),
    };

    Regex::new(&pattern).expect("the mention pattern is a regular expression")
  }

  /// How the heading names it.
  pub fn shown(&self) -> String {
    match self {
      Self::Before => VERSIONS.release.before_first.clone(),
      Self::At(version) => version.to_string(),
    }
  }
}

/// One value the release sets: the file, what it is, and the version it moves from and to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
  pub path: String,
  pub what: String,
  pub from: String,
  pub to: String,
}

/// What a repository's files come to: the values set, each changed file's new text, the files left naming an
/// own version, and the files that do not read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
  pub changes: Vec<Change>,
  pub edits: Vec<(String, String)>,
  pub left: Vec<String>,
  pub unread: Vec<String>,
}

// What one file's reading sets: what, from and to, and its new text.
type Rewritten = (Vec<(String, String, String)>, String);

// A string value set where it stands, its decor kept.
fn set_string(item: &mut Item, to: &str) {
  if let Some(value) = item.as_value_mut() {
    let decor = value.decor().clone();

    *value = toml_edit::Value::from(to);
    *value.decor_mut() = decor;
  }
}

// A version that is the unit's own as a TOML item holds it, read past an operator a requirement opens with.
fn own_in(item: &Item, standing: &Standing) -> Option<(String, Version)> {
  let text = item.as_str()?;
  let operator: String = text.chars().take_while(|one| "=^~".contains(*one)).collect();
  let version = standing.owns_text(&text[operator.len()..])?;

  Some((operator, version))
}

// The item at a dotted path, reached only where every table on the way stands: no step adds one.
fn reach<'a>(item: &'a mut Item, path: &[&str]) -> Option<&'a mut Item> {
  path.iter().try_fold(item, |item, key| item.as_table_like_mut()?.get_mut(key))
}

// Each entry of a dependency table taken from a sibling by path, its `version` handed to `own` with its label.
fn pinned_siblings(table: &mut dyn TableLike, named: &str, own: &mut dyn FnMut(&mut Item, String)) {
  for (name, entry) in table.iter_mut() {
    let Some(entry) = entry.as_table_like_mut() else {
      continue;
    };

    if entry.get("path").is_none() {
      continue;
    }

    if let Some(version) = entry.get_mut("version") {
      own(
        version,
        fill_marks(&VERSIONS.release.what.dependency, &[("table", named), ("name", name.get())]),
      );
    }
  }
}

// A Cargo manifest's package and workspace versions, and the version of every entry its dependency tables take
// from a sibling by path.
fn cargo_manifest(text: &str, standing: &Standing, to: &Version) -> Result<Rewritten, String> {
  let what = &VERSIONS.release.what;
  let mut document: DocumentMut = text.parse().map_err(|why: toml_edit::TomlError| why.to_string())?;
  let mut set = Vec::new();
  let own = |item: &mut Item, named: String, set: &mut Vec<(String, String, String)>| {
    if let Some((operator, version)) = own_in(item, standing) {
      let written = format!("{operator}{to}");

      set.push((named, format!("{operator}{version}"), written.clone()));
      set_string(item, &written);
    }
  };

  if let Some(item) = reach(document.as_item_mut(), &["package", "version"]) {
    own(item, what.package.clone(), &mut set);
  }

  if let Some(item) = reach(document.as_item_mut(), &["workspace", "package", "version"]) {
    own(item, what.workspace.clone(), &mut set);
  }

  let kinds = &read_words().dependency_tables;

  for (key, item) in document.as_table_mut().iter_mut() {
    let key = key.get().to_owned();

    if kinds.contains(&key) {
      if let Some(table) = item.as_table_like_mut() {
        pinned_siblings(table, &key, &mut |item, label| own(item, label, &mut set));
      }
    } else if key == "workspace" {
      if let Some(table) = reach(item, &["dependencies"]).and_then(Item::as_table_like_mut) {
        pinned_siblings(table, "workspace.dependencies", &mut |item, label| own(item, label, &mut set));
      }
    } else if key == "target"
      && let Some(targets) = item.as_table_like_mut()
    {
      for (platform, item) in targets.iter_mut() {
        let Some(platform_table) = item.as_table_like_mut() else {
          continue;
        };

        for (kind, item) in platform_table.iter_mut() {
          if kinds.iter().any(|one| one == kind.get())
            && let Some(table) = item.as_table_like_mut()
          {
            let named = format!("target.{}.{}", platform.get(), kind.get());

            pinned_siblings(table, &named, &mut |item, label| own(item, label, &mut set));
          }
        }
      }
    }
  }

  Ok((set, document.to_string()))
}

// A Cargo lock's packages read from a path; a registry's package names its source and is never a unit's.
fn cargo_lock(text: &str, standing: &Standing, to: &Version) -> Result<Rewritten, String> {
  let what = &VERSIONS.release.what;
  let mut document: DocumentMut = text.parse().map_err(|why: toml_edit::TomlError| why.to_string())?;
  let mut set = Vec::new();

  if let Some(packages) = document.get_mut("package").and_then(Item::as_array_of_tables_mut) {
    for package in packages.iter_mut() {
      if package.contains_key("source") {
        continue;
      }

      let name = package.get("name").and_then(Item::as_str).unwrap_or_default().to_owned();

      if let Some(item) = package.get_mut("version")
        && let Some((_, version)) = own_in(item, standing)
      {
        set.push((fill_marks(&what.lock, &[("name", &name)]), version.to_string(), to.to_string()));
        set_string(item, &to.to_string());
      }
    }
  }

  Ok((set, document.to_string()))
}

// A Python project's version, under the standard table or the tool that keeps its own.
fn pyproject(text: &str, standing: &Standing, to: &Version) -> Result<Rewritten, String> {
  let what = &VERSIONS.release.what;
  let mut document: DocumentMut = text.parse().map_err(|why: toml_edit::TomlError| why.to_string())?;
  let mut set = Vec::new();

  for path in &read_words().python_tables {
    let reached: Vec<&str> = path.iter().map(String::as_str).chain(["version"]).collect();

    if let Some(item) = reach(document.as_item_mut(), &reached)
      && let Some((_, version)) = own_in(item, standing)
    {
      set.push((
        fill_marks(&what.project, &[("table", &path.join("."))]),
        version.to_string(),
        to.to_string(),
      ));
      set_string(item, &to.to_string());
    }
  }

  Ok((set, document.to_string()))
}

// A version file's one line.
fn version_file(text: &str, standing: &Standing, to: &Version) -> Rewritten {
  match standing.owns_text(text) {
    Some(version) => (
      vec![(VERSIONS.release.what.version_file.clone(), version.to_string(), to.to_string())],
      text.replacen(text.trim(), &to.to_string(), 1),
    ),
    None => (Vec::new(), text.to_owned()),
  }
}

// A JSON document's top-level members and its OpenAPI `info.version` that hold an own version. JSON is read by
// its reader; the value is then changed where it stands in the text, found by its member's key and its value,
// since no JSON reader the testbed takes writes a document back in the layout its repository gave it. A member
// whose key and value the text holds in more places than the document sets is not placed, and the file is
// named as not read.
fn json_document(text: &str, standing: &Standing, to: &Version) -> Result<Rewritten, String> {
  let what = &VERSIONS.release.what;
  let document: serde_json::Value = serde_json::from_str(text).map_err(|why| why.to_string())?;
  let Some(top) = document.as_object() else {
    return Ok((Vec::new(), text.to_owned()));
  };
  let info = top
    .get("info")
    .and_then(serde_json::Value::as_object)
    .and_then(|info| info.get("version"));
  let members = top
    .iter()
    .map(|(key, value)| (key.clone(), key.clone(), value))
    .chain(info.map(|value| ("version".to_owned(), "info.version".to_owned(), value)));
  let mut set = Vec::new();
  let mut out = text.to_owned();

  for (key, label, value) in members {
    let Some(written) = value.as_str() else {
      continue;
    };
    let Some(version) = standing.owns_text(written) else {
      continue;
    };
    let moved = Version {
      build: version.build.clone(),
      ..to.clone()
    };
    let pattern =
      Regex::new(&format!(r#"("{}"\s*:\s*"){}(")"#, regex::escape(&key), regex::escape(written)))
        .expect("a member's pattern is a regular expression");

    if pattern.find_iter(&out).count() != 1 {
      return Err(fill_marks(&what.unplaced, &[("member", &label)]));
    }

    out = pattern.replace(&out, format!("${{1}}{moved}${{2}}")).into_owned();
    set.push((fill_marks(&what.member, &[("member", &label)]), written.to_owned(), moved.to_string()));
  }

  Ok((set, out))
}

// A document's "Fixed at" line: the version after the words, written bare or as code. The line is the
// set's own phrase, read as a phrase; the markdown around it is not read.
fn fixed_at(text: &str, standing: &Standing, to: &Version) -> Rewritten {
  let pattern = Regex::new(
    r"(?i)(\bfixed at (?:version )?`?)([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?)",
  )
  .expect("the fixed-at pattern is a regular expression");
  let mut set = Vec::new();
  let out = pattern.replace_all(text, |found: &regex::Captures<'_>| match standing.owns_text(&found[2]) {
    Some(version) => {
      set.push((VERSIONS.release.what.fixed_at.clone(), version.to_string(), to.to_string()));
      format!("{}{to}", &found[1])
    }
    None => found[0].to_owned(),
  });

  (set, out.into_owned())
}

// Whether a file is left unread: inside an installed package or a fixture, or one an exclude glob reaches.
fn excluded(path: &str, exclude: &[String]) -> bool {
  path.split('/').any(|part| read_words().unread_folders.iter().any(|folder| folder == part))
    || exclude.iter().any(|glob| glob_matches(glob, path))
}

// Whether a file is a lock, whose mentions of versions are a package manager's to move.
fn is_lock(path: &str) -> bool {
  let name = path.rsplit('/').next().unwrap_or(path);

  name.ends_with(".lock") || name.ends_with(".lockb") || name.contains("-lock.")
}

/// What a repository's files come to at a release to `to`, each file handed as its path and its text: every
/// own version set where it stands, every other file naming one left, the excluded ones not read.
pub fn plan_files(
  files: &[(String, String)],
  standing: &Standing,
  to: &Version,
  exclude: &[String],
) -> Plan {
  let mention = standing.mention();
  let mut plan = Plan::default();

  for (path, text) in files {
    if excluded(path, exclude) {
      continue;
    }

    let name = path.rsplit('/').next().unwrap_or(path);
    let rewritten = if name == read_words().cargo_manifest {
      cargo_manifest(text, standing, to)
    } else if name == read_words().cargo_lock {
      cargo_lock(text, standing, to)
    } else if name == read_words().pyproject {
      pyproject(text, standing, to)
    } else if name == VERSIONS.version_file {
      Ok(version_file(text, standing, to))
    } else if name.ends_with(&read_words().json_suffix) {
      json_document(text, standing, to)
    } else if name.ends_with(&read_words().markdown_suffix) {
      Ok(fixed_at(text, standing, to))
    } else {
      Ok((Vec::new(), text.clone()))
    };

    match rewritten {
      Err(why) => plan.unread.push(format!("{path}: {why}")),
      Ok((set, _)) if set.is_empty() => {
        if !is_lock(path) && mention.is_match(text) {
          plan.left.push(path.clone());
        }
      }
      Ok((set, out)) => {
        plan.changes.extend(set.into_iter().map(|(what, from, to)| Change {
          path: path.clone(),
          what,
          from,
          to,
        }));
        plan.edits.push((path.clone(), out));
      }
    }
  }

  plan
}

/// A repository read for a release: where its main stands, and what its files come to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRead {
  pub member: Member,
  pub head: String,
  pub plan: Plan,
}

fn main_ref(member: &Member) -> String {
  format!("origin/{}", member.main)
}

/// The repository as its remote's main holds it, at a release from `standing` to `to`: the files that may
/// declare an own version found there and worked out.
pub fn read_member(
  member: &Member,
  standing: &Standing,
  to: &Version,
  git: Git,
) -> Result<MemberRead, String> {
  let at = main_ref(member);
  let root = &member.checkout;
  let head = git(&["rev-parse", "--verify", &format!("{at}^{{commit}}")], root)?.trim().to_owned();
  // `git grep` ends 1 where nothing matches, which is no fault.
  let found =
    git(&["grep", "-l", "-I", "-E", &standing.candidates(), &head, "--"], root).unwrap_or_default();
  let mut paths: Vec<String> = found
    .lines()
    .filter_map(|line| line.strip_prefix(&format!("{head}:")).map(str::to_owned))
    .collect();
  let show = |path: &str| git(&["show", &format!("{head}:{path}")], root);
  let declaration = read_declaration(&show(&DECLARATION.file).unwrap_or_default());
  let exclude: Vec<String> = declared(&declaration, &["check", "shared_versions", "exclude"])
    .and_then(toml::Value::as_array)
    .map(|globs| globs.iter().filter_map(toml::Value::as_str).map(str::to_owned).collect())
    .unwrap_or_default();

  paths.sort();

  let files: Vec<(String, String)> =
    paths.into_iter().filter_map(|path| show(&path).ok().map(|text| (path, text))).collect();

  Ok(MemberRead {
    member: member.clone(),
    head,
    plan: plan_files(&files, standing, to, &exclude),
  })
}

/// The lines a repository's reading prints: each value set, the files left, and what the release does in git.
pub fn member_lines(read: &MemberRead, to: &Version) -> Vec<String> {
  let said = &VERSIONS.release;
  let member = &read.member;
  let short: String = read.head.chars().take(12).collect();
  let mut lines = vec![fill_marks(
    &said.repository,
    &[("name", &member.name), ("folder", &member.folder), ("head", &short)],
  )];

  if read.plan.changes.is_empty() {
    lines.push(said.untouched.clone());
  }

  let mut locks: Vec<(&str, usize)> = Vec::new();

  for change in &read.plan.changes {
    // A lock's packages are many and move together, so a lock is one line.
    if change.path.rsplit('/').next() == Some(read_words().cargo_lock.as_str()) {
      match locks.iter_mut().find(|(path, _)| *path == change.path) {
        Some((_, count)) => *count += 1,
        None => locks.push((&change.path, 1)),
      }

      continue;
    }

    lines.push(fill_marks(
      &said.change,
      &[
        ("path", &change.path),
        ("what", &change.what),
        ("from", &change.from),
        ("to", &change.to),
      ],
    ));
  }

  for (path, count) in locks {
    lines.push(fill_marks(
      &said.locked,
      &[("path", path), ("count", &count.to_string()), ("to", &to.to_string())],
    ));
  }

  if !read.plan.left.is_empty() {
    let shown_count = read_words().left_shown;
    let shown = read.plan.left.iter().take(shown_count).cloned().collect::<Vec<_>>().join(&said.joiner);
    let files = if read.plan.left.len() > shown_count {
      fill_marks(
        &said.more,
        &[("shown", &shown), ("more", &(read.plan.left.len() - shown_count).to_string())],
      )
    } else {
      shown
    };

    lines.push(fill_marks(&said.left, &[("files", &files)]));
  }

  let tag = release_tag(&to.to_string());
  let subject = fill_marks(&said.subject, &[("version", &to.to_string())]);
  let then = if read.plan.edits.is_empty() { &said.tag_only } else { &said.commit };

  lines.push(fill_marks(then, &[("subject", &subject), ("main", &member.main), ("tag", &tag)]));
  lines
}

/// Why a release to `to` cannot be made over the repositories read, or none: a version that is no plain
/// semantic version above the last release, a tag of it a repository already holds, or files that do not read.
pub fn refusal(
  to: &Version,
  standing: &Standing,
  tagged: &[String],
  reads: &[MemberRead],
) -> Option<String> {
  let said = &VERSIONS.release;

  if !to.pre.is_empty() || !to.build.is_empty() || *to <= standing.version() {
    return Some(fill_marks(
      &said.refused_version,
      &[("version", &to.to_string()), ("last", &standing.version().to_string())],
    ));
  }

  if !tagged.is_empty() {
    return Some(fill_marks(
      &said.tagged,
      &[("repositories", &tagged.join(&said.joiner)), ("tag", &release_tag(&to.to_string()))],
    ));
  }

  let unread: Vec<String> = reads
    .iter()
    .flat_map(|read| read.plan.unread.iter().map(|one| format!("{} {one}", read.member.name)))
    .collect();

  (!unread.is_empty()).then(|| fill_marks(&said.unreadable_files, &[("files", &unread.join(&said.joiner))]))
}

/// The release made in git over the repositories read, in the order the document gives: every release commit
/// made in a worktree of its own under `scratch`, then every push, then every tag pushed. Its count of release
/// commits, or the line naming what failed and what landed.
pub fn make_release(reads: &[MemberRead], to: &Version, scratch: &Path, git: Git) -> Result<usize, String> {
  let said = &VERSIONS.release;
  let version = to.to_string();
  let subject = fill_marks(&said.subject, &[("version", &version)]);
  let tag = release_tag(&version);
  let failed = |read: &MemberRead, step: &str, why: String| {
    fill_marks(&said.step_failed, &[("folder", &read.member.folder), ("step", step), ("why", &why)])
  };
  let mut released: Vec<(&MemberRead, PathBuf, String)> = Vec::new();

  std::fs::create_dir_all(scratch).map_err(|why| why.to_string())?;

  let outcome = (|| {
    for (at, read) in reads.iter().enumerate() {
      let root = &read.member.checkout;

      if read.plan.edits.is_empty() {
        released.push((read, PathBuf::new(), read.head.clone()));
        continue;
      }

      let tree = scratch.join(format!("{at:03}-{}", read.member.name));
      let tree_text = tree.to_string_lossy().into_owned();

      git(&["worktree", "add", "--quiet", "--detach", &tree_text, &read.head], root)
        .map_err(|why| failed(read, "worktree add", why))?;
      released.push((read, tree.clone(), String::new()));

      for (path, text) in &read.plan.edits {
        std::fs::write(tree.join(path), text).map_err(|why| failed(read, path, why.to_string()))?;
      }

      git(&["commit", "--quiet", "--all", "-m", &subject], &tree)
        .map_err(|why| failed(read, "commit", why))?;

      let commit = git(&["rev-parse", "HEAD"], &tree).map_err(|why| failed(read, "rev-parse", why))?;

      if let Some(last) = released.last_mut() {
        last.2 = commit.trim().to_owned();
      }
    }

    let mut landed: Vec<&str> = Vec::new();
    let landed_line = |landed: &[&str], why: String| {
      let rest: Vec<&str> = reads
        .iter()
        .map(|read| read.member.name.as_str())
        .filter(|name| !landed.contains(name))
        .collect();
      let none = said.nothing.as_str();
      let list = |names: &[&str]| if names.is_empty() { none.to_owned() } else { names.join(&said.joiner) };

      format!(
        "{why}\n{}",
        fill_marks(&said.landed, &[("landed", &list(landed)), ("rest", &list(&rest))])
      )
    };

    for (read, tree, commit) in &released {
      if !tree.as_os_str().is_empty() {
        let target = format!("{commit}:refs/heads/{}", read.member.main);

        git(&["push", "--quiet", "origin", &target], &read.member.checkout)
          .map_err(|why| landed_line(&landed, failed(read, "push", why)))?;
      }

      landed.push(&read.member.name);
    }

    for (read, _, commit) in &released {
      let root = &read.member.checkout;

      git(&["tag", "--annotate", "--message", &subject, &tag, commit], root)
        .map_err(|why| failed(read, "tag", why))?;
      git(&["push", "--quiet", "origin", &format!("refs/tags/{tag}")], root)
        .map_err(|why| failed(read, "tag push", why))?;
    }

    Ok(released.iter().filter(|(_, tree, _)| !tree.as_os_str().is_empty()).count())
  })();

  for (read, tree, _) in &released {
    if !tree.as_os_str().is_empty() {
      let _ = git(&["worktree", "remove", "--force", &tree.to_string_lossy()], &read.member.checkout);
    }
  }

  let _ = std::fs::remove_dir_all(scratch);
  outcome
}

/// The step a release takes by semver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
  Patch,
  Minor,
  Major,
}

impl Step {
  /// The word the proposal names it by.
  pub fn word(self) -> &'static str {
    let said = &VERSIONS.propose;

    match self {
      Self::Patch => &said.patch_word,
      Self::Minor => &said.minor_word,
      Self::Major => &said.major_word,
    }
  }

  /// The version this step leads to from where the set stands.
  pub fn from(self, standing: &Standing) -> Version {
    let last = standing.version();
    let (major, minor, patch) = match self {
      Self::Major => (last.major + 1, 0, 0),
      Self::Minor => (last.major, last.minor + 1, 0),
      Self::Patch => (last.major, last.minor, last.patch + 1),
    };

    Version {
      major,
      minor,
      patch,
      pre: Prerelease::EMPTY,
      build: BuildMetadata::EMPTY,
    }
  }
}

/// The step a commit's subject proposes: a major one where it is marked breaking, opens with a word for taking
/// away or renaming, or is a conventional commit marked `!`; a minor one where it opens with a word for adding
/// or is a conventional commit of a type for that; a patch otherwise.
pub fn step_of(subject: &str) -> Step {
  let said = &VERSIONS.propose;
  let conventional =
    Regex::new(r"^([A-Za-z]+)(?:\([^)]*\))?(!)?:\s*(.*)$").expect("the pattern is a regular expression");

  if said.breaking.iter().any(|mark| subject.contains(mark.as_str())) {
    return Step::Major;
  }

  let (kind, rest) = match conventional.captures(subject) {
    Some(found) if found.get(2).is_some() => return Step::Major,
    Some(found) => (Some(found[1].to_lowercase()), found[3].to_owned()),
    None => (None, subject.to_owned()),
  };
  let first = rest.split_whitespace().next().unwrap_or_default().to_lowercase();
  let named = |words: &[String]| words.contains(&first);

  if named(&said.major) {
    Step::Major
  } else if named(&said.minor) || kind.is_some_and(|kind| said.minor_types.contains(&kind)) {
    Step::Minor
  } else {
    Step::Patch
  }
}

/// A repository's commits since its last release, each counted by the step it proposes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
  pub name: String,
  pub commits: usize,
  pub counts: [usize; 3],
  pub strongest: Option<(Step, String)>,
}

/// The proposal of the subjects a repository's commits since its last release carry.
pub fn propose(name: &str, subjects: &[String]) -> Proposal {
  let mut counts = [0; 3];
  let mut strongest: Option<(Step, String)> = None;

  for subject in subjects {
    let step = step_of(subject);

    counts[step as usize] += 1;

    if strongest.as_ref().is_none_or(|(held, _)| step > *held) {
      strongest = Some((step, subject.clone()));
    }
  }

  Proposal {
    name: name.to_owned(),
    commits: subjects.len(),
    counts,
    strongest,
  }
}

/// The subjects of a repository's commits on its main since its newest release tag, or of every one.
pub fn subjects_since_release(member: &Member, git: Git) -> Result<Vec<String>, String> {
  let at = main_ref(member);
  let root = &member.checkout;
  let tags = git(&["tag", "--list", &format!("{}*", VERSIONS.tag_prefix), "--merged", &at], root)?;
  let range = match last_release(tags.lines()) {
    Some(version) => format!("{}..{at}", release_tag(&version.to_string())),
    None => at,
  };

  Ok(
    git(&["log", "--no-merges", "--format=%s", &range], root)?
      .lines()
      .map(str::to_owned)
      .collect(),
  )
}

/// The line a repository's proposal prints.
pub fn proposal_line(proposal: &Proposal) -> String {
  let said = &VERSIONS.propose;
  let example = proposal.strongest.as_ref().map_or_else(String::new, |(step, subject)| {
    fill_marks(&said.example, &[("step", step.word()), ("subject", subject)])
  });

  fill_marks(
    &said.repository,
    &[
      ("name", &proposal.name),
      ("commits", &proposal.commits.to_string()),
      ("major", &proposal.counts[Step::Major as usize].to_string()),
      ("minor", &proposal.counts[Step::Minor as usize].to_string()),
      ("patch", &proposal.counts[Step::Patch as usize].to_string()),
      ("example", &example),
    ],
  )
}
