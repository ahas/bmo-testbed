//! The two lanes as they stand, as the queue page answers them, the runs placed elsewhere from each among
//! them, and the watch that writes a `queue.changed` event where a lane's holders, line or placed runs differ
//! from its last reading.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use bmo_testbed_client::{EventKind, LaneName, Page, PageQuery, Queue, QueueEntry};
use bmo_testbed_core::placement::{PlacedEntry, read_placed};
use bmo_testbed_core::turn::{Env, Lane, TurnEntry, lane_width, lock_path, machine_threads, read_queue};
use bmo_testbed_record::{Record, say_unwritten};

use crate::{Answer, Ground, query};

fn entry_on(entry: &TurnEntry, machine: Option<&str>) -> QueueEntry {
  QueueEntry {
    scope: entry.scope.clone(),
    checkout: entry.checkout.clone(),
    command: entry.command.clone(),
    pid: entry.pid,
    asked_at: entry.asked,
    held_at: entry.since,
    machine: machine.map(str::to_owned),
    kept: false,
    front: entry.front,
    next: entry.next,
  }
}

fn entry(entry: &TurnEntry) -> QueueEntry {
  entry_on(entry, None)
}

fn placed(placed: &PlacedEntry) -> QueueEntry {
  entry_on(&placed.entry, Some(&placed.machine))
}

/// One lane as it stands now.
pub fn queue_of(lane: Lane, env: &Env) -> Queue {
  let lock = lock_path(env, lane);
  let line = read_queue(&lock);
  let holders: Vec<QueueEntry> = line.holders.iter().map(entry).collect();

  Queue {
    name: match lane {
      Lane::Wide => LaneName::Wide,
      Lane::Narrow => LaneName::Narrow,
    },
    width: lane_width(lane, env, machine_threads()) as i64,
    holder: holders.first().cloned(),
    holders,
    waiting: line.waiting.iter().map(entry).collect(),
    placed: read_placed(&lock).iter().map(placed).collect(),
  }
}

/// Both lanes as they stand now, the wide one first, up to the limit the query names.
pub fn queue_page(env: &Env, asked: &PageQuery) -> Page<Queue> {
  let lanes: Vec<Queue> = Lane::ALL.iter().map(|lane| queue_of(*lane, env)).collect();
  let limit = asked.limit.map_or(lanes.len(), |limit| limit.max(1) as usize);
  let more = lanes.len() > limit;

  Page {
    items: lanes.into_iter().take(limit).collect(),
    more,
  }
}

pub(crate) async fn queues(
  State(ground): State<Arc<Ground>>,
  asked: Result<Query<PageQuery>, QueryRejection>,
) -> Answer<Page<Queue>> {
  let asked = query(asked)?;

  Ok(Json(queue_page(&ground.env, &asked)))
}

/// Reads the lanes every interval for as long as the service runs, and writes a `queue.changed` event for
/// each lane whose holders, line or placed runs differ from its last reading. The lanes as they stand when it
/// is called are the baseline, which writes nothing.
pub fn watch_queues(ground: Arc<Ground>, every: Duration) -> impl Future<Output = ()> {
  let reading = |env: &Env| -> Vec<Queue> { Lane::ALL.iter().map(|lane| queue_of(*lane, env)).collect() };
  let mut last = reading(&ground.env);

  async move {
    loop {
      tokio::time::sleep(every).await;

      let now = reading(&ground.env);
      let changed: Vec<Queue> = now
        .iter()
        .zip(&last)
        .filter(|(now, last)| now != last)
        .map(|(now, _)| now.clone())
        .collect();

      if !changed.is_empty() {
        let data_root = ground.data_root.clone();
        let posted = tokio::task::spawn_blocking(move || {
          let mut record = Record::open(&data_root)?;

          changed
            .iter()
            .try_for_each(|queue| record.post_event(EventKind::QueueChanged, queue).map(|_| ()))
        })
        .await;

        if let Ok(Err(why)) = posted {
          say_unwritten(&why);
        }
      }

      last = now;
    }
  }
}
