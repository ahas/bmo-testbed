//! What a deployment does when a service it restarted does not come up: flips the deploy tree back onto
//! the release that was live before, starts the service again, and says what came of it, whatever that was
//! (docs/deploy.md, 「Rollback」). The release tree and the service manager are reached through two hands, so
//! a test drives a rollback with stand-ins for both; the tree's own hands here are the real ones.

use std::future::Future;
use std::path::PathBuf;

use bmo_testbed_core::policy::deploy::DEPLOY;
use bmo_testbed_core::policy::{fill_marks, fill_shown};
use serde_json::{Value, json};

use crate::release::{GitRun, flip, release_dir, stands_built};
use crate::tree::{DeployRecord, relink_units, short, unit_link_plan, unit_links, write_deploy_record};

/// Why the deployment failed: a unit refused its restart, the health did not answer, or the service
/// answered and then did not stay up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootFailure {
  RestartRefused,
  NotAnswering,
  DiedAfterStart,
}

/// What the deployment knows when a service did not come up.
#[derive(Debug, Clone)]
pub struct FailedDeploy {
  /// The commit the deployment laid down.
  pub commit: String,
  pub failure: BootFailure,
  /// The record the run began with: what was live before, or none on a first deployment.
  pub previous: Option<DeployRecord>,
  /// The record this run wrote, whose backup the rollback keeps.
  pub written: DeployRecord,
  /// Whether a command ran before the start, which may have reshaped the data for the new code.
  pub before_start: bool,
  /// The unit that did not come up, whose journal holds the cause.
  pub unit: String,
  /// How long the deployment waits for the health, in seconds.
  pub health_wait_seconds: u64,
  /// How long a service that answered must stay up, in seconds.
  pub stay_up_seconds: u64,
}

/// The deploy tree put back on a commit's release, as the deployment itself would have flipped it.
pub trait TreeHands {
  /// Flips the deploy tree onto the commit's release; why it could not where it did not.
  fn flip_to(&mut self, commit: &str) -> Result<(), String>;
  /// Relinks the unit links the flip left dangling and names those still dangling.
  fn relink(&mut self, commit: &str) -> Vec<String>;
  /// Writes the record back as what is live, with a log line saying it is a rollback of `of`.
  fn record(&mut self, record: &DeployRecord, of: &str);
}

/// The services, on the user's service manager.
pub trait ServiceHands {
  /// Stops the service that did not come up; false where it would not stop.
  fn stop(&self) -> bool;
  /// Reloads the units and restarts the service with the `binary` units whose binaries differ; false
  /// where systemctl would not.
  fn restart(&self) -> bool;
  /// Whether the health answers within the deployment's wait, and the service stays up.
  fn answers(&self) -> impl Future<Output = bool>;
  fn say(&self, line: &str);
  /// The moment now, in ISO.
  fn now_iso(&self) -> String;
}

/// Why nothing was rolled back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRolled {
  First,
  BeforeStart,
  Tree,
}

/// What the rollback came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rollback {
  RolledBack { to: String, answers: bool },
  NotRolled { why: NotRolled, detail: String },
}

/// Rolls the service back to the release that was live before, where that can be done. Only the code
/// moves: the data stays as the failed release left it, and the backup the deployment took stays for the
/// case where that is not tolerable.
pub async fn roll_back(
  failed: &FailedDeploy,
  tree: &mut impl TreeHands,
  services: &impl ServiceHands,
) -> Rollback {
  if failed.before_start {
    return Rollback::NotRolled {
      why: NotRolled::BeforeStart,
      detail: DEPLOY.rolling.before_start.clone(),
    };
  }

  let previous = match &failed.previous {
    Some(previous) if previous.commit != failed.commit => previous,
    _ => {
      return Rollback::NotRolled {
        why: NotRolled::First,
        detail: DEPLOY.rolling.first.clone(),
      };
    }
  };

  services.say(&fill_shown(&DEPLOY.rolling.back_to, &[("commit", &short(&previous.commit))]));

  // The service that did not come up goes first, before the tree is flipped under it.
  if !services.stop() {
    services.say(&DEPLOY.rolling.would_not_stop);
  }

  // The release before still holds its own setup and its own `bin/`, so the flip is the whole move.
  if let Err(detail) = tree.flip_to(&previous.commit) {
    return Rollback::NotRolled {
      why: NotRolled::Tree,
      detail,
    };
  }

  let dangling = tree.relink(&previous.commit);

  if !dangling.is_empty() {
    let words = &DEPLOY.rolling;

    services.say(&fill_shown(
      &words.dangling,
      &[
        ("names", &dangling.join(", ")),
        ("links", if dangling.len() == 1 { &words.links_one } else { &words.links_many }),
      ],
    ));
  }

  let back = DeployRecord {
    commit: previous.commit.clone(),
    subject: previous.subject.clone(),
    at: services.now_iso(),
    backup: failed.written.backup.clone(),
  };

  tree.record(&back, &failed.commit);

  if !services.restart() {
    return Rollback::RolledBack {
      to: previous.commit.clone(),
      answers: false,
    };
  }

  let answers = services.answers().await;

  services.say(&fill_shown(
    if answers { &DEPLOY.rolling.answers } else { &DEPLOY.rolling.silent },
    &[("commit", &short(&previous.commit))],
  ));

  Rollback::RolledBack {
    to: previous.commit.clone(),
    answers,
  }
}

/// The command that reads the journal of the unit that did not come up.
pub fn journal_command(failed: &FailedDeploy) -> String {
  format!("journalctl --user -u {} -n {}", failed.unit, DEPLOY.numbers.journal_lines)
}

/// Why the deployment failed, in one line.
pub fn failure_line(failed: &FailedDeploy) -> String {
  let words = &DEPLOY.rollback;
  let commit = short(&failed.commit);

  match failed.failure {
    BootFailure::RestartRefused => fill_marks(&words.restart_refused, &[("commit", &commit)]),
    BootFailure::NotAnswering => fill_marks(
      &words.not_answering,
      &[("commit", &commit), ("seconds", &failed.health_wait_seconds.to_string())],
    ),
    BootFailure::DiedAfterStart => fill_marks(
      &words.died_after_start,
      &[("commit", &commit), ("seconds", &failed.stay_up_seconds.to_string())],
    ),
  }
}

/// The account of the failed deployment the deploy output and the `deploy-failed` event carry: first why,
/// then what the rollback came to, then where to read the cause.
pub fn deploy_failure_notice(failed: &FailedDeploy, outcome: &Rollback) -> String {
  let words = &DEPLOY.rollback;
  let mut lines = vec![failure_line(failed)];

  match outcome {
    Rollback::RolledBack { to, answers } => {
      let line = if *answers { &words.answers } else { &words.silent };

      lines.push(fill_marks(line, &[("to", &short(to))]));
    }
    Rollback::NotRolled { why, detail } => {
      let why = match why {
        NotRolled::First => words.first.clone(),
        NotRolled::BeforeStart => words.before_start.clone(),
        NotRolled::Tree => fill_marks(&words.tree, &[("detail", detail)]),
      };

      lines.push(format!("{why} {}", words.bot_down));
    }
  }

  lines.push(fill_marks(&words.cause, &[("cause", &journal_command(failed))]));
  lines.join("\n")
}

/// The detail of the `deploy-rolled-back` event (docs/boundary.md), where the deployment was rolled back;
/// none where it was not.
pub fn rolled_back_detail(failed: &FailedDeploy, outcome: &Rollback) -> Option<Value> {
  let Rollback::RolledBack { to, answers } = outcome else {
    return None;
  };

  Some(json!({
    "failed_commit": failed.commit,
    "back_to": to,
    "reason": fill_marks(&DEPLOY.rollback.reason, &[("commit", &short(&failed.commit)), ("back", &short(to))]),
    "answers": answers,
    "journal": journal_command(failed),
  }))
}

/// The deploy tree's own steps, run for real on the deploy tree named and the releases beside it.
pub struct DeployTree<'a> {
  pub deploy_dir: PathBuf,
  /// Where the deployment record and its log are kept.
  pub record_path: PathBuf,
  pub log_path: PathBuf,
  pub unit_dir: PathBuf,
  pub git: GitRun<'a>,
  pub say: &'a dyn Fn(&str),
}

impl TreeHands for DeployTree<'_> {
  fn flip_to(&mut self, commit: &str) -> Result<(), String> {
    let release = release_dir(&self.deploy_dir, commit);

    if !stands_built(&release, commit, self.git) {
      return Err(fill_shown(
        &DEPLOY.rolling.not_built,
        &[("commit", &short(commit)), ("release", &release.display())],
      ));
    }

    flip(&self.deploy_dir, &release).map_err(|why| {
      fill_shown(&DEPLOY.rolling.flip_failed, &[("release", &release.display()), ("why", &why)])
    })?;
    (self.say)(&fill_shown(&DEPLOY.rolling.flipped, &[("release", &release.display())]));

    Ok(())
  }

  fn relink(&mut self, commit: &str) -> Vec<String> {
    let listed = (self.git)(&["ls-tree", "-r", "-z", "--name-only", commit], &self.deploy_dir);
    let tracked: Vec<String> = if listed.ok {
      listed.out.split('\0').filter(|path| !path.is_empty()).map(str::to_owned).collect()
    } else {
      vec![]
    };
    let deploy_dir = self.deploy_dir.clone();
    // The tree already names the release, so a path it holds that the commit does not track is an
    // untracked file the release keeps.
    let (relinks, faults) = unit_link_plan(&unit_links(&self.unit_dir), &deploy_dir, &tracked, &|path| {
      deploy_dir.join(path).exists()
    });

    for one in &relinks {
      (self.say)(&fill_shown(&DEPLOY.run.relinked, &[("name", &one.name), ("to", &one.to.display())]));
    }

    for fault in &faults {
      (self.say)(fault);
    }

    relink_units(&self.unit_dir, &deploy_dir, &relinks).unwrap_or_default()
  }

  fn record(&mut self, record: &DeployRecord, of: &str) {
    if let Err(why) = write_deploy_record(&self.record_path, record) {
      (self.say)(&fill_shown(&DEPLOY.rolling.unrecorded, &[("why", &why)]));
    }

    let line = format!(
      "{} {} {} rollback of {}: {}\n",
      record.at,
      record.commit,
      record.backup,
      short(of),
      record.subject
    );
    let appended = std::fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(&self.log_path)
      .and_then(|mut log| std::io::Write::write_all(&mut log, line.as_bytes()));

    if let Err(why) = appended {
      (self.say)(&fill_shown(&DEPLOY.rolling.unlogged, &[("why", &why)]));
    }
  }
}
