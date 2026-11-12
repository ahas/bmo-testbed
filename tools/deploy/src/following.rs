//! A deployment followed through the testbed's record (docs/deploy.md, 「The hosted deploy tool」): its
//! `deployment.created` and `deployment.changed` events read after the last number read, the numbers
//! `GET /events` resumes from by `Last-Event-ID`, and the deployment read by its id, which says whether its
//! process went without ending it. A read that fails is no outcome: the next turn reads again from where
//! the last one stood.

use std::path::{Path, PathBuf};

use bmo_testbed_client::{Deployment, DeploymentState, EventKind, PageQuery};
use bmo_testbed_record::{EventsRead, Record};

use crate::checkout::same_name;

// The events a deployment's course is read from.
const KINDS: [EventKind; 2] = [EventKind::DeploymentCreated, EventKind::DeploymentChanged];

/// Where the following of one deployment stands: the record it is read from, the unit it runs in, the
/// number of the last event read, and the deployment as last read.
#[derive(Clone, Debug)]
pub struct Course {
  record: PathBuf,
  unit: Option<String>,
  cursor: Option<i64>,
  deployment: Option<Deployment>,
}

impl Course {
  /// The course of a deployment about to be started: the record's last event taken now, so none of the
  /// deployment's own is missed; where the record cannot be read, the first turn that reads it starts at the
  /// window's start.
  pub fn before_start(record: &Path) -> Self {
    Self {
      record: record.to_path_buf(),
      unit: None,
      cursor: Record::open_file(record).and_then(|read| read.last_event_id()).ok(),
      deployment: None,
    }
  }

  /// The course of a deployment the record already holds, followed from the record's last event on.
  pub fn of(record: &Path, deployment: Deployment) -> Self {
    Self {
      unit: deployment.unit_name.clone(),
      deployment: Some(deployment),
      ..Self::before_start(record)
    }
  }

  /// The deployment's unit, once the verb has named it.
  pub fn run_in(&mut self, unit: &str) {
    self.unit = Some(unit.to_owned());
  }

  pub fn unit(&self) -> Option<&str> {
    self.unit.as_deref()
  }

  /// The deployment as last read, once the record has held it.
  pub fn deployment(&self) -> Option<&Deployment> {
    self.deployment.as_ref()
  }

  /// The deployment, where it has ended: by its own outcome, or with its process gone.
  pub fn ended(&self) -> Option<&Deployment> {
    self
      .deployment
      .as_ref()
      .filter(|deployment| deployment.state != DeploymentState::Running)
  }

  /// One turn of reading: the events after the last one read, then the deployment by its id. A record that
  /// cannot be opened or read leaves the course where it stood, for the next turn.
  pub fn read(&mut self) {
    let Ok(record) = Record::open_file(&self.record) else {
      return;
    };

    if self.read_events(&record).is_err() {
      return;
    }

    if let Some(id) = self.deployment.as_ref().map(|deployment| deployment.id)
      && let Ok(Some(now)) = record.deployment(id)
    {
      self.deployment = Some(now);
    }
  }

  fn read_events(&mut self, record: &Record) -> Result<(), String> {
    loop {
      let after = match self.cursor {
        Some(after) => after,
        None => record.window_start()?,
      };

      match record.events(after, &KINDS, None)? {
        // The events after the cursor are past the window: the course goes on from its start, and a
        // deployment not yet known is looked for by its unit among those the record holds.
        EventsRead::Gone(through) => {
          self.cursor = Some(through);

          if self.deployment.is_none()
            && let Some(unit) = self.unit.clone()
          {
            self.deployment = newest(record, |deployment| deployment.unit_name.as_deref() == Some(&unit))?;
          }
        }
        EventsRead::Page(page) => {
          for event in page.items {
            self.cursor = Some(event.id);

            if let Ok(deployment) = serde_json::from_value::<Deployment>(event.data)
              && self.is_ours(&deployment)
            {
              self.deployment = Some(deployment);
            }
          }

          if !page.more {
            return Ok(());
          }
        }
      }
    }
  }

  // Whether a deployment an event carries is the one followed: by its id once known, by its unit before.
  fn is_ours(&self, deployment: &Deployment) -> bool {
    match (&self.deployment, &self.unit) {
      (Some(known), _) => known.id == deployment.id,
      (None, Some(unit)) => deployment.unit_name.as_deref() == Some(unit),
      (None, None) => false,
    }
  }
}

/// The newest deployment the record holds that `wanted` takes, read a page at a time.
pub fn newest(record: &Record, wanted: impl Fn(&Deployment) -> bool) -> Result<Option<Deployment>, String> {
  let mut query = PageQuery::default();

  loop {
    let page = record.deployments(&query)?;

    if let Some(found) = page.items.iter().find(|deployment| wanted(deployment)) {
      return Ok(Some(found.clone()));
    }

    match page.items.last() {
      Some(last) if page.more => query.after = Some(last.id),
      _ => return Ok(None),
    }
  }
}

/// Whether a deployment is of the repository named, a leading `bmo-` taken off both.
pub fn of_repository(deployment: &Deployment, repository: &str) -> bool {
  deployment.repository_name.as_deref().is_some_and(|name| same_name(name, repository))
}

/// Whether a deployment is of the commit asked for, whole or shortened.
pub fn of_commit(deployment: &Deployment, commit: &str) -> bool {
  let asked = commit.trim().to_ascii_lowercase();

  deployment
    .commit
    .as_deref()
    .is_some_and(|whole| whole.to_ascii_lowercase().starts_with(&asked))
}
