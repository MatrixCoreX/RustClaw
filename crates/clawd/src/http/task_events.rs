use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::repo::{check_task_view_access, get_task_query_record, TaskViewerAccessError};
use crate::{ApiResponse, AppState};

const SSE_KEEPALIVE_SECONDS: u64 = 15;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct TaskEventQuery {
    cursor: Option<u64>,
    follow: Option<bool>,
}

struct EventStreamState {
    app: AppState,
    task_id: String,
    receiver: broadcast::Receiver<u64>,
    pending: VecDeque<Value>,
    cursor: u64,
    available_newest_seq: Option<u64>,
    follow_live: bool,
    terminal_seen: bool,
    done: bool,
}

pub(crate) async fn stream_task_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
    Query(query): Query<TaskEventQuery>,
) -> Response {
    let read_result = get_task_query_record(&state, task_id);
    let Some((_task, task_user_key, channel)) = (match read_result {
        Ok(record) => record,
        Err(error) => {
            tracing::error!("read task for event stream failed: {}", error);
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "task_event_store_error");
        }
    }) else {
        return api_error(StatusCode::NOT_FOUND, "task_not_found");
    };
    let provided_key = headers
        .get("x-rustclaw-key")
        .and_then(|value| value.to_str().ok());
    if let Err(error) =
        check_task_view_access(&state, task_user_key.as_deref(), &channel, provided_key)
    {
        return match error {
            TaskViewerAccessError::AuthLookup(source) => {
                tracing::error!("resolve task event viewer failed: {}", source);
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "task_event_auth_lookup_failed",
                )
            }
            TaskViewerAccessError::TaskOwnerMismatch => {
                api_error(StatusCode::UNAUTHORIZED, "task_owner_mismatch")
            }
            TaskViewerAccessError::InvalidUserKey => {
                api_error(StatusCode::UNAUTHORIZED, "invalid_user_key")
            }
        };
    }

    let cursor = match requested_cursor(&headers, query.cursor) {
        Ok(cursor) => cursor,
        Err(()) => return api_error(StatusCode::BAD_REQUEST, "invalid_event_cursor"),
    };
    let task_id = task_id.to_string();
    let receiver = state.metrics.task_event_notifier.subscribe(&task_id);
    crate::task_event_transport::publish_persisted_task_events(&state, &task_id);
    crate::task_event_transport::publish_task_status_projection(&state, &task_id);
    let replay = match crate::task_event_transport::replay_events_after(&state, &task_id, cursor) {
        Ok(replay) => replay,
        Err(error) => {
            tracing::error!(
                "read task event replay failed task_id={} error={}",
                task_id,
                error
            );
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_event_replay_failed",
            );
        }
    };
    let mut pending = VecDeque::from(replay.events);
    if replay.cursor_expired {
        pending.push_front(cursor_expired_control_event(
            &task_id,
            cursor,
            replay.oldest_seq,
            replay.newest_seq,
            replay.replay_source,
        ));
    } else if replay.archive_recovered {
        pending.push_front(archive_replay_control_event(
            &task_id,
            cursor,
            replay.oldest_seq,
            replay.newest_seq,
            replay.latest_snapshot.as_ref(),
        ));
    }
    let stream = event_stream(EventStreamState {
        app: state,
        task_id,
        receiver,
        pending,
        cursor,
        available_newest_seq: replay.newest_seq,
        follow_live: query.follow.unwrap_or(true),
        terminal_seen: false,
        done: false,
    });
    let mut response = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(SSE_KEEPALIVE_SECONDS))
                .text("heartbeat"),
        )
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    response.headers_mut().insert(
        axum::http::HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}

pub(crate) async fn get_task_event_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((task_id, artifact_id)): Path<(Uuid, String)>,
) -> Response {
    let read_result = get_task_query_record(&state, task_id);
    let Some((_task, task_user_key, channel)) = (match read_result {
        Ok(record) => record,
        Err(error) => {
            tracing::error!("read task for event artifact failed: {}", error);
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "task_event_store_error");
        }
    }) else {
        return api_error(StatusCode::NOT_FOUND, "task_not_found");
    };
    let provided_key = headers
        .get("x-rustclaw-key")
        .and_then(|value| value.to_str().ok());
    if let Err(error) =
        check_task_view_access(&state, task_user_key.as_deref(), &channel, provided_key)
    {
        return match error {
            TaskViewerAccessError::AuthLookup(source) => {
                tracing::error!("resolve task event artifact viewer failed: {}", source);
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "task_event_auth_lookup_failed",
                )
            }
            TaskViewerAccessError::TaskOwnerMismatch => {
                api_error(StatusCode::UNAUTHORIZED, "task_owner_mismatch")
            }
            TaskViewerAccessError::InvalidUserKey => {
                api_error(StatusCode::UNAUTHORIZED, "invalid_user_key")
            }
        };
    }
    match crate::task_event_transport::read_event_artifact(
        &state,
        &task_id.to_string(),
        &artifact_id,
    ) {
        Ok(Some(value)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(value),
                error: None,
            }),
        )
            .into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "task_event_artifact_not_found"),
        Err(error) => {
            tracing::error!(
                "read task event artifact failed task_id={} error={}",
                task_id,
                error
            );
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_event_artifact_read_failed",
            )
        }
    }
}

fn event_stream(
    state: EventStreamState,
) -> impl Stream<Item = Result<Event, Infallible>> + Send + 'static {
    stream::unfold(state, |mut state| async move {
        loop {
            if state.done {
                return None;
            }
            if let Some(value) = state.pending.pop_front() {
                let event = sse_event(&value);
                if let Some(seq) = value.get("seq").and_then(Value::as_u64) {
                    if seq <= state.cursor {
                        continue;
                    }
                    state.cursor = seq;
                }
                if event_is_terminal(&value) {
                    state.terminal_seen = true;
                }
                if (state.terminal_seen
                    || (!state.follow_live
                        && state.pending.is_empty()
                        && !has_unread_persisted_events(&state)))
                    && state.pending.is_empty()
                {
                    state.done = true;
                }
                return Some((Ok(event), state));
            }
            if has_unread_persisted_events(&state) {
                match crate::task_event_transport::replay_events_after(
                    &state.app,
                    &state.task_id,
                    state.cursor,
                ) {
                    Ok(replay) if !replay.events.is_empty() => {
                        state.available_newest_seq = replay.newest_seq;
                        state.pending.extend(replay.events);
                        continue;
                    }
                    Ok(replay) => {
                        state.available_newest_seq = replay.newest_seq;
                    }
                    Err(error) => {
                        tracing::warn!(
                            "task event archive replay failed task_id={} error={}",
                            state.task_id,
                            error
                        );
                        state.pending.push_back(stream_error_control_event(
                            &state.task_id,
                            "task_event_archive_replay_failed",
                        ));
                        state.done = true;
                        continue;
                    }
                }
            }
            if !state.follow_live {
                state.done = true;
                continue;
            }

            match state.receiver.recv().await {
                Ok(seq) if seq <= state.cursor => continue,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    match crate::task_event_transport::replay_events_after(
                        &state.app,
                        &state.task_id,
                        state.cursor,
                    ) {
                        Ok(replay) => {
                            state.available_newest_seq = replay.newest_seq;
                            state.pending.extend(replay.events);
                            if replay.cursor_expired {
                                state.pending.push_front(cursor_expired_control_event(
                                    &state.task_id,
                                    state.cursor,
                                    replay.oldest_seq,
                                    replay.newest_seq,
                                    replay.replay_source,
                                ));
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                "task event follow replay failed task_id={} error={}",
                                state.task_id,
                                error
                            );
                            state.pending.push_back(stream_error_control_event(
                                &state.task_id,
                                "task_event_follow_replay_failed",
                            ));
                            state.done = true;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    state.done = true;
                }
            }
        }
    })
}

fn has_unread_persisted_events(state: &EventStreamState) -> bool {
    state
        .available_newest_seq
        .is_some_and(|newest| state.cursor < newest)
}

fn requested_cursor(headers: &HeaderMap, query_cursor: Option<u64>) -> Result<u64, ()> {
    let header_cursor = headers
        .get(axum::http::HeaderName::from_static("last-event-id"))
        .map(|value| value.to_str().map_err(|_| ()))
        .transpose()?
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<u64>().map_err(|_| ()))
        .transpose()?;
    Ok(query_cursor.or(header_cursor).unwrap_or(0))
}

fn sse_event(value: &Value) -> Event {
    let kind = value
        .get("event_kind")
        .or_else(|| value.get("event_type"))
        .and_then(Value::as_str)
        .unwrap_or("task_event");
    let mut event = Event::default().event(kind);
    if let Some(seq) = value.get("seq").and_then(Value::as_u64) {
        event = event.id(seq.to_string());
    }
    event
        .json_data(value)
        .unwrap_or_else(|_| Event::default().event("stream_encoding_error"))
}

fn event_is_terminal(value: &Value) -> bool {
    value
        .get("event_kind")
        .or_else(|| value.get("event_type"))
        .and_then(Value::as_str)
        == Some("task_final")
}

fn cursor_expired_control_event(
    task_id: &str,
    requested_cursor: u64,
    oldest_seq: Option<u64>,
    newest_seq: Option<u64>,
    replay_source: &str,
) -> Value {
    json!({
        "schema_version": 1,
        "task_id": task_id,
        "event_kind": "cursor_expired",
        "event_type": "cursor_expired",
        "payload": {
            "requested_cursor": requested_cursor,
            "oldest_available_seq": oldest_seq,
            "newest_available_seq": newest_seq,
            "replay_mode": "available_suffix",
            "replay_source": replay_source,
        },
    })
}

fn archive_replay_control_event(
    task_id: &str,
    requested_cursor: u64,
    oldest_seq: Option<u64>,
    newest_seq: Option<u64>,
    latest_snapshot: Option<&Value>,
) -> Value {
    json!({
        "schema_version": 1,
        "task_id": task_id,
        "event_kind": "archive_replay",
        "event_type": "archive_replay",
        "payload": {
            "requested_cursor": requested_cursor,
            "oldest_available_seq": oldest_seq,
            "newest_available_seq": newest_seq,
            "replay_mode": "archive_recovery",
            "latest_snapshot": latest_snapshot,
        },
    })
}

fn stream_error_control_event(task_id: &str, error_code: &str) -> Value {
    json!({
        "schema_version": 1,
        "task_id": task_id,
        "event_kind": "stream_error",
        "event_type": "stream_error",
        "payload": { "error_code": error_code },
    })
}

fn api_error(status: StatusCode, error_code: &'static str) -> Response {
    (
        status,
        Json(ApiResponse::<Value> {
            ok: false,
            data: None,
            error: Some(error_code.to_string()),
        }),
    )
        .into_response()
}

#[cfg(test)]
#[path = "task_events_tests.rs"]
mod tests;
