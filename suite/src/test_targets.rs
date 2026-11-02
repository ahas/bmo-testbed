//! Which of a crate's tests a changed test file owes (docs/checks.md, 「The change check」): a test target's own
//! source runs that target, and a module of a harness, a target whose source is a `main.rs`, runs by the
//! module's name filter. A crate whose tests are a binary per file and one whose tests are one harness are read
//! the same way, from the test targets `cargo metadata` names. A file no target owns this way, or a module
//! other test files stand on, owes the crate's tests whole, which is the answer of none.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use crate::crates::{CrateDeclaration, TestTarget};
use crate::quick::source::{every_node, node_text, parse};

/// The tests of one target a change owes: the target whole where `filters` is empty, or else only the tests
/// whose names hold one of the filters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetTests {
  pub krate: String,
  pub target: String,
  pub filters: Vec<String>,
}

// What one changed file owes of a crate's tests: a target, whole or by one module's filter.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Owed {
  target: String,
  filter: Option<String>,
}

// The folder a harness's modules stand in: the folder of a target whose source is a `main.rs`.
fn harness_dir(target: &TestTarget) -> Option<&str> {
  target.path.strip_suffix("/main.rs")
}

// Every `.rs` file at and under `path` in the checkout at `root`, from the checkout's root.
fn rust_files(root: &Path, path: &str) -> Vec<String> {
  let mut found = Vec::new();
  let mut left = vec![path.to_owned()];

  while let Some(at) = left.pop() {
    let whole = root.join(&at);

    if whole.is_dir() {
      for entry in std::fs::read_dir(&whole).into_iter().flatten().flatten() {
        left.push(format!("{at}/{}", entry.file_name().to_string_lossy()));
      }
    } else if at.ends_with(".rs") && whole.is_file() {
      found.push(at);
    }
  }

  found.sort();
  found
}

// A path with `.` and `..` read off, as written from the checkout's root.
fn normalized(path: &Path) -> String {
  let mut parts: Vec<String> = Vec::new();

  for part in path.components() {
    match part {
      Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
      Component::ParentDir => {
        parts.pop();
      }
      _ => {}
    }
  }

  parts.join("/")
}

/// What a Rust file of a crate's tests says of the modules around it: whether it carries a test attribute,
/// `#[test]` or one ending in `::test`; the module names it walks a path through; and the files its `#[path]`
/// attributes name, from the checkout's root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestFileReading {
  pub has_tests: bool,
  pub path_segments: Vec<String>,
  pub path_files: Vec<String>,
}

/// A Rust file of a crate's tests read by the tree-sitter Rust grammar. `file` is its path from the
/// checkout's root, which a `#[path]` attribute is read against. Inside a macro's arguments a path is a run of
/// tokens, so a name followed by `::` is read as a segment wherever it stands.
pub fn read_test_file(text: &str, file: &str) -> TestFileReading {
  let Some(tree) = parse(tree_sitter_rust::LANGUAGE.into(), text) else {
    return TestFileReading::default();
  };
  let folder = Path::new(file).parent().unwrap_or(Path::new(""));
  let mut reading = TestFileReading::default();

  for node in every_node(&tree) {
    match node.kind() {
      "attribute" => {
        let Some(path) = node.named_child(0) else { continue };
        let name = node_text(path, text);

        if name == "test" || name.ends_with("::test") {
          reading.has_tests = true;
        }

        if name == "path"
          && let Some(value) = node.child_by_field_name("value")
          && let Some(content) = value.named_child(0).filter(|content| content.kind() == "string_content")
        {
          reading.path_files.push(normalized(&folder.join(node_text(content, text))));
        }
      }
      "::" => {
        if let Some(before) = node.prev_sibling() {
          reading
            .path_segments
            .extend(node_text(before, text).split("::").map(|segment| segment.trim().to_owned()));
        }
      }
      "use_list" => {
        for index in 0..node.named_child_count() {
          if let Some(child) = node.named_child(index as u32)
            && child.kind() == "identifier"
          {
            reading.path_segments.push(node_text(child, text).to_owned());
          }
        }
      }
      _ => {}
    }
  }

  reading.path_segments.sort();
  reading.path_segments.dedup();
  reading
}

fn read_at(root: &Path, file: &str) -> TestFileReading {
  std::fs::read_to_string(root.join(file))
    .map(|text| read_test_file(&text, file))
    .unwrap_or_default()
}

// The folder of the crate's tests from the checkout's root.
fn tests_dir(krate: &CrateDeclaration) -> String {
  if krate.dir.is_empty() { "tests".to_owned() } else { format!("{}/tests", krate.dir) }
}

// Whether a harness's module stands alone: its files carry a test, and no other file of the crate's tests walks
// a path through it or names one of its files by a `#[path]` attribute.
fn stands_alone(root: &Path, krate: &CrateDeclaration, module: &str, own: &[String]) -> bool {
  if !own.iter().any(|file| read_at(root, file).has_tests) {
    return false;
  }

  rust_files(root, &tests_dir(krate))
    .iter()
    .filter(|file| !own.contains(file))
    .all(|file| {
      let reading = read_at(root, file);

      !reading.path_segments.iter().any(|segment| segment == module)
        && !reading.path_files.iter().any(|named| own.contains(named))
    })
}

// What a changed path owes of its crate's tests; none where it owes them whole.
fn owed(root: &Path, krate: &CrateDeclaration, path: &str) -> Option<Owed> {
  if let Some(target) = krate.tests.iter().find(|target| target.path == path) {
    return Some(Owed {
      target: target.name.clone(),
      filter: None,
    });
  }

  if !path.ends_with(".rs") {
    return None;
  }

  let (target, dir) = krate
    .tests
    .iter()
    .filter_map(|target| harness_dir(target).map(|dir| (target, dir)))
    .filter(|(_, dir)| path.starts_with(&format!("{dir}/")))
    .max_by_key(|(_, dir)| dir.len())?;
  let below = &path[dir.len() + 1..];
  let module = match below.split_once('/') {
    Some((folder, _)) => folder,
    None => below.strip_suffix(".rs")?,
  };
  let own: Vec<String> = rust_files(root, &format!("{dir}/{module}.rs"))
    .into_iter()
    .chain(rust_files(root, &format!("{dir}/{module}")))
    .collect();

  stands_alone(root, krate, module, &own).then(|| Owed {
    target: target.name.clone(),
    filter: Some(format!("{module}::")),
  })
}

/// The tests a crate owes for its changed paths, in the checkout at `root`: each target they reach, whole or by
/// its modules' filters; none where any of the paths owes the crate's tests whole.
pub fn owed_tests(root: &Path, krate: &CrateDeclaration, paths: &[String]) -> Option<Vec<TargetTests>> {
  let mut by_target: BTreeMap<String, Option<Vec<String>>> = BTreeMap::new();

  for path in paths {
    let one = owed(root, krate, path)?;
    let filters = by_target.entry(one.target).or_insert_with(|| Some(Vec::new()));

    match (one.filter, filters.as_mut()) {
      (Some(filter), Some(held)) => held.push(filter),
      _ => *filters = None,
    }
  }

  (!by_target.is_empty()).then(|| {
    by_target
      .into_iter()
      .map(|(target, filters)| {
        let mut filters = filters.unwrap_or_default();

        filters.sort();
        filters.dedup();

        TargetTests {
          krate: krate.name.clone(),
          target,
          filters,
        }
      })
      .collect()
  })
}
