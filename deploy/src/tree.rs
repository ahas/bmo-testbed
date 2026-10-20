//! The part of a deployment with no service in it (docs/deploy.md): what makes a checkout undeployable,
//! what a backup is called, the record of what runs, which unit links the move would leave dangling and
//! which units the tree asks to keep, and the mark of what `bin/` was built from.
//!
//! A deployment answers one question — which commit is running — so everything here is written to make
//! that question answerable.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::fill_shown;
use serde::{Deserialize, Serialize};

/// The deployment that is running, as `deployed.json` records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployRecord {
  /// The commit the deploy tree stands on.
  pub commit: String,
  /// That commit's subject line.
  pub subject: String,
  /// When it was laid down, in ISO.
  pub at: String,
  /// The backup taken before it.
  pub backup: String,
}

/// Where the running deployment is written down, under the deploy root.
pub const DEPLOY_RECORD: &str = "deployed.json";

/// Where every deployment is appended, one line apiece, under the same root.
pub const DEPLOY_LOG: &str = "deployed.log";

/// What is deployed now, or none where no deployment has been recorded — a first run, or a record that
/// has been lost.
pub fn read_deploy_record(path: &Path) -> Option<DeployRecord> {
  serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The record written whole, as every reader of it reads it.
pub fn write_deploy_record(path: &Path, record: &DeployRecord) -> std::io::Result<()> {
  std::fs::write(path, format!("{}\n", serde_json::to_string_pretty(record).unwrap_or_default()))
}

/// What to call a backup of the declared name taken at the given local moment, avoiding the names
/// already on disk: named for the second, with an ordinal of two digits when a second already holds one.
pub fn backup_name(name: &str, stamp: &str, taken: &[String]) -> Option<String> {
  let first = format!("{name}-{stamp}.tar.zst");

  if !taken.contains(&first) {
    return Some(first);
  }

  (2..=99)
    .map(|ordinal| format!("{name}-{stamp}-{ordinal:02}.tar.zst"))
    .find(|one| !taken.contains(one))
}

/// The local moment a backup is named for: `YYYYMMDD-HHMMSS`.
pub fn backup_stamp(now: chrono::DateTime<chrono::Local>) -> String {
  now.format("%Y%m%d-%H%M%S").to_string()
}

/// Why a checkout cannot be deployed, or none: changes this run left uncommitted, no commit, or a commit
/// the remote has never seen.
pub fn tree_fault(dirty: &[String], head: &str, on_remote: bool, remote_branch: &str) -> Option<String> {
  let shown = DEPLOY.numbers.dirty_shown;

  if !dirty.is_empty() {
    let named = dirty.iter().take(shown).cloned().collect::<Vec<_>>().join(", ");
    let rest = if dirty.len() > shown {
      fill_shown(&DEPLOY.tree.more, &[("count", &(dirty.len() - shown))])
    } else {
      String::new()
    };

    return Some(fill_shown(&DEPLOY.tree.dirty, &[("named", &named), ("rest", &rest)]));
  }

  if head.is_empty() {
    return Some(DEPLOY.tree.no_commit.clone());
  }

  (!on_remote)
    .then(|| fill_shown(&DEPLOY.tree.unpushed, &[("commit", &short(head)), ("branch", &remote_branch)]))
}

/// The first twelve characters of a commit, as every line names one.
pub fn short(commit: &str) -> String {
  commit.chars().take(12).collect()
}

/// Where systemd reads the user's own units from: under XDG_CONFIG_HOME where it is set.
pub fn user_unit_dir(home: &Path) -> PathBuf {
  user_unit_dir_from(std::env::var("XDG_CONFIG_HOME").ok().as_deref(), home)
}

/// Where systemd reads the user's own units from, given XDG_CONFIG_HOME as the environment holds it; an
/// empty one counts as unset.
pub fn user_unit_dir_from(config_home: Option<&str>, home: &Path) -> PathBuf {
  let config = config_home
    .filter(|dir| !dir.is_empty())
    .map_or_else(|| home.join(".config"), PathBuf::from);

  config.join("systemd").join("user")
}

/// An installed unit that is a symlink: its name in the unit directory and the absolute path it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitLink {
  pub name: String,
  pub target: PathBuf,
}

// A path made absolute against a directory and its `.` and `..` resolved by their words alone.
fn resolved(base: &Path, path: &Path) -> PathBuf {
  let joined = base.join(path);
  let mut out = PathBuf::new();

  for part in joined.components() {
    match part {
      std::path::Component::ParentDir => {
        out.pop();
      }
      std::path::Component::CurDir => {}
      other => out.push(other.as_os_str()),
    }
  }

  out
}

/// Every installed unit that is a symlink, a relative link read against the directory it stands in.
pub fn unit_links(unit_dir: &Path) -> Vec<UnitLink> {
  let Ok(entries) = std::fs::read_dir(unit_dir) else {
    return vec![];
  };
  let mut names: Vec<String> = entries
    .flatten()
    .map(|entry| entry.file_name().to_string_lossy().into_owned())
    .collect();

  names.sort();
  names
    .into_iter()
    .filter_map(|name| {
      let link = unit_dir.join(&name);
      let is_link = std::fs::symlink_metadata(&link).is_ok_and(|meta| meta.file_type().is_symlink());

      is_link.then(|| {
        let target = std::fs::read_link(&link).unwrap_or_default();

        UnitLink {
          target: resolved(unit_dir, &target),
          name,
        }
      })
    })
    .collect()
}

/// A unit link to be pointed somewhere else, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitRelink {
  pub name: String,
  pub from: PathBuf,
  pub to: PathBuf,
}

/// What a deployment has to do to the unit links before it flips the deploy tree, judged against the
/// commit being deployed rather than the disk, so a refusal comes while nothing has changed. A link into
/// the tree that the commit keeps a file for — or that names an untracked file — needs nothing; one the
/// commit would leave dangling is relinked to the commit's one file of the unit's own name; none, or
/// several, is a fault. A link anywhere but the deploy tree is not this deployment's.
pub fn unit_link_plan(
  links: &[UnitLink],
  deploy_dir: &Path,
  tracked: &[String],
  untracked_now: &dyn Fn(&str) -> bool,
) -> (Vec<UnitRelink>, Vec<String>) {
  let tree = resolved(Path::new("/"), deploy_dir);
  let tracked_set: BTreeSet<&str> = tracked.iter().map(String::as_str).collect();
  let mut relinks = vec![];
  let mut faults = vec![];

  for link in links {
    let Ok(path) = link.target.strip_prefix(&tree) else {
      continue;
    };

    if path.as_os_str().is_empty() {
      continue;
    }

    let path = path.to_string_lossy().into_owned();

    if tracked_set.contains(path.as_str()) || untracked_now(&path) {
      continue;
    }

    let named: Vec<&String> = tracked
      .iter()
      .filter(|file| Path::new(file).file_name().is_some_and(|name| name.to_string_lossy() == link.name))
      .collect();

    match named.as_slice() {
      [one] => relinks.push(UnitRelink {
        name: link.name.clone(),
        from: link.target.clone(),
        to: tree.join(one),
      }),
      [] => faults.push(fill_shown(&DEPLOY.tree.link_gone, &[("unit", &link.name), ("path", &path)])),
      several => faults.push(fill_shown(
        &DEPLOY.tree.link_several,
        &[
          ("unit", &link.name),
          ("path", &path),
          ("count", &several.len()),
          ("files", &several.iter().map(|one| one.as_str()).collect::<Vec<_>>().join(", ")),
        ],
      )),
    }
  }

  (relinks, faults)
}

/// Points each unit link at the file its plan named, and hands back every link into the deploy tree that
/// still resolves nowhere afterwards — which must be empty before the units are reloaded.
pub fn relink_units(
  unit_dir: &Path,
  deploy_dir: &Path,
  relinks: &[UnitRelink],
) -> std::io::Result<Vec<String>> {
  for one in relinks {
    let link = unit_dir.join(&one.name);

    std::fs::remove_file(&link)?;
    std::os::unix::fs::symlink(&one.to, &link)?;
  }

  let tree = resolved(Path::new("/"), deploy_dir);

  Ok(
    unit_links(unit_dir)
      .into_iter()
      .filter(|link| link.target.starts_with(&tree) && link.target != tree && !link.target.exists())
      .map(|link| link.name)
      .collect(),
  )
}

/// The key a unit file carries in its `[Install]` section to be kept by every deployment, before its mark
/// (docs/deploy.md, 「The unit links」).
pub const DEPLOY_MARK_KEY: &str = "X-BMO-Deploy";

/// What a unit file's mark asks of every deployment: the one service whose health it waits on, restarted,
/// restarted only where what it runs changed, kept running as a timer with its service, linked alone, or
/// taken out of the user units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitMark {
  Bot,
  Restart,
  Binary,
  Timer,
  Link,
  Retire,
}

impl UnitMark {
  /// The mark's own word, as the line after the key names it.
  pub fn word(self) -> &'static str {
    match self {
      Self::Bot => "bot",
      Self::Restart => "restart",
      Self::Binary => "binary",
      Self::Timer => "timer",
      Self::Link => "link",
      Self::Retire => "retire",
    }
  }

  /// The one line a unit file carrying the mark holds.
  pub fn line(self) -> String {
    format!("{DEPLOY_MARK_KEY}={}", self.word())
  }

  // systemd ignores keys beginning `X-` and shows them through no command, so the mark is found as the
  // one exact line it is, the file not otherwise read.
  fn carried_by(self, text: &str) -> bool {
    let line = self.line();

    text.lines().any(|one| one.trim() == line)
  }
}

/// The tracked files of a tree ending in a suffix and carrying a mark, in name order.
pub fn marked_units(
  tracked: &[String],
  read: &dyn Fn(&str) -> String,
  suffix: &str,
  mark: UnitMark,
) -> Vec<String> {
  let mut found: Vec<String> = tracked
    .iter()
    .filter(|path| path.ends_with(suffix) && mark.carried_by(&read(path)))
    .cloned()
    .collect();

  found.sort();
  found
}

/// The unit whose health a deployment waits on: the one service of a tree marked `bot`, by its unit name;
/// none where the tree marks none.
pub fn bot_unit(tracked: &[String], read: &dyn Fn(&str) -> String) -> Option<String> {
  marked_units(tracked, read, ".service", UnitMark::Bot)
    .first()
    .map(|path| base_name(path).trim_end_matches(".service").to_owned())
}

/// The installed units a tree marks as going, each a timer with the service of its own name after it: the
/// names under the user units, in name order.
pub fn retired_units(
  tracked: &[String],
  read: &dyn Fn(&str) -> String,
  installed: &[String],
) -> Vec<String> {
  let mut names: Vec<String> = vec![];

  for suffix in [".timer", ".service"] {
    for path in marked_units(tracked, read, suffix, UnitMark::Retire) {
      let name = base_name(&path);
      let with = if suffix == ".timer" { vec![name.clone(), timer_service(&name)] } else { vec![name] };

      for one in with {
        if installed.contains(&one) && !names.contains(&one) {
          names.push(one);
        }
      }
    }
  }

  names
}

/// The service a timer starts, which systemd takes to be the one of the timer's own name.
pub fn timer_service(timer: &str) -> String {
  match timer.strip_suffix(".timer") {
    Some(stem) => format!("{stem}.service"),
    None => timer.to_owned(),
  }
}

/// The base name of a path.
pub fn base_name(path: &str) -> String {
  Path::new(path)
    .file_name()
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_default()
}

/// The declared units installed as plain files rather than links, moved aside as `<unit>.before-deploy`
/// beside where they stood — never deleted — so the deployment's link takes their place.
pub fn set_aside_plain_units(unit_dir: &Path, declared: &[String]) -> Vec<String> {
  declared
    .iter()
    .map(|path| base_name(path))
    .filter(|name| {
      let at = unit_dir.join(name);
      let plain = std::fs::symlink_metadata(&at).is_ok_and(|meta| !meta.file_type().is_symlink());

      plain && std::fs::rename(&at, unit_dir.join(format!("{name}.before-deploy"))).is_ok()
    })
    .collect()
}

/// The program a unit file's `ExecStart` runs: the first word of that line, with systemd's prefix
/// characters taken off, or none where the file has none.
///
/// Only that one key's first word is wanted, and none of the testbed's dependencies reads systemd's unit
/// syntax, so the key is found as the line it begins.
pub fn unit_program(text: &str) -> Option<String> {
  let line = text.lines().map(str::trim).find(|line| line.starts_with("ExecStart="))?;
  let word = line["ExecStart=".len()..]
    .split_whitespace()
    .next()?
    .trim_start_matches(['-', '@', ':', '+', '!']);

  (!word.is_empty()).then(|| word.to_owned())
}

/// The units marked `binary` a deployment restarts: those whose program is one of the binaries it replaced
/// with different bytes, and those it linked or relinked itself. The rest go on running.
pub fn binary_restarts(units: &[(String, String)], changed: &[String], linked: &[String]) -> Vec<String> {
  units
    .iter()
    .filter(|(name, text)| {
      linked.contains(name)
        || unit_program(text).is_some_and(|program| {
          let program = Path::new(&program);

          program.parent().and_then(Path::file_name).is_some_and(|dir| dir == "bin")
            && program
              .file_name()
              .is_some_and(|binary| changed.iter().any(|one| binary == one.as_str()))
        })
    })
    .map(|(name, _)| name.clone())
    .collect()
}

/// The marked units installed as links to anywhere but the deploy tree — a checkout's copy of the file —
/// each to be pointed at the tree's file of its name.
pub fn outside_relinks(links: &[UnitLink], declared: &[String], deploy_dir: &Path) -> Vec<UnitRelink> {
  let tree = resolved(Path::new("/"), deploy_dir);
  let by_name: HashMap<String, PathBuf> =
    declared.iter().map(|path| (base_name(path), tree.join(path))).collect();

  links
    .iter()
    .filter_map(|link| {
      let to = by_name.get(&link.name)?;

      (!link.target.starts_with(&tree)).then(|| UnitRelink {
        name: link.name.clone(),
        from: link.target.clone(),
        to: to.clone(),
      })
    })
    .collect()
}

/// A unit the deployment links in: its name among the user units, and its file in the deploy tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitInstall {
  pub name: String,
  pub to: PathBuf,
}

/// The declared units no installed unit of their name stands for; one installed already is left as it is.
pub fn units_to_link(declared: &[String], deploy_dir: &Path, installed: &[String]) -> Vec<UnitInstall> {
  let tree = resolved(Path::new("/"), deploy_dir);

  declared
    .iter()
    .filter(|path| !installed.contains(&base_name(path)))
    .map(|path| UnitInstall {
      name: base_name(path),
      to: tree.join(path),
    })
    .collect()
}

/// Each unit linked among the user units.
pub fn link_units(unit_dir: &Path, installs: &[UnitInstall]) -> std::io::Result<()> {
  std::fs::create_dir_all(unit_dir)?;

  for one in installs {
    std::os::unix::fs::symlink(&one.to, unit_dir.join(&one.name))?;
  }

  Ok(())
}

/// The file in `bin/` naming the commit it was built from.
pub const BUILT_FROM: &str = "built-from";

/// `bin/` marked as built from the commit, written beside its name and renamed over the old as a binary is;
/// none takes the mark away, for a tree that builds nothing.
pub fn mark_built_from(bin: &Path, commit: Option<&str>) -> std::io::Result<()> {
  let path = bin.join(BUILT_FROM);

  let Some(commit) = commit else {
    return match std::fs::remove_file(&path) {
      Err(why) if why.kind() != std::io::ErrorKind::NotFound => Err(why),
      _ => Ok(()),
    };
  };

  std::fs::create_dir_all(bin)?;

  let staged = PathBuf::from(format!("{}.new", path.display()));

  std::fs::write(&staged, format!("{commit}\n"))?;
  std::fs::rename(&staged, &path)
}
