//! `pack` (docs/artifacts.md): a repository's release built once at a commit, for the policy's floor, kept to what
//! it runs on, and written as one archive with its manifest and, where a key is given, its signature.

use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;
use bmo_testbed_core::declaration::{Artifact, Declaration, Repository};
use bmo_testbed_core::policy::artifacts::ARTIFACTS;
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::version::{deployed_version, platform_day};
use object::read::elf::{ElfFile64, FileHeader};
use object::Endianness;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::deployment::workspace_binaries;

/// What `pack` is asked: the checkout, the commit, where the archive goes, the signing key and the Cargo target
/// directory the build writes into.
pub struct PackAsk {
  pub checkout: PathBuf,
  pub commit: Option<String>,
  pub out: PathBuf,
  pub key: Option<PathBuf>,
  pub target_dir: PathBuf,
  pub cache: PathBuf,
}

/// What a pack came to: the archive and its manifest.
#[derive(Debug)]
pub struct Packed {
  pub archive: PathBuf,
  pub manifest: PathBuf,
  pub manifest_json: serde_json::Value,
}

fn said(line: &str, values: &[(&str, &str)]) -> String {
  fill_marks(line, values)
}

// A command run to its end; its standard output, or why it failed.
fn ran(command: &mut Command) -> Result<String, String> {
  let output = command.output().map_err(|why| why.to_string())?;

  if output.status.success() {
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
  } else {
    let mut why = String::from_utf8_lossy(&output.stderr).trim().to_owned();

    if why.is_empty() {
      why = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    }

    Err(why.chars().rev().take(1200).collect::<Vec<_>>().into_iter().rev().collect())
  }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
  ran(Command::new("git").arg("-C").arg(dir).args(args))
}

/// The deploy steps of a declaration that build or read a checkout, each with the word that says so
/// (docs/artifacts.md, 「The declaration」).
pub fn building_steps(declaration: &Declaration) -> Vec<(String, String)> {
  declaration
    .deploy
    .steps
    .iter()
    .filter_map(|step| {
      ARTIFACTS
        .refused
        .words
        .iter()
        .find(|word| step.command.contains(word.as_str()))
        .map(|word| (step.id.clone(), word.trim().to_owned()))
    })
    .collect()
}

/// Whether a release path, from its root, is kept by an artifact: under one of the kept paths and under none of
/// the left-out ones (docs/artifacts.md, 「The declaration」).
pub fn kept(path: &str, artifact: &Artifact) -> bool {
  let keep = ARTIFACTS.keep.always.iter().chain(&artifact.keep);
  let drop = ARTIFACTS.keep.never.iter().chain(&artifact.drop);
  let under = |pattern: &str| {
    let Ok(glob) = glob::Pattern::new(pattern) else { return false };
    let mut prefix = String::new();

    path.split('/').any(|part| {
      if !prefix.is_empty() {
        prefix.push('/');
      }

      prefix.push_str(part);
      glob.matches(&prefix)
    })
  };

  keep.into_iter().any(|pattern| under(pattern)) && !drop.into_iter().any(|pattern| under(pattern))
}

/// The highest GNU C library symbol version an ELF file asks for, as `2.35`; none where the file is not a 64-bit
/// ELF file or asks for none.
pub fn glibc_needed(bytes: &[u8]) -> Option<String> {
  let file = ElfFile64::<Endianness>::parse(bytes).ok()?;
  let endian = file.endian();
  let sections = file.elf_header().sections(endian, bytes).ok()?;
  let (mut needs, link) = sections.gnu_verneed(endian, bytes).ok()??;
  let strings = sections.strings(endian, bytes, link).ok()?;
  let mut highest: Option<Vec<u64>> = None;

  while let Ok(Some((_, mut auxes))) = needs.next() {
    while let Ok(Some(aux)) = auxes.next() {
      let Ok(name) = aux.name(endian, strings) else { continue };
      let name = String::from_utf8_lossy(name);
      let Some(version) = name.strip_prefix("GLIBC_") else { continue };
      let parts: Vec<u64> = version.split('.').filter_map(|part| part.parse().ok()).collect();

      if !parts.is_empty() && highest.as_ref().is_none_or(|now| parts > *now) {
        highest = Some(parts);
      }
    }
  }

  highest.map(|parts| parts.iter().map(u64::to_string).collect::<Vec<_>>().join("."))
}

fn version_parts(version: &str) -> Vec<u64> {
  version.split('.').filter_map(|part| part.parse().ok()).collect()
}

// Every file under a directory, as paths from it, links among them and never followed.
fn files_under(root: &Path) -> Vec<String> {
  let mut found = Vec::new();
  let mut dirs = vec![PathBuf::new()];

  while let Some(dir) = dirs.pop() {
    let Ok(entries) = std::fs::read_dir(root.join(&dir)) else { continue };

    for entry in entries.flatten() {
      let relative = dir.join(entry.file_name());
      let Ok(kind) = entry.file_type() else { continue };

      if kind.is_dir() {
        dirs.push(relative);
      } else {
        found.push(relative.to_string_lossy().into_owned());
      }
    }
  }

  found.sort();
  found
}

fn copy_kept(from: &Path, to: &Path, path: &str) -> std::io::Result<()> {
  let source = from.join(path);
  let target = to.join(path);

  if let Some(parent) = target.parent() {
    std::fs::create_dir_all(parent)?;
  }

  let meta = std::fs::symlink_metadata(&source)?;

  if meta.file_type().is_symlink() {
    std::os::unix::fs::symlink(std::fs::read_link(&source)?, &target)
  } else {
    std::fs::copy(&source, &target).map(|_| ())
  }
}

// The runtime a release names, fetched once into the cache, checked against its digest, and its binary placed
// under the release's `runtime/<name>/`.
fn place_runtime(name: &str, release: &Path, cache: &Path) -> Result<(), String> {
  let said_runtime = |why: &str| said(&ARTIFACTS.said.runtime_failed, &[("runtime", name), ("why", why)]);
  let runtime = ARTIFACTS.runtimes.get(name).ok_or_else(|| said_runtime("the policy does not carry it"))?;
  let folder = cache.join(format!("{name}-{}", runtime.version));
  let archive = folder.join("archive.zip");

  std::fs::create_dir_all(&folder).map_err(|why| said_runtime(&why.to_string()))?;

  if !archive.exists() {
    ran(Command::new("curl").args(["-fsSL", "-o"]).arg(&archive).arg(&runtime.url)).map_err(|why| said_runtime(&why))?;
  }

  let bytes = std::fs::read(&archive).map_err(|why| said_runtime(&why.to_string()))?;
  let digest = format!("{:x}", Sha256::digest(&bytes));

  if !runtime.sha256.is_empty() && digest != runtime.sha256 {
    let _ = std::fs::remove_file(&archive);
    return Err(said_runtime(&format!("its digest {digest} is not the policy's {}", runtime.sha256)));
  }

  ran(Command::new("unzip").args(["-o", "-q"]).arg(&archive).arg("-d").arg(&folder)).map_err(|why| said_runtime(&why))?;

  let binary = folder.join(&runtime.binary);
  let file_name = binary.file_name().ok_or_else(|| said_runtime("its binary has no name"))?;
  let placed = release.join("runtime").join(name);

  std::fs::create_dir_all(&placed).map_err(|why| said_runtime(&why.to_string()))?;
  std::fs::copy(&binary, placed.join(file_name)).map_err(|why| said_runtime(&why.to_string()))?;
  Ok(())
}

// Where a pack builds: a build tree and a release beside it, both standing where the checkout stands, so a path a
// Cargo manifest names from the checkout and a build step's `--show-toplevel` reach what they reach from it.
struct Stage {
  build: PathBuf,
  release: PathBuf,
  pack: PathBuf,
  checkout: PathBuf,
}

impl Stage {
  fn new(checkout: &Path, name: &str, commit: &str) -> Self {
    let parent = checkout.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let short = &commit[..commit.len().min(12)];

    Self {
      build: parent.join(format!(".pack-{name}")),
      release: parent.join("deploy-releases").join(short),
      pack: parent.join(format!(".pack-{name}-kept")),
      checkout: checkout.to_path_buf(),
    }
  }

  fn clear(&self) {
    if self.build.exists() {
      let _ = git(&self.checkout, &["worktree", "remove", "--force", &self.build.to_string_lossy()]);
      let _ = std::fs::remove_dir_all(&self.build);
    }

    let _ = git(&self.checkout, &["worktree", "prune"]);
    let _ = std::fs::remove_dir_all(&self.release);
    let _ = std::fs::remove_dir_all(&self.pack);

    if let Some(releases) = self.release.parent() {
      let _ = std::fs::remove_dir(releases);
    }
  }
}

/// The repository at the checkout packed at the commit (docs/artifacts.md, 「The words」).
pub fn pack(ask: &PackAsk, say: &dyn Fn(&str)) -> Result<Packed, String> {
  let artifacts = &ARTIFACTS.said;
  let repository = Repository::read(&ask.checkout).map_err(|why| format!("{why:?}"))?;
  let declaration = &repository.declaration;
  let name = declaration.name.clone().unwrap_or_else(|| {
    repository.root.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
  });
  let artifact = declaration.artifact.clone().ok_or_else(|| said(&artifacts.no_table, &[("name", &name)]))?;

  if let Some((id, word)) = building_steps(declaration).into_iter().next() {
    return Err(said(&artifacts.building_step, &[("name", &name), ("id", &id), ("word", &word)]));
  }

  for runtime in &artifact.runtimes {
    if !ARTIFACTS.runtimes.contains_key(runtime) {
      return Err(said(&artifacts.unknown_runtime, &[("name", &name), ("runtime", runtime)]));
    }
  }

  let commit = git(&repository.root, &["rev-parse", &format!("{}^{{commit}}", ask.commit.as_deref().unwrap_or("HEAD"))])?
    .trim()
    .to_owned();
  let at: Vec<String> = git(&repository.root, &["tag", "--points-at", &commit])?.lines().map(str::to_owned).collect();
  let reached: Vec<String> = git(&repository.root, &["tag", "--merged", &commit])?.lines().map(str::to_owned).collect();
  let now_ms = chrono::Utc::now().timestamp_millis();
  let version = deployed_version(&at, &reached, platform_day(now_ms));
  let arch = ARTIFACTS.floor.arches.first().cloned().unwrap_or_else(|| "x86_64".to_owned());
  let floor = ARTIFACTS.floor.glibc.clone();
  let stage = Stage::new(&repository.root, &name, &commit);

  stage.clear();

  let result = (|| {
    // The build tree, a worktree at the commit, and the release laid from the commit's own files.
    git(&repository.root, &["worktree", "add", "--detach", &stage.build.to_string_lossy(), &commit])?;
    std::fs::create_dir_all(&stage.release).map_err(|why| why.to_string())?;

    ran(Command::new("sh").arg("-c").arg(format!(
      "git -C {} archive --format=tar {} | tar -x -C {}",
      shlex::try_quote(&repository.root.to_string_lossy()).map_err(|why| why.to_string())?,
      commit,
      shlex::try_quote(&stage.release.to_string_lossy()).map_err(|why| why.to_string())?
    )))?;

    say(&format!("{name} at {}: building for glibc {floor} on {arch}", &commit[..12]));
    build(declaration, &stage, &ask.target_dir, &arch, &floor, &version, &commit)
      .map_err(|why| why.replace("{name}", &name))?;

    for runtime in &artifact.runtimes {
      place_runtime(runtime, &stage.release, &ask.cache)?;
    }

    // What the release runs on, copied apart, and every ELF file of it held to the floor.
    let mut highest: Vec<u64> = Vec::new();

    for path in files_under(&stage.release).iter().filter(|path| kept(path, &artifact)) {
      copy_kept(&stage.release, &stage.pack, path).map_err(|why| format!("{path}: {why}"))?;

      let file = stage.pack.join(path);

      if std::fs::symlink_metadata(&file).is_ok_and(|meta| meta.is_file())
        && let Ok(bytes) = std::fs::read(&file)
        && let Some(needed) = glibc_needed(&bytes)
      {
        let parts = version_parts(&needed);

        if parts > version_parts(&floor) {
          return Err(said(&artifacts.above_floor, &[("file", path), ("version", &needed), ("floor", &floor)]));
        }

        if parts > highest {
          highest = parts;
        }
      }
    }

    std::fs::create_dir_all(&ask.out).map_err(|why| why.to_string())?;

    let stem = format!("{name}-{version}-{arch}");
    let archive = ask.out.join(format!("{stem}.tar.zst"));
    let manifest = ask.out.join(format!("{stem}.json"));

    ran(Command::new("tar").arg("--zstd").arg("-cf").arg(&archive).arg("-C").arg(&stage.pack).arg("."))?;

    let bytes = std::fs::read(&archive).map_err(|why| why.to_string())?;
    let digest = Sha256::digest(&bytes);
    let glibc = highest.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
    let mut manifest_json = json!({
      "name": name,
      "version": version,
      "commit": commit,
      "arch": arch,
      "glibc": glibc,
      "sha256": format!("{digest:x}"),
    });

    if let Some(key) = &ask.key {
      manifest_json["signature"] = json!(sign(key, &digest)?);
    }

    std::fs::write(&manifest, serde_json::to_string_pretty(&manifest_json).unwrap_or_default() + "\n")
      .map_err(|why| why.to_string())?;
    say(&said(
      &artifacts.built,
      &[
        ("name", &name),
        ("version", &version),
        ("file", &archive.display().to_string()),
        ("size", &format!("{} MB", bytes.len() / 1_000_000)),
        ("glibc", &glibc),
      ],
    ));

    Ok(Packed {
      archive,
      manifest,
      manifest_json,
    })
  })();

  stage.clear();
  result
}

// The digest signed with the key the file holds, its 32-byte seed written in base64; the signature in base64.
fn sign(key: &Path, digest: &[u8]) -> Result<String, String> {
  let text = std::fs::read_to_string(key).map_err(|why| format!("{}: {why}", key.display()))?;
  let seed = base64::engine::general_purpose::STANDARD
    .decode(text.trim())
    .map_err(|why| format!("{}: {why}", key.display()))?;
  let seed: [u8; 32] = seed.try_into().map_err(|_| format!("{}: not a 32-byte seed", key.display()))?;
  let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
  let signature = ed25519_dalek::Signer::sign(&signing, digest);

  Ok(base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()))
}

// The release built in the build tree: the Cargo workspace for the floor, the binaries placed in the release's
// `bin/` with `built-from`, then the declared build steps, which write into the release beside the build tree.
fn build(
  declaration: &Declaration,
  stage: &Stage,
  target_dir: &Path,
  arch: &str,
  floor: &str,
  version: &str,
  commit: &str,
) -> Result<(), String> {
  let failed = |step: &str, why: String| said(&ARTIFACTS.said.build_failed, &[("step", step), ("said", &why)]);
  let build = &declaration.build;

  if stage.build.join("Cargo.toml").exists() {
    let triple = format!("{arch}-unknown-linux-gnu");
    let profile = &build.cargo_profile;

    ran(
      Command::new("cargo")
        .args(["zigbuild", "--workspace", "--locked", "--profile", profile, "--target"])
        .arg(format!("{triple}.{floor}"))
        .arg("--target-dir")
        .arg(target_dir)
        .current_dir(&stage.build)
        .env("BMO_VERSION", version),
    )
    .map_err(|why| failed("cargo zigbuild", why))?;

    let binaries = match &build.binaries {
      Some(declared) => declared.clone(),
      None => {
        let metadata = ran(
          Command::new("cargo")
            .args(["metadata", "--no-deps", "--offline", "--format-version", "1"])
            .current_dir(&stage.build),
        )
        .map_err(|why| failed("cargo metadata", why))?;

        workspace_binaries(&metadata).map_err(|why| failed("cargo metadata", why))?
      }
    };
    let built = target_dir.join(&triple).join(if profile == "dev" { "debug" } else { profile.as_str() });
    let bin = stage.release.join("bin");

    std::fs::create_dir_all(&bin).map_err(|why| why.to_string())?;

    for binary in &binaries {
      std::fs::copy(built.join(binary), bin.join(binary)).map_err(|why| failed("place", format!("{binary}: {why}")))?;
    }

    std::fs::write(bin.join("built-from"), format!("{commit}\n")).map_err(|why| why.to_string())?;
  }

  for step in &build.steps {
    let dir = step.dir.as_ref().map_or_else(|| stage.build.clone(), |dir| stage.build.join(dir));

    ran(Command::new("sh").arg("-c").arg(&step.command).current_dir(dir).env("BMO_VERSION", version))
      .map_err(|why| failed(&step.id, why))?;
  }

  Ok(())
}
