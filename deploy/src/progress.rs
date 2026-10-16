//! Where a deployment stands while it runs, written into one record under the data root (docs/deploy.md,
//! 「What a deployment says of itself as it runs」). It needs no service to be read: the file and the run's
//! process are the whole of it, so a service restarting under a deployment leaves the reading as it was.
//!
//! The outermost run begins the record; a deployment started inside that run's unit carries it on, writing
//! its own stages into it, and the outermost run ends it. The two are separate processes writing one file,
//! so every change reads the file as it stands and writes it whole.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The record's file under the data root.
pub const DEPLOY_PROGRESS: &str = "deploy-progress.json";

/// Names another file in place of the data root's, for a rehearsal.
pub const DEPLOY_PROGRESS_VARIABLE: &str = "BMO_DEPLOY_PROGRESS";

/// The stages a run can pass, in the order it passes them; a rollback is never planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
  Check,
  Backup,
  Tree,
  Build,
  Before,
  Restart,
  StayUp,
  After,
  Rollback,
}

impl Stage {
  /// Every stage a run may plan, in order.
  pub const PLANNED: [Stage; 8] = [
    Stage::Check,
    Stage::Backup,
    Stage::Tree,
    Stage::Build,
    Stage::Before,
    Stage::Restart,
    Stage::StayUp,
    Stage::After,
  ];
}

/// The parts of the backup stage written as steps of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Part {
  Snapshot,
  Pack,
  Check,
}

/// How a run ended: live, rolled back, failed, or stopped where nothing went live by its own judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
  Live,
  RolledBack,
  Failed,
  Stopped,
}

/// One heavy step of a run — a check, the suite, a build — with its wait for its turn and its run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepDone {
  pub name: String,
  pub waited_seconds: f64,
  pub ran_seconds: f64,
  /// Its exit code.
  pub code: i64,
  /// When it ended, in milliseconds.
  pub at: i64,
  /// The stage under way when it ended.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub stage: Option<Stage>,
  /// Which part of its stage it is, where it is one.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub part: Option<Part>,
}

/// A step written before it runs, as one of the stage's steps still to come.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepAhead {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub part: Option<Part>,
}

/// The heavy step under way, written as it begins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepUnderWay {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub part: Option<Part>,
  /// The stage under way when it began.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub stage: Option<Stage>,
  /// When it began, in milliseconds.
  pub at: i64,
}

/// One stage passed, with when it began.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageAt {
  pub stage: Stage,
  pub at: i64,
}

/// How a run ended, with the reason beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ending {
  pub outcome: Outcome,
  pub at: i64,
  pub why: Option<String>,
}

/// The record as it stands in its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployProgress {
  /// The deploy unit the run stands in, whose journal is its log; none where it stands in none.
  pub unit: Option<String>,
  /// The process of the run that began the record.
  pub pid: i64,
  /// The commit going live, once it is known.
  pub commit: Option<String>,
  /// When the run began, in milliseconds.
  pub started_at: i64,
  /// Every stage the run means to pass, in order, written as it begins.
  #[serde(default)]
  pub plan: Vec<Stage>,
  /// Every stage passed so far with when it began, the last one under way until the run ends.
  #[serde(default)]
  pub stages: Vec<StageAt>,
  /// Every heavy step ended so far, in the order they ended.
  #[serde(default)]
  pub steps: Vec<StepDone>,
  /// The steps of the stage under way still to come, in order.
  #[serde(default)]
  pub ahead: Vec<StepAhead>,
  /// The heavy step under way; none between steps.
  #[serde(default)]
  pub running: Option<StepUnderWay>,
  /// Whether the commit that was live before was put back.
  #[serde(default)]
  pub rolled_back: bool,
  /// How it ended, none while it runs.
  #[serde(default)]
  pub ending: Option<Ending>,
}

/// The record's file: the data root's, or the one the environment names.
pub fn deploy_progress_path(data_root: &Path) -> PathBuf {
  match std::env::var(DEPLOY_PROGRESS_VARIABLE) {
    Ok(named) if !named.is_empty() => PathBuf::from(named),
    _ => data_root.join(DEPLOY_PROGRESS),
  }
}

/// The record out of its file's text; none where there is none or it does not read as one.
pub fn read_deploy_progress(text: Option<&str>) -> Option<DeployProgress> {
  serde_json::from_str(text?).ok()
}

// The record written beside itself and renamed over, so a reader never meets half of it.
fn write_record(path: &Path, record: &DeployProgress) -> std::io::Result<()> {
  let beside = PathBuf::from(format!("{}.{}.part", path.display(), std::process::id()));
  let text = format!("{}\n", serde_json::to_string_pretty(record).unwrap_or_default());

  std::fs::write(&beside, text)?;
  std::fs::rename(&beside, path)
}

/// What a run begins its record with.
#[derive(Debug, Clone)]
pub struct DeployStart {
  pub unit: Option<String>,
  pub pid: i64,
  pub commit: Option<String>,
  /// Every stage the run means to pass, in order; a run carrying another's record on leaves its plan.
  pub plan: Vec<Stage>,
  pub now: i64,
}

/// One run's hold on the record. Every change goes to the record the run began or carried on, read afresh,
/// and to no other: a record another run has since begun is left alone.
#[derive(Debug, Clone)]
pub struct DeployRun {
  path: PathBuf,
  started_at: i64,
  pid: i64,
  /// Whether this run began the record and so is the one that ends it.
  pub owner: bool,
  now: fn() -> i64,
}

impl DeployRun {
  fn change(&self, edit: impl FnOnce(&mut DeployProgress)) {
    let text = std::fs::read_to_string(&self.path).ok();
    let Some(mut record) = read_deploy_progress(text.as_deref()) else {
      return;
    };

    if record.started_at != self.started_at || record.pid != self.pid || record.ending.is_some() {
      return;
    }

    edit(&mut record);

    // What the record says is for whoever watches; a deployment never stops over it.
    let _ = write_record(&self.path, &record);
  }

  /// The run has reached a stage, which begins with no step ahead or under way. Reaching the stage it is
  /// already in writes nothing.
  pub fn stage(&self, stage: Stage) {
    let at = (self.now)();

    self.change(|record| {
      if record.stages.last().map(|last| last.stage) != Some(stage) {
        record.stages.push(StageAt { stage, at });
        record.ahead.clear();
        record.running = None;
      }
    });
  }

  /// The stages this run turns out not to pass, taken out of the plan.
  pub fn passes_over(&self, stages: &[Stage]) {
    self.change(|record| record.plan.retain(|stage| !stages.contains(stage)));
  }

  /// The steps the stage under way will run, in order, by name and part.
  pub fn steps_ahead(&self, steps: &[(&str, Option<Part>)]) {
    self.change(|record| {
      record.ahead = steps
        .iter()
        .map(|(name, part)| StepAhead {
          name: (*name).to_owned(),
          part: *part,
        })
        .collect();
    });
  }

  /// A heavy step, or a part of a stage, has begun: the step under way, and no longer one ahead.
  pub fn began(&self, name: &str, part: Option<Part>) {
    let at = (self.now)();

    self.change(|record| {
      take_ahead(record, name, part);
      record.running = Some(StepUnderWay {
        name: name.to_owned(),
        part,
        stage: record.stages.last().map(|last| last.stage),
        at,
      });
    });
  }

  /// A heavy step, or a part of a stage, has ended: written with the stage under way.
  pub fn step(&self, name: &str, part: Option<Part>, code: i64, waited_seconds: f64, ran_seconds: f64) {
    let at = (self.now)();

    self.change(|record| {
      let stage = record.stages.last().map(|last| last.stage);

      take_ahead(record, name, part);
      record.running = None;
      record.steps.push(StepDone {
        name: name.to_owned(),
        waited_seconds,
        ran_seconds,
        code,
        at,
        stage,
        part,
      });
    });
  }

  /// What goes live, once it is known.
  pub fn carries(&self, commit: &str) {
    self.change(|record| record.commit = Some(commit.to_owned()));
  }

  /// The commit that was live before was put back.
  pub fn rolled_back(&self) {
    self.change(|record| record.rolled_back = true);
  }

  /// Ends the record, where this run began it. A failure after a rollback is told as the rollback.
  pub fn end(&self, outcome: Outcome, why: Option<&str>) {
    if !self.owner {
      return;
    }

    let at = (self.now)();

    self.change(|record| {
      let outcome = if outcome == Outcome::Failed && record.rolled_back {
        Outcome::RolledBack
      } else {
        outcome
      };

      record.running = None;
      record.ahead.clear();
      record.ending = Some(Ending {
        outcome,
        at,
        why: why.map(str::to_owned),
      });
    });
  }
}

// A step that begins or ends leaves the steps ahead, the first written under its name and part.
fn take_ahead(record: &mut DeployProgress, name: &str, part: Option<Part>) {
  if let Some(index) = record.ahead.iter().position(|one| one.name == name && one.part == part) {
    record.ahead.remove(index);
  }
}

/// Begins the record, or carries on the one a run in the same unit began and has not ended: a deployment
/// another run starts inside its own unit writes into that run's record.
pub fn begin_deploy(path: &Path, start: &DeployStart, now: fn() -> i64) -> DeployRun {
  let standing = read_deploy_progress(std::fs::read_to_string(path).ok().as_deref());

  if let Some(standing) = standing
    && standing.ending.is_none()
    && start.unit.is_some()
    && standing.unit == start.unit
  {
    let run = DeployRun {
      path: path.to_path_buf(),
      started_at: standing.started_at,
      pid: standing.pid,
      owner: false,
      now,
    };

    if let Some(commit) = &start.commit {
      run.carries(commit);
    }

    return run;
  }

  let record = DeployProgress {
    unit: start.unit.clone(),
    pid: start.pid,
    commit: start.commit.clone(),
    started_at: start.now,
    plan: start.plan.clone(),
    stages: vec![],
    steps: vec![],
    ahead: vec![],
    running: None,
    rolled_back: false,
    ending: None,
  };

  // Unwritten, the run goes on all the same; its changes then find no record of theirs.
  let _ = write_record(path, &record);

  DeployRun {
    path: path.to_path_buf(),
    started_at: start.now,
    pid: start.pid,
    owner: true,
    now,
  }
}
