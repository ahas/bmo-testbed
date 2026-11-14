//! The console probe (docs/deploy.md, 「The console probe」), over a stand-in service and a stand-in for the
//! layer's reading: a face keeping the contract holds every point and no credential is said; each way a face
//! breaks it is refused at its point; share reads open the module with no role; a service holding no token of
//! its own is asked with one the probe made; the layer is held to the release of the pin's range the release
//! vendors where one is pinned and passed over, said, where none is; and that release is found among the files
//! a release tracks by its packed file's name and bytes, the pinned one first.

use std::cell::RefCell;
use std::path::Path;

use bmo_testbed_deploy::console::{
  Answer, Answering, ConsoleHands, LayerPin, LayerRelease, PinnedLayer, Refusal, VendoredLayer, probe,
  read_reach, vendored_layer,
};
use serde_json::{Value, json};

use crate::stand::must;

const ADDRESS: &str = "http://service.test/api/";
const CONSOLE: &str = "console-credential";
const OWN: &str = "service-credential";
const PINNED: &str = "1.0.0";
const NEWER: &str = "1.1.0";

/// A service's console face standing in for one, each way it may break the contract a switch.
pub struct Face {
  pub manifest: Value,
  /// Answers its module and stylesheet with no role.
  pub open_without_role: bool,
  /// Answers its own token on the console's routes.
  pub takes_own_token: bool,
  /// Answers any credential at all on the console's routes.
  pub takes_any_token: bool,
  pub words: String,
  pub reach: Result<String, String>,
  /// The version of the layer's release the release vendors, none where it vendors none.
  pub vendors: Option<&'static str>,
  pub refused_reading: Option<String>,
  pub asked: RefCell<Vec<String>>,
}

impl Default for Face {
  fn default() -> Self {
    Self {
      manifest: json!({
        "name": "sample",
        "layer": PINNED,
        "bundle": "/console-bundle/sample-1a2b.js",
        "stylesheet": "/console-bundle/sample-3c4d.css",
        "words": "/console-words",
        "role": "member",
        "ownerKey": true,
        "shareReads": [],
      }),
      open_without_role: false,
      takes_own_token: false,
      takes_any_token: false,
      words: "{\"en\": {\"sample\": {\"title\": \"Sample\"}}}".to_owned(),
      reach: Ok(format!(
        "{{\"address\": \"{ADDRESS}\", \"consoleToken\": \"{CONSOLE}\", \"token\": \"{OWN}\"}}\n"
      )),
      vendors: Some(PINNED),
      refused_reading: None,
      asked: RefCell::new(Vec::new()),
    }
  }
}

impl Face {
  fn path(&self, field: &str) -> Option<&str> {
    self.manifest.get(field).and_then(Value::as_str)
  }

  fn answer(&self, path: &str, headers: &[(String, String)]) -> Answer {
    let header = |name: &str| headers.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str());
    let token = header("Authorization").and_then(|value| value.strip_prefix("Bearer "));
    let role = header("Owner-Role");
    let key = header("Owner-Key");
    let status = |status: u16, body: &str| Answer {
      status,
      body: body.to_owned(),
    };

    match token {
      Some(CONSOLE) => {}
      Some(OWN) if self.takes_own_token => {}
      Some(_) if self.takes_any_token => {}
      _ => return status(401, "{}"),
    }

    if path == "/console-manifest" {
      return status(200, &self.manifest.to_string());
    }

    if Some(path) == self.path("words") {
      return status(200, &self.words);
    }

    if Some(path) == self.path("bundle") || Some(path) == self.path("stylesheet") {
      let opened = self.open_without_role || (role == Some("member") && key.is_some());

      return if opened { status(200, "export default {}") } else { status(403, "{}") };
    }

    status(404, "{}")
  }
}

impl ConsoleHands for Face {
  fn reach(&self, _: &str, _: &Path) -> Result<String, String> {
    self.reach.clone()
  }

  fn get<'a>(&'a self, address: &'a str, headers: &'a [(String, String)]) -> Answering<'a> {
    Box::pin(async move {
      let path = address.strip_prefix(ADDRESS.trim_end_matches('/')).unwrap_or(address);

      self.asked.borrow_mut().push(format!("{path} {}", headers.len()));
      Ok(self.answer(path, headers))
    })
  }

  fn vendored_layer(&self, release: &Path, pin: &LayerPin) -> Result<VendoredLayer, String> {
    pin
      .releases()
      .find(|layer| Some(layer.version.as_str()) == self.vendors)
      .map(|layer| VendoredLayer {
        release: layer.clone(),
        path: release.join(&layer.file),
      })
      .ok_or_else(|| "no release of the range vendored".to_owned())
  }

  fn read(&self, manifest: &str, _: &Path) -> Result<Value, String> {
    match &self.refused_reading {
      Some(why) => Err(why.clone()),
      None => serde_json::from_str(manifest).map_err(|why| why.to_string()),
    }
  }
}

fn probed(face: &Face, pinned: &PinnedLayer) -> (Result<usize, Refusal>, Vec<String>) {
  let said = RefCell::new(Vec::new());
  let say = |line: &str| said.borrow_mut().push(line.to_owned());
  let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
  let outcome = runtime.block_on(probe("answer-reach", Path::new("."), pinned, face, &say));

  (outcome, said.into_inner())
}

fn layer(version: &str, sha256: &str) -> LayerRelease {
  LayerRelease {
    version: version.to_owned(),
    file: format!("layer-{version}.tgz"),
    sha256: sha256.to_owned(),
  }
}

fn pinned() -> PinnedLayer {
  PinnedLayer::Pinned(LayerPin {
    pinned: layer(PINNED, &"0".repeat(64)),
    compatible: Vec::new(),
  })
}

// The newer release pinned, the face's own listed compatible with it.
fn ranged() -> PinnedLayer {
  PinnedLayer::Pinned(LayerPin {
    pinned: layer(NEWER, &"1".repeat(64)),
    compatible: vec![layer(PINNED, &"0".repeat(64))],
  })
}

fn refused_at(face: &Face, pinned: &PinnedLayer) -> Refusal {
  probed(face, pinned).0.expect_err("the probe refuses the face")
}

// "a face keeping the contract holds every point, the four paths asked with the service's own token, and no
// credential is ever said"
#[test]
fn a_face_keeping_the_contract_holds_every_point_and_no_credential_is_said() {
  let face = Face::default();
  let (outcome, said) = probed(&face, &pinned());

  assert_eq!(outcome, Ok(13));
  assert_eq!(
    said,
    [
      format!("probing the console face at {ADDRESS}"),
      "the console face holds to the contract (13 points)".to_owned(),
    ]
  );
  assert_eq!(face.asked.borrow().len(), 10);
  assert!(said.iter().all(|line| !line.contains(CONSOLE) && !line.contains(OWN)));
}

// "a module and stylesheet answered with no role are refused at that point; with share reads they are owed"
#[test]
fn a_module_answered_with_no_role_is_refused_unless_share_reads_open_it() {
  let mut face = Face {
    open_without_role: true,
    ..Face::default()
  };
  let refusal = refused_at(&face, &pinned());

  assert_eq!(refusal.point, "the module with no role");
  assert_eq!(
    refusal.why,
    "GET /console-bundle/sample-1a2b.js answered 200 where the contract asks 403"
  );

  face.manifest["shareReads"] = json!(["/shares/{token}"]);

  assert_eq!(probed(&face, &pinned()).0, Ok(13));

  face.open_without_role = false;

  assert_eq!(refused_at(&face, &pinned()).point, "the module with no role");
}

// "a face taking the service's own token on the console's routes is refused at the manifest"
#[test]
fn a_face_taking_the_services_own_token_is_refused() {
  let face = Face {
    takes_own_token: true,
    ..Face::default()
  };
  let refusal = refused_at(&face, &pinned());

  assert_eq!(refusal.point, "the manifest with the service's own token");
  assert_eq!(refusal.why, "GET /console-manifest answered 200 where the contract asks 401");
}

// "a service holding no token of its own is asked with a credential the probe made, which the contract refuses
// as any credential not the console's"
#[test]
fn a_service_holding_no_token_is_asked_with_a_credential_the_probe_made() {
  let mut face = Face {
    reach: Ok(format!("{{\"address\": \"{ADDRESS}\", \"consoleToken\": \"{CONSOLE}\"}}\n")),
    ..Face::default()
  };
  let (outcome, said) = probed(&face, &pinned());

  assert_eq!(outcome, Ok(13));
  assert!(
    said
      .iter()
      .any(|line| line.starts_with("the reach names no token of the service's own")),
    "{said:?}"
  );

  face.takes_any_token = true;

  assert_eq!(refused_at(&face, &pinned()).point, "the manifest with a credential the probe made");
}

// "a release vendoring a compatible release of the pin's range holds with its layer that release, a layer
// other than the vendored release is refused, and with none pinned it is passed over and said"
#[test]
fn the_layer_is_held_to_the_vendored_release_of_the_range_and_passed_over_where_none_is_pinned() {
  let face = Face::default();

  assert_eq!(probed(&face, &ranged()).0, Ok(13));

  let moved = Face {
    vendors: Some(NEWER),
    ..Face::default()
  };
  let refusal = refused_at(&moved, &ranged());

  assert_eq!(refusal.point, "the layer");
  assert_eq!(
    refusal.why,
    "the manifest names the layer 1.0.0 where the release vendors the layer's release 1.1.0"
  );

  let (outcome, said) = probed(&face, &PinnedLayer::Unpinned);

  assert_eq!(outcome, Ok(11));
  assert_eq!(
    said[1],
    "the shared versions pin no bmo-ui, so the manifest is read by no release of it and its layer 1.0.0 is held to none"
  );

  let unread = refused_at(&face, &PinnedLayer::Unread("the shared versions do not read".to_owned()));

  assert_eq!(
    (unread.point.as_str(), unread.why.as_str()),
    ("the layer's release", "the shared versions do not read")
  );

  let unvendored = Face {
    vendors: None,
    ..Face::default()
  };

  assert_eq!(refused_at(&unvendored, &ranged()).point, "the layer's release");
}

// "the vendored release is the first of the range, the pinned one first, whose packed file the tree tracks
// under its name with its bytes, a stale copy of the name passed over, and none is named with the range's files"
#[test]
fn the_vendored_release_is_found_among_the_tracked_files_by_its_name_and_bytes_pinned_first() {
  use sha2::{Digest, Sha256};

  let tree = tempfile::tempdir().unwrap();
  let write = |path: &str, text: &str| {
    let at = tree.path().join(path);

    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, text).unwrap();
  };
  let hashed =
    |text: &str| -> String { Sha256::digest(text).iter().map(|byte| format!("{byte:02x}")).collect() };

  write("a/vendor/layer-1.0.0.tgz", "a stale build");
  write("b/vendor/layer-1.0.0.tgz", "the compatible release");
  write("c/vendor/layer-1.1.0.tgz", "the pinned release, untracked");
  must(&["init", "--quiet"], tree.path());
  must(&["add", "a", "b"], tree.path());

  let mut pin = LayerPin {
    pinned: layer(NEWER, &hashed("the pinned release, untracked")),
    compatible: vec![layer(PINNED, &hashed("the compatible release"))],
  };

  assert_eq!(
    vendored_layer(tree.path(), &pin),
    Ok(VendoredLayer {
      release: pin.compatible[0].clone(),
      path: tree.path().join("b/vendor/layer-1.0.0.tgz"),
    })
  );

  must(&["add", "c"], tree.path());

  assert_eq!(vendored_layer(tree.path(), &pin).map(|found| found.release), Ok(pin.pinned.clone()));

  pin.pinned.sha256 = "f".repeat(64);
  pin.compatible[0].sha256 = "f".repeat(64);

  let missing = vendored_layer(tree.path(), &pin).unwrap_err();

  assert!(
    missing
      .starts_with("the release tracks none of the layer's packed files layer-1.1.0.tgz, layer-1.0.0.tgz"),
    "{missing}"
  );
}

// "a reach that fails, or does not read, is refused at the reach without asking the service"
#[test]
fn a_reach_that_fails_or_does_not_read_is_refused_before_anything_is_asked() {
  let failing = Face {
    reach: Err("the reach command ended 1: no token file".to_owned()),
    ..Face::default()
  };

  assert_eq!(refused_at(&failing, &pinned()).point, "the reach");
  assert!(failing.asked.borrow().is_empty());

  let unread = read_reach("{\"address\": \"http://service.test\", \"consoleToken\": \"\"}").unwrap_err();

  assert!(unread.contains("consoleToken"), "{unread}");

  let base = "\"address\": \"http://service.test\", \"consoleToken\": \"console\"";
  let none = read_reach(&format!("{{{base}, \"token\": null}}")).expect("a null token reads as none");

  assert_eq!(none.token, None);

  let empty = read_reach(&format!("{{{base}, \"token\": \"\"}}")).unwrap_err();

  assert!(empty.contains("token"), "{empty}");
}

// "a manifest the layer's reading refuses is refused in the reading's words, and words that are no JSON object
// are refused at the words"
#[test]
fn the_readings_refusal_and_words_that_are_no_object_are_refused_at_their_points() {
  let refused = Face {
    refused_reading: Some("The console manifest does not keep the contract: layer".to_owned()),
    ..Face::default()
  };
  let refusal = refused_at(&refused, &pinned());

  assert_eq!(refusal.point, "the manifest's reading");
  assert_eq!(refusal.why, "The console manifest does not keep the contract: layer");

  let listed = Face {
    words: "[]".to_owned(),
    ..Face::default()
  };

  assert_eq!(refused_at(&listed, &pinned()).point, "the words");
}
