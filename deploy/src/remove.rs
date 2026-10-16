//! The guarded removal (docs/deploy.md, 「What it refuses」): every deletion a deployment makes goes
//! through one removal that refuses anything inside the data root and anything the data root is under.

use std::path::{Path, PathBuf};

// A path made absolute against the working directory.
fn absolute(path: &Path) -> PathBuf {
  if path.is_absolute() {
    path.to_path_buf()
  } else {
    std::env::current_dir().unwrap_or_default().join(path)
  }
}

/// The git working tree a path stands in, or none.
pub fn git_tree_above(path: &Path) -> Option<PathBuf> {
  absolute(path).ancestors().find(|dir| dir.join(".git").exists()).map(Path::to_path_buf)
}

/// Why a path may not be deleted, or none: nothing under the data root goes, and neither does anything
/// the data root is under.
pub fn removal_fault(root: &Path, path: &Path) -> Option<String> {
  let target = absolute(path);
  let data = absolute(root);

  if target.starts_with(&data) {
    return Some(format!(
      "refusing to remove {}: it is inside the runtime data root {}",
      target.display(),
      data.display()
    ));
  }

  data.starts_with(&target).then(|| {
    format!(
      "refusing to remove {}: the runtime data root {} is inside it",
      target.display(),
      data.display()
    )
  })
}

/// The only deletion a deployment makes: refused rather than made where the guard above says no.
pub fn remove_tree(path: &Path, root: &Path) -> Result<(), String> {
  if let Some(fault) = removal_fault(root, path) {
    return Err(fault);
  }

  match std::fs::symlink_metadata(path) {
    Err(_) => Ok(()),
    Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path).map_err(|why| why.to_string()),
    Ok(_) => std::fs::remove_file(path).map_err(|why| why.to_string()),
  }
}
