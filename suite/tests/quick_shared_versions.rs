//! The shared versions check (docs/checks.md, 「The quick checks」): a listed crate's entries are the shared
//! line byte for byte or take the workspace's adding nothing but `optional`, a lock's registry packages on a
//! snapshot's line stand at its version, an exception stands in for its crate, a stale one is named, a file
//! an exclude glob reaches is left unread, the shared versions are found through the machine's registry as
//! the shared repository's main branch holds them, and the report holds every checkout to them.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use bmo_testbed_core::declaration::SharedException;
use bmo_testbed_core::turn::Env;
use bmo_testbed_suite::quick::shared_versions::{
  Shared, checkout_shared_versions, find_shared, hold, read_shared, shared_versions_report,
};

use crate::quick::{checkout, repository, write};

const SHARED: &str = "[dependencies]\nrt = { version = \"1.4.0\", features = [\"net\", \"time\"] }\nweb = { version = \"0.8.0\", features = [\"json\"] }\n\n[snapshot]\nrt = \"1.4.0\"\nsys = \"0.2.9\"\nrand = [\"0.8.5\", \"0.9.2\"]\n";

const RT: &str = "rt = { version = \"1.4.0\", features = [\"net\", \"time\"] }";

fn shared() -> Shared {
  read_shared(SHARED).unwrap()
}

fn lock(packages: &[(&str, &str)]) -> String {
  packages
    .iter()
    .map(|(name, version)| {
      format!(
        "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n"
      )
    })
    .collect::<String>()
}

fn except(reason: &str, line: Option<&str>, locked: &[&str]) -> SharedException {
  SharedException {
    reason: reason.to_owned(),
    line: line.map(str::to_owned),
    locked: locked.iter().map(|one| (*one).to_owned()).collect(),
  }
}

fn said(held: &bmo_testbed_suite::quick::shared_versions::Held) -> Vec<String> {
  held.divergences.iter().map(|one| one.said()).collect()
}

// "the shared versions read the line after each key as written, and a snapshot of one version or a list"
#[test]
fn the_shared_versions_read_each_line_as_written_and_a_snapshot_of_one_or_a_list() {
  let shared = shared();

  assert_eq!(shared.lines["rt"], "{ version = \"1.4.0\", features = [\"net\", \"time\"] }");
  assert_eq!(shared.snapshot["sys"].len(), 1);
  assert_eq!(shared.snapshot["rand"].len(), 2);
  assert!(read_shared("[snapshot]\nsys = \"not a version\"\n").is_err());
}

// "a workspace writing the shared line and crates taking it pass, every entry and locked package counted"
#[test]
fn a_workspace_writing_the_shared_line_and_crates_taking_it_pass() {
  let dir = checkout(&[
    (
      "Cargo.toml",
      &format!(
        "[workspace]\nmembers = [\"a\"]\n\n[workspace.dependencies]\n{RT}  # the runtime\nother = \"1\"\n"
      ),
    ),
    (
      "a/Cargo.toml",
      "[package]\nname = \"a\"\n\n[dependencies]\nrt.workspace = true\n\n[dev-dependencies]\nrt = { workspace = true, optional = true }\n",
    ),
    (
      "Cargo.lock",
      &lock(&[("rt", "1.4.0"), ("sys", "0.2.9"), ("rand", "0.8.5"), ("rand", "0.7.3")]),
    ),
  ]);
  let held = hold(dir.path(), &shared(), &BTreeMap::new(), &[]);

  assert_eq!(said(&held), Vec::<String>::new());
  assert_eq!(held.entries, 6);
}

// "an entry other than the shared line, or taking the workspace's and adding to it, is named with the
// line to write"
#[test]
fn an_entry_other_than_the_line_or_adding_to_the_workspace_is_named_with_the_line_to_write() {
  let dir = checkout(&[
    (
      "Cargo.toml",
      "[workspace]\nmembers = [\"a\"]\n\n[workspace.dependencies]\nrt = { version = \"1.4.0\", features = [\"time\", \"net\"] }\n",
    ),
    (
      "a/Cargo.toml",
      "[package]\nname = \"a\"\n\n[dependencies]\nrt = { workspace = true, features = [\"fs\"] }\nweb = { workspace = true, features = [\"ws\"], optional = true }\n\n[target.'cfg(unix)'.dev-dependencies]\nnet = { package = \"rt\", version = \"1.4.0\", features = [\"net\", \"time\"] }\n",
    ),
  ]);
  let held = hold(dir.path(), &shared(), &BTreeMap::new(), &[]);

  assert_eq!(
    said(&held),
    [
      "Cargo.toml: [workspace.dependencies] rt = { version = \"1.4.0\", features = [\"time\", \"net\"] }; write: rt = { version = \"1.4.0\", features = [\"net\", \"time\"] }",
      "a/Cargo.toml: [dependencies] rt = { workspace = true, features = [\"fs\"] } adds to the workspace's line; write: rt.workspace = true",
      "a/Cargo.toml: [dependencies] web = { workspace = true, features = [\"ws\"], optional = true } adds to the workspace's line; write: web = { workspace = true, optional = true }",
      "a/Cargo.toml: [target.cfg(unix).dev-dependencies] net = { package = \"rt\", version = \"1.4.0\", features = [\"net\", \"time\"] }; write: rt = { version = \"1.4.0\", features = [\"net\", \"time\"] }",
    ]
  );
}

// "a locked package on the snapshot's line at another version is named with the update that moves it, and
// one on another line or from no registry passes"
#[test]
fn a_locked_package_at_another_version_of_the_line_is_named_with_the_update_that_moves_it() {
  let path_crate = "[[package]]\nname = \"sys\"\nversion = \"0.2.1\"\n\n";
  let dir = checkout(&[(
    "Cargo.lock",
    &format!(
      "version = 4\n\n{}{path_crate}",
      lock(&[("sys", "0.2.8"), ("sys", "0.3.0"), ("rand", "0.9.1")])
    ),
  )]);
  let held = hold(dir.path(), &shared(), &BTreeMap::new(), &[]);

  assert_eq!(
    said(&held),
    [
      "Cargo.lock: sys 0.2.8 where the snapshot holds 0.2.9; run: bmo-testbed cargo update -p sys@0.2.8 --precise 0.2.9",
      "Cargo.lock: rand 0.9.1 where the snapshot holds 0.9.2; run: bmo-testbed cargo update -p rand@0.9.1 --precise 0.9.2",
    ]
  );
  assert_eq!(held.entries, 2);
}

// "an exception stands in for its crate's line and lets the lock hold its versions"
#[test]
fn an_exception_stands_in_for_its_crates_line_and_lets_the_lock_hold_its_versions() {
  let line = "rt = { version = \"1.4.0\", features = [\"full\"] }";
  let dir = checkout(&[
    ("Cargo.toml", &format!("[package]\nname = \"one\"\n\n[dependencies]\n{line}\n")),
    ("Cargo.lock", &lock(&[("sys", "0.2.8")])),
  ]);
  let exceptions = BTreeMap::from([
    ("rt".to_owned(), except("a dependency asks for every feature", Some(line), &[])),
    ("sys".to_owned(), except("a dependency pins it", None, &["0.2.8"])),
  ]);

  assert_eq!(said(&hold(dir.path(), &shared(), &exceptions, &[])), Vec::<String>::new());

  let dir = checkout(&[("Cargo.toml", &format!("[package]\nname = \"one\"\n\n[dependencies]\n{RT}\n"))]);

  assert_eq!(
    said(&hold(dir.path(), &shared(), &exceptions, &[])),
    [
      "Cargo.toml: [dependencies] rt = { version = \"1.4.0\", features = [\"net\", \"time\"] }; write: rt = { version = \"1.4.0\", features = [\"full\"] }",
      "the exception for sys names 0.2.8, which no lock holds",
    ]
  );
}

// "an exception that does not stand is named"
#[test]
fn an_exception_that_does_not_stand_is_named() {
  let dir = checkout(&[("README.md", "nothing built\n")]);
  let exceptions = BTreeMap::from([
    ("rt".to_owned(), except("why", Some(RT), &[])),
    ("web".to_owned(), except("why", Some("other = \"1\""), &[])),
    ("unlisted".to_owned(), except("why", None, &["1.0.0"])),
  ]);

  assert_eq!(
    said(&hold(dir.path(), &shared(), &exceptions, &[])),
    [
      "the exception for rt writes the shared line itself",
      "the exception for unlisted is for a crate the shared versions list no line or version of",
      "the exception for web does not read as one line for web: other = \"1\"",
      "the exception for unlisted names 1.0.0, which no lock holds",
    ]
  );
}

// "a fork's upstream manifests, locks and package manifests an exclude glob reaches are left unread, and
// its own files stay held"
#[test]
fn the_files_an_exclude_glob_reaches_are_left_unread_and_the_rest_held() {
  let dir = checkout(&[
    (
      "Cargo.toml",
      &format!("[package]\nname = \"own\"\n\n[dependencies]\n{RT}\nweb = \"0.8\"\n"),
    ),
    (
      "upstream/native/Cargo.toml",
      "[package]\nname = \"native\"\n\n[dependencies]\nrt = { version = \"1\", features = [\"full\"] }\n",
    ),
    ("upstream/native/Cargo.lock", &lock(&[("sys", "0.2.8")])),
    ("upstream/web/package.json", "{\"dependencies\": {\"pkg\": \"^1.0.0\"}}"),
  ]);
  let exclude = ["upstream/**".to_owned()];
  let held = hold(dir.path(), &read_shared(&pinning()).unwrap(), &BTreeMap::new(), &exclude);

  assert_eq!(
    said(&held),
    [
      "Cargo.toml: [dependencies] web = \"0.8\"; write: web = { version = \"0.8.0\", features = [\"json\"] }"
    ]
  );
  assert_eq!(held.entries, 2);
}

// A git command run in a scratch checkout, git's own settings kept out and a neutral author named.
fn git(root: &Path, args: &[&str]) {
  let done = Command::new("git")
    .args(["-c", "user.name=tester", "-c", "user.email=tester@example.com"])
    .args(args)
    .current_dir(root)
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_SYSTEM", "/dev/null")
    .stdin(Stdio::null())
    .output()
    .unwrap();

  assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
}

// Every file of a scratch checkout committed on the branch it stands on.
fn commit(root: &Path) {
  git(root, &["add", "-A"]);
  git(root, &["commit", "-q", "-m", "shared versions"]);
}

// A scratch checkout of the shared repository whose main branch holds the files given.
fn shared_base(files: &[(&str, &str)]) -> tempfile::TempDir {
  let dir = checkout(files);

  git(dir.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
  commit(dir.path());
  dir
}

// A registry file under a scratch data root listing the checkouts given, and the environment naming it.
fn registry_env(data: &std::path::Path, checkouts: &[&std::path::Path]) -> Env {
  let listed: Vec<String> = checkouts.iter().map(|path| format!("{:?}", path.to_string_lossy())).collect();

  std::fs::create_dir_all(data.join("testbed")).unwrap();
  std::fs::write(data.join("testbed/checkouts.toml"), format!("checkouts = [{}]\n", listed.join(", ")))
    .unwrap();

  Env::from([
    ("HOME".to_owned(), "/nonexistent".to_owned()),
    ("BMO_DATA_DIR".to_owned(), data.to_string_lossy().into_owned()),
  ])
}

const DECLARING: &str = "testbed = 0\n\n[check]\nquick = [\"shared-versions\"]\n\n[check.shared_versions]\nrepository = \"shared-base\"\n";

// "the shared versions are read from the checkout the registry lists under the declared name, and a name
// no checkout or two declare is the one fault"
#[test]
fn the_shared_versions_are_found_through_the_registry_under_the_declared_name() {
  let data = tempfile::tempdir().unwrap();
  let base = shared_base(&[
    ("testbed.toml", "testbed = 0\nname = \"shared-base\"\n"),
    ("shared-versions.toml", SHARED),
  ]);
  let unit = checkout(&[
    ("testbed.toml", DECLARING),
    ("Cargo.toml", "[package]\nname = \"one\"\n\n[dependencies]\nrt = \"1.4.0\"\n"),
  ]);
  let found =
    checkout_shared_versions(&repository(&unit), &registry_env(data.path(), &[base.path(), unit.path()]));

  assert_eq!(found.paths, 1);
  assert_eq!(
    found.findings,
    [
      "Cargo.toml: [dependencies] rt = \"1.4.0\"; write: rt = { version = \"1.4.0\", features = [\"net\", \"time\"] }"
    ]
  );

  let none = checkout_shared_versions(&repository(&unit), &registry_env(data.path(), &[unit.path()]));

  assert_eq!(none.findings.len(), 1);
  assert!(none.findings[0].contains("declares shared-base"), "{:?}", none.findings);

  let twin = checkout(&[("testbed.toml", "testbed = 0\nname = \"shared-base\"\n")]);
  let two =
    checkout_shared_versions(&repository(&unit), &registry_env(data.path(), &[base.path(), twin.path()]));

  assert!(two.findings[0].starts_with("more than one checkout"), "{:?}", two.findings);
}

// "the report holds every checkout of the registry, each with its own exceptions and files left out, and
// counts the ones that differ"
#[test]
fn the_report_holds_every_checkout_with_its_own_exceptions_and_exclusions_and_counts_those_that_differ() {
  let data = tempfile::tempdir().unwrap();
  let base = shared_base(&[
    ("testbed.toml", "testbed = 0\nname = \"shared-base\"\n"),
    ("shared-versions.toml", SHARED),
  ]);
  let asking = checkout(&[("testbed.toml", DECLARING)]);
  let excepted = checkout(&[
    (
      "testbed.toml",
      "testbed = 0\nname = \"excepted\"\n\n[check.shared_versions]\nrepository = \"shared-base\"\nexclude = [\"upstream/**\"]\n\n[check.shared_versions.exceptions.sys]\nreason = \"pinned below\"\nlocked = [\"0.2.8\"]\n",
    ),
    ("Cargo.lock", &lock(&[("sys", "0.2.8")])),
    ("upstream/Cargo.toml", "[package]\nname = \"upstream\"\n\n[dependencies]\nrt = \"1\"\n"),
  ]);
  let differing = checkout(&[
    ("testbed.toml", "testbed = 0\nname = \"differing\"\n"),
    ("Cargo.toml", "[package]\nname = \"two\"\n\n[dependencies]\nweb = \"0.8\"\nrt = \"1\"\n"),
    ("Cargo.lock", &lock(&[("sys", "0.2.8")])),
  ]);
  let env = registry_env(data.path(), &[base.path(), excepted.path(), differing.path()]);
  let report = shared_versions_report(&repository(&asking), &env).unwrap();

  assert_eq!((report.differs, report.all), (1, 3));
  assert_eq!(
    report.rows,
    [
      "shared-base: holds to the shared versions (0 entries)",
      "excepted: holds to the shared versions (1 entries)",
      "differing: 3 to fix: lines web, rt; lock sys 0.2.8\u{2192}0.2.9",
    ]
  );
}

// The bytes of a pinned package's release, and the shared versions pinning it beside the crates.
const RELEASE: &str = "the packed release";

fn sha256_of(text: &str) -> String {
  use sha2::{Digest, Sha256};

  Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn pinning() -> String {
  format!(
    "{SHARED}\n[packages.pkg]\nversion = \"1.0.0\"\nfile = \"pkg-1.0.0.tgz\"\nsha256 = \"{}\"\n",
    sha256_of(RELEASE)
  )
}

// "the shared versions read each pinned package's version, file and sha256, and refuse a pin missing one"
#[test]
fn the_shared_versions_read_each_pinned_package_and_refuse_a_pin_missing_a_key() {
  let shared = read_shared(&pinning()).unwrap();

  assert_eq!(shared.packages["pkg"].version, "1.0.0");
  assert_eq!(shared.packages["pkg"].file, "pkg-1.0.0.tgz");
  assert_eq!(shared.packages["pkg"].sha256, sha256_of(RELEASE));
  assert!(read_shared("[packages.pkg]\nversion = \"1.0.0\"\nfile = \"pkg-1.0.0.tgz\"\n").is_err());
}

// "a package manifest naming the pinned file with the release's bytes and the pinned version passes, and an
// installed package's manifest is not read"
#[test]
fn a_manifest_naming_the_pinned_release_passes_and_installed_packages_are_not_read() {
  let dir = checkout(&[
    (
      "client/package.json",
      "{\"devDependencies\": {\"pkg\": \"file:vendor/pkg-1.0.0.tgz\"}, \"peerDependencies\": {\"pkg\": \"1.0.0\"}, \"dependencies\": {\"other\": \"2.0.0\"}}",
    ),
    ("client/vendor/pkg-1.0.0.tgz", RELEASE),
    ("client/node_modules/pkg/package.json", "{\"dependencies\": {\"pkg\": \"0.1.0\"}}"),
  ]);
  let held = hold(dir.path(), &read_shared(&pinning()).unwrap(), &BTreeMap::new(), &[]);

  assert_eq!(said(&held), Vec::<String>::new());
  assert_eq!(held.entries, 2);
}

// "each way an entry differs from the pinned release is named with the manifest, the list and what to write"
#[test]
fn each_way_an_entry_differs_from_the_pinned_release_is_named_with_what_to_write() {
  let dir = checkout(&[
    (
      "client/package.json",
      "{\"dependencies\": {\"pkg\": \"^1.0.0\"}, \"devDependencies\": {\"pkg\": \"file:vendor/pkg-0.9.0.tgz\"}, \"optionalDependencies\": {\"pkg\": \"file:gone/pkg-1.0.0.tgz\"}, \"overrides\": {\"pkg\": \"file:other/pkg-1.0.0.tgz\"}, \"resolutions\": {\"pkg\": {\".\": \"1.0.0\"}}}",
    ),
    ("client/vendor/pkg-0.9.0.tgz", RELEASE),
    ("client/other/pkg-1.0.0.tgz", "a build packed elsewhere"),
    ("broken/package.json", "[]"),
  ]);
  let held = hold(dir.path(), &read_shared(&pinning()).unwrap(), &BTreeMap::new(), &[]);
  let lines = said(&held);
  let pin = sha256_of(RELEASE);
  let other = sha256_of("a build packed elsewhere");

  assert_eq!(held.entries, 5);
  assert_eq!(
    lines,
    [
      "broken/package.json does not read as a package manifest: it is no JSON object".to_owned(),
      "client/package.json: dependencies pkg = ^1.0.0; write 1.0.0, or file: naming the release pkg-1.0.0.tgz"
        .to_owned(),
      "client/package.json: devDependencies pkg = file:vendor/pkg-0.9.0.tgz; vendor the release pkg-1.0.0.tgz as it stands and name it, the version 1.0.0".to_owned(),
      "client/package.json: optionalDependencies pkg = file:gone/pkg-1.0.0.tgz; client/gone/pkg-1.0.0.tgz does not stand; vendor the release pkg-1.0.0.tgz there as it stands".to_owned(),
      format!(
        "client/package.json: overrides pkg = file:other/pkg-1.0.0.tgz; client/other/pkg-1.0.0.tgz hashes to {other} where the release pkg-1.0.0.tgz hashes to {pin}; vendor the release as it stands"
      ),
      "client/package.json: resolutions pkg = {\".\":\"1.0.0\"}; write 1.0.0, or file: naming the release pkg-1.0.0.tgz"
        .to_owned(),
    ]
  );
}

// "the report names, beside the pin, what each checkout writes for a pinned package, clean or not"
#[test]
fn the_report_names_what_each_checkout_writes_for_a_pinned_package_beside_the_pin() {
  let data = tempfile::tempdir().unwrap();
  let base = shared_base(&[
    ("testbed.toml", "testbed = 0\nname = \"shared-base\"\n"),
    ("shared-versions.toml", &pinning()),
  ]);
  let asking = checkout(&[("testbed.toml", DECLARING)]);
  let kept = checkout(&[
    ("testbed.toml", "testbed = 0\nname = \"kept\"\n"),
    ("package.json", "{\"devDependencies\": {\"pkg\": \"file:vendor/pkg-1.0.0.tgz\"}}"),
    ("vendor/pkg-1.0.0.tgz", RELEASE),
  ]);
  let behind = checkout(&[
    ("testbed.toml", "testbed = 0\nname = \"behind\"\n"),
    (
      "package.json",
      "{\"devDependencies\": {\"pkg\": \"file:vendor/pkg-1.0.0.tgz\"}, \"peerDependencies\": {\"pkg\": \"0.9.0\"}}",
    ),
    ("vendor/pkg-1.0.0.tgz", "a build packed elsewhere"),
  ]);
  let env = registry_env(data.path(), &[base.path(), kept.path(), behind.path()]);
  let report = shared_versions_report(&repository(&asking), &env).unwrap();

  assert_eq!((report.differs, report.all), (1, 3));
  assert_eq!(
    report.rows,
    [
      "shared-base: holds to the shared versions (0 entries)",
      "kept: holds to the shared versions (1 entries); pkg file:vendor/pkg-1.0.0.tgz beside the pin 1.0.0",
      "behind: 2 to fix: pkg 0.9.0, file:vendor/pkg-1.0.0.tgz (other bytes) beside the pin 1.0.0",
    ]
  );
}

// The shared versions pinning the package at another version than `pinning`'s, as a release under way would.
fn pinning_next(version: &str) -> String {
  format!(
    "{SHARED}\n[packages.pkg]\nversion = \"{version}\"\nfile = \"pkg-{version}.tgz\"\nsha256 = \"{}\"\n",
    sha256_of("a release under way")
  )
}

// "a checkout of the shared repository on another branch, or holding the file changed and not committed,
// leaves the shared versions where its main branch has them; a placed run handed its checkouts reads the
// copy as it stands, and a checkout whose main branch holds no file is the one fault"
#[test]
fn no_branch_the_shared_checkout_stands_on_moves_the_shared_versions_from_its_main() {
  let data = tempfile::tempdir().unwrap();
  let base = shared_base(&[
    ("testbed.toml", "testbed = 0\nname = \"shared-base\"\n"),
    ("shared-versions.toml", &pinning()),
  ]);
  let unit = checkout(&[
    ("testbed.toml", DECLARING),
    ("package.json", "{\"devDependencies\": {\"pkg\": \"file:vendor/pkg-1.0.0.tgz\"}}"),
    ("vendor/pkg-1.0.0.tgz", RELEASE),
  ]);

  git(base.path(), &["checkout", "-q", "-b", "release-next"]);
  write(base.path(), "shared-versions.toml", &pinning_next("1.1.0"));
  commit(base.path());
  write(base.path(), "shared-versions.toml", &pinning_next("1.2.0"));

  let env = registry_env(data.path(), &[base.path(), unit.path()]);
  let held = checkout_shared_versions(&repository(&unit), &env);

  assert_eq!((held.paths, held.findings), (1, Vec::<String>::new()));

  let mut placed = env.clone();

  placed.insert("BMO_TESTBED_PLACED".to_owned(), "1".to_owned());
  placed.insert(
    "BMO_TESTBED_CHECKOUTS".to_owned(),
    data.path().join("testbed/checkouts.toml").to_string_lossy().into_owned(),
  );
  assert_eq!(find_shared("shared-base", &placed).unwrap().packages["pkg"].version, "1.2.0");

  let unborn = checkout(&[
    ("testbed.toml", "testbed = 0\nname = \"shared-base\"\n"),
    ("shared-versions.toml", &pinning()),
  ]);
  let none = checkout_shared_versions(&repository(&unit), &registry_env(data.path(), &[unborn.path()]));

  assert!(none.findings[0].contains("does not read on the branch main"), "{:?}", none.findings);
}
