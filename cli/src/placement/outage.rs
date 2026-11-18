//! One outage of the placed machine, remembered under the data root's `testbed/placed-outage/` (docs/boundary.md,
//! 「The placed machine」), so a machine that stops answering is told once however many runs give up on it, and
//! told once more when it answers again; and a testbed there that could not be brought to the live release,
//! remembered under `testbed/placed-release-failed/`, so the failure is told once for each live release. Each mark
//! is a file made with `create_new` or taken away whole, so of the processes that race to begin, tell or end one,
//! one alone wins each.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The directory under the data root the outages stand in.
pub const OUTAGE_DIR: &str = "testbed/placed-outage";

/// The outage of one placed machine, by its label.
pub struct Outage {
  dir: PathBuf,
  machine: String,
}

impl Outage {
  /// The outage of the machine labelled `machine` under a data root.
  pub fn of(data_root: &Path, machine: &str) -> Self {
    Self {
      dir: data_root.join(OUTAGE_DIR),
      machine: machine.to_owned(),
    }
  }

  // The file whose standing says an outage stands, holding the moment it began.
  fn file(&self) -> PathBuf {
    self.dir.join(&self.machine)
  }

  // The file whose standing says the outage was told.
  fn told_file(&self) -> PathBuf {
    self.dir.join(format!("{}.told", self.machine))
  }

  /// The machine found not answering: an outage begun at this moment where none stands.
  pub fn begun(&self) {
    let _ = std::fs::create_dir_all(&self.dir);
    let made = std::fs::OpenOptions::new().write(true).create_new(true).open(self.file());

    if let Ok(mut file) = made {
      let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
      let _ = file.write_all(now.as_bytes());
    }
  }

  /// When the outage standing began, as RFC 3339; none where none stands.
  pub fn since(&self) -> Option<String> {
    std::fs::read_to_string(self.file()).ok().map(|text| text.trim().to_owned())
  }

  /// Whether the caller is the one to tell the outage standing: the first to ask alone, an outage begun first
  /// where none stands.
  pub fn first_to_tell(&self) -> bool {
    self.begun();
    std::fs::OpenOptions::new().write(true).create_new(true).open(self.told_file()).is_ok()
  }

  /// The machine answering again: the outage standing ended. When it began, for the one caller that ends an
  /// outage that was told; none for every other caller, and where no outage stood or it was never told.
  pub fn ended(&self) -> Option<String> {
    let since = self.since()?;

    std::fs::remove_file(self.file()).ok()?;
    std::fs::remove_file(self.told_file()).ok()?;

    Some(since)
  }
}

/// The directory under the data root the release failures told stand in.
pub const RELEASE_FAILED_DIR: &str = "testbed/placed-release-failed";

/// The placed machine's testbed that could not be brought to the live release, remembered under the data root's
/// `testbed/placed-release-failed/` (docs/boundary.md, 「The placed machine」), so a failure is told once for each
/// live release however many runs meet it, and told again once a run has found the testbed there brought.
pub struct ReleaseFailure {
  dir: PathBuf,
  machine: String,
}

impl ReleaseFailure {
  /// The release failures of the machine labelled `machine` under a data root.
  pub fn of(data_root: &Path, machine: &str) -> Self {
    Self {
      dir: data_root.join(RELEASE_FAILED_DIR),
      machine: machine.to_owned(),
    }
  }

  // The start every file of the machine's names: its label and a dot.
  fn prefix(&self) -> String {
    format!("{}.", self.machine)
  }

  /// Whether the caller is the one to tell that the testbed there could not be brought to the live release
  /// `live`: the first to ask for that release alone.
  pub fn first_to_tell(&self, live: &str) -> bool {
    let _ = std::fs::create_dir_all(&self.dir);

    std::fs::OpenOptions::new()
      .write(true)
      .create_new(true)
      .open(self.dir.join(format!("{}{live}", self.prefix())))
      .is_ok()
  }

  /// The testbed there found at the live release: every failure of the machine told is taken away.
  pub fn cleared(&self) {
    let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
    let prefix = self.prefix();

    for entry in entries.flatten() {
      if entry.file_name().to_string_lossy().starts_with(&prefix) {
        let _ = std::fs::remove_file(entry.path());
      }
    }
  }
}
