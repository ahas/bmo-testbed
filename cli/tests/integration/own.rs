//! A checkout of the testbed runs its own (docs/boundary.md, 「The binary」): a checkout is the testbed's where
//! a member of its Cargo workspace is the package the binary is built from, and no other is.

use std::path::Path;

use bmo_testbed_cli::own::builds_testbed;

fn write(root: &Path, files: &[(&str, &str)]) {
  for (path, text) in files {
    let at = root.join(path);

    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, text).unwrap();
  }
}

#[test]
fn a_checkout_is_the_testbeds_where_a_member_is_the_binarys_package() {
  let testbed = tempfile::tempdir().unwrap();
  let other = tempfile::tempdir().unwrap();

  write(
    testbed.path(),
    &[
      ("Cargo.toml", "[workspace]\nmembers = [\"core\", \"tool\"]\n"),
      ("core/Cargo.toml", "[package]\nname = \"core\"\n"),
      ("tool/Cargo.toml", "[package]\nname = \"bmo-testbed-cli\"\n"),
    ],
  );
  write(
    other.path(),
    &[
      ("Cargo.toml", "[workspace]\nmembers = [\"cli\", \"gone\"]\n"),
      ("cli/Cargo.toml", "[package]\nname = \"other-cli\"\n"),
    ],
  );

  assert!(builds_testbed(testbed.path()));
  assert!(!builds_testbed(other.path()));
  assert!(!builds_testbed(&other.path().join("cli")));
  assert!(builds_testbed(&Path::new(env!("CARGO_MANIFEST_DIR")).join("..")));
}
