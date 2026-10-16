use std::path::Path;

use bmo_testbed_core::declaration::Repository;
use bmo_testbed_suite::moved_imports::{
  ImportRepair, Outcome, apply_repairs, code_files, import_repairs, relative_imports, repository_files,
  spelling,
};
use bmo_testbed_suite::workspace::workspace_declarations;

fn specs_and_lines(file: &str, text: &str) -> Vec<(String, usize)> {
  relative_imports(file, text).into_iter().map(|site| (site.spec, site.line)).collect()
}

// A workspace as a move leaves it: `packages/archive/archive.ts` carried into the archive package and
// `packages/archive/archive-kinds.ts` into a directory of it, with every import that named them still naming
// their old places. Two `config.ts` files stand in two places, so an import of `./config` has no one answer.
fn workspace() -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();
  let files = [
    ("package.json", r#"{"name":"fixture","workspaces":["server","client","packages/*"]}"#),
    ("server/package.json", r#"{"name":"@sample/bot"}"#),
    ("client/package.json", r#"{"name":"sample-web"}"#),
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
    (
      "server/uses.test.ts",
      "import { mock } from \"bun:test\";\n\nmock.module(\"./archive\", () => ({ archive: 2 }));\n",
    ),
    ("server/two.ts", "import { config } from \"./config\";\n\nexport const two = config;\n"),
    ("server/gone.ts", "import \"./vanished\";\n"),
  ];

  for (path, text) in files {
    let at = dir.path().join(path);

    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, text).unwrap();
  }

  let done = std::process::Command::new("git")
    .args(["init", "-q"])
    .current_dir(dir.path())
    .output()
    .unwrap();

  assert!(done.status.success());
  dir
}

fn repairs_of(dir: &Path) -> Vec<ImportRepair> {
  let all = repository_files(&Repository::read(dir).unwrap()).unwrap();

  import_repairs(dir, &code_files(&all), &all, &workspace_declarations(dir))
}

fn of(repairs: &[ImportRepair], file: &str) -> Vec<(String, String)> {
  repairs
    .iter()
    .filter(|repair| repair.file == file)
    .map(|repair| {
      let to = match &repair.outcome {
        Outcome::To(to) => to.clone(),
        Outcome::Ambiguous { .. } => "ambiguous".to_owned(),
        Outcome::Missing { .. } => "missing".to_owned(),
      };

      (repair.site.spec.clone(), to)
    })
    .collect()
}

fn pairs(given: &[(&str, &str)]) -> Vec<(String, String)> {
  given.iter().map(|(one, other)| ((*one).to_owned(), (*other).to_owned())).collect()
}

#[test]
fn every_quoted_relative_path_is_read_and_a_bare_package_name_is_not() {
  let text = [
    "import { a } from \"./a\";",
    "import type { B } from '../b';",
    "export { c } from \"./c\";",
    "import \"./d\";",
    "const e = await import(\"./e\");",
    "mock.module(\"./f\", () => ({}));",
    "const g = require(\"./g\");",
    "import { z } from \"zod\";",
  ]
  .join("\n");
  let wanted: Vec<(String, usize)> =
    [("./a", 1), ("../b", 2), ("./c", 3), ("./d", 4), ("./e", 5), ("./f", 6), ("./g", 7)]
      .iter()
      .map(|(spec, line)| ((*spec).to_owned(), *line))
      .collect();

  assert_eq!(specs_and_lines("one.ts", &text), wanted);
}

#[test]
fn an_import_written_inside_a_string_a_template_or_a_comment_is_text_and_is_not_read() {
  let text = [
    "const script = [`import { a } from \"./server/a\";`, 'import \"./server/b\";'];",
    "// import { c } from \"./c\";",
    "/* export { d } from \"./d\"; */",
    "const shown = `${await import(\"./e\")} and import(\"./f\")`;",
    "import { g } from \"./g\";",
  ]
  .join("\n");
  let read: Vec<String> = relative_imports("one.ts", &text).into_iter().map(|site| site.spec).collect();

  assert_eq!(read, ["./e", "./g"]);
}

#[test]
fn a_components_imports_are_read_off_its_scripts_where_they_stand_in_the_file() {
  let text = "<template>\n  <p>import \"./not\"</p>\n</template>\n<script setup lang=\"ts\">\nimport One from \"./one.vue\";\n</script>\n";
  let sites = relative_imports("page.vue", text);

  assert_eq!(sites.len(), 1);
  assert_eq!((sites[0].spec.as_str(), sites[0].line), ("./one.vue", 5));
  assert_eq!(&text[sites[0].start..sites[0].end], "./one.vue");
}

#[test]
fn the_packages_are_the_ones_the_root_lists_each_by_the_name_its_own_manifest_gives() {
  let dir = workspace();
  let packages: Vec<(String, String)> =
    workspace_declarations(dir.path()).into_iter().map(|one| (one.dir, one.name)).collect();

  assert_eq!(
    packages,
    pairs(&[
      ("server", "@sample/bot"),
      ("client", "sample-web"),
      ("packages/archive", "@sample/archive"),
      ("packages/util", "@sample/util"),
    ])
  );
}

#[test]
fn across_packages_it_is_spelled_by_the_packages_name_whatever_form_the_import_took() {
  let dir = workspace();
  let repairs = repairs_of(dir.path());

  assert_eq!(
    of(&repairs, "server/uses.ts"),
    pairs(&[("./archive", "@sample/archive/archive"), ("./archive", "@sample/archive/archive")])
  );
  assert_eq!(of(&repairs, "server/uses.test.ts"), pairs(&[("./archive", "@sample/archive/archive")]));
}

#[test]
fn inside_one_package_it_is_spelled_relative_and_a_tail_no_longer_whole_is_found_by_its_end() {
  let dir = workspace();

  assert_eq!(
    of(&repairs_of(dir.path()), "packages/archive/store.ts"),
    pairs(&[("../../packages/archive/archive-kinds", "./kinds/archive-kinds")])
  );
}

#[test]
fn an_import_that_still_resolves_and_one_that_is_not_relative_are_left_alone() {
  let dir = workspace();

  assert!(
    !repairs_of(dir.path())
      .iter()
      .any(|repair| repair.site.spec == "./notes" || repair.site.spec == "zod")
  );
}

#[test]
fn a_tail_two_files_end_with_is_refused_with_both_named_and_one_nothing_ends_with_is_refused_too() {
  let dir = workspace();
  let repairs = repairs_of(dir.path());
  let two = repairs
    .iter()
    .find(|repair| repair.file == "server/two.ts")
    .map(|repair| repair.outcome.clone());
  let gone = repairs
    .iter()
    .find(|repair| repair.file == "server/gone.ts")
    .map(|repair| repair.outcome.clone());

  match two {
    Some(Outcome::Ambiguous {
      tail,
      mut candidates,
    }) => {
      candidates.sort();
      assert_eq!(tail, "config");
      assert_eq!(candidates, ["client/shared/config.ts", "packages/util/config.ts"]);
    }
    other => panic!("server/two.ts: {other:?}"),
  }

  assert_eq!(
    gone,
    Some(Outcome::Missing {
      tail: "vanished".to_owned()
    })
  );
}

#[test]
fn an_extension_the_import_spelled_is_kept_and_an_index_is_named_by_its_directory() {
  let packages = Vec::new();

  assert_eq!(spelling("a/one.ts", "./gone.js", "a/b/gone.ts", &packages), "./b/gone.js");
  assert_eq!(spelling("a/one.ts", "./gone", "a/b/gone/index.ts", &packages), "./b/gone");
  assert_eq!(spelling("a/one.ts", "./gone/index", "a/b/gone/index.ts", &packages), "./b/gone/index");
  assert_eq!(spelling("a/c/one.ts", "./gone.json", "a/gone.json", &packages), "../gone.json");
}

#[test]
fn the_repairs_are_written_all_or_none() {
  let dir = workspace();
  let repairs = repairs_of(dir.path());

  assert!(apply_repairs(dir.path(), &repairs).is_err());

  let chosen: Vec<ImportRepair> = repairs.into_iter().filter(|repair| !repair.refused()).collect();

  assert_eq!(
    apply_repairs(dir.path(), &chosen).unwrap(),
    [
      ("packages/archive/store.ts".to_owned(), 1),
      ("server/uses.test.ts".to_owned(), 1),
      ("server/uses.ts".to_owned(), 2)
    ]
  );
  assert!(
    std::fs::read_to_string(dir.path().join("server/uses.ts"))
      .unwrap()
      .contains("import(\"@sample/archive/archive\")")
  );
}
