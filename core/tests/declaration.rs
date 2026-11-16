//! A repository's declaration (docs/declaration.md): every default where nothing is declared, every key
//! of the schema read, and each declaration the testbed refuses, refused naming where it breaks.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use bmo_testbed_core::declaration::{Declaration, Phase, QuickCheck, Repository, own_major};
use bmo_testbed_core::policy::declaration::DECLARATION;
use bmo_testbed_core::silence::Silence;
use bmo_testbed_core::turn::{Env, Lane};

fn parse(text: &str) -> Result<Declaration, String> {
  Declaration::parse("testbed.toml", text).map_err(|refusal| refusal.0)
}

fn refused(text: &str) -> String {
  parse(text).expect_err("the declaration was read")
}

fn declared(rest: &str) -> String {
  format!("testbed = {}\n{rest}", own_major())
}

fn checkout() -> tempfile::TempDir {
  let dir = tempfile::tempdir().unwrap();
  let made = Command::new("git")
    .arg("-C")
    .arg(dir.path())
    .args(["init", "--quiet"])
    .stdin(Stdio::null())
    .status()
    .unwrap();
  assert!(made.success());
  dir
}

fn canonical(path: &Path) -> PathBuf {
  path.canonicalize().unwrap()
}

// "with no declaration every default stands": the defaults test, check and deploy nothing.
#[test]
fn a_checkout_with_no_declaration_has_every_default_and_its_directorys_name() {
  let dir = checkout();
  let inner = dir.path().join("inner");
  std::fs::create_dir(&inner).unwrap();

  let repository = Repository::read(&inner).unwrap();
  let declaration = &repository.declaration;

  assert_eq!(canonical(&repository.root), canonical(dir.path()));
  assert_eq!(repository.name(), dir.path().file_name().unwrap().to_string_lossy());
  assert_eq!(declaration.testbed, own_major());
  assert!(declaration.upstreams.is_empty());
  assert!(declaration.test.languages.is_empty());
  assert_eq!(declaration.test.files, None);
  assert_eq!(
    (
      declaration.test.guard_seconds,
      declaration.test.file_guard_seconds,
      declaration.test.test_timeout_seconds,
      declaration.test.memory_gb
    ),
    (None, None, None, 12)
  );
  assert_eq!(declaration.test.silence(), Silence::default());
  assert!(declaration.test.database.is_none());
  assert!(declaration.projects.discover);
  assert_eq!(declaration.projects.check_script, "check");
  assert_eq!(declaration.tools.dir, None);
  assert_eq!(
    (declaration.tools.manifest.as_str(), declaration.tools.check_key.as_str()),
    ("tool.toml", "tool.check")
  );
  assert!(declaration.check.quick.is_empty() && declaration.check.steps.is_empty());
  assert_eq!(declaration.check.main, "main");
  assert_eq!(declaration.check.wording_files, None);
  assert_eq!(declaration.build.binaries, None);
  assert_eq!(declaration.build.cargo_profile, "release");
  assert_eq!(declaration.deploy.root, None);
  assert_eq!(declaration.deploy.link, "deploy");
  assert_eq!(
    (declaration.deploy.health_wait_seconds, declaration.deploy.stay_up_seconds),
    (DECLARATION.defaults.deploy.health_wait_seconds, None)
  );
  assert!(declaration.deploy.gate);
  assert!(declaration.deploy.backup.is_none());
  assert_eq!(declaration.runner.placement, None);
  assert_eq!((declaration.events.hook.clone(), declaration.events.timeout_seconds), (None, None));
}

// "outside any checkout the directory stands as the root, with every default"
#[test]
fn outside_any_checkout_the_directory_is_the_root_and_every_default_stands() {
  let dir = tempfile::tempdir().unwrap();

  let repository = Repository::read(dir.path()).unwrap();

  assert_eq!(repository.root, dir.path());
  assert!(repository.declaration.test.languages.is_empty());
  assert_eq!(repository.declaration.deploy.root, None);
}

// "the declaration at the root is read from anywhere in the checkout"
#[test]
fn the_declaration_at_the_root_is_read_from_a_directory_inside_the_checkout() {
  let dir = checkout();
  std::fs::write(dir.path().join("testbed.toml"), declared("name = \"declared\"\n")).unwrap();
  let inner = dir.path().join("inner");
  std::fs::create_dir(&inner).unwrap();

  assert_eq!(Repository::read(&inner).unwrap().name(), "declared");
}

// "a declaration that does not read is refused, naming its file"
#[test]
fn a_declaration_of_broken_toml_in_a_checkout_is_refused_naming_its_file() {
  let dir = checkout();
  let file = dir.path().join("testbed.toml");
  std::fs::write(&file, "testbed = \n").unwrap();

  let refusal = Repository::read(dir.path()).unwrap_err().0;

  assert!(refusal.contains(&*file.file_name().unwrap().to_string_lossy()), "{refusal}");
  assert!(refusal.contains("does not read"), "{refusal}");
}

// "every key of the schema reads"
#[test]
fn every_key_of_the_schema_reads() {
  let declaration = parse(&declared(
    r#"
name = "example"
upstreams = ["shared-library", "other-upstream"]

[test]
languages = ["bun", "cargo", "node", "browser"]
files = ["tests/**/*.test.ts"]
browser = ["browser/**/*.test.ts"]
guard_seconds = 100
file_guard_seconds = 50
test_timeout_seconds = 10
silence_seconds = 1200
memory_gb = 4
setup = ["bun install"]

[test.database]
migrations = ["migrations/**"]
migrate = "cargo run -p migrate"
max_files = 3

[projects]
discover = false
paths = ["site"]
check_script = "verify"

[tools]
dir = "tools"
manifest = "manifest.toml"
check_key = "build.check"

[check]
quick = ["wording", "dashes", "language", "layout", "dependencies", "cargo-profile"]
wording_document = "docs/wording.md"
wording_files = ["src/**"]
people_text = ["src/**/*.ts"]
language_ledger = "docs/ledger.json"
base_libraries = ["shared-library"]
upstreams = ["beside-upstream"]
main = "trunk"

[[check.step]]
id = "types"
command = "bun run check"
dir = "site"
lane = "narrow"
when = ["site/**"]
retry_out_of_memory = true

[[check.step]]
id = "lint"
command = "cargo clippy"
lane = "wide"

[build]
binaries = ["example"]
cargo_profile = "dist"

[[build.step]]
id = "site"
command = "bun run build"
dir = "site"
lane = "narrow"

[deploy]
root = "~/deploys/example"
link = "live"
data_root = "/srv/example"
env_links = [{ link = "config/.env", target = "env/.env" }]
units = ["units/*.service"]
health = "example health"
health_wait_seconds = 60
stay_up_seconds = 5
gate = false

[[deploy.step]]
id = "migrate"
command = "bin/example migrate"
phase = "before-flip"

[[deploy.step]]
id = "warm"
command = "bin/example warm"
phase = "after-health"
timeout_seconds = 60

[deploy.backup]
dir = "~/backups"
name = "example-data"
snapshot = "bin/example snapshot"
verify = "bin/example verify"
exclude = ["cache"]
remote = "s3://bucket/example"
profile = "backup"
keep = 2

[runner]
placement = "http://placement.invalid/place"

[events]
hook = "example-events"
timeout_seconds = 5
"#,
  ))
  .unwrap();

  assert_eq!(declaration.name.as_deref(), Some("example"));
  assert_eq!(declaration.upstreams, ["shared-library", "other-upstream"]);
  assert_eq!(
    declaration.test.languages().iter().map(|language| language.name()).collect::<Vec<_>>(),
    ["bun", "cargo", "node", "browser"]
  );
  assert_eq!(declaration.test.memory_gb, 4);
  assert_eq!(
    declaration.test.silence(),
    Silence {
      limit: Duration::from_secs(1200),
      ..Silence::default()
    }
  );
  let database = declaration.test.database.as_ref().unwrap();
  assert_eq!((database.migrate.as_deref(), database.max_files), (Some("cargo run -p migrate"), 3));
  assert!(!declaration.projects.discover);
  assert_eq!(declaration.tools.check_key, "build.check");
  assert_eq!(
    declaration.check.quick,
    [
      QuickCheck::Wording,
      QuickCheck::Dashes,
      QuickCheck::Language,
      QuickCheck::Layout,
      QuickCheck::Dependencies,
      QuickCheck::CargoProfile
    ]
  );
  assert_eq!(declaration.check.wording_files.as_deref(), Some(&["src/**".to_owned()][..]));
  assert_eq!(declaration.check.upstreams, ["beside-upstream"]);
  assert_eq!(declaration.check.steps.len(), 2);
  assert_eq!(declaration.check.steps[0].lane, Some(Lane::Narrow));
  assert_eq!(declaration.check.steps[0].when.as_deref(), Some(&["site/**".to_owned()][..]));
  assert!(declaration.check.steps[0].retry_out_of_memory);
  assert_eq!(declaration.check.steps[1].lane, Some(Lane::Wide));
  assert_eq!(declaration.check.steps[1].dir, None);
  assert!(!declaration.check.steps[1].retry_out_of_memory);
  assert_eq!(declaration.build.steps[0].id, "site");
  assert_eq!(declaration.deploy.env_links[0].target, PathBuf::from("env/.env"));
  assert!(!declaration.deploy.gate);
  assert_eq!(declaration.deploy.health_wait_seconds, 60);
  assert_eq!(
    declaration
      .deploy
      .steps
      .iter()
      .map(|step| (step.phase, step.timeout_seconds))
      .collect::<Vec<_>>(),
    [(Phase::BeforeFlip, None), (Phase::AfterHealth, Some(60))]
  );
  assert_eq!(declaration.deploy.backup.as_ref().unwrap().keep, 2);
  assert_eq!(declaration.runner.placement.as_deref(), Some("http://placement.invalid/place"));
  assert_eq!(declaration.events.timeout_seconds, Some(5));
}

// "a backup leaves out what has a default"
#[test]
fn a_backup_naming_only_its_directory_has_every_other_default() {
  let declaration = parse(&declared("[deploy.backup]\ndir = \"backups\"\n")).unwrap();
  let backup = declaration.deploy.backup.unwrap();

  assert_eq!((backup.name, backup.snapshot, backup.keep), (None, None, 5));
  assert!(backup.exclude.is_empty());

  // A backup naming no directory stands under the deploy root, which the deployment resolves.
  let bare = parse(&declared("[deploy.backup]\n")).unwrap().deploy.backup.unwrap();

  assert_eq!(bare.dir, None);
}

// "a key the schema does not name is refused, naming the key and its table"
#[test]
fn a_key_the_schema_does_not_name_is_refused_naming_the_key_and_its_table() {
  let refusal = refused(&declared("[test]\nguard_second = 10\n"));
  assert!(refusal.contains("at test"), "{refusal}");
  assert!(refusal.contains("guard_second"), "{refusal}");

  let refusal = refused(&declared("[[check.step]]\nid = \"a\"\ncommand = \"true\"\nlanes = \"wide\"\n"));
  assert!(refusal.contains("check.step"), "{refusal}");
  assert!(refusal.contains("lanes"), "{refusal}");

  let refusal = refused(&declared("colour = \"blue\"\n"));
  assert!(refusal.contains("colour"), "{refusal}");
}

// "a quick check enabled without the input it reads is refused, naming the check and the input"
#[test]
fn a_quick_check_enabled_without_the_input_it_reads_is_refused_naming_the_check_and_the_input() {
  let refusal = refused(&declared("[check]\nquick = [\"language\"]\n"));
  assert!(refusal.contains("the quick check language reads language_ledger"), "{refusal}");

  let refusal = refused(&declared("[check]\nquick = [\"dashes\", \"wording\"]\n"));
  assert!(refusal.contains("the quick check wording reads wording_document"), "{refusal}");
}

// "a value of the wrong kind is refused where it stands"
#[test]
fn a_value_of_the_wrong_kind_is_refused_where_it_stands() {
  let refusal = refused(&declared("[deploy]\ngate = \"yes\"\n"));
  assert!(refusal.contains("deploy.gate"), "{refusal}");

  let refusal = refused(&declared("[[check.step]]\nid = \"a\"\ncommand = \"true\"\nlane = \"middle\"\n"));
  assert!(refusal.contains("check.step"), "{refusal}");
  assert!(refusal.contains("middle"), "{refusal}");
}

// "a declaration naming no major is refused at the top level"
#[test]
fn a_declaration_naming_no_major_is_refused_at_the_top_level() {
  let refusal = refused("name = \"example\"\n");

  assert!(refusal.contains("the top level"), "{refusal}");
  assert!(refusal.contains("testbed"), "{refusal}");
}

// "a declaration of another major is refused, naming both"
#[test]
fn a_declaration_of_another_major_is_refused_naming_both_majors() {
  let other = own_major() + 7;
  let refusal = refused(&format!("testbed = {other}\nkey_of_another_schema = true\n"));

  assert!(refusal.contains(&format!("major {other}")), "{refusal}");
  assert!(refusal.contains(&format!("major {}", own_major())), "{refusal}");
}

// "a language the testbed has no member for is refused, naming the ones there are"
#[test]
fn a_language_the_testbed_has_no_member_for_is_refused_naming_those_there_are() {
  let refusal = refused(&declared("[test]\nlanguages = [\"cargo\", \"cobol\"]\n"));

  assert!(refusal.contains("test.languages"), "{refusal}");
  assert!(refusal.contains("cobol"), "{refusal}");
  assert!(refusal.contains("browser, bun, cargo, node"), "{refusal}");
}

// "a quick check the testbed has no member for is refused"
#[test]
fn a_quick_check_the_testbed_has_no_member_for_is_refused() {
  let refusal = refused(&declared("[check]\nquick = [\"spelling\"]\n"));

  assert!(refusal.contains("check.quick"), "{refusal}");
  assert!(refusal.contains("spelling"), "{refusal}");
}

// "a quick check without the input it reads is refused, naming the input"
#[test]
fn a_quick_check_without_the_input_it_reads_is_refused_naming_the_input() {
  let refusal = refused(&declared("[check]\nquick = [\"wording\"]\n"));
  assert!(refusal.contains("check.quick"), "{refusal}");
  assert!(refusal.contains("wording_document"), "{refusal}");

  let refusal = refused(&declared("[check]\nquick = [\"dashes\", \"language\"]\n"));
  assert!(refusal.contains("language_ledger"), "{refusal}");

  assert!(parse(&declared("[check]\nquick = [\"language\"]\nlanguage_ledger = \"ledger.json\"\n")).is_ok());
  assert!(parse(&declared("[check]\nquick = [\"dashes\", \"layout\", \"dependencies\"]\n")).is_ok());
}

// "a deploy root without its data root is refused"
#[test]
fn a_deploy_root_without_its_data_root_is_refused() {
  let refusal = refused(&declared("[deploy]\nroot = \"~/deploys\"\n"));

  assert!(refusal.contains("deploy.root"), "{refusal}");
  assert!(refusal.contains("data root"), "{refusal}");
}

// "the migrations and the migrate command are named together or not at all"
#[test]
fn the_migrations_and_the_migrate_command_are_named_together_or_not_at_all() {
  for half in ["migrations = [\"migrations/**\"]\n", "migrate = \"migrate\"\n"] {
    let refusal = refused(&declared(&format!("[test.database]\n{half}")));
    assert!(refusal.contains("test.database"), "{refusal}");
  }

  let server_alone = parse(&declared("[test.database]\nmax_files = 2\n")).unwrap();
  let database = server_alone.test.database.unwrap();
  assert!(database.migrations.is_empty() && database.migrate.is_none());
  assert_eq!(database.max_files, 2);
}

// "two steps of one table under the same id are refused, naming the table and the id"
#[test]
fn two_steps_of_one_table_under_the_same_id_are_refused_naming_the_table_and_the_id() {
  let step =
    |table: &str, id: &str, extra: &str| format!("[[{table}]]\nid = \"{id}\"\ncommand = \"true\"\n{extra}");

  for (table, extra) in
    [("check.step", ""), ("build.step", ""), ("deploy.step", "phase = \"before-flip\"\n")]
  {
    let text = declared(&format!("{}{}", step(table, "same", extra), step(table, "same", extra)));
    let refusal = refused(&text);
    assert!(refusal.contains(table), "{refusal}");
    assert!(refusal.contains("same"), "{refusal}");
  }

  let across = declared(&format!("{}{}", step("check.step", "same", ""), step("build.step", "same", "")));
  assert!(parse(&across).is_ok());
}

// "a deploy step names its phase"
#[test]
fn a_deploy_step_without_its_phase_is_refused() {
  let refusal = refused(&declared("[[deploy.step]]\nid = \"a\"\ncommand = \"true\"\n"));

  assert!(refusal.contains("deploy.step"), "{refusal}");
  assert!(refusal.contains("phase"), "{refusal}");
}

// "paths are relative to the root unless they begin with ~/ or /"
#[test]
fn a_path_stands_under_the_root_under_home_or_as_written() {
  let repository = Repository {
    root: PathBuf::from("/checkout"),
    declaration: Declaration::default(),
  };
  let env: Env = [("HOME".to_owned(), "/test-home".to_owned())].into_iter().collect();

  assert_eq!(
    repository.path(Path::new("docs/ledger.json"), &env),
    PathBuf::from("/checkout/docs/ledger.json")
  );
  assert_eq!(repository.path(Path::new("~/deploys"), &env), PathBuf::from("/test-home/deploys"));
  assert_eq!(repository.path(Path::new("/srv/data"), &env), PathBuf::from("/srv/data"));
  assert_eq!(repository.path(Path::new("~name/x"), &env), PathBuf::from("/checkout/~name/x"));
}

// "this repository's own declaration reads"
#[test]
fn this_repositorys_own_declaration_reads() {
  let here = Path::new(env!("CARGO_MANIFEST_DIR"));

  let repository = Repository::read(here).unwrap();
  let declaration = &repository.declaration;

  assert_eq!(canonical(&repository.root), canonical(&here.join("..")));
  assert_eq!(repository.name(), "bmo-testbed");
  assert_eq!(
    declaration.test.languages().iter().map(|language| language.name()).collect::<Vec<_>>(),
    ["cargo", "bun"]
  );
  assert!(declaration.test.database.is_some());
  assert_eq!(
    declaration.check.quick,
    [
      QuickCheck::Dashes,
      QuickCheck::Language,
      QuickCheck::Layout,
      QuickCheck::Dependencies,
      QuickCheck::CargoProfile,
      QuickCheck::SharedVersions
    ]
  );
  assert_eq!(
    declaration.check.shared_versions.as_ref().map(|shared| shared.repository.as_str()),
    Some("bmo-chat")
  );
  assert_eq!(
    declaration.build.binaries.as_deref(),
    Some(&["bmo-testbed".to_owned(), "deploy-tool".to_owned()][..])
  );
  assert!(declaration.deploy.root.is_some());
}

// "the language check and its ledger are read under their earlier names too, and both ledger keys at once
// are refused"
#[test]
fn the_language_check_and_its_ledger_are_read_under_their_earlier_names() {
  let earlier =
    parse(&declared("[check]\nquick = [\"korean\"]\nkorean_ledger = \"ledger.json\"\n")).unwrap();

  assert_eq!(earlier.check.quick, [QuickCheck::Language]);
  assert_eq!(earlier.check.language_ledger.as_deref(), Some(std::path::Path::new("ledger.json")));

  let refusal = refused(&declared(
    "[check]\nquick = [\"language\"]\nlanguage_ledger = \"a.json\"\nkorean_ledger = \"b.json\"\n",
  ));

  assert!(refusal.contains("ledger"), "{refusal}");
}

// "the shared versions name their repository, the files left out and the exceptions, and the check without the table, an
// exception without a reason and one naming nothing are refused"
#[test]
fn the_shared_versions_and_their_exceptions_read_and_what_breaks_them_is_refused() {
  let declaration = parse(&declared(
    "[check]\nquick = [\"shared-versions\"]\n\n[check.shared_versions]\nrepository = \"base\"\nexclude = [\"upstream/**\"]\n\n[check.shared_versions.exceptions.rt]\nreason = \"a dependency asks for more\"\nline = \"rt = \\\"1\\\"\"\nlocked = [\"1.0.1\"]\n",
  ))
  .unwrap();
  let shared = declaration.check.shared_versions.unwrap();
  let exception = &shared.exceptions["rt"];

  assert_eq!(declaration.check.quick, [QuickCheck::SharedVersions]);
  assert_eq!(shared.repository, "base");
  assert_eq!(shared.exclude, ["upstream/**"]);
  assert_eq!(
    (exception.line.as_deref(), exception.locked.as_slice()),
    (Some("rt = \"1\""), &["1.0.1".to_owned()][..])
  );

  let without = refused(&declared("[check]\nquick = [\"shared-versions\"]\n"));
  let no_reason = refused(&declared(
    "[check.shared_versions]\nrepository = \"base\"\n\n[check.shared_versions.exceptions.rt]\nreason = \" \"\nlocked = [\"1.0.1\"]\n",
  ));
  let naming_nothing = refused(&declared(
    "[check.shared_versions]\nrepository = \"base\"\n\n[check.shared_versions.exceptions.rt]\nreason = \"why\"\n",
  ));

  assert!(without.contains("shared_versions.repository"), "{without}");
  assert!(no_reason.contains("the exception for rt gives no reason"), "{no_reason}");
  assert!(naming_nothing.contains("the exception for rt names neither"), "{naming_nothing}");
}

// "how long one test may run before it fails": the declaration policy's 120 seconds where none is declared,
// the declared seconds otherwise, and no seconds refused naming the key
#[test]
fn the_per_test_limit_is_the_declared_one_else_the_policy_s_and_never_no_seconds() {
  let unnamed = parse(&declared("[test]\n")).expect("the declaration reads");
  let named = parse(&declared("[test]\ntest_timeout_seconds = 30\n")).expect("the declaration reads");
  let refusal = refused(&declared("[test]\ntest_timeout_seconds = 0\n"));

  assert_eq!((unnamed.test.per_test_seconds(), named.test.per_test_seconds()), (120, 30));
  assert!(refusal.contains("test.test_timeout_seconds"), "{refusal}");
  assert!(refusal.contains("at least one second"), "{refusal}");
}

// "a silence limit of no seconds" is refused naming the key
#[test]
fn a_silence_limit_of_no_seconds_is_refused_naming_the_key() {
  let refusal = refused(&declared("[test]\nsilence_seconds = 0\n"));

  assert!(refusal.contains("test.silence_seconds"), "{refusal}");
  assert!(refusal.contains("at least one second"), "{refusal}");
}
