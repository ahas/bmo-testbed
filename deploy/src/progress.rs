//! Where a deployment stands while it runs, written into one record under the deploy root (docs/deploy.md,
//! 「What a deployment says of itself as it runs」). It needs no service to be read: the file and the run's
//! process are the whole of it, so a service restarting under a deployment leaves the reading as it was.
//!
//! Where the run is handed the testbed's data root, every change of the record is written into the testbed's
//! record too, as the deployment member the HTTP service answers, with every line the run says and a restart
//! for every unit it stops (docs/boundary.md, 「The record」).
//!
//! The outermost run begins the record; a deployment started inside that run's unit carries it on, writing
//! its own stages into it, and the outermost run ends it. The two are separate processes writing one file,
//! so every change reads the file as it stands and writes it whole.

use std::path::{Path, PathBuf};

use bmo_testbed_client::{
  Deployment, DeploymentStage, DeploymentState, DeploymentStep, StageName, StepPart, StepState, StopCode,
};
use bmo_testbed_core::queue::process_alive;
use bmo_testbed_record::{Record, say_unwritten};
use serde::{Deserialize, Serialize};

/// The record's file under the deploy root.
pub const DEPLOY_PROGRESS: &str = "deploy-progress.json";

/// Names another file in place of the deploy root's, for a rehearsal.
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
  /// Whether it ended on a refusal, before anything changed.
  #[serde(default)]
  pub refused: bool,
  /// The repository's declared name.
  #[serde(default)]
  pub repository: Option<String>,
  /// How it ended, none while it runs.
  #[serde(default)]
  pub ending: Option<Ending>,
}

/// The record's file: the deploy root's, or the one the environment names.
pub fn deploy_progress_path(root: &Path) -> PathBuf {
  match std::env::var(DEPLOY_PROGRESS_VARIABLE) {
    Ok(named) if !named.is_empty() => PathBuf::from(named),
    _ => root.join(DEPLOY_PROGRESS),
  }
}

/// The record out of its file's text; none where there is none or it does not read as one.
pub fn read_deploy_progress(text: Option<&str>) -> Option<DeployProgress> {
  serde_json::from_str(text?).ok()
}

impl DeployProgress {
  /// Whether a run in the unit given carries this record on: it is unended and begun in that same unit, so
  /// the run is a deployment started inside the unit of the run that began it.
  pub fn carried_in(&self, unit: Option<&str>) -> bool {
    self.ending.is_none() && unit.is_some() && self.unit.as_deref() == unit
  }

  /// The commit of the run this record holds while it is under way: unended, its process alive.
  pub fn under_way(&self) -> Option<&str> {
    (self.ending.is_none() && process_alive(self.pid))
      .then_some(self.commit.as_deref())
      .flatten()
  }
}

/// The record at the path given, as it stands; none where there is none or it does not read as one.
pub fn deploy_progress_at(path: &Path) -> Option<DeployProgress> {
  read_deploy_progress(std::fs::read_to_string(path).ok().as_deref())
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
  pub repository: Option<String>,
  /// The testbed's data root whose record the run is written into too; none writes into none.
  pub record: Option<PathBuf>,
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
  /// The testbed's data root whose record the run is written into too.
  record: Option<PathBuf>,
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
    self.mirror(&record);
  }

  // The testbed's record opened, where the run writes into one; a failure is said and nothing more.
  fn recorded(&self, write: impl FnOnce(&mut Record) -> Result<(), String>) {
    let Some(root) = &self.record else { return };

    if let Err(why) = Record::open(root).and_then(|mut record| write(&mut record)) {
      say_unwritten(&why);
    }
  }

  fn mirror(&self, progress: &DeployProgress) {
    self.recorded(|record| record.write_deployment(&deployment_member(progress)).map(|_| ()));
  }

  /// The deployment's id in the testbed's record, where the run writes into one and it stands there.
  pub fn deployment_id(&self) -> Option<i64> {
    let root = self.record.as_ref()?;

    Record::open(root).ok()?.deployment_of(self.pid, self.started_at).ok().flatten()
  }

  /// A line the run said, written as one of the deployment's log lines.
  pub fn said(&self, line: &str) {
    self.recorded(|record| match record.deployment_of(self.pid, self.started_at)? {
      Some(id) => record.add_log_line(id, line).map(|_| ()),
      None => Ok(()),
    });
  }

  /// Why the run stops or restarts a unit, written before it does.
  pub fn restarting(&self, unit: &str, reason: &str) {
    self.recorded(|record| match record.deployment_of(self.pid, self.started_at)? {
      Some(id) => record.add_restart(unit, id, reason).map(|_| ()),
      None => Ok(()),
    });
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

  /// Ends the record on a refusal, where this run began it: stopped, before anything changed.
  pub fn refuse(&self, why: &str) {
    if self.owner {
      self.change(|record| record.refused = true);
    }
    self.end(Outcome::Stopped, Some(why));
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
  if let Some(standing) = deploy_progress_at(path)
    && standing.carried_in(start.unit.as_deref())
  {
    let run = DeployRun {
      path: path.to_path_buf(),
      started_at: standing.started_at,
      pid: standing.pid,
      owner: false,
      now,
      record: start.record.clone(),
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
    refused: false,
    repository: start.repository.clone(),
    ending: None,
  };

  // Unwritten, the run goes on all the same; its changes then find no record of theirs.
  let _ = write_record(path, &record);

  let run = DeployRun {
    path: path.to_path_buf(),
    started_at: start.now,
    pid: start.pid,
    owner: true,
    now,
    record: start.record.clone(),
  };

  run.mirror(&record);
  run
}

impl From<Stage> for StageName {
  fn from(stage: Stage) -> Self {
    match stage {
      Stage::Check => Self::Check,
      Stage::Backup => Self::Backup,
      Stage::Tree => Self::Tree,
      Stage::Build => Self::Build,
      Stage::Before => Self::Before,
      Stage::Restart => Self::Restart,
      Stage::StayUp => Self::StayUp,
      Stage::After => Self::After,
      Stage::Rollback => Self::Rollback,
    }
  }
}

impl From<Part> for StepPart {
  fn from(part: Part) -> Self {
    match part {
      Part::Snapshot => Self::Snapshot,
      Part::Pack => Self::Pack,
      Part::Check => Self::Check,
    }
  }
}

// The code of why a run did not go live: the last stage it reached short of a rollback, or its refusal.
fn stopped_for(progress: &DeployProgress) -> Option<StopCode> {
  let ending = progress.ending.as_ref()?;

  if ending.outcome == Outcome::Live {
    return None;
  }
  if progress.refused {
    return Some(StopCode::Refused);
  }

  progress.stages.iter().rev().find_map(|at| match at.stage {
    Stage::Check => Some(StopCode::Check),
    Stage::Backup => Some(StopCode::Backup),
    Stage::Tree => Some(StopCode::Tree),
    Stage::Build => Some(StopCode::Build),
    Stage::Before => Some(StopCode::Before),
    Stage::Restart => Some(StopCode::Restart),
    Stage::StayUp => Some(StopCode::StayUp),
    Stage::After | Stage::Rollback => None,
  })
}

/// The progress record as the deployment member the HTTP service answers: its steps ended, under way and
/// still ahead, in that order. Its id is the testbed record's own, written there.
pub fn deployment_member(progress: &DeployProgress) -> Deployment {
  let stage_now = progress.stages.last().map(|last| StageName::from(last.stage));
  let ended = progress.steps.iter().map(|step| {
    let spent = (step.ran_seconds * 1000.0).round() as i64;

    DeploymentStep {
      name: step.name.clone(),
      stage: step.stage.map(StageName::from),
      part: step.part.map(StepPart::from),
      state: StepState::Ended,
      exit_code: Some(step.code),
      waited_ms: Some((step.waited_seconds * 1000.0).round() as i64),
      spent_ms: Some(spent),
      started_at: Some(step.at - spent),
      ended_at: Some(step.at),
    }
  });
  let running = progress.running.iter().map(|step| DeploymentStep {
    name: step.name.clone(),
    stage: step.stage.map(StageName::from),
    part: step.part.map(StepPart::from),
    state: StepState::Running,
    exit_code: None,
    waited_ms: None,
    spent_ms: None,
    started_at: Some(step.at),
    ended_at: None,
  });
  let ahead = progress.ahead.iter().map(|step| DeploymentStep {
    name: step.name.clone(),
    stage: stage_now,
    part: step.part.map(StepPart::from),
    state: StepState::Ahead,
    exit_code: None,
    waited_ms: None,
    spent_ms: None,
    started_at: None,
    ended_at: None,
  });

  Deployment {
    id: 0,
    repository_name: progress.repository.clone(),
    unit_name: progress.unit.clone(),
    pid: progress.pid,
    commit: progress.commit.clone(),
    state: match progress.ending.as_ref().map(|ending| ending.outcome) {
      None => DeploymentState::Running,
      Some(Outcome::Live) => DeploymentState::Live,
      Some(Outcome::RolledBack) => DeploymentState::RolledBack,
      Some(Outcome::Failed) => DeploymentState::Failed,
      Some(Outcome::Stopped) => DeploymentState::Stopped,
    },
    plan: progress.plan.iter().copied().map(StageName::from).collect(),
    stages: progress
      .stages
      .iter()
      .map(|at| DeploymentStage {
        stage: at.stage.into(),
        started_at: at.at,
      })
      .collect(),
    steps: ended.chain(running).chain(ahead).collect(),
    rolled_back: progress.rolled_back,
    stopped_for: stopped_for(progress),
    reason: progress.ending.as_ref().and_then(|ending| ending.why.clone()),
    started_at: progress.started_at,
    ended_at: progress.ending.as_ref().map(|ending| ending.at),
  }
}
