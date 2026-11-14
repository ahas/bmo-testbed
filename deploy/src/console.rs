//! The console probe (docs/deploy.md, 「The console probe」): a service that brings a screen to a console is
//! asked, once it has started, the parts of bmo-ui's console plugin contract a console meets first, as a
//! console asks them. The manifest is read by the layer's own reading through bmo-chat's console face, run on
//! the packed file of the release of the pin's range the release being deployed vendors, and the contract's
//! route and headers are the console face's own constants, so the testbed restates neither.
//!
//! The asks go through hands of their own, so a test stands a service and the reading in for the machine's.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::time::Duration;

use bmo_console_face::{CONSOLE_MANIFEST_ROUTE, OWNER_KEY_HEADER, OWNER_ROLE_HEADER, read_manifest};
use bmo_testbed_core::policy::deploy::{ConsoleWords, DEPLOY};
use bmo_testbed_core::policy::fill_marks;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::unit::first_chars;

fn words() -> &'static ConsoleWords {
  &DEPLOY.console
}

/// One release of the console layer: its version, the file it is packed in and that file's sha256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerRelease {
  pub version: String,
  pub file: String,
  pub sha256: String,
}

/// The console layer as the shared versions pin it: the pinned release and the earlier releases it stays
/// compatible with, in the order the shared versions list them. Together they are the pin's range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerPin {
  pub pinned: LayerRelease,
  pub compatible: Vec<LayerRelease>,
}

impl LayerPin {
  /// The releases of the range in the order a release's vendored copy is looked for: the pinned one, then
  /// each compatible one.
  pub fn releases(&self) -> impl Iterator<Item = &LayerRelease> {
    std::iter::once(&self.pinned).chain(&self.compatible)
  }
}

/// A release of the pin's range as the release being deployed vendors it: the release and the tracked file
/// holding its packed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendoredLayer {
  pub release: LayerRelease,
  pub path: PathBuf,
}

/// The console layer as the shared versions pin it, whose vendored release a manifest is read by and its
/// `layer` held to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PinnedLayer {
  /// The declaration names no shared versions.
  #[default]
  Unnamed,
  /// The shared versions pin no release of the layer.
  Unpinned,
  /// The range the shared versions pin.
  Pinned(LayerPin),
  /// Why the shared versions did not read.
  Unread(String),
}

/// How a console reaches the service, as the declared `reach` command answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
  /// The base address of the service's API.
  pub address: String,
  /// The credential the console calls the service with.
  pub console_token: String,
  /// The service's own token, which the console's routes refuse; none where the service holds none.
  pub token: Option<String>,
}

/// What a service answered an ask: its status and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
  pub status: u16,
  pub body: String,
}

/// An answer still to come.
pub type Answering<'a> = Pin<Box<dyn Future<Output = Result<Answer, String>> + 'a>>;

/// What the probe does outside itself.
pub trait ConsoleHands {
  /// The declared `reach` command run from the release to its end: what it wrote on its standard output, or
  /// why it did not answer.
  fn reach(&self, command: &str, release: &Path) -> Result<String, String>;
  /// A `GET` of an address with the headers given, waited for the policy's seconds: what came, or why
  /// nothing did.
  fn get<'a>(&'a self, address: &'a str, headers: &'a [(String, String)]) -> Answering<'a>;
  /// The release of the pin's range the release being deployed vendors, with its packed file, or why none
  /// stands there.
  fn vendored_layer(&self, release: &Path, pin: &LayerPin) -> Result<VendoredLayer, String>;
  /// A manifest's text read by the layer's own reading in its packed file: the manifest with its defaults
  /// filled, or the reading's fault as it words it.
  fn read(&self, manifest: &str, layer_file: &Path) -> Result<Value, String>;
}

/// Where the probe refused the release: the point and what came.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
  pub point: String,
  pub why: String,
}

impl Refusal {
  fn at(point: &str, why: String) -> Self {
    Self {
      point: point.to_owned(),
      why,
    }
  }

  /// The line the deployment says for it.
  pub fn said(&self) -> String {
    fill_marks(&words().refused, &[("point", &self.point), ("why", &self.why)])
  }
}

/// The reach the command's output answers, or why it does not read as one.
pub fn read_reach(output: &str) -> Result<Reach, String> {
  let words = words();
  let unread = |why: String| fill_marks(&words.reach_unread, &[("why", &why)]);
  let value: Value = serde_json::from_str(output.trim()).map_err(|why| unread(why.to_string()))?;
  let field = |key: &str| {
    value
      .get(key)
      .and_then(Value::as_str)
      .filter(|text| !text.is_empty())
      .map(str::to_owned)
      .ok_or_else(|| unread(key.to_owned()))
  };
  // The service's own token, absent or null where it holds none; an empty one is a reading gone wrong.
  let token = match value.get(&words.token_key) {
    None | Some(Value::Null) => None,
    Some(_) => Some(field(&words.token_key)?),
  };

  Ok(Reach {
    address: field(&words.address_key)?,
    console_token: field(&words.console_token_key)?,
    token,
  })
}

fn hex_sha256(bytes: &[u8]) -> String {
  Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_of(path: &Path) -> Option<String> {
  std::fs::read(path).ok().map(|bytes| hex_sha256(&bytes))
}

// A credential no service holds, made for one probe from the moment and the process: what it asks with in
// place of the service's own token where the service holds none.
fn made_credential() -> String {
  let moment = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |since| since.as_nanos());

  hex_sha256(format!("{moment}-{}", std::process::id()).as_bytes())
}

/// The first release of the pin's range a git tree vendors: a file it tracks named as that release names its
/// packed file and holding its bytes, or why none does. The pinned release is tried first, then each
/// compatible one; every copy the tree tracks under a release's name is tried, so a stale copy beside the
/// release does not hide it.
pub fn vendored_layer(tree: &Path, pin: &LayerPin) -> Result<VendoredLayer, String> {
  let words = words();
  let listed = Command::new("git")
    .args(["ls-files", "-z"])
    .current_dir(tree)
    .stdin(Stdio::null())
    .output()
    .map_err(|why| why.to_string())?;
  let missing = || {
    let files: Vec<&str> = pin.releases().map(|release| release.file.as_str()).collect();

    fill_marks(&words.no_release, &[("files", &files.join(&words.files_joiner))])
  };

  if !listed.status.success() {
    return Err(missing());
  }

  let tracked = String::from_utf8_lossy(&listed.stdout);
  let copy_of = |release: &LayerRelease| {
    tracked
      .split('\0')
      .filter(|path| path.rsplit('/').next() == Some(release.file.as_str()))
      .map(|path| tree.join(path))
      .find(|path| sha256_of(path).as_deref() == Some(release.sha256.as_str()))
  };

  pin
    .releases()
    .find_map(|release| {
      copy_of(release).map(|path| VendoredLayer {
        release: release.clone(),
        path,
      })
    })
    .ok_or_else(missing)
}

// A text field of the manifest, or the refusal of the point that read it.
fn text_field(manifest: &Value, field: &str, point: &str) -> Result<String, Refusal> {
  manifest
    .get(field)
    .and_then(Value::as_str)
    .map(str::to_owned)
    .ok_or_else(|| Refusal::at(point, fill_marks(&words().no_field, &[("field", field)])))
}

// The headers of an ask: the credential, then the headers a console sets.
fn headers(token: &str, set: &[(String, String)]) -> Vec<(String, String)> {
  let words = words();
  let credential = (words.credential_header.clone(), fill_marks(&words.credential, &[("token", token)]));

  std::iter::once(credential).chain(set.iter().cloned()).collect()
}

// One of the paths the probe asks: its name in the points, and its path on the service's API.
struct Asked {
  what: String,
  path: String,
}

// The probe under way: the service it asks and the points that held.
struct Probe<'a> {
  reach: Reach,
  hands: &'a dyn ConsoleHands,
  held: usize,
}

impl Probe<'_> {
  fn address(&self, path: &str) -> String {
    format!("{}{path}", self.reach.address.trim_end_matches('/'))
  }

  // An ask of a path holding to a status, its body where it does.
  async fn holds(
    &mut self,
    point: &str,
    path: &str,
    token: &str,
    set: &[(String, String)],
    wanted: u16,
  ) -> Result<String, Refusal> {
    let words = words();
    let address = self.address(path);
    let sent = headers(token, set);
    let answer =
      self.hands.get(&address, &sent).await.map_err(|why| {
        Refusal::at(point, fill_marks(&words.unanswered, &[("path", path), ("why", &why)]))
      })?;

    if answer.status != wanted {
      return Err(Refusal::at(
        point,
        fill_marks(
          &words.status,
          &[("path", path), ("found", &answer.status.to_string()), ("wanted", &wanted.to_string())],
        ),
      ));
    }

    self.held += 1;
    Ok(answer.body)
  }

  // The manifest as the vendored release of the pin's range reads it, its layer held to that release; where no
  // release is pinned, the manifest as it was answered, the passing over said.
  fn read(
    &mut self,
    text: &str,
    release: &Path,
    pinned: &PinnedLayer,
    say: &dyn Fn(&str),
  ) -> Result<Value, Refusal> {
    let words = words();
    let answered = |raw: &str| {
      serde_json::from_str::<Value>(raw).map_err(|why| {
        Refusal::at(&words.point_reading, fill_marks(&words.reading_failed, &[("why", &why.to_string())]))
      })
    };
    let layer_of = |manifest: &Value| {
      manifest
        .get(&words.layer_field)
        .and_then(Value::as_str)
        .unwrap_or(&words.no_layer)
        .to_owned()
    };

    match pinned {
      PinnedLayer::Pinned(pin) => {
        let vendored = self
          .hands
          .vendored_layer(release, pin)
          .map_err(|why| Refusal::at(&words.point_release, why))?;
        let manifest = self.hands.read(text, &vendored.path).map_err(|why| {
          Refusal::at(&words.point_reading, fill_marks(&words.reading_failed, &[("why", &why)]))
        })?;

        self.held += 1;

        let layer = text_field(&manifest, &words.layer_field, &words.point_layer)?;

        if layer != vendored.release.version {
          return Err(Refusal::at(
            &words.point_layer,
            fill_marks(&words.layer_differs, &[("found", &layer), ("wanted", &vendored.release.version)]),
          ));
        }

        self.held += 1;
        Ok(manifest)
      }
      PinnedLayer::Unpinned => {
        let manifest = answered(text)?;

        say(&fill_marks(
          &words.no_pin,
          &[("package", &words.layer_package), ("layer", &layer_of(&manifest))],
        ));
        Ok(manifest)
      }
      PinnedLayer::Unnamed => {
        let manifest = answered(text)?;

        say(&fill_marks(&words.no_shared, &[("layer", &layer_of(&manifest))]));
        Ok(manifest)
      }
      PinnedLayer::Unread(why) => Err(Refusal::at(&words.point_release, why.clone())),
    }
  }
}

/// The service's console face asked point by point, the reach its declared command answers and the manifest
/// read by the release of the pin's range the release vendors and held to it; how many points held, or the first that did not. Each line it
/// says goes to `say`; neither credential is ever in one.
pub async fn probe(
  command: &str,
  release: &Path,
  pinned: &PinnedLayer,
  hands: &dyn ConsoleHands,
  say: &dyn Fn(&str),
) -> Result<usize, Refusal> {
  let words = words();
  let reach = hands
    .reach(command, release)
    .and_then(|output| read_reach(&output))
    .map_err(|why| Refusal::at(&words.point_reach, why))?;

  say(&fill_marks(&words.probing, &[("address", &reach.address)]));

  let mut probe = Probe {
    reach,
    hands,
    held: 1,
  };
  let console = probe.reach.console_token.clone();
  // The service's own token, or a credential the probe made where it holds none, and the point it is asked at.
  let (own, own_point) = match probe.reach.token.clone() {
    Some(token) => (token, &words.point_own_token),
    None => {
      say(&words.no_token);
      (made_credential(), &words.point_made_token)
    }
  };

  // The manifest, read by the layer's own reading and held to the vendored release of the pin's range.
  let text = probe
    .holds(&words.point_manifest, CONSOLE_MANIFEST_ROUTE, &console, &[], words.answered)
    .await?;
  let manifest = probe.read(&text, release, pinned, say)?;

  // The words, answered with no role as one JSON object.
  let words_path = text_field(&manifest, &words.words_field, &words.point_reading)?;
  let body = probe.holds(&words.point_words, &words_path, &console, &[], words.answered).await?;
  let not_object = |why: String| {
    Refusal::at(
      &words.point_words,
      fill_marks(&words.not_object, &[("path", &words_path), ("why", &why)]),
    )
  };

  match serde_json::from_str::<Value>(&body) {
    Ok(Value::Object(_)) => {}
    Ok(other) => return Err(not_object(first_chars(&other.to_string(), DEPLOY.numbers.quoted_chars))),
    Err(why) => return Err(not_object(why.to_string())),
  }

  // The module and the stylesheet: refused with no role unless share reads load them with none, and
  // answered with the manifest's role.
  let role = text_field(&manifest, &words.role_field, &words.point_reading)?;
  let owner_key = manifest.get(&words.owner_key_field).and_then(Value::as_bool).unwrap_or(false);
  let shared = manifest
    .get(&words.share_reads_field)
    .and_then(Value::as_array)
    .is_some_and(|reads| !reads.is_empty());
  let mut with_role = vec![(OWNER_ROLE_HEADER.to_owned(), role.clone())];

  if owner_key {
    with_role.push((OWNER_KEY_HEADER.to_owned(), words.owner_key.clone()));
  }

  let mut screens = vec![Asked {
    what: words.module.clone(),
    path: text_field(&manifest, &words.bundle_field, &words.point_reading)?,
  }];

  if let Some(stylesheet) = manifest.get(&words.stylesheet_field).and_then(Value::as_str) {
    screens.push(Asked {
      what: words.stylesheet.clone(),
      path: stylesheet.to_owned(),
    });
  }

  let no_role_wanted = if shared { words.answered } else { words.no_role };

  for asked in &screens {
    let point = fill_marks(&words.point_no_role, &[("what", &asked.what)]);

    probe.holds(&point, &asked.path, &console, &[], no_role_wanted).await?;

    let point = fill_marks(&words.point_role, &[("what", &asked.what), ("role", &role)]);

    probe.holds(&point, &asked.path, &console, &with_role, words.answered).await?;
  }

  // The four paths of the console face, refused to the service's own token, or the probe's own, whatever the
  // role.
  let own_paths = [
    Asked {
      what: words.manifest.clone(),
      path: CONSOLE_MANIFEST_ROUTE.to_owned(),
    },
    Asked {
      what: words.words.clone(),
      path: words_path,
    },
  ]
  .into_iter()
  .chain(screens);

  for asked in own_paths {
    let point = fill_marks(own_point, &[("what", &asked.what)]);

    probe.holds(&point, &asked.path, &own, &with_role, words.not_console).await?;
  }

  say(&fill_marks(&words.held, &[("points", &probe.held.to_string())]));
  Ok(probe.held)
}

/// The machine's own hands: the reach command through `sh`, the asks over HTTP, and the reading through
/// bmo-chat's console face.
pub struct MachineConsole;

impl ConsoleHands for MachineConsole {
  fn reach(&self, command: &str, release: &Path) -> Result<String, String> {
    let words = words();
    let done = Command::new("sh")
      .args(["-c", command])
      .current_dir(release)
      .stdin(Stdio::null())
      .output()
      .map_err(|why| fill_marks(&words.reach_failed, &[("code", "127"), ("said", &why.to_string())]))?;

    if done.status.success() {
      return Ok(String::from_utf8_lossy(&done.stdout).into_owned());
    }

    let code = done.status.code().map_or_else(|| words.signal.clone(), |code| code.to_string());
    // A failing reach is quoted from its errors alone: its standard output may hold a credential.
    let said = first_chars(String::from_utf8_lossy(&done.stderr).trim(), DEPLOY.numbers.quoted_chars);

    Err(fill_marks(&words.reach_failed, &[("code", &code), ("said", &said)]))
  }

  fn get<'a>(&'a self, address: &'a str, headers: &'a [(String, String)]) -> Answering<'a> {
    Box::pin(async move {
      let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(words().wait_seconds))
        .build()
        .map_err(|why| why.to_string())?;
      let mut ask = client.get(address);

      for (name, value) in headers {
        ask = ask.header(name, value);
      }

      let answer = ask.send().await.map_err(|why| why.without_url().to_string())?;
      let status = answer.status().as_u16();
      let body = answer.text().await.map_err(|why| why.without_url().to_string())?;

      Ok(Answer { status, body })
    })
  }

  fn vendored_layer(&self, release: &Path, pin: &LayerPin) -> Result<VendoredLayer, String> {
    vendored_layer(release, pin)
  }

  fn read(&self, manifest: &str, layer_file: &Path) -> Result<Value, String> {
    let given: Value = serde_json::from_str(manifest).map_err(|why| why.to_string())?;

    read_manifest(&given, layer_file).map_err(|why| why.to_string())
  }
}
