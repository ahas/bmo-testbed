use std::collections::HashSet;
use std::path::PathBuf;

use bmo_testbed_core::policy::fill_shown;
use bmo_testbed_core::policy::suite::SUITE;
use bmo_testbed_suite::superseded::{
  BUILT_UNITS_NAME, BuildIdentity, Builds, REPORTED_FORMAT, build_generation, built_units, plan_superseded,
  read_record, recorded_units, remove_superseded, reporting_args, shape_of, unit_hash,
};
use serde_json::{Value, json};

fn now() -> i64 {
  std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn words(given: &[&str]) -> Vec<String> {
  given.iter().map(|one| (*one).to_owned()).collect()
}

// Whether a line is the policy's removal line for `count` units, whatever target and size it names.
fn removed_line(line: &str, count: usize) -> bool {
  let filled = fill_shown(&SUITE.cargo.removed, &[("count", &count)]);
  let pattern = regex::escape(&filled).replace(r"\{target\}", ".+").replace(r"\{megabytes\}", r"\d+");

  regex::Regex::new(&format!("^{pattern}$")).unwrap().is_match(line)
}

// A build directory shaped as Cargo leaves it, and a checkout beside it whose own target directory stands
// under the build directory's `checkouts`.
struct Scene {
  _dir: tempfile::TempDir,
  build_root: PathBuf,
  scratch: PathBuf,
}

impl Scene {
  fn new() -> Self {
    let dir = tempfile::tempdir().unwrap();
    let scratch = dir.path().canonicalize().unwrap();

    Self {
      build_root: scratch.join("cargo-target"),
      scratch,
      _dir: dir,
    }
  }

  fn builds(&self, name: &str) -> Builds {
    let root = self.scratch.join(name);

    std::fs::create_dir_all(&root).unwrap();

    Builds {
      root,
      build_root: self.build_root.clone(),
      target_dir: self.build_root.join("checkouts").join(format!("{name}-000000000000")),
    }
  }

  fn deps(&self) -> PathBuf {
    self.build_root.join("debug").join("deps")
  }

  // A unit's files as Cargo writes them: its fingerprint directory and its outputs under `deps`.
  fn unit(&self, name: &str, hash: &str, outputs: &[String]) {
    let fingerprint = self
      .build_root
      .join("debug/.fingerprint")
      .join(format!("{}-{hash}", name.replace('_', "-")));

    std::fs::create_dir_all(&fingerprint).unwrap();
    std::fs::write(fingerprint.join("lib"), "").unwrap();
    std::fs::create_dir_all(self.deps()).unwrap();

    for output in outputs {
      std::fs::write(self.deps().join(output), "x".repeat(64)).unwrap();
    }
  }

  fn has(&self, output: &str) -> bool {
    self.deps().join(output).exists()
  }

  // One build of a service's tests at `hash`: its library, its one integration test binary, its
  // binary handed back into the checkout's own target directory, and the shared dependency, as Cargo reports
  // them among lines that are not its JSON.
  fn test_build(&self, builds: &Builds, hash: &str) -> String {
    let other = |first: char| format!("{first}{}", &hash[1..]);
    let (test, binary) = (other('a'), other('b'));

    self.unit(
      "service_core",
      hash,
      &[
        format!("libservice_core-{hash}.rlib"),
        format!("libservice_core-{hash}.rmeta"),
        format!("service_core-{hash}.d"),
      ],
    );
    self.unit("integration", &test, &[format!("integration-{test}")]);
    self.unit("service_core", &binary, &[format!("service_core-{binary}")]);
    self.unit("serde", "5e5e5e5e5e5e5e5e", &["libserde-5e5e5e5e5e5e5e5e.rlib".to_owned()]);

    let handed = builds.target_dir.join("debug/service-core");
    let fresh = handed.with_extension("new");

    std::fs::create_dir_all(handed.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&fresh);
    std::fs::hard_link(self.deps().join(format!("service_core-{binary}")), &fresh).unwrap();
    std::fs::rename(&fresh, &handed).unwrap();

    let deps = |file: String| self.deps().join(file).to_string_lossy().into_owned();
    let artifact =
      |krate: &str, kind: &str, name: &str, test: bool, files: Vec<String>, executable: Option<String>| {
        let package = if krate == "serde" {
          "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0".to_owned()
        } else {
          format!("path+file://{}/crates/{krate}#0.1.0", builds.root.display())
        };

        json!({
          "reason": "compiler-artifact",
          "package_id": package,
          "target": { "kind": [kind], "name": name },
          "profile": { "test": test },
          "filenames": files,
          "executable": executable,
        })
        .to_string()
      };
    let handed = handed.to_string_lossy().into_owned();

    [
      "warning: not JSON".to_owned(),
      artifact(
        "serde",
        "lib",
        "serde",
        false,
        vec![deps("libserde-5e5e5e5e5e5e5e5e.rlib".to_owned())],
        None,
      ),
      artifact(
        "service-core",
        "lib",
        "service_core",
        false,
        vec![
          deps(format!("libservice_core-{hash}.rlib")),
          deps(format!("libservice_core-{hash}.rmeta")),
        ],
        None,
      ),
      artifact(
        "service-core",
        "test",
        "integration",
        true,
        vec![deps(format!("integration-{test}"))],
        Some(deps(format!("integration-{test}"))),
      ),
      artifact("service-core", "bin", "service-core", false, vec![handed.clone()], Some(handed)),
      String::new(),
    ]
    .join("\n")
  }
}

fn binary_only(report: &str) -> String {
  report
    .lines()
    .filter(|line| !line.contains("\"integration\""))
    .collect::<Vec<_>>()
    .join("\n")
}

const WRAPPER: &str = "/wrapper/workspace-rustc";

#[test]
fn a_command_is_asked_again_for_what_it_stood_on_building_and_running_nothing_more() {
  let asked = |given: &[&str]| reporting_args(&words(given));

  assert_eq!(
    asked(&["test", "-p", "service-core", "receiver::", "--", "--nocapture"]).unwrap(),
    words(&[
      "test",
      "-p",
      "service-core",
      "receiver::",
      "--no-run",
      REPORTED_FORMAT,
      "--",
      "--nocapture"
    ])
  );
  assert_eq!(
    asked(&["run", "-p", "service-core", "--bin", "service", "--", "note"]).unwrap(),
    words(&["build", "-p", "service-core", "--bin", "service", REPORTED_FORMAT])
  );
  assert_eq!(
    asked(&["clippy", "--workspace", "--", "-D", "warnings"]).unwrap(),
    words(&["clippy", "--workspace", REPORTED_FORMAT, "--", "-D", "warnings"])
  );
  assert_eq!(
    asked(&["test", "-p", "sample-reader", "--no-run", "--message-format", "short"]).unwrap(),
    words(&["test", "-p", "sample-reader", "--no-run", REPORTED_FORMAT])
  );
  assert_eq!(asked(&["b", "--message-format=json"]).unwrap(), words(&["b", REPORTED_FORMAT]));
  assert_eq!(asked(&["fmt", "--all", "--check"]), None);
}

#[test]
fn only_the_checkouts_own_units_are_read_a_handed_back_binary_by_the_unit_holding_its_file() {
  let scene = Scene::new();
  let builds = scene.builds("work");
  let units = built_units(&builds, &scene.test_build(&builds, "1111111111111111"));
  let mut named: Vec<&str> = units.iter().map(|one| one.unit.as_str()).collect();

  named.sort_unstable();

  assert_eq!(named, ["debug:1111111111111111", "debug:a111111111111111", "debug:b111111111111111"]);
  assert_eq!(units.iter().map(|one| &one.place).collect::<HashSet<_>>().len(), 3);
  assert!(!units.iter().any(|one| one.place.contains("serde")));
  assert!(units[0].place.contains("|crates/service-core|"));
}

#[test]
fn a_later_build_of_the_same_shape_removes_what_the_earlier_one_left_and_nothing_a_record_still_names() {
  let scene = Scene::new();
  let builds = scene.builds("work");
  let said = std::cell::RefCell::new(Vec::new());
  let mut say = |line: &str| said.borrow_mut().push(line.to_owned());

  remove_superseded(&builds, WRAPPER, &scene.test_build(&builds, "2222222222222222"), &mut say);

  // Another shape of the same checkout, a build of the binary alone, stands on the library too.
  let binary = binary_only(&scene.test_build(&builds, "2222222222222222"));

  remove_superseded(&builds, WRAPPER, &binary, &mut say);
  remove_superseded(&builds, WRAPPER, &scene.test_build(&builds, "3333333333333333"), &mut say);

  // The earlier test binary goes; the library and the binary the other shape still names stay.
  assert!(!scene.has("integration-a222222222222222"));
  assert!(!scene.build_root.join("debug/.fingerprint/integration-a222222222222222").exists());
  assert!(scene.has("service_core-b222222222222222"));
  assert!(scene.has("libservice_core-2222222222222222.rlib"));
  assert!(scene.has("integration-a333333333333333"));
  // The shared dependency is never removed after a build.
  assert!(scene.has("libserde-5e5e5e5e5e5e5e5e.rlib"));
  assert!(removed_line(said.borrow().last().unwrap(), 1), "{said:?}");

  // Once the other shape moves on too, its library and its binary go.
  let moved = binary
    .replace("2222222222222222", "3333333333333333")
    .replace("b222222222222222", "b333333333333333");

  remove_superseded(&builds, WRAPPER, &moved, &mut say);

  assert!(!scene.has("libservice_core-2222222222222222.rlib"));
  assert!(!scene.has("service_core-b222222222222222"));
  assert!(scene.has("libservice_core-3333333333333333.rlib"));
  assert!(removed_line(said.borrow().last().unwrap(), 2), "{said:?}");
}

#[test]
fn another_checkouts_record_holds_what_it_names_and_a_lint_keeps_a_shape_of_its_own() {
  let scene = Scene::new();
  let (mine, other) = (scene.builds("work"), scene.builds("other"));

  remove_superseded(&mine, WRAPPER, &scene.test_build(&mine, "3333333333333333"), &mut |_| ());

  let units = built_units(&other, &scene.test_build(&other, "4444444444444444"));

  assert_ne!(shape_of(WRAPPER, &units), shape_of("/wrapper/clippy-driver", &units));

  std::fs::write(
    other.target_dir.join(BUILT_UNITS_NAME),
    json!({ "old": { "at": now(), "units": ["debug:3333333333333333"] } }).to_string(),
  )
  .unwrap();

  let build = BuildIdentity {
    shape: shape_of(WRAPPER, &units),
    wrapper: WRAPPER.to_owned(),
    generation: build_generation(&other.root).unwrap(),
  };
  let record = read_record(&mine.record_path()).record;
  let (shape, entry) = record
    .iter()
    .find(|(_, entry)| entry.places.values().any(|unit| unit == "debug:3333333333333333"))
    .unwrap();
  let again = BuildIdentity {
    shape: shape.clone(),
    wrapper: WRAPPER.to_owned(),
    generation: entry.generation.clone(),
  };

  assert!(
    !plan_superseded(&mine, &again, &[], now())
      .gone
      .contains(&"debug:3333333333333333".to_owned())
  );
  // A record of the earlier form is superseded whole by its first build since, but for what another
  // checkout's record still names.
  assert!(plan_superseded(&other, &build, &units, now()).gone.is_empty());
}

#[test]
fn a_new_generation_removes_what_another_shape_built_at_the_same_place_and_the_same_generation_keeps_it() {
  let scene = Scene::new();
  let third = scene.builds("third");
  let mut say = |_: &str| ();

  std::fs::write(third.root.join("Cargo.lock"), "one\n").unwrap();
  remove_superseded(&third, WRAPPER, &binary_only(&scene.test_build(&third, "6666666666666666")), &mut say);
  remove_superseded(&third, WRAPPER, &scene.test_build(&third, "6666666666666666"), &mut say);

  // A test build of the same generation under other features fills the library's place with another unit;
  // the binary's build keeps its own.
  remove_superseded(&third, WRAPPER, &scene.test_build(&third, "7777777777777777"), &mut say);

  assert!(scene.has("libservice_core-6666666666666666.rlib"));

  // The lockfile moves: the next test build's library replaces the one the binary's build named.
  std::fs::write(third.root.join("Cargo.lock"), "two\n").unwrap();
  remove_superseded(&third, WRAPPER, &scene.test_build(&third, "8888888888888888"), &mut say);

  assert!(!scene.has("libservice_core-6666666666666666.rlib"));
  assert!(!scene.has("service_core-b666666666666666"));
  assert!(!scene.has("libservice_core-7777777777777777.rlib"));
  assert!(scene.has("libservice_core-8888888888888888.rlib"));

  let mut units = recorded_units(&read_record(&third.record_path()).record);

  units.sort();

  assert_eq!(units, ["debug:8888888888888888", "debug:a888888888888888", "debug:b888888888888888"]);

  // Every checkout's removal reads every record, an earlier version's too, which knows each entry's units by
  // their list alone.
  let written: Value =
    serde_json::from_str(&std::fs::read_to_string(third.record_path()).unwrap()).unwrap();
  let mut listed: Vec<String> = written
    .as_object()
    .unwrap()
    .values()
    .flat_map(|entry| {
      entry["units"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
    })
    .collect();

  listed.sort();

  assert_eq!(listed, ["debug:8888888888888888", "debug:a888888888888888", "debug:b888888888888888"]);
}

#[test]
fn reads_a_units_hash_off_its_file_name() {
  assert_eq!(unit_hash("libserde-5e5e5e5e5e5e5e5e.rlib"), Some("5e5e5e5e5e5e5e5e"));
  assert_eq!(unit_hash("integration-a111111111111111"), Some("a111111111111111"));
  assert_eq!(unit_hash("service-core"), None);
}
