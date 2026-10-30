//! The tool's manifest and input schema held to the executable (bmo-pocket's `docs/api.md`, 「The manifest」;
//! docs/deploy.md, 「The hosted deploy tool」): the tool named for its folder, its executable the crate's own
//! binary, its one entry the one the executable answers, and its schema the arguments it reads.

use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::deploy_tool::DEPLOY_TOOL;
use deploy_tool::read_arguments;
use serde_json::{Value, json};

fn folder() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn toml_of(path: &Path) -> toml::Value {
  toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn text<'a>(value: &'a toml::Value, path: &[&str]) -> &'a str {
  path.iter().fold(value, |at, key| &at[*key]).as_str().unwrap()
}

fn words(value: &toml::Value) -> Vec<&str> {
  value.as_array().unwrap().iter().map(|word| word.as_str().unwrap()).collect()
}

fn schema() -> Value {
  let manifest = toml_of(&folder().join("tool.toml"));
  let input = text(&manifest["entry"][0], &["input"]);

  serde_json::from_str(&std::fs::read_to_string(folder().join(input)).unwrap()).unwrap()
}

// "the manifest names the tool for its folder, the crate's binary and an hour's timeout"
#[test]
fn the_manifest_names_the_tool_for_its_folder_and_runs_the_crates_binary() {
  let manifest = toml_of(&folder().join("tool.toml"));
  let crate_manifest = toml_of(&folder().join("Cargo.toml"));
  let package = text(&crate_manifest, &["package", "name"]);
  let binary = text(&crate_manifest["bin"][0], &["name"]);

  assert_eq!(text(&manifest, &["tool", "name"]), folder().file_name().unwrap().to_str().unwrap());
  assert_eq!(manifest["tool"]["protocol"].as_integer(), Some(i64::from(DEPLOY_TOOL.protocol)));
  assert_eq!(text(&manifest, &["tool", "language"]), "rust");
  assert_eq!(words(&manifest["tool"]["run"]), [binary]);
  assert_eq!(manifest["tool"]["timeout"].as_integer(), Some(3600));
  assert!(text(&manifest, &["tool", "build"]).contains(&format!("-p {package} ")));
  assert!(text(&manifest, &["tool", "build"]).contains(&format!("/release/{binary}\"")));
}

// "its one entry is the one the executable answers, asking for the capability deploy"
#[test]
fn its_one_entry_is_the_one_the_executable_answers() {
  let manifest = toml_of(&folder().join("tool.toml"));
  let entries = manifest["entry"].as_array().unwrap();
  let entry = &entries[0];

  assert_eq!(entries.len(), 1);
  assert_eq!(format!("{}/{}", text(entry, &["namespace"]), text(entry, &["name"])), DEPLOY_TOOL.entry);
  assert_eq!(words(&entry["capabilities"]), ["deploy"]);
  assert!(entry.get("grant").is_none());
  assert_eq!(entry["hints"]["read_only"].as_bool(), Some(false));
  assert!(!text(entry, &["description"]).is_empty());
  assert!(manifest.get("resources").is_none());
}

// "the deployment places the binary, the manifest and the schema, allowing the capability the entry asks
// for"
#[test]
fn the_deployment_places_what_the_manifest_names() {
  let manifest = toml_of(&folder().join("tool.toml"));
  let crate_manifest = toml_of(&folder().join("Cargo.toml"));
  let binary = text(&crate_manifest["bin"][0], &["name"]);
  let input = text(&manifest["entry"][0], &["input"]);
  let declaration = toml_of(&folder().join("../../testbed.toml"));
  let steps = declaration["deploy"]["step"].as_array().unwrap();
  let step = steps
    .iter()
    .find(|step| text(step, &["command"]).contains("tools/deploy/.placed"))
    .expect("a step places the tool");
  let command = text(step, &["command"]);

  assert_eq!(text(step, &["phase"]), "before-flip");
  assert!(words(&declaration["build"]["binaries"]).contains(&binary));
  assert!(command.contains(&format!("bin/{binary} ")), "{command}");
  assert!(command.contains("tools/deploy/tool.toml "), "{command}");
  assert!(command.contains(&format!("tools/deploy/{input} ")), "{command}");

  for capability in words(&manifest["entry"][0]["capabilities"]) {
    assert!(command.contains(&format!("--allow {capability}")), "{command}");
  }
}

// "the schema states the arguments the executable reads, and no other"
#[test]
fn the_schema_states_the_arguments_the_executable_reads() {
  let schema = schema();
  let mut properties: Vec<&str> =
    schema["properties"].as_object().unwrap().keys().map(String::as_str).collect();

  properties.sort();

  assert_eq!(schema["type"], "object");
  assert_eq!(schema["additionalProperties"], false);
  assert_eq!(properties, ["commit", "dry_run", "repository", "why"]);
  assert_eq!(schema["required"], json!(["repository", "commit", "why"]));
  assert_eq!(
    schema["properties"]["commit"]["pattern"],
    format!(
      "^[0-9a-fA-F]{{{},{}}}$",
      DEPLOY_TOOL.numbers.commit_shortest, DEPLOY_TOOL.numbers.commit_longest
    )
  );

  let whole =
    json!({ "repository": "agency", "commit": "abc1234", "why": "Ship the fix", "dry_run": true });

  assert_eq!(read_arguments(&whole).unwrap().dry_run, Some(true));

  for required in ["repository", "commit", "why"] {
    let mut without = whole.clone();

    without.as_object_mut().unwrap().remove(required);
    assert!(read_arguments(&without).is_err(), "{required}");
  }

  let mut more = whole.clone();

  more["extra"] = json!(1);
  assert!(read_arguments(&more).unwrap_err().contains("extra"));
}
