//! The release build's mark (docs/deploy.md, 「The releases」): a file a deployment writes into its build
//! tree's target directory before its build stage begins. The build tree is a checkout the gate builds in
//! like any other, and the release build's target directory stands inside it, apart from the shared build
//! root, so the prune reads a `target/` there as a stray a build typed outside the Cargo verb left; the mark
//! is what tells it apart, and it holds between the release build's end and the binaries' placement, while
//! no Cargo process works in the tree.

use std::path::Path;

/// The mark's file name inside the target directory.
pub const RELEASE_TARGET_MARK: &str = "bmo-release-target";

/// The target directory at `target` marked as a deployment's release build's own, the directory made where it
/// is not there yet.
pub fn mark_release_target(target: &Path) -> std::io::Result<()> {
  let mark = target.join(RELEASE_TARGET_MARK);

  if mark.is_file() {
    return Ok(());
  }

  std::fs::create_dir_all(target)?;
  std::fs::write(mark, "")
}

/// Whether the directory at `target` is a deployment's release build's own target directory.
pub fn is_release_target(target: &Path) -> bool {
  target.join(RELEASE_TARGET_MARK).is_file()
}
