//! The service's one event stream: server-sent events from the record, resumed with `Last-Event-ID` or
//! `after` and narrowed by `kinds`, or the same events as a page to a reader asking for JSON.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use bmo_testbed_client::{ErrorCode, Event, EventKind, EventsQuery};
use bmo_testbed_core::policy::fill_marks;
use bmo_testbed_core::policy::service::SERVICE;
use bmo_testbed_record::EventsRead;
use futures_util::stream::{self, Stream};

use crate::{Failure, Ground, query, reading};

fn kinds_of(asked: Option<&str>) -> Result<Vec<EventKind>, Failure> {
  asked
    .unwrap_or_default()
    .split(',')
    .map(str::trim)
    .filter(|word| !word.is_empty())
    .map(|word| {
      EventKind::from_str(word).map_err(|_| {
        Failure::new(ErrorCode::BadRequest, fill_marks(&SERVICE.errors.unknown_kind, &[("kind", word)]))
      })
    })
    .collect()
}

fn gone(through: i64) -> Failure {
  Failure::new(ErrorCode::Gone, fill_marks(&SERVICE.errors.gone, &[("through", &through.to_string())]))
}

fn wants_json(headers: &HeaderMap) -> bool {
  headers
    .get(header::ACCEPT)
    .and_then(|value| value.to_str().ok())
    .is_some_and(|accept| accept.contains("application/json"))
}

fn last_event_id(headers: &HeaderMap) -> Option<i64> {
  headers
    .get("last-event-id")
    .and_then(|value| value.to_str().ok())
    .and_then(|id| id.trim().parse().ok())
}

fn sse_event(event: &Event) -> SseEvent {
  SseEvent::default()
    .event(event.kind.name())
    .id(event.id.to_string())
    .data(serde_json::to_string(event).unwrap_or_default())
}

pub(crate) async fn events(
  State(ground): State<Arc<Ground>>,
  headers: HeaderMap,
  asked: Result<Query<EventsQuery>, QueryRejection>,
) -> Result<Response, Failure> {
  let asked = query(asked)?;
  let kinds = kinds_of(asked.kinds.as_deref())?;
  let after = last_event_id(&headers).or(asked.after);

  if wants_json(&headers) {
    let limit = asked.limit;
    let read = reading(&ground, move |record| {
      let after = match after {
        Some(after) => after,
        None => record.window_start()?,
      };

      record.events(after, &kinds, limit)
    })
    .await?;

    return match read {
      EventsRead::Page(page) => Ok(Json(page).into_response()),
      EventsRead::Gone(through) => Err(gone(through)),
    };
  }

  let from = match after {
    Some(after) => after,
    None => reading(&ground, |record| record.last_event_id()).await?,
  };
  let first = {
    let kinds = kinds.clone();

    reading(&ground, move |record| record.events(from, &kinds, None)).await?
  };

  match first {
    EventsRead::Gone(through) => Err(gone(through)),
    EventsRead::Page(page) => Ok(
      Sse::new(stream_from(ground, kinds, from, page.items))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(SERVICE.keep_alive_seconds)))
        .into_response(),
    ),
  }
}

// The events the first reading found, then every one written after, asked of the record each policy
// interval for as long as the reader stays.
fn stream_from(
  ground: Arc<Ground>,
  kinds: Vec<EventKind>,
  from: i64,
  first: Vec<Event>,
) -> impl Stream<Item = Result<SseEvent, std::convert::Infallible>> {
  let cursor = first.last().map_or(from, |last| last.id);
  let every = ground.event_poll;

  stream::unfold((first, cursor), move |(mut waiting, cursor)| {
    let ground = Arc::clone(&ground);
    let kinds = kinds.clone();

    async move {
      let mut cursor = cursor;

      while waiting.is_empty() {
        tokio::time::sleep(every).await;

        let kinds = kinds.clone();

        if let Ok(EventsRead::Page(page)) =
          reading(&ground, move |record| record.events(cursor, &kinds, None)).await
        {
          if let Some(last) = page.items.last() {
            cursor = last.id;
          }
          waiting = page.items;
        }
      }

      let event = waiting.remove(0);

      Some((Ok(sse_event(&event)), (waiting, cursor)))
    }
  })
}
