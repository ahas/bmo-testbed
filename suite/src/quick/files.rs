//! The files of a checkout as its quick checks read them (docs/checks.md, 「The quick checks」): every file
//! git tracks or would track, a checked-out submodule's among them, that stands in the tree; and what no
//! quick check reads, recorded input and installed packages, or what only some do, test files.

use std::path::Path;

use crate::files::checkout_files;

/// A file of the checkout as text, any byte that is not UTF-8 read as the replacement character; empty
/// where it cannot be read.
pub fn read_text(root: &Path, path: &str) -> String {
  std::fs::read(root.join(path))
    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    .unwrap_or_default()
}

/// A file of the checkout as text where its bytes are valid UTF-8; none where they are not, as a packed
/// release's, an image's or a font's are, or where it cannot be read.
pub fn read_utf8(root: &Path, path: &str) -> Option<String> {
  std::fs::read(root.join(path)).ok().and_then(|bytes| String::from_utf8(bytes).ok())
}

/// The checkout's files that `wanted` takes and that stand in the tree, recorded input and installed
/// packages left out.
pub fn checkout_files_where(root: &Path, wanted: impl Fn(&str) -> bool) -> Vec<String> {
  checkout_files(root)
    .into_iter()
    .filter(|path| !is_recorded(path) && wanted(path) && root.join(path).is_file())
    .collect()
}

// Whether a path has a folder of the name among its folders.
fn under(path: &str, folder: &str) -> bool {
  path.split('/').rev().skip(1).any(|part| part == folder)
}

/// Whether a path is recorded input or an installed package: somebody else's words, under a `fixtures/`
/// or a `node_modules/` folder.
pub fn is_recorded(path: &str) -> bool {
  under(path, "fixtures") || under(path, "node_modules")
}

/// Whether a path is a test file, whose words are made up: a `.test.` file, a file under a `tests/` or a
/// `test/` folder, or a `tests.rs`.
pub fn is_test_file(path: &str) -> bool {
  let name = path.rsplit('/').next().unwrap_or_default();

  name.contains(".test.") || name == "tests.rs" || under(path, "tests") || under(path, "test")
}
