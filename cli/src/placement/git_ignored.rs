//! What a bench copy leaves alone in a tree: the paths git itself names as ignored there, by its own rules —
//! every `.gitignore` with its `!` lines, the repository's exclude file and the account's — so the bench
//! holds what git sees in the tree (docs/testbed.md, 「A run on another machine」).

use std::path::Path;
use std::process::{Command, Stdio};

/// The git command naming the ignored paths of the tree it runs in, each from the tree's top and ended by a
/// NUL, a folder whose every path git ignores written once, ending in `/`. Run here and in a bench there alike.
pub const LISTING: [&str; 7] =
  ["git", "ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"];

/// The paths git ignores in a tree here, as `LISTING` names them; what git said where it cannot list them, a
/// tree no repository holds among them.
pub fn ignored_in(tree: &Path) -> Result<Vec<Vec<u8>>, String> {
  let [program, args @ ..] = LISTING;
  let done = Command::new(program)
    .args(args)
    .current_dir(tree)
    .stdin(Stdio::null())
    .output()
    .map_err(|why| why.to_string())?;

  if !done.status.success() {
    return Err(String::from_utf8_lossy(&done.stderr).trim().to_owned());
  }

  Ok(listed(&done.stdout))
}

/// The paths `LISTING` wrote.
pub fn listed(output: &[u8]) -> Vec<Vec<u8>> {
  output
    .split(|byte| *byte == 0)
    .filter(|path| !path.is_empty())
    .map(<[u8]>::to_vec)
    .collect()
}

/// Whether a folder, written from its checkout's top with `/` between its parts, is one of `ignored` or lies
/// inside one, so a bench copy would not hold it.
pub fn lies_in_ignored(folder: &str, ignored: &[Vec<u8>]) -> bool {
  let folder = format!("{folder}/");

  ignored.iter().any(|path| path.ends_with(b"/") && folder.as_bytes().starts_with(path))
}

/// A path written as the pattern of an `rsync` filter rule that matches that path alone: anchored at the
/// copy's top, a folder still ending in `/`. `rsync` reads its own filter syntax and no library writes it, so
/// the path is made literal here: `rsync` reads a backslash as an escape only in a pattern holding a wildcard,
/// so the wildcards and backslashes of such a path are escaped and any other path stands as it is.
pub fn rsync_pattern(path: &[u8]) -> Vec<u8> {
  let special = |byte: &u8| matches!(byte, b'*' | b'?' | b'[');
  let mut pattern = Vec::with_capacity(path.len() + 1);

  pattern.push(b'/');

  if path.iter().any(special) {
    for byte in path {
      if special(byte) || *byte == b'\\' {
        pattern.push(b'\\');
      }

      pattern.push(*byte);
    }
  } else {
    pattern.extend_from_slice(path);
  }

  pattern
}
