//! The files of a checkout as the suite and its checks read them: every file git tracks or would track,
//! and the files of each submodule checked out in it under the submodule's path, so a project kept in a
//! repository of its own is read as the files around it are (docs/checks.md, 「Which tests a change
//! runs」). A submodule not checked out holds nothing to read and is left out.

use std::path::Path;
use std::process::{Command, Stdio};

// The paths `git ls-files` lists with `options`, or none where git cannot answer.
fn listed(root: &Path, options: &[&str]) -> Vec<String> {
  let ran = Command::new("git")
    .args(["ls-files", "-z"])
    .args(options)
    .current_dir(root)
    .stdin(Stdio::null())
    .stderr(Stdio::null())
    .output();

  ran
    .map(|ran| {
      String::from_utf8_lossy(&ran.stdout)
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
    })
    .unwrap_or_default()
}

/// Every file the checkout at `root` tracks or would track, each submodule's own among them; paths from
/// `root`.
pub fn checkout_files(root: &Path) -> Vec<String> {
  // A submodule is a gitlink's entry in the index.
  let submodules: Vec<String> = listed(root, &["--stage"])
    .into_iter()
    .filter(|entry| entry.starts_with("160000 "))
    .filter_map(|entry| entry.split_once('\t').map(|(_, path)| path.to_owned()))
    .collect();
  let mut files: Vec<String> = listed(root, &["--cached", "--others", "--exclude-standard"])
    .into_iter()
    .filter(|path| !submodules.contains(path))
    .collect();

  for path in submodules.iter().filter(|path| root.join(path).join(".git").exists()) {
    files.extend(checkout_files(&root.join(path)).into_iter().map(|file| format!("{path}/{file}")));
  }

  files
}
