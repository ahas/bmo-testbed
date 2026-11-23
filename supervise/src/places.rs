//! Where the supervisor reads and keeps things (docs/supervise.md, 「Where it stands」): the user unit directory
//! a deployment links into, the wants folders that say a unit is enabled, and its own folder under the data root.

use std::path::{Path, PathBuf};

use bmo_testbed_core::policy::supervise::SUPERVISE;

/// The targets whose wants folders say a unit is enabled, a service's and a timer's.
pub const SERVICE_TARGET: &str = "default.target";
pub const TIMER_TARGET: &str = "timers.target";

#[derive(Debug, Clone)]
pub struct Places {
  /// The user unit directory, where a deployment links its units.
  pub unit_dir: PathBuf,
  /// The supervisor's own folder under the data root.
  pub folder: PathBuf,
}

impl Places {
  /// The places for a data root, with the unit directory `XDG_CONFIG_HOME` or the home names.
  pub fn new(data_root: &Path, config_home: Option<&str>, home: &Path) -> Self {
    let config = config_home
      .filter(|dir| !dir.is_empty())
      .map(PathBuf::from)
      .unwrap_or_else(|| home.join(".config"));

    Self {
      unit_dir: config.join("systemd/user"),
      folder: data_root.join(&SUPERVISE.places.folder),
    }
  }

  /// The places of this process's environment and the data root it names.
  pub fn of_this_process(data_root: &Path) -> Self {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));

    Self::new(data_root, std::env::var("XDG_CONFIG_HOME").ok().as_deref(), &home)
  }

  pub fn socket(&self) -> PathBuf {
    self.folder.join(&SUPERVISE.places.socket)
  }

  pub fn state(&self) -> PathBuf {
    self.folder.join(&SUPERVISE.places.state)
  }

  pub fn lock(&self) -> PathBuf {
    self.folder.join(&SUPERVISE.places.lock)
  }

  pub fn logs(&self) -> PathBuf {
    self.folder.join(&SUPERVISE.places.logs)
  }

  pub fn log_of(&self, unit: &str) -> PathBuf {
    self.logs().join(format!("{unit}.log"))
  }

  /// The runtime directory every unit and every command under the supervisor reads as `XDG_RUNTIME_DIR`, as a user
  /// manager hands one on: `named`, the one the account's environment names or `/run/user/<uid>`, where it stands,
  /// else the supervisor's own, made readable by the account alone. A machine with no login manager has none, and
  /// a program that falls back to one of its own where the variable is unset would otherwise disagree with a unit
  /// that falls back to another.
  pub fn runtime(&self, named: &Path) -> std::io::Result<PathBuf> {
    if named.is_dir() {
      return Ok(named.to_path_buf());
    }

    let own = self.folder.join(&SUPERVISE.places.runtime);

    std::fs::create_dir_all(&own)?;
    std::fs::set_permissions(&own, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    Ok(own)
  }

  /// The folder the stand-ins stand in, first on every PATH the supervisor hands on.
  pub fn stand_ins(&self) -> PathBuf {
    self.folder.join(&SUPERVISE.places.stand_ins)
  }

  /// The wants folder of the target a unit of this name is enabled under.
  pub fn wants_of(&self, unit: &str) -> PathBuf {
    let target = if unit.ends_with(".timer") { TIMER_TARGET } else { SERVICE_TARGET };

    self.unit_dir.join(format!("{target}.wants"))
  }

  /// Every service and timer the unit directory holds, by name, in name order.
  pub fn unit_names(&self) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(&self.unit_dir)
      .into_iter()
      .flatten()
      .flatten()
      .filter_map(|entry| entry.file_name().into_string().ok())
      .filter(|name| name.ends_with(".service") || name.ends_with(".timer"))
      .filter(|name| self.unit_dir.join(name).is_file())
      .collect();

    names.sort();
    names
  }

  /// Whether the unit is enabled: linked into its target's wants folder.
  pub fn enabled(&self, unit: &str) -> bool {
    std::fs::symlink_metadata(self.wants_of(unit).join(unit)).is_ok()
  }

  /// Enables the unit as systemd does, a link in its target's wants folder to its file; the link made, where one
  /// was made.
  pub fn enable(&self, unit: &str) -> std::io::Result<Option<(PathBuf, PathBuf)>> {
    if self.enabled(unit) {
      return Ok(None);
    }

    let wants = self.wants_of(unit);
    let link = wants.join(unit);
    let target = self.unit_dir.join(unit);

    std::fs::create_dir_all(&wants)?;
    std::os::unix::fs::symlink(&target, &link)?;
    Ok(Some((link, target)))
  }

  /// Disables the unit, its wants link taken away; the link taken, where one stood.
  pub fn disable(&self, unit: &str) -> std::io::Result<Option<PathBuf>> {
    let link = self.wants_of(unit).join(unit);

    if std::fs::symlink_metadata(&link).is_err() {
      return Ok(None);
    }

    std::fs::remove_file(&link)?;
    Ok(Some(link))
  }

  /// Links a unit file named by its path into the unit directory under its own name, as `systemctl enable` does
  /// with a path; the unit's name.
  pub fn link_file(&self, file: &Path) -> std::io::Result<String> {
    let name = file
      .file_name()
      .and_then(|name| name.to_str())
      .map(str::to_owned)
      .ok_or_else(|| std::io::Error::other("the path names no unit file"))?;
    let link = self.unit_dir.join(&name);

    if std::fs::canonicalize(&link).ok() != std::fs::canonicalize(file).ok() {
      std::fs::create_dir_all(&self.unit_dir)?;
      let _ = std::fs::remove_file(&link);
      std::os::unix::fs::symlink(file, &link)?;
    }

    Ok(name)
  }

  /// The deploy tree a unit's file stands in: the nearest folder above the file its link names that holds a
  /// repository's declaration, the release the deployment laid; none for a unit outside every release.
  pub fn deploy_tree_of(&self, unit: &str) -> Option<PathBuf> {
    let link = self.unit_dir.join(unit);
    let named = std::fs::read_link(&link)
      .ok()
      .map(|target| if target.is_absolute() { target } else { self.unit_dir.join(target) })?;

    named
      .ancestors()
      .skip(1)
      .find(|dir| dir.join("testbed.toml").is_file())
      .map(Path::to_path_buf)
  }
}
