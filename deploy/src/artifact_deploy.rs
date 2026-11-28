//! A deployment from an artifact (docs/artifacts.md, 「A deployment from an artifact」): the artifact checked, its
//! release laid under the deploy root, the steps before the flip, the flip and the record, the units linked and
//! restarted, the health waited for and the steps after it, and the live release flipped back to where any of the
//! last fail. Nothing is fetched, gated or built.

use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;
use bmo_testbed_core::declaration::{Phase, Repository};
use bmo_testbed_core::policy::artifacts::ARTIFACTS;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::turn::Env;
use sha2::{Digest, Sha256};

use crate::boot::{HealthWatch, wait_for_health};
use crate::deployment::{DeployPlaces, unit_files};
use crate::release::{flip, live_release, put_link, release_dir};
use crate::tree::{
  DeployRecord, UnitMark, base_name, link_units, marked_units, read_deploy_record, set_aside_plain_units,
  timer_service, units_to_link, write_deploy_record,
};

/// What a deployment from an artifact is asked.
pub struct ArtifactAsk {
  pub archive: PathBuf,
  pub message: Option<String>,
  pub dry_run: bool,
}

fn said(line: &str, values: &[(&str, &str)]) -> String {
  fill_marks(line, values)
}

// A command's exit code and what it said, both streams.
fn ran(command: &mut Command) -> (i32, String) {
  match command.output() {
    Ok(output) => {
      let mut text = String::from_utf8_lossy(&output.stdout).into_owned();

      text.push_str(&String::from_utf8_lossy(&output.stderr));
      (output.status.code().unwrap_or(-1), text.trim().to_owned())
    }
    Err(why) => (-1, why.to_string()),
  }
}

/// The GNU C library version this machine runs, as `2.35`.
pub fn machine_glibc() -> String {
  // SAFETY: gnu_get_libc_version returns a pointer to a static, NUL-terminated string the C library owns.
  unsafe { std::ffi::CStr::from_ptr(libc::gnu_get_libc_version()) }.to_string_lossy().into_owned()
}

fn parts(version: &str) -> Vec<u64> {
  version.split('.').filter_map(|part| part.parse().ok()).collect()
}

/// The manifest beside an archive: `<stem>.json` for `<stem>.tar.zst`.
pub fn manifest_of(archive: &Path) -> PathBuf {
  let name = archive.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();

  archive.with_file_name(format!("{}.json", name.trim_end_matches(".tar.zst")))
}

/// The artifact and its manifest checked against each other, the set's key and this machine: the manifest where
/// it holds, why not where it does not.
pub fn checked(archive: &Path, env: &Env) -> Result<serde_json::Value, String> {
  let words = &ARTIFACTS.said;
  let shown = archive.display().to_string();
  let manifest_path = manifest_of(archive);
  let manifest: serde_json::Value = std::fs::read_to_string(&manifest_path)
    .map_err(|why| why.to_string())
    .and_then(|text| serde_json::from_str(&text).map_err(|why| why.to_string()))
    .map_err(|why| said(&words.no_manifest, &[("file", &manifest_path.display().to_string()), ("why", &why)]))?;
  let field = |name: &str| manifest[name].as_str().unwrap_or_default().to_owned();
  let bytes = std::fs::read(archive).map_err(|why| format!("{shown}: {why}"))?;
  let digest = Sha256::digest(&bytes);
  let hex = format!("{digest:x}");

  if hex != field("sha256") {
    return Err(said(&words.digest_differs, &[("file", &shown), ("digest", &hex), ("expected", &field("sha256"))]));
  }

  let signing = &ARTIFACTS.signing;
  let allowed = env.get(&signing.allow_unsigned).is_some_and(|value| value == "1");

  match (field("signature").as_str(), signing.public_key.is_empty()) {
    ("", _) | (_, true) if !allowed => {
      return Err(said(&words.unsigned, &[("file", &shown), ("variable", &signing.allow_unsigned)]));
    }
    ("", _) | (_, true) => {}
    (signature, false) => {
      let decode = |text: &str| base64::engine::general_purpose::STANDARD.decode(text.trim()).ok();
      let key: Option<[u8; 32]> = decode(&signing.public_key).and_then(|bytes| bytes.try_into().ok());
      let signature: Option<[u8; 64]> = decode(signature).and_then(|bytes| bytes.try_into().ok());
      let holds = match (key, signature) {
        (Some(key), Some(signature)) => ed25519_dalek::VerifyingKey::from_bytes(&key).is_ok_and(|key| {
          key.verify_strict(&digest, &ed25519_dalek::Signature::from_bytes(&signature)).is_ok()
        }),
        _ => false,
      };

      if !holds {
        return Err(said(&words.bad_signature, &[("file", &shown)]));
      }
    }
  }

  let needs = field("glibc");
  let has = machine_glibc();

  if parts(&needs) > parts(&has) {
    return Err(said(&words.floor_above, &[("name", &field("name")), ("needs", &needs), ("has", &has)]));
  }

  Ok(manifest)
}

// Every file under a directory, as paths from it.
fn files_under(root: &Path) -> Vec<String> {
  let mut found = Vec::new();
  let mut dirs = vec![PathBuf::new()];

  while let Some(dir) = dirs.pop() {
    let Ok(entries) = std::fs::read_dir(root.join(&dir)) else { continue };

    for entry in entries.flatten() {
      let relative = dir.join(entry.file_name());

      if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
        dirs.push(relative);
      } else {
        found.push(relative.to_string_lossy().into_owned());
      }
    }
  }

  found.sort();
  found
}

// The `PATH` a step runs on: the runtimes the release carries first, then the caller's own.
fn step_path(release: &Path, env: &Env) -> String {
  let mut paths: Vec<String> = std::fs::read_dir(release.join("runtime"))
    .map(|entries| entries.flatten().map(|entry| entry.path().to_string_lossy().into_owned()).collect())
    .unwrap_or_default();

  paths.sort();
  paths.push(env.get("PATH").cloned().unwrap_or_default());
  paths.join(":")
}

fn systemctl(args: &[&str]) -> bool {
  ran(Command::new("systemctl").arg("--user").args(args)).0 == 0
}

/// The release the artifact holds put live, as docs/artifacts.md states; the line saying it is live, or why not.
pub async fn deploy_artifact(ask: &ArtifactAsk, env: &Env, say: &dyn Fn(&str)) -> Result<String, String> {
  let words = &ARTIFACTS.said;
  let manifest = checked(&ask.archive, env)?;
  let field = |name: &str| manifest[name].as_str().unwrap_or_default().to_owned();
  let (name, version, commit) = (field("name"), field("version"), field("commit"));

  // The declaration it carries, read before anything is laid, so the places are known first.
  let peek = tempdir_beside(&ask.archive)?;
  let (code, out) = ran(
    Command::new("tar").arg("--zstd").arg("-xf").arg(&ask.archive).arg("-C").arg(&peek).arg("./testbed.toml"),
  );

  if code != 0 {
    let _ = std::fs::remove_dir_all(&peek);
    return Err(out);
  }

  let declared = Repository::read(&peek).map_err(|why| format!("{why:?}"));
  let _ = std::fs::remove_dir_all(&peek);
  let declared = declared?;
  let declared_name = declared.declaration.name.clone().unwrap_or_default();

  if declared_name != name {
    return Err(said(&words.other_repository, &[("name", &name), ("declared", &declared_name)]));
  }

  let places = DeployPlaces::of(&declared, env)?;
  let release = release_dir(&places.deploy_dir, &commit);
  let shown = release.display().to_string();

  if ask.dry_run {
    return Ok(said(&words.planned, &[("name", &name), ("version", &version), ("release", &shown)]));
  }

  // The release laid afresh from the artifact, its env links made into the data root.
  let _ = std::fs::remove_dir_all(&release);
  std::fs::create_dir_all(&release).map_err(|why| format!("{shown}: {why}"))?;

  let (code, out) =
    ran(Command::new("tar").arg("--zstd").arg("-xf").arg(&ask.archive).arg("-C").arg(&release));

  if code != 0 {
    return Err(out);
  }

  let repository = Repository::read(&release).map_err(|why| format!("{why:?}"))?;
  let deploy = &repository.declaration.deploy;

  std::fs::create_dir_all(&places.data_root).map_err(|why| why.to_string())?;

  for link in &deploy.env_links {
    let target = if link.target.is_absolute() { link.target.clone() } else { places.data_root.join(&link.target) };

    if !target.exists() {
      return Err(said(&words.env_missing, &[("file", &target.display().to_string())]));
    }

    let at = release.join(&link.link);

    if let Some(parent) = at.parent() {
      std::fs::create_dir_all(parent).map_err(|why| why.to_string())?;
    }

    put_link(&at, &target).map_err(|why| why.to_string())?;
  }

  say(&said(&words.laid, &[("name", &name), ("version", &version), ("release", &shown)]));

  let path = step_path(&release, env);
  let step = |id: &str, command: &str, cwd: &Path| -> Result<(), String> {
    let (code, out) = ran(Command::new("sh").arg("-c").arg(command).current_dir(cwd).env("PATH", &path));

    if code == 0 {
      Ok(())
    } else {
      Err(said(&words.step_failed, &[("id", id), ("code", &code.to_string()), ("said", &out)]))
    }
  };

  for one in deploy.steps.iter().filter(|one| one.phase == Phase::BeforeFlip) {
    step(&one.id, &one.command, &release)?;
  }

  // The flip and the record, the live release kept to flip back to.
  let live = live_release(&places.deploy_dir);
  let previous = read_deploy_record(&places.record);

  std::fs::create_dir_all(&places.deploy_root).map_err(|why| why.to_string())?;
  flip(&places.deploy_dir, &release).map_err(|why| why.to_string())?;
  say(&said(&words.flipped, &[("release", &shown)]));

  let record = DeployRecord {
    commit: commit.clone(),
    subject: format!("{name} {version}"),
    message: ask.message.clone().unwrap_or_else(|| format!("{name} {version}")),
    version: version.clone(),
    at: chrono::Utc::now().to_rfc3339(),
    backup: String::new(),
  };

  write_deploy_record(&places.record, &record).map_err(|why| why.to_string())?;

  let line = format!("{} {} {} {}\n", record.at, record.commit, record.backup, record.subject);
  let _ = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(&places.log)
    .and_then(|mut log| std::io::Write::write_all(&mut log, line.as_bytes()));

  let back = |why: String| -> String {
    if let Some(live) = &live {
      let _ = flip(&places.deploy_dir, live);

      if let Some(previous) = &previous {
        let _ = write_deploy_record(&places.record, previous);
      }

      systemctl(&["daemon-reload"]);
      say(&said(&words.rolled_back, &[("release", &live.display().to_string())]));
    }

    why
  };

  // The units the release marks, linked into the user units and started as their marks say.
  let deploy_dir = &places.deploy_dir;
  let files = unit_files(&files_under(&release), deploy.units.as_deref());
  let read = |path: &str| std::fs::read_to_string(deploy_dir.join(path)).unwrap_or_default();
  let marked = |suffix: &str, mark: UnitMark| marked_units(&files, &read, suffix, mark);
  let bots = marked(".service", UnitMark::Bot);
  let restarted = marked(".service", UnitMark::Restart);
  let bound = marked(".service", UnitMark::Binary);
  let timers = marked(".timer", UnitMark::Timer);
  let linked_only: Vec<String> =
    [".service", ".slice"].into_iter().flat_map(|suffix| marked(suffix, UnitMark::Link)).collect();
  let declared_units: Vec<String> = restarted
    .iter()
    .chain(&bound)
    .chain(&timers)
    .cloned()
    .chain(timers.iter().map(|timer| timer_service(timer)))
    .chain(linked_only.iter().cloned())
    .chain(bots.iter().cloned())
    .collect();

  std::fs::create_dir_all(&places.unit_dir).map_err(|why| why.to_string())?;
  set_aside_plain_units(&places.unit_dir, &declared_units);

  let installed: Vec<String> = std::fs::read_dir(&places.unit_dir)
    .map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
    .unwrap_or_default();
  let installs = units_to_link(&declared_units, deploy_dir, &installed);

  link_units(&places.unit_dir, &installs).map_err(|why| back(why.to_string()))?;

  for one in &installs {
    say(&said(&words.unit_linked, &[("name", &one.name)]));
  }

  systemctl(&["daemon-reload"]);

  for timer in timers.iter().map(|path| base_name(path)) {
    systemctl(&["enable", "--now", &timer]);
  }

  for unit in restarted.iter().chain(&bound).chain(&bots).map(|path| base_name(path)) {
    systemctl(&["enable", &unit]);
    systemctl(&["reset-failed", &unit]);

    if systemctl(&["restart", &unit]) {
      say(&said(&words.restarted, &[("unit", &unit)]));
    }
  }

  // The health, then the steps after it.
  if let Some(health) = &deploy.health {
    let watch = HealthWatch::new(health, deploy_dir, deploy.health_wait_seconds);
    let waited = wait_for_health(&watch).await;

    if !waited.answered() {
      return Err(back(said(&words.health_failed, &[("name", &name), ("said", &waited.last)])));
    }
  }

  for one in deploy.steps.iter().filter(|one| one.phase == Phase::AfterHealth) {
    step(&one.id, &one.command, deploy_dir).map_err(back)?;
  }

  Ok(said(&words.live, &[("name", &name), ("version", &version)]))
}

// A fresh directory beside the archive to read one file of it into.
fn tempdir_beside(archive: &Path) -> Result<PathBuf, String> {
  let parent = archive.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf);
  let dir = parent.join(format!(".peek-{}", std::process::id()));

  let _ = std::fs::remove_dir_all(&dir);
  std::fs::create_dir_all(&dir).map_err(|why| format!("{}: {why}", dir.display()))?;
  Ok(dir)
}
