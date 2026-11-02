//! The `imports` verb (docs/checks.md, 「Imports a move left behind」): the imports a move left behind,
//! found, named and written again all or none.

use std::path::Path;

use bmo_testbed_cli::imports::imports_in;
use bmo_testbed_core::declaration::Repository;

use crate::stand::git;

// The workspace a move leaves behind: the archive package's module and its kinds moved, every import naming
// their old places, and two `config.ts` files so `./config` has no one answer.
fn workspace() -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();
  let files = [
    ("package.json", r#"{"name":"fixture","workspaces":["server","client","packages/*"]}"#),
    ("server/package.json", r#"{"name":"@sample/server"}"#),
    ("client/package.json", r#"{"name":"sample-client"}"#),
    ("packages/archive/package.json", r#"{"name":"@sample/archive"}"#),
    ("packages/util/package.json", r#"{"name":"@sample/util"}"#),
    ("packages/archive/archive.ts", "export const archive = 1;\n"),
    ("packages/archive/kinds/archive-kinds.ts", "export const kinds = 1;\n"),
    (
      "packages/archive/store.ts",
      "import { kinds } from \"../../packages/archive/archive-kinds\";\n\nexport const store = kinds;\n",
    ),
    ("packages/util/config.ts", "export const config = 1;\n"),
    ("client/shared/config.ts", "export const config = 2;\n"),
    ("server/notes.ts", "export const notes = 1;\n"),
    (
      "server/uses.ts",
      "import { archive } from \"./archive\";\nimport type { notes } from \"./notes\";\nimport { z } from \"zod\";\n\nexport const later = () => import(\"./archive\");\nexport const used = archive;\n",
    ),
    ("server/two.ts", "import { config } from \"./config\";\n\nexport const two = config;\n"),
  ];

  for (path, text) in files {
    let at = dir.path().join(path);

    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, text).unwrap();
  }

  git(dir.path(), &["init", "-q"]);
  dir
}

fn run(root: &Path, base: &Path, given: &[&str], fix: bool) -> (String, u8) {
  let named: Vec<String> = given.iter().map(|one| (*one).to_owned()).collect();
  let said = imports_in(&named, fix, &Repository::read(root).unwrap(), base);

  (format!("{}\n{}", said.out.join("\n"), said.err.join("\n")), said.code)
}

fn read(dir: &Path, file: &str) -> String {
  std::fs::read_to_string(dir.join(file)).unwrap()
}

#[test]
fn fix_rewrites_every_import_of_the_files_named_and_a_second_run_finds_nothing_left() {
  let dir = workspace();
  let root = dir.path();
  let (out, code) = run(root, root, &["server/uses.ts", "packages/archive/store.ts"], true);

  assert_eq!(code, 0, "{out}");
  assert!(
    out.contains("server/uses.ts:1  \"./archive\" \u{2192} \"@sample/archive/archive\""),
    "{out}"
  );
  assert!(out.contains("3 import(s) fixed (2 file(s))"), "{out}");
  assert!(read(root, "server/uses.ts").contains("import { archive } from \"@sample/archive/archive\";"));
  assert!(read(root, "server/uses.ts").contains("import(\"@sample/archive/archive\")"));
  assert!(read(root, "packages/archive/store.ts").contains("from \"./kinds/archive-kinds\""));

  let (again, code) = run(root, root, &["server/uses.ts", "packages/archive/store.ts"], false);

  assert_eq!(code, 0, "{again}");
  assert!(again.contains("no relative import is left unresolved"), "{again}");
}

#[test]
fn a_run_holding_an_ambiguous_import_writes_nothing_at_all_and_names_every_file_the_tail_fits() {
  let dir = workspace();
  let root = dir.path();
  let before = read(root, "server/uses.ts");
  let (out, code) = run(root, root, &["server/uses.ts", "server/two.ts"], true);

  assert_eq!(code, 1, "{out}");
  assert!(
    out.contains("server/two.ts:1  \"./config\": 2 files end in config, so none was chosen"),
    "{out}"
  );
  assert!(out.contains("client/shared/config.ts"), "{out}");
  assert!(out.contains("packages/util/config.ts"), "{out}");
  assert!(out.contains("nothing was written"), "{out}");
  assert_eq!(read(root, "server/uses.ts"), before);
}

#[test]
fn without_fix_it_only_says_and_ends_non_zero_while_anything_is_left() {
  let dir = workspace();
  let root = dir.path();
  let before = read(root, "server/uses.ts");
  let (out, code) = run(root, root, &["server/uses.ts"], false);

  assert_eq!(code, 1, "{out}");
  assert!(out.contains("2 import(s) to fix (1 file(s))"), "{out}");
  assert_eq!(read(root, "server/uses.ts"), before);
}

#[test]
fn a_path_is_read_from_where_the_command_was_asked() {
  let dir = workspace();
  let root = dir.path();
  let (out, code) = run(root, &root.join("server"), &["uses.ts"], false);

  assert_eq!(code, 1, "{out}");
  assert!(out.contains("2 import(s) to fix (1 file(s))"), "{out}");
}

#[test]
fn a_path_that_is_not_one_of_the_checkouts_files_is_refused() {
  let dir = workspace();
  let root = dir.path();

  assert_eq!(run(root, root, &["server/nowhere.ts"], false).1, 2);
  assert_eq!(run(root, &root.join("server"), &["server/uses.ts"], false).1, 2);
}
