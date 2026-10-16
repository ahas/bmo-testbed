//! The test count (docs/checks.md, 「The test count」): the TypeScript tests a range removed,
//! each either moved, rewritten in place, replaced by a Rust test naming it, or retired with a reason in a
//! commit of the range; a removal nothing accounts for fails the count.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use bmo_testbed_core::policy::count::COUNT;
use bmo_testbed_core::policy::fill_marks;
use regex::Regex;

use crate::titles::test_titles;

/// One TypeScript test, by its file and its title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedTest {
  pub file: String,
  pub title: String,
}

/// The count over one range.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RangeCount {
  /// The tests the changed files held at the range's start and hold at HEAD.
  pub before: usize,
  pub after: usize,
  /// Every title gone from its file and standing in no other, accounted for or not.
  pub removed: Vec<NamedTest>,
  /// The removals beyond what a file's new tests and the replacements and retirements account for.
  pub unaccounted: Vec<NamedTest>,
}

/// Which end of the range a file is read at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeEnd {
  Start,
  Head,
}

/// What a range is read from: the test files it changed, a file at either end, its commits' words, and the
/// Rust sources.
pub trait RangeReading {
  fn changed_tests(&self) -> Result<Vec<String>, String>;
  fn file_at(&self, end: RangeEnd, file: &str) -> Option<String>;
  fn messages(&self) -> Result<String, String>;
  fn rust_sources(&self) -> Result<Vec<String>, String>;
}

// The line a commit retires a test with: `Retired-test: <title> — <reason>`, the reason not empty. The
// last dash parts the two, since a title may hold a dash of its own.
fn retired_title(line: &str) -> Option<&str> {
  static RETIREMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Retired-test:\s*(.+)\s+—\s+\S").expect("the retirement line reads"));

  RETIREMENT
    .captures(line.trim())
    .and_then(|found| found.get(1))
    .map(|title| title.as_str())
}

// A Rust test, by the attribute that makes one.
static RUST_TEST: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(?m)^\s*#\[(?:tokio::)?test\b").expect("the test attribute reads"));

// What stands in `from` and not in `against`, as many times as `from` holds it beyond `against`.
fn beyond(from: &[String], against: &[String]) -> Vec<String> {
  let mut left: HashMap<&str, usize> = HashMap::new();

  for title in against {
    *left.entry(title.as_str()).or_default() += 1;
  }

  from
    .iter()
    .filter(|title| match left.get_mut(title.as_str()) {
      Some(held) if *held > 0 => {
        *held -= 1;
        false
      }
      _ => true,
    })
    .cloned()
    .collect()
}

/// The count over one range, read through `reading`.
pub fn count_range(reading: &dyn RangeReading) -> Result<RangeCount, String> {
  let files = reading.changed_tests()?;
  let titles = |end: RangeEnd, file: &str| -> Result<Vec<String>, String> {
    match reading.file_at(end, file) {
      None => Ok(Vec::new()),
      Some(source) => test_titles(&source).map_err(|why| {
        let said = &*COUNT;
        let at = match end {
          RangeEnd::Start => &said.start,
          RangeEnd::Head => &said.head,
        };

        fill_marks(&said.unreadable, &[("file", file), ("end", at), ("why", &why)])
      }),
    }
  };
  let mut changed: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();

  for file in files {
    let before = titles(RangeEnd::Start, &file)?;
    let after = titles(RangeEnd::Head, &file)?;

    changed.push((file, before, after));
  }

  let messages = reading.messages()?;
  let retired: HashSet<&str> = messages.lines().filter_map(retired_title).collect();
  let rust = reading.rust_sources()?;
  let accounted = |title: &str| retired.contains(title) || rust.iter().any(|source| source.contains(title));
  let mut count = RangeCount {
    before: changed.iter().map(|(_, before, _)| before.len()).sum(),
    after: changed.iter().map(|(_, _, after)| after.len()).sum(),
    ..RangeCount::default()
  };

  for (file, before, after) in &changed {
    let elsewhere: HashSet<&String> = changed
      .iter()
      .filter(|(other, _, _)| other != file)
      .flat_map(|(_, _, after)| after)
      .collect();
    let gone: Vec<String> =
      beyond(before, after).into_iter().filter(|title| !elsewhere.contains(title)).collect();
    let rewritten = beyond(after, before).len();
    let open: Vec<&String> = gone.iter().filter(|title| !accounted(title)).collect();

    count.removed.extend(gone.iter().map(|title| NamedTest {
      file: file.clone(),
      title: title.clone(),
    }));

    if open.len() > rewritten {
      count.unaccounted.extend(open.into_iter().map(|title| NamedTest {
        file: file.clone(),
        title: title.clone(),
      }));
    }
  }

  Ok(count)
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
  let done = Command::new("git")
    .args(args)
    .current_dir(root)
    .stdin(Stdio::null())
    .output()
    .map_err(|why| why.to_string())?;

  if done.status.success() {
    Ok(String::from_utf8_lossy(&done.stdout).into_owned())
  } else {
    Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&done.stderr).trim()))
  }
}

fn lines(text: &str) -> Vec<String> {
  text.lines().filter(|line| !line.is_empty()).map(str::to_owned).collect()
}

// The checkout's tracked files a pathspec names, as git lists them, a checked-out submodule's among them.
fn tracked(root: &Path, pattern: &str) -> Result<Vec<String>, String> {
  git(root, &["ls-files", "--recurse-submodules", "--", pattern]).map(|out| lines(&out))
}

// The submodule a file lies in at a commit, with the commit that commit pins it to; none where the file lies
// in none.
fn pinned_holder(root: &Path, at: &str, file: &str) -> Option<(String, String)> {
  file.match_indices('/').find_map(|(index, _)| {
    let dir = &file[..index];
    let entry = git(root, &["ls-tree", at, "--", dir]).ok()?;
    let pinned = entry.strip_prefix("160000 commit ")?.split('\t').next()?;

    Some((dir.to_owned(), pinned.to_owned()))
  })
}

// A submodule's folder where it stands checked out; none where it does not, since git asked in a folder that
// is no checkout of its own answers for the repository around it.
fn checked_out(root: &Path, dir: &str) -> Option<std::path::PathBuf> {
  Some(root.join(dir)).filter(|inside| inside.join(".git").exists())
}

// Every submodule a range moved from one pin to another, by its path, with both pins.
fn moved_pins(root: &Path, since: &str) -> Result<Vec<(String, String, String)>, String> {
  let raw = git(root, &["diff", "--raw", "--no-renames", "--no-abbrev", since, "HEAD"])?;

  Ok(
    raw
      .lines()
      .filter_map(|line| {
        let (meta, path) = line.split_once('\t')?;
        let fields: Vec<&str> = meta.split_whitespace().collect();

        match fields.as_slice() {
          [":160000", "160000", from, to, ..] => {
            Some((path.to_owned(), (*from).to_owned(), (*to).to_owned()))
          }
          _ => None,
        }
      })
      .collect(),
  )
}

// Where the Rust sources stand: wherever the repository itself tracks one, a submodule's own left out.
const RUST_SOURCES: &str = "*.rs";

// The Rust sources as the checkout holds them: a range's count reads a committed tree, so its files are HEAD's.
fn rust_sources(root: &Path) -> Result<Vec<String>, String> {
  let files = git(root, &["ls-files", "--", RUST_SOURCES]).map(|out| lines(&out))?;

  Ok(
    files
      .into_iter()
      .filter_map(|file| std::fs::read_to_string(root.join(file)).ok())
      .collect(),
  )
}

/// A range of a checkout, from `since` to HEAD, read with git.
pub struct GitRange<'a> {
  pub root: &'a Path,
  pub since: &'a str,
}

impl RangeReading for GitRange<'_> {
  // A submodule whose pin the range moved changed the test files that differ between its two pins.
  fn changed_tests(&self) -> Result<Vec<String>, String> {
    let mut files = lines(&git(
      self.root,
      &["diff", "--name-only", "--no-renames", self.since, "HEAD", "--", "*.test.ts"],
    )?);

    for (dir, from, to) in moved_pins(self.root, self.since)? {
      let Some(inside) = checked_out(self.root, &dir) else {
        continue;
      };
      let changed = git(&inside, &["diff", "--name-only", "--no-renames", &from, &to, "--", "*.test.ts"]);

      files.extend(lines(&changed.unwrap_or_default()).into_iter().map(|file| format!("{dir}/{file}")));
    }

    Ok(files)
  }

  // A file inside a submodule is read at the commit that end pins the submodule to.
  fn file_at(&self, end: RangeEnd, file: &str) -> Option<String> {
    let at = match end {
      RangeEnd::Start => self.since,
      RangeEnd::Head => "HEAD",
    };

    git(self.root, &["show", &format!("{at}:{file}")]).ok().or_else(|| {
      let (dir, pinned) = pinned_holder(self.root, at, file)?;

      git(
        &checked_out(self.root, &dir)?,
        &["show", &format!("{pinned}:{}", &file[dir.len() + 1..])],
      )
      .ok()
    })
  }

  fn messages(&self) -> Result<String, String> {
    git(self.root, &["log", "--format=%B", &format!("{}..HEAD", self.since)])
  }

  fn rust_sources(&self) -> Result<Vec<String>, String> {
    rust_sources(self.root)
  }
}

/// The tests a checkout holds: the TypeScript ones by their calls, the Rust ones by their attributes.
pub fn tree_count(root: &Path) -> Result<(usize, usize), String> {
  let mut typescript = 0;

  for file in tracked(root, "*.test.ts")? {
    if let Ok(source) = std::fs::read_to_string(root.join(&file)) {
      typescript += test_titles(&source).map_err(|why| format!("{file}: {why}"))?.len();
    }
  }

  let rust = rust_sources(root)?.iter().map(|source| RUST_TEST.find_iter(source).count()).sum();

  Ok((typescript, rust))
}
