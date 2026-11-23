//! The presets file (docs/install.md, 「The presets file」).

use bmo_testbed_install::presets::{PresetsFile, Refused};

const FILE: &str = r#"
presets = 1

[[repository]]
name = "base"
folder = "base/base"
url = "https://example.invalid/base.git"

[[repository]]
name = "core"
folder = "services/core"
url = "https://example.invalid/core.git"

[[repository]]
name = "chat-a"
folder = "services/chat-a"
url = "https://example.invalid/chat-a.git"

[[repository]]
name = "chat-b"
folder = "services/chat-b"
url = "https://example.invalid/chat-b.git"

[[repository]]
name = "study"
folder = "services/study"
url = "https://example.invalid/study.git"
features = ["study"]

[preset.small]
summary = "The core and one chat service."
clone = ["base"]
units = ["core", "chat-a", "chat-b"]

[[preset.small.choose]]
what = "the chat service"
units = ["chat-a", "chat-b"]

[preset.middle]
summary = "Small, and the study."
extends = "small"
units = ["study"]
features = ["teams"]

[preset.large]
summary = "Everything."
extends = "middle"
units = ["chat-a", "chat-b"]
"#;

#[test]
fn a_preset_lists_what_it_extends_before_its_own_with_its_choice_settled() {
  let file = PresetsFile::parse(FILE).unwrap();
  let resolved = file.resolve("middle", &["chat-b".to_owned()]).unwrap();

  assert_eq!(resolved.units, ["core", "chat-b", "study"]);
  assert_eq!(resolved.clone, ["base"]);
  assert!(resolved.open.is_empty());
  assert_eq!(resolved.features.iter().cloned().collect::<Vec<_>>(), ["study", "teams"]);
}

#[test]
fn a_choice_left_open_installs_none_of_its_repositories() {
  let file = PresetsFile::parse(FILE).unwrap();
  let resolved = file.resolve("small", &[]).unwrap();

  assert_eq!(resolved.units, ["core"]);
  assert_eq!(resolved.open.len(), 1);
  assert_eq!(resolved.open[0].what, "the chat service");
}

#[test]
fn a_choice_a_later_preset_installs_whole_is_settled() {
  let file = PresetsFile::parse(FILE).unwrap();
  let resolved = file.resolve("large", &[]).unwrap();

  assert!(resolved.open.is_empty());
  assert_eq!(resolved.units, ["core", "chat-a", "chat-b", "study"]);
}

#[test]
fn naming_two_of_one_choice_is_refused() {
  let file = PresetsFile::parse(FILE).unwrap();

  assert!(matches!(
    file.resolve("small", &["chat-a".to_owned(), "chat-b".to_owned()]),
    Err(Refused::ChoiceMany { .. })
  ));
}

#[test]
fn an_unknown_preset_is_refused_naming_the_presets() {
  let file = PresetsFile::parse(FILE).unwrap();

  match file.resolve("huge", &[]) {
    Err(Refused::NoPreset { presets, .. }) => assert_eq!(presets, ["large", "middle", "small"]),
    other => panic!("{other:?}"),
  }
}

#[test]
fn a_preset_naming_a_repository_the_file_does_not_list_is_refused() {
  let text = FILE.replace("units = [\"study\"]", "units = [\"ghost\"]");

  assert_eq!(PresetsFile::parse(&text).unwrap_err(), Refused::NoRepository("ghost".to_owned()));
}

#[test]
fn presets_extending_in_a_ring_are_refused() {
  let text =
    FILE.replace("summary = \"The core and one chat service.\"", "summary = \"x\"\nextends = \"large\"");
  let file = PresetsFile::parse(&text).unwrap();

  assert!(matches!(file.resolve("middle", &[]), Err(Refused::ExtendsRing(_))));
}

#[test]
fn every_feature_any_preset_or_repository_names_is_known() {
  let file = PresetsFile::parse(FILE).unwrap();

  assert_eq!(file.every_feature().into_iter().collect::<Vec<_>>(), ["study", "teams"]);
}

#[test]
fn a_key_the_schema_does_not_name_is_refused() {
  assert!(matches!(PresetsFile::parse(&format!("{FILE}\nstray = 1\n")), Err(Refused::Unread(_))));
}

// The index's own presets file, where this checkout stands in one: it reads, and every repository a preset names
// stands among its entries. A checkout standing alone has none to read.
#[test]
fn the_index_presets_file_reads_where_it_stands() {
  let index = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
  let file = index.join("presets.toml");

  if !file.is_file() {
    return;
  }

  let presets = PresetsFile::read(&index).unwrap();

  for (name, preset) in &presets.preset {
    let with: Vec<String> =
      preset.choose.iter().filter_map(|choice| choice.units.first().cloned()).collect();
    let resolved = presets.resolve(name, &with).unwrap();

    assert!(!resolved.units.is_empty(), "{name}");
    assert!(resolved.open.is_empty(), "{name}");
  }
}
