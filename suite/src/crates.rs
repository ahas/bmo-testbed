//! The crates of the Rust workspace, as Cargo itself reads them (docs/checks.md, 「Which tests a change
//! runs」), and the paths every crate stands on. Nothing here keeps a list of crates.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// The paths at a checkout's root that every crate stands on: the workspace's manifest, its lockfile and
/// the formatting every crate is checked against; and Cargo's own configuration directory.
pub const CARGO_ROOT_CONFIGURATION: [&str; 3] = ["Cargo.toml", "Cargo.lock", "rustfmt.toml"];
pub const CARGO_CONFIGURATION_DIR: &str = ".cargo/";

/// One crate of the workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrateDeclaration {
  /// The crate's directory from the checkout's root, or its whole path where it stands outside the root.
  pub dir: String,
  pub name: String,
  /// The crates of this workspace it depends on, by name.
  pub dependencies: Vec<String>,
  /// Globs for paths outside the workspace its tests read, declared in its manifest's
  /// `[package.metadata.bmo]`.
  pub reads: Vec<String>,
  /// The binaries it builds, by name.
  pub binaries: Vec<String>,
  /// Whether it has a library of its own.
  pub library: bool,
  /// Its test targets, the integration tests Cargo builds as binaries of their own.
  pub tests: Vec<TestTarget>,
}

/// One test target of a crate: its name, which `--test` takes, and its source from the checkout's root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestTarget {
  pub name: String,
  pub path: String,
}

/// Whether a path, from the checkout's root, is one of the workspace's own files rather than a package's: its
/// root configuration, which every crate stands on.
pub fn is_cargo_path(path: &str) -> bool {
  path.starts_with(CARGO_CONFIGURATION_DIR) || CARGO_ROOT_CONFIGURATION.contains(&path)
}

/// Whether a path is the workspace's: one of its own files, or a path inside a crate's directory wherever the
/// workspace lists it, a Rust tool's folder under `tools/` as much as a crate in a role folder.
pub fn is_workspace_path(crates: &[CrateDeclaration], path: &str) -> bool {
  is_cargo_path(path) || owning_crate(crates, path).is_some()
}

/// The account's own home, which a test's `HOME` does not move: the toolchain stands there.
pub fn account_home() -> PathBuf {
  // The reentrant lookup, into a record and a buffer of this call's own: the plain one answers a record every
  // other thread's lookup writes over.
  // SAFETY: an all-zero `passwd` is a valid value of the plain C struct the lookup writes into.
  let mut record: libc::passwd = unsafe { std::mem::zeroed() };
  let mut buffer = vec![0 as libc::c_char; 16 * 1024];
  let mut found: *mut libc::passwd = std::ptr::null_mut();
  // SAFETY: the record, the buffer and the answer pointer are this call's own and outlive the call; the
  // buffer's length is the one handed.
  let read = unsafe {
    libc::getpwuid_r(libc::getuid(), &raw mut record, buffer.as_mut_ptr(), buffer.len(), &raw mut found)
  };
  let from_passwd = (read == 0 && !found.is_null() && !record.pw_dir.is_null())
    // SAFETY: a record found points its home into the buffer, which still stands.
    .then(|| unsafe { std::ffi::CStr::from_ptr(record.pw_dir) }.to_string_lossy().into_owned());

  from_passwd
    .or_else(|| std::env::var("HOME").ok())
    .map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// Whether a variable is one Cargo sets for a process it runs for a package, as its documentation lists them
/// for crates. A Cargo started with one inherited would build as though for that package, so every verb
/// running Cargo takes them off (docs/deploy.md, 「The Cargo build directory」).
pub fn set_by_cargo(name: &str) -> bool {
  const NAMED: [&str; 9] = [
    "CARGO",
    "CARGO_MANIFEST_DIR",
    "CARGO_MANIFEST_PATH",
    "CARGO_CRATE_NAME",
    "CARGO_BIN_NAME",
    "CARGO_PRIMARY_PACKAGE",
    "CARGO_TARGET_TMPDIR",
    "CARGO_RUSTC_CURRENT_DIR",
    "OUT_DIR",
  ];

  NAMED.contains(&name) || name.starts_with("CARGO_PKG_") || name.starts_with("CARGO_BIN_EXE_")
}

/// The toolchain's homes, read off the account where the environment names none.
pub fn toolchain_environment() -> Vec<(String, String)> {
  let account = account_home();
  let named = |variable: &str, fallback: PathBuf| {
    std::env::var(variable)
      .ok()
      .filter(|value| !value.is_empty())
      .unwrap_or_else(|| fallback.to_string_lossy().into_owned())
  };

  vec![
    ("CARGO_HOME".to_owned(), named("CARGO_HOME", account.join(".cargo"))),
    ("RUSTUP_HOME".to_owned(), named("RUSTUP_HOME", account.join(".rustup"))),
  ]
}

/// A path as the runtime's own resolution writes it: absolute, with `.` and `..` read off.
pub fn resolved(path: &Path) -> PathBuf {
  let whole = if path.is_absolute() {
    path.to_path_buf()
  } else {
    std::env::current_dir().unwrap_or_default().join(path)
  };
  let mut parts = PathBuf::from("/");

  for part in whole.components() {
    match part {
      std::path::Component::Normal(name) => parts.push(name),
      std::path::Component::ParentDir => {
        parts.pop();
      }
      _ => {}
    }
  }

  parts
}

/// The build directory every checkout's Cargo builds in, `BMO_CARGO_TARGET_DIR` naming another
/// (docs/deploy.md, 「The Cargo build directory」).
pub fn cargo_target_root() -> PathBuf {
  std::env::var_os("BMO_CARGO_TARGET_DIR").filter(|named| !named.is_empty()).map_or_else(
    || account_home().join(".local/share/bmo/cargo-target"),
    |named| resolved(Path::new(&named)),
  )
}

/// The file in a checkout's own target directory naming the checkout's path, which the prune reads.
pub const CHECKOUT_RECORD_NAME: &str = "checkout-root";

/// The name of the checkout's own target directory: the checkout's directory and a hash of its whole path,
/// so two checkouts with one directory name stay apart.
pub fn checkout_target_name(root: &Path) -> String {
  use sha2::{Digest, Sha256};

  let path = resolved(root);
  let hash = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
  let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();

  format!("{name}-{}", &hash[..12])
}

/// The checkout's own target directory under the shared one, by its name.
pub fn checkout_target_dir(root: &Path) -> PathBuf {
  cargo_target_root().join("checkouts").join(checkout_target_name(root))
}

/// A file or a link put in place by a rename, so a build reading it never meets it half made.
pub(crate) fn put_in_place(path: &Path, make: impl Fn(&Path) -> std::io::Result<()>) {
  let made = path.with_extension(format!("{}", std::process::id()));

  if let Some(dir) = path.parent() {
    let _ = std::fs::create_dir_all(dir);
  }

  let _ = std::fs::remove_file(&made);

  if make(&made).is_ok() {
    let _ = std::fs::rename(&made, path);
  }
}

/// A program by name, the way the PATH finds it.
pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
  std::env::var_os("PATH").and_then(|path| {
    std::env::split_paths(&path)
      .map(|dir| dir.join(name))
      .find(|candidate| candidate.is_file())
  })
}

/// The checkout's workspace wrapper: a link to `env`, which Cargo mixes into the workspace crates' output
/// names so each checkout's crates stay apart while the dependencies are shared. It is the link committed
/// beside the checkout's Cargo configuration, by the path Cargo reads it by in a build typed by hand —
/// under the checkout as the kernel names it — so a verb's build and one typed by hand give the
/// checkout's crates one set of names; a commit holding no such link gets one in its own target directory
/// (docs/deploy.md, 「The Cargo build directory」).
fn compiler_wrapper(root: &Path, target: &Path) -> PathBuf {
  let real = std::fs::canonicalize(root).unwrap_or_else(|_| resolved(root));
  let committed = real.join(".cargo").join("workspace-rustc");

  if std::fs::symlink_metadata(&committed).is_ok_and(|held| held.file_type().is_symlink()) {
    return committed;
  }

  let wrapper = target.join("workspace-rustc");
  let program = on_path("env").unwrap_or_else(|| PathBuf::from("/usr/bin/env"));

  if std::fs::read_link(&wrapper).ok().as_deref() != Some(program.as_path()) {
    put_in_place(&wrapper, |made| std::os::unix::fs::symlink(&program, made));
  }

  wrapper
}

/// The environment Cargo builds in for the checkout at `root`: the checkout's own directory under the build root
/// as its build and target directory at once, its workspace wrapper, and the toolchain's homes. The checkout's
/// path is written into its directory, which the prune reads.
pub fn cargo_environment(root: &Path) -> Vec<(String, String)> {
  let target = checkout_target_dir(root);
  let checkout = resolved(root).to_string_lossy().into_owned();
  let record = target.join(CHECKOUT_RECORD_NAME);

  if std::fs::read_to_string(&record).ok().as_deref() != Some(checkout.as_str()) {
    put_in_place(&record, |made| std::fs::write(made, &checkout));
  }

  let wrapper = compiler_wrapper(root, &target);

  // Cargo locks a build directory for a whole build, so each checkout builds in its own and the checkouts'
  // builds run side by side.
  let mut env = vec![
    ("CARGO_BUILD_BUILD_DIR".to_owned(), target.to_string_lossy().into_owned()),
    ("CARGO_TARGET_DIR".to_owned(), target.to_string_lossy().into_owned()),
    ("RUSTC_WORKSPACE_WRAPPER".to_owned(), wrapper.to_string_lossy().into_owned()),
  ];

  env.extend(toolchain_environment());
  env
}

/// The `cargo` to run: the one in the Cargo home, where the toolchain installer put it, else the PATH's.
pub fn cargo_program(cargo_home: &str) -> String {
  let installed = Path::new(cargo_home).join("bin").join("cargo");

  if installed.exists() {
    installed.to_string_lossy().into_owned()
  } else {
    "cargo".to_owned()
  }
}

fn relative_dir(root: &Path, manifest: &str) -> String {
  let manifest = Path::new(manifest);
  let dir = manifest.parent().unwrap_or(manifest);

  let Ok(inside) = dir.strip_prefix(root) else {
    return dir.to_string_lossy().into_owned();
  };

  inside
    .components()
    .map(|part| part.as_os_str().to_string_lossy().into_owned())
    .collect::<Vec<_>>()
    .join("/")
}

/// The crates of the checkout's workspace, as `cargo metadata` reads them without resolving anything beyond
/// the workspace, so nothing is fetched. A checkout with no workspace has none; a workspace Cargo cannot
/// read is refused, since a selection that guessed would leave a crate's tests unrun.
pub fn workspace_crates(root: &Path) -> Result<Vec<CrateDeclaration>, String> {
  if !root.join("Cargo.toml").exists() {
    return Ok(Vec::new());
  }

  let toolchain = toolchain_environment();
  let cargo = cargo_program(&toolchain[0].1);
  let done = Command::new(cargo)
    .args(["metadata", "--no-deps", "--offline", "--format-version", "1"])
    .current_dir(root)
    .envs(toolchain.iter().map(|(name, value)| (name, value)))
    .stdin(Stdio::null())
    .output()
    .map_err(|why| why.to_string())?;

  if !done.status.success() {
    return Err(format!(
      "cargo metadata in {}: {}",
      root.display(),
      String::from_utf8_lossy(&done.stderr).trim()
    ));
  }

  let metadata: Value = serde_json::from_slice(&done.stdout).map_err(|why| why.to_string())?;
  let members: Vec<&str> = metadata["workspace_members"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .collect();
  let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
  let crates: Vec<&Value> = metadata["packages"]
    .as_array()
    .into_iter()
    .flatten()
    .filter(|one| one["id"].as_str().is_some_and(|id| members.contains(&id)))
    .collect();
  let by_dir: Vec<(String, String)> = crates
    .iter()
    .map(|one| {
      (
        relative_dir(&root, one["manifest_path"].as_str().unwrap_or_default()),
        one["name"].as_str().unwrap_or_default().to_owned(),
      )
    })
    .collect();
  let strings = |value: &Value| -> Vec<String> {
    value
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(|one| one.as_str().map(str::to_owned))
      .collect()
  };

  Ok(
    crates
      .iter()
      .zip(&by_dir)
      .map(|(one, (dir, name))| CrateDeclaration {
        dir: dir.clone(),
        name: name.clone(),
        dependencies: one["dependencies"]
          .as_array()
          .into_iter()
          .flatten()
          .filter_map(|dependency| {
            let path = dependency["path"].as_str()?;
            let at = Path::new(path).strip_prefix(&root).ok()?.to_string_lossy().into_owned();

            by_dir.iter().find(|(dir, _)| *dir == at).map(|(_, name)| name.clone())
          })
          .collect(),
        reads: strings(&one["metadata"]["bmo"]["reads"]),
        binaries: targets(one)
          .filter(|target| of_kind(target, &["bin"]))
          .filter_map(|target| target["name"].as_str().map(str::to_owned))
          .collect(),
        library: targets(one).any(|target| of_kind(target, LIBRARY_KINDS)),
        tests: targets(one)
          .filter(|target| of_kind(target, &["test"]))
          .filter_map(|target| {
            let source = Path::new(target["src_path"].as_str()?);

            Some(TestTarget {
              name: target["name"].as_str()?.to_owned(),
              path: source.strip_prefix(&root).ok()?.to_string_lossy().into_owned(),
            })
          })
          .collect(),
      })
      .collect(),
  )
}

// The target kinds Cargo builds as a crate's library, the ones its `--lib` names.
const LIBRARY_KINDS: &[&str] = &["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"];

fn targets(package: &Value) -> impl Iterator<Item = &Value> {
  package["targets"].as_array().into_iter().flatten()
}

fn of_kind(target: &Value, kinds: &[&str]) -> bool {
  target["kind"]
    .as_array()
    .is_some_and(|named| named.iter().any(|kind| kind.as_str().is_some_and(|kind| kinds.contains(&kind))))
}

/// The crate a path lies in, the one whose directory is the longest that holds it; none where none does. A
/// crate standing at the workspace's root has no directory of its own here: it holds what nothing else of
/// the layout holds, which is the selection's rule.
pub fn owning_crate<'a>(crates: &'a [CrateDeclaration], path: &str) -> Option<&'a CrateDeclaration> {
  crates
    .iter()
    .filter(|one| !one.dir.is_empty() && path.starts_with(&format!("{}/", one.dir)))
    .max_by_key(|one| one.dir.len())
}

/// A crate's tests as the suite runs them: that crate alone, since Cargo settles the features of every
/// dependency over all the packages one command names (docs/suite.md, 「Heavy units run in a lane of one」).
pub fn crate_test_args(name: &str) -> Vec<String> {
  ["test", "--locked", "-p", name].map(str::to_owned).to_vec()
}

/// A crate's library's own tests alone, in the words of its whole tests and so with their features, which
/// a build of the whole after them finds built (docs/suite.md, 「Heavy units run in a lane of one」).
pub fn crate_library_test_args(name: &str) -> Vec<String> {
  let mut args = crate_test_args(name);

  args.push("--lib".to_owned());
  args
}

/// A crate's binaries as the testkit builds them: that crate alone, for the same reason as its tests.
pub fn crate_build_args(name: &str, binaries: &[String]) -> Vec<String> {
  let mut args: Vec<String> = ["build", "--locked", "-p", name].map(str::to_owned).to_vec();

  for binary in binaries {
    args.extend(["--bin".to_owned(), binary.clone()]);
  }

  args
}
