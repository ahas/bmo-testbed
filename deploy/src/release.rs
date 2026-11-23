//! The deploy tree as a link onto one release among several (docs/deploy.md, 「The releases」): each
//! deployed commit a detached worktree of its own beside the link, finished — set up and its binaries
//! placed — before the link is moved onto it by one rename, so every reader sees one release or the other
//! and a running process keeps the release it started from.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;

use crate::remove::remove_tree;
use crate::run::Ran;
use crate::tree::{BUILT_FROM, short};
use crate::unit::first_chars;

/// The variable naming the release a process reads, handed on to what it starts.
pub const DEPLOY_VARIABLE: &str = "BMO_DEPLOY_DIR";

/// Runs git in a directory, answering whether it passed and what it printed.
pub type GitRun<'a> = &'a dyn Fn(&[&str], &Path) -> Ran;

/// A directory beside the deploy tree's link, named after it with a suffix, so a moved deploy tree moves
/// its neighbours with it. A release named in the link's place is read back to the link first.
pub fn beside(deploy_dir: &Path, suffix: &str) -> PathBuf {
  let deploy_dir = &link_of(deploy_dir);
  let name = deploy_dir
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_default();

  deploy_dir
    .parent()
    .map(Path::to_path_buf)
    .unwrap_or_default()
    .join(format!("{name}{suffix}"))
}

/// The directory the releases stand in: `deploy-releases` beside a deploy tree named `deploy`.
pub fn releases_dir(deploy_dir: &Path) -> PathBuf {
  beside(deploy_dir, "-releases")
}

/// The suffix a retired tree's name carries after the deploy tree's, ahead of its time.
pub const RETIRED: &str = ".retired-";

/// The release of a commit: named by its first twelve hex digits.
pub fn release_dir(deploy_dir: &Path, commit: &str) -> PathBuf {
  releases_dir(deploy_dir).join(short(commit))
}

/// Whether a working tree is one of the deployment's own — the release the link names or any other, or
/// the build tree — and so nobody's checkout to work in. Compared by real path, since a process started
/// through the link stands in the release it names.
pub fn is_deployment_tree(deploy_dir: &Path, tree: &Path) -> bool {
  let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
  let tree = real(tree);

  tree == real(deploy_dir)
    || tree == real(&crate::expansion::expansion_tree(deploy_dir))
    || tree.parent() == Some(real(&releases_dir(deploy_dir)).as_path())
}

/// The deploy tree's link a path names: a release under `<name>-releases` read back to the `<name>` it
/// stands beside, any other path as it is. A process pinned to its release names the release where the
/// link is asked for, and what stands beside the tree is found from the link alone.
pub fn link_of(path: &Path) -> PathBuf {
  let link = path
    .file_name()
    .filter(|name| is_release_name(&name.to_string_lossy()))
    .and(path.parent())
    .and_then(|store| {
      let name = store.file_name()?.to_str()?.strip_suffix("-releases")?;

      (!name.is_empty()).then(|| store.parent().map(|parent| parent.join(name)))?
    });

  link.unwrap_or_else(|| path.to_path_buf())
}

/// The release of the deploy tree at `deploy_dir` a path lies in, by its real path; none for a path in
/// no release.
pub fn release_holding(deploy_dir: &Path, path: &Path) -> Option<PathBuf> {
  let store = std::fs::canonicalize(releases_dir(&link_of(deploy_dir))).ok()?;
  let real = std::fs::canonicalize(path).ok()?;
  let name = real.strip_prefix(&store).ok()?.components().next()?.as_os_str().to_str()?.to_owned();

  is_release_name(&name).then(|| store.join(name))
}

/// The release the deploy tree's link names now, by its real path. None where the tree is no link onto a
/// release.
pub fn live_release_real(deploy_dir: &Path) -> Option<PathBuf> {
  live_release(deploy_dir).and_then(|release| release_holding(deploy_dir, &release))
}

// The directories standing directly under a place, in name order; none where the place does not stand.
fn standing_dirs(dir: &Path) -> Vec<PathBuf> {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return vec![];
  };
  let mut dirs: Vec<PathBuf> = entries
    .flatten()
    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    .map(|entry| entry.path())
    .collect::<Vec<PathBuf>>();

  dirs.sort();
  dirs
}

/// What stands beside the deploy tree that a deployment makes again from a checkout: every release but the
/// one the link names, the build tree, and every retired tree. The link and the release it names are what a
/// restore needs, so they are not among them; neither is a tree that does not stand. A release named in
/// the link's place is read back to the link first, so a pinned reader gets the same list.
pub fn rebuilt_beside(deploy_dir: &Path) -> Vec<PathBuf> {
  let link = link_of(deploy_dir);
  let name =
    |path: &Path| path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
  let live = live_release(&link).map(|release| name(&release)).unwrap_or_default();
  let retired = format!("{}{RETIRED}", name(&link));

  let mut left_out: Vec<PathBuf> = [crate::expansion::expansion_tree(&link)]
    .into_iter()
    .filter(|tree| tree.is_dir())
    .chain(standing_dirs(&releases_dir(&link)).into_iter().filter(|release| {
      let entry = name(release);

      is_release_name(&entry) && live != entry
    }))
    .chain(
      standing_dirs(link.parent().unwrap_or(Path::new("/")))
        .into_iter()
        .filter(|tree| name(tree).starts_with(&retired)),
    )
    .collect();

  left_out.sort();
  left_out
}

/// Whether a name is one a release is made under: twelve lower-case hex digits.
pub fn is_release_name(name: &str) -> bool {
  name.len() == 12 && name.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// What stands at the deploy tree's path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
  /// Nothing: a first deployment.
  Nothing,
  /// A link, and the release it names, read against the directory the link stands in.
  Release(PathBuf),
  /// A directory or a file, which no rename of a link goes over.
  NotLink,
}

/// What stands at the deploy tree's path now.
pub fn standing(deploy_dir: &Path) -> Standing {
  match std::fs::symlink_metadata(deploy_dir) {
    Err(_) => Standing::Nothing,
    Ok(meta) if meta.file_type().is_symlink() => {
      let named = std::fs::read_link(deploy_dir).unwrap_or_default();

      Standing::Release(deploy_dir.parent().map_or_else(|| named.clone(), |parent| parent.join(&named)))
    }
    Ok(_) => Standing::NotLink,
  }
}

/// The release the deploy tree's link names, or none where it names nothing.
pub fn live_release(deploy_dir: &Path) -> Option<PathBuf> {
  match standing(deploy_dir) {
    Standing::Release(release) => Some(release),
    _ => None,
  }
}

/// The commit a release's `bin/` was built from, or none where its build did not finish.
pub fn built_from(release: &Path) -> Option<String> {
  std::fs::read_to_string(release.join("bin").join(BUILT_FROM))
    .ok()
    .map(|text| text.trim().to_owned())
    .filter(|commit| !commit.is_empty())
}

/// Whether the release of a commit stands whole: on that commit, and its `bin/` built from it.
pub fn stands_built(release: &Path, commit: &str, git: GitRun<'_>) -> bool {
  release.join(".git").exists()
    && built_from(release).as_deref() == Some(commit)
    && git(&["rev-parse", "HEAD"], release).out.trim() == commit
}

/// The repository's primary checkout, which a deployment reads its repository through once it has picked its
/// commit (docs/deploy.md, 「The releases」): the main worktree of the git directory the checkout shares, or the
/// checkout itself where git names none that stands, as for a bare repository.
pub fn primary_checkout(checkout: &Path, git: GitRun<'_>) -> PathBuf {
  let listed = git(&["worktree", "list", "--porcelain"], checkout);
  // Git's own porcelain listing: one stanza of `key value` lines for each worktree, the main worktree's first,
  // opening with `worktree <path>` and holding a line `bare` where the repository has no work tree.
  let main_stanza = listed.out.split("\n\n").next().unwrap_or_default();
  let main = main_stanza
    .lines()
    .next()
    .and_then(|line| line.strip_prefix("worktree "))
    .map(PathBuf::from)
    .filter(|main| !main_stanza.lines().any(|line| line == "bare") && main.is_dir());

  match main {
    Some(main) if listed.ok => main,
    _ => checkout.to_path_buf(),
  }
}

/// A release prepared for a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
  pub release: PathBuf,
  /// Whether it was made afresh, rather than taken as it stood.
  pub made: bool,
}

/// What a release is prepared out of.
pub struct Preparing<'a> {
  /// The repository the release is a worktree of.
  pub source: &'a Path,
  pub deploy_dir: &'a Path,
  pub commit: &'a str,
  /// The data root, which no removal reaches.
  pub data_root: &'a Path,
}

/// The release of the commit, taken as it stands where it stands built, otherwise made afresh: whatever a
/// cut-off run left under its name taken away, a detached worktree added at the commit and set up by
/// `setup` — its submodules, the declared setup steps and the env links.
pub fn prepare(
  preparing: &Preparing<'_>,
  git: GitRun<'_>,
  setup: &dyn Fn(&Path) -> Result<(), String>,
  say: &dyn Fn(&str),
) -> Result<Prepared, String> {
  let release = release_dir(preparing.deploy_dir, preparing.commit);
  let made = !stands_built(&release, preparing.commit, git);

  if made {
    // The release the deploy tree names is what runs; one that does not stand built is never made again
    // over it.
    if live_release(preparing.deploy_dir).is_some_and(|live| same_place(&live, &release)) {
      return Err(fill_shown(
        &DEPLOY.release.live_unbuilt,
        &[("release", &release.display()), ("commit", &short(preparing.commit))],
      ));
    }

    if std::fs::symlink_metadata(&release).is_ok() {
      say(&fill_shown(&DEPLOY.release.leftover, &[("release", &release.display())]));
      remove_tree(&release, preparing.data_root)?;
    }

    // A release taken away is still registered, and git refuses a second worktree under a name it thinks
    // it already has; the prune forgets only registrations whose directory is gone.
    git(&["worktree", "prune"], preparing.source);
    std::fs::create_dir_all(releases_dir(preparing.deploy_dir)).map_err(|why| why.to_string())?;

    let added = git(
      &["worktree", "add", "--detach", &release.to_string_lossy(), preparing.commit],
      preparing.source,
    );

    if !added.ok {
      return Err(fill_shown(
        &DEPLOY.release.not_added,
        &[
          ("release", &release.display()),
          ("commit", &short(preparing.commit)),
          ("said", &first_chars(&added.out, DEPLOY.numbers.quoted_chars)),
        ],
      ));
    }

    setup(&release)?;
  }

  Ok(Prepared { release, made })
}

/// A link naming `target` put at a place by one rename over whatever link or file stood there, a folder
/// taken away first; a link already naming it is left alone.
pub fn put_link(link: &Path, target: &Path) -> std::io::Result<()> {
  match std::fs::symlink_metadata(link) {
    Ok(meta) if meta.file_type().is_symlink() => {
      if std::fs::read_link(link).is_ok_and(|named| named == target) {
        return Ok(());
      }
    }
    Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(link)?,
    _ => {}
  }

  let name = link.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
  let staged = link.with_file_name(format!(".{name}.link-{}", std::process::id()));
  let _ = std::fs::remove_file(&staged);

  std::os::unix::fs::symlink(target, &staged)?;
  std::fs::rename(&staged, link).inspect_err(|_| {
    let _ = std::fs::remove_file(&staged);
  })
}

/// A folder's files hard-linked into another place, its directories made afresh.
pub fn hard_link_tree(from: &Path, to: &Path) -> std::io::Result<()> {
  std::fs::create_dir_all(to)?;

  for entry in std::fs::read_dir(from)? {
    let entry = entry?;
    let kind = entry.file_type()?;
    let target = to.join(entry.file_name());

    if kind.is_dir() {
      hard_link_tree(&entry.path(), &target)?;
    } else if kind.is_symlink() {
      std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, target)?;
    } else {
      std::fs::hard_link(entry.path(), target)?;
    }
  }

  Ok(())
}

/// Whether two paths name the same place once every link on the way is followed.
pub fn same_place(one: &Path, other: &Path) -> bool {
  match (std::fs::canonicalize(one), std::fs::canonicalize(other)) {
    (Ok(one), Ok(other)) => one == other,
    _ => one == other,
  }
}

// Whether two files hold the same bytes: the same file by its inode, or equal read whole.
fn same_bytes(one: &Path, other: &Path) -> bool {
  use std::os::unix::fs::MetadataExt;

  match (std::fs::metadata(one), std::fs::metadata(other)) {
    (Ok(a), Ok(b)) if a.dev() == b.dev() && a.ino() == b.ino() => true,
    (Ok(a), Ok(b)) if a.len() != b.len() => false,
    (Ok(_), Ok(_)) => matches!((std::fs::read(one), std::fs::read(other)), (Ok(a), Ok(b)) if a == b),
    _ => false,
  }
}

/// Each binary built into `built` placed into the release's `bin`: hard-linked to the previous release's
/// file of its name where the bytes are equal, copied in where they differ or there is none. Hands back
/// the ones that differ, which are what the deployment restarts; a binary missing from `built` is an error
/// naming its path.
pub fn place_binaries(
  binaries: &[String],
  built: &Path,
  previous_bin: Option<&Path>,
  bin: &Path,
) -> std::io::Result<Vec<String>> {
  std::fs::create_dir_all(bin)?;

  let mut changed = vec![];

  for binary in binaries {
    let from = built.join(binary);
    let to = bin.join(binary);
    let previous = previous_bin.map(|dir| dir.join(binary)).filter(|previous| same_bytes(previous, &from));
    let staged = bin.join(format!("{binary}.new"));
    let _ = std::fs::remove_file(&staged);

    // A hard link is the same bytes, the same mode, and no second copy on disk; one refused — another
    // filesystem — is copied instead.
    let linked = previous.as_ref().is_some_and(|previous| std::fs::hard_link(previous, &staged).is_ok());

    if !linked {
      // A binary gone from the build names the file looked for, since the error alone names none.
      std::fs::copy(&from, &staged)
        .map_err(|why| std::io::Error::new(why.kind(), format!("{}: {why}", from.display())))?;
      std::fs::set_permissions(&staged, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    }

    std::fs::rename(&staged, &to)?;
    // A rename onto a link of the same file does nothing and leaves the staged name standing.
    let _ = std::fs::remove_file(&staged);

    if previous.is_none() {
      changed.push(binary.clone());
    }
  }

  Ok(changed)
}

/// The binaries whose bytes differ between two releases' `bin/`, one missing from either counting; the
/// mark of what each was built from is not a binary.
pub fn differing_binaries(one: &Path, other: &Path) -> Vec<String> {
  let names = |bin: &Path| -> BTreeSet<String> {
    std::fs::read_dir(bin)
      .map(|entries| {
        entries
          .flatten()
          .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
          .map(|entry| entry.file_name().to_string_lossy().into_owned())
          .filter(|name| name != BUILT_FROM && !name.ends_with(".new"))
          .collect()
      })
      .unwrap_or_default()
  };
  let all: BTreeSet<String> = names(&one.join("bin")).union(&names(&other.join("bin"))).cloned().collect();

  all
    .into_iter()
    .filter(|name| !same_bytes(&one.join("bin").join(name), &other.join("bin").join(name)))
    .collect()
}

/// The deploy tree's link moved onto a release: a new link written beside it and renamed over it, so every
/// path through the deploy tree names the one release or the other. The link names the release relative to
/// the directory it stands in, so the pair moves together.
pub fn flip(deploy_dir: &Path, release: &Path) -> std::io::Result<()> {
  let parent = deploy_dir.parent().unwrap_or(Path::new("/"));
  let target = release.strip_prefix(parent).map_or_else(|_| release.to_path_buf(), Path::to_path_buf);
  let name = deploy_dir
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_default();
  let staged = parent.join(format!(".{name}.flip-{}", std::process::id()));
  let _ = std::fs::remove_file(&staged);

  std::fs::create_dir_all(parent)?;
  std::os::unix::fs::symlink(&target, &staged)?;

  std::fs::rename(&staged, deploy_dir).inspect_err(|_| {
    let _ = std::fs::remove_file(&staged);
  })
}

/// One live process of this account as `/proc` says of it: where it stands, what it names as its code
/// tree, and the unit it runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveProcess {
  pub pid: u32,
  /// The unit its cgroup runs it in, without its `.service` or `.scope`; none where its cgroup names none.
  pub unit: Option<String>,
  /// Its command line, its arguments joined by spaces and cut short.
  pub command: String,
  pub cwd: Option<PathBuf>,
  pub exe: Option<PathBuf>,
  /// Whether its environment names a `BMO_DEPLOY_DIR`, empty counting as unnamed.
  pub names_deploy_dir: bool,
  /// The real tree its `BMO_DEPLOY_DIR` names; none where it names a link, which it follows, or nothing
  /// that stands.
  pub named_tree: Option<PathBuf>,
  /// Whether its working directory, its executable and its environment could all be read.
  pub read: bool,
}

impl LiveProcess {
  // Whether it stands inside a tree read by its real path: its working directory or its executable lies
  // in it, or its `BMO_DEPLOY_DIR` names it.
  fn stands_in_real(&self, real: &Path) -> bool {
    [&self.cwd, &self.exe, &self.named_tree]
      .into_iter()
      .flatten()
      .any(|place| place.starts_with(real))
  }

  /// Whether it stands inside a tree.
  pub fn stands_in(&self, tree: &Path) -> bool {
    std::fs::canonicalize(tree).is_ok_and(|real| self.stands_in_real(&real))
  }
}

/// Every live process of this account under a `/proc`, each read for where it stands; none where the
/// directory itself could not be listed. A process gone between the listing and its reading is left out.
pub fn live_processes(proc_root: &Path) -> Option<Vec<LiveProcess>> {
  use std::os::unix::fs::MetadataExt;

  let own = std::fs::metadata("/proc/self").ok()?.uid();
  let mut found = vec![];

  for entry in std::fs::read_dir(proc_root).ok()?.flatten() {
    let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
      continue;
    };
    let dir = entry.path();

    if std::fs::metadata(&dir).map(|meta| meta.uid()).ok() != Some(own) {
      continue;
    }

    let Ok(cgroup) = std::fs::read_to_string(dir.join("cgroup")) else {
      continue;
    };

    // A process that has exited but not been reaped reads no code and holds no place.
    if exited(&dir) {
      continue;
    }

    let cwd = std::fs::read_link(dir.join("cwd")).ok();
    let exe = std::fs::read_link(dir.join("exe")).ok();
    let environ = std::fs::read(dir.join("environ")).ok();
    let named = environ.as_deref().and_then(deploy_variable);
    let named_tree = named.as_deref().and_then(|named| {
      let named = Path::new(named);
      let link = std::fs::symlink_metadata(named).is_ok_and(|meta| meta.file_type().is_symlink());

      if link { None } else { std::fs::canonicalize(named).ok() }
    });
    let command = std::fs::read(dir.join("cmdline"))
      .map(|line| {
        line
          .split(|byte| *byte == 0)
          .filter(|part| !part.is_empty())
          .map(|part| String::from_utf8_lossy(part).into_owned())
          .collect::<Vec<_>>()
          .join(" ")
      })
      .unwrap_or_default();

    found.push(LiveProcess {
      pid,
      unit: unit_of_cgroup(&cgroup),
      command: first_chars(&command, DEPLOY.numbers.command_chars),
      read: cwd.is_some() && exe.is_some() && environ.is_some(),
      cwd,
      exe,
      names_deploy_dir: named.is_some(),
      named_tree,
    });
  }

  found.sort_by_key(|process| process.pid);

  Some(found)
}

// Whether a process has exited, its parent not yet having reaped it, or is gone: its state, the field after
// its parenthesised name in `stat`, is a zombie's or a dead one's, or `stat` is gone.
fn exited(dir: &Path) -> bool {
  let Ok(stat) = std::fs::read_to_string(dir.join("stat")) else {
    return true;
  };

  stat
    .rfind(')')
    .and_then(|at| stat.get(at + 2..))
    .and_then(|after| after.chars().next())
    .is_some_and(|state| matches!(state, 'Z' | 'X'))
}

// What an environment's `BMO_DEPLOY_DIR` holds, out of its NUL-separated entries; none where it is unset
// or empty.
fn deploy_variable(environ: &[u8]) -> Option<String> {
  let key = format!("{DEPLOY_VARIABLE}=");

  environ
    .split(|byte| *byte == 0)
    .find_map(|entry| entry.strip_prefix(key.as_bytes()))
    .map(|value| String::from_utf8_lossy(value).into_owned())
    .filter(|value| !value.is_empty())
}

// The unit a process's cgroup runs it in: the innermost part of its path that is a service or a scope.
fn unit_of_cgroup(cgroup: &str) -> Option<String> {
  let path = cgroup
    .lines()
    .find_map(|line| line.strip_prefix("0::"))
    .or_else(|| cgroup.lines().last().and_then(|line| line.rsplit(':').next()))?;

  path
    .rsplit('/')
    .find_map(|part| part.strip_suffix(".service").or_else(|| part.strip_suffix(".scope")))
    .map(str::to_owned)
}

/// What a prune of the releases came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pruned {
  pub removed: Vec<String>,
  pub warnings: Vec<String>,
  /// How many releases stand once it is done.
  pub standing: usize,
  /// Each release kept only because a live process stands in it, with the processes that do.
  pub pinned: Vec<(String, Vec<LiveProcess>)>,
}

/// The detail of the `releases-pinned` event a prune sends where more releases stand than the policy
/// tells above (docs/boundary.md): each release a process holds, with the processes holding it. None
/// where no more stand than that.
pub fn pinned_detail(pruned: &Pruned) -> Option<serde_json::Value> {
  (pruned.standing > DEPLOY.numbers.releases_told_above).then(|| {
    serde_json::json!({
      "releases": pruned.pinned.iter().map(|(release, processes)| serde_json::json!({
        "release": release,
        "processes": processes.iter().map(|process| serde_json::json!({
          "pid": process.pid,
          "unit": process.unit,
          "command": process.command,
        })).collect::<Vec<_>>(),
      })).collect::<Vec<_>>(),
    })
  })
}

/// The commit deployed before the one live, out of the deployment log: the last it names other than the
/// live one. Each of its lines names the commit second.
pub fn logged_before(log: &str, live: &str) -> Option<String> {
  log
    .lines()
    .rev()
    .filter_map(|line| line.split_whitespace().nth(1))
    .find(|commit| *commit != live)
    .map(str::to_owned)
}

/// Every release but the ones kept and the ones a live process stands in taken away, and the repository
/// told to forget their worktrees. Only names a release is made under are touched; a release that cannot
/// be removed is a warning. Where the live processes could not be read, nothing is taken away.
pub fn prune(
  deploy_dir: &Path,
  keep: &[PathBuf],
  processes: Option<&[LiveProcess]>,
  source: &Path,
  data_root: &Path,
  git: GitRun<'_>,
) -> Pruned {
  let releases = releases_dir(deploy_dir);
  let Ok(entries) = std::fs::read_dir(&releases) else {
    return Pruned::default();
  };
  let mut names: Vec<String> = entries
    .flatten()
    .map(|entry| entry.file_name().to_string_lossy().into_owned())
    .filter(|name| is_release_name(name))
    .collect();
  let mut pruned = Pruned::default();

  names.sort();

  let Some(processes) = processes else {
    pruned.standing = names.len();
    pruned.warnings.push(DEPLOY.release.unread_processes.clone());

    return pruned;
  };

  for name in names {
    let release = releases.join(&name);

    if keep.iter().any(|kept| same_place(kept, &release)) {
      pruned.standing += 1;
      continue;
    }

    let holders: Vec<LiveProcess> = std::fs::canonicalize(&release)
      .map(|real| processes.iter().filter(|process| process.stands_in_real(&real)).cloned().collect())
      .unwrap_or_default();

    if !holders.is_empty() {
      pruned.standing += 1;
      pruned.pinned.push((name, holders));
      continue;
    }

    match remove_tree(&release, data_root) {
      Ok(()) => pruned.removed.push(name),
      Err(why) => {
        pruned
          .warnings
          .push(fill_shown(&DEPLOY.release.not_removed, &[("name", &name), ("why", &why)]));
      }
    }
  }

  if !pruned.removed.is_empty() {
    git(&["worktree", "prune"], source);
  }

  pruned
}
