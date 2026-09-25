use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use tokio::sync::broadcast;

use claw_core::conversation_input::{ConversationInputScopeRef, OwnedConversationInputScope};

use crate::repo::conversation_inputs::ConversationInputEventRecord;
use crate::AppState;

const SSE_KEEPALIVE_SECONDS: u64 = 15;
const EVENT_PAGE_SIZE: u32 = 100;

#[derive(Debug, Deserialize)]
pub(crate) struct ConversationInputEventStreamQuery {
    #[serde(default = "default_agent_id")]
    agent_id: String,
    channel: String,
    #[serde(default)]
    channel_account_id: String,
    #[serde(default)]
    cursor: Option<u64>,
    #[serde(default)]
    follow: Option<bool>,
}

struct EventStreamState {
    app: AppState,
    user_key: String,
    scope: OwnedConversationInputScope,
    receiver: broadcast::Receiver<u64>,
    pending: VecDeque<ConversationInputEventRecord>,
    cursor: u64,
    replay_may_continue: bool,
    follow: bool,
    done: bool,
}

pub(crate) async fn stream_conversation_input_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    Query(query): Query<ConversationInputEventStreamQuery>,
) -> Response {
    let identity = match crate::require_auth_identity_for_api::<serde_json::Value>(&state, &headers)
    {
        Ok(identity) => identity,
        Err(response) => return response.into_response(),
    };
    let scope = OwnedConversationInputScope {
        owner_principal_id: identity.principal_id.clone(),
        conversation: ConversationInputScopeRef {
            conversation_id,
            agent_id: query.agent_id,
            channel: query.channel,
            channel_account_id: query.channel_account_id,
        },
    };
    if !scope.validate() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "conversation_input_invalid_request",
        );
    }
    let cursor = match requested_cursor(&headers, query.cursor) {
        Ok(cursor) => cursor,
        Err(()) => return api_error(StatusCode::BAD_REQUEST, "invalid_event_cursor"),
    };
    let receiver = state
        .metrics
        .conversation_input_event_notifier
        .subscribe(crate::conversation_input_event_transport::GLOBAL_CONVERSATION_EVENT_KEY);
    let pending = match load_events(&state, &scope, cursor) {
        Ok(events) => VecDeque::from(events),
        Err(response) => return response,
    };
    let replay_may_continue = pending.len() >= EVENT_PAGE_SIZE as usize;
    let stream = event_stream(EventStreamState {
        app: state,
        user_key: identity.user_key,
        scope,
        receiver,
        pending,
        cursor,
        replay_may_continue,
        follow: query.follow.unwrap_or(true),
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

fn event_stream(
    state: EventStreamState,
) -> impl Stream<Item = Result<Event, Infallible>> + Send + 'static {
    stream::unfold(state, |mut state| async move {
        loop {
            if state.done {
                return None;
            }
            if let Some(record) = state.pending.pop_front() {
                state.cursor = state.cursor.max(record.event_seq);
                return Some((Ok(sse_event(&record)), state));
            }
            if state.replay_may_continue {
                match load_events(&state.app, &state.scope, state.cursor) {
                    Ok(events) => {
                        state.replay_may_continue = events.len() >= EVENT_PAGE_SIZE as usize;
                        state.pending = VecDeque::from(events);
                        continue;
                    }
                    Err(_) => {
                        state.done = true;
                        continue;
                    }
                }
            }
            if !state.follow {
                state.done = true;
                continue;
            }
            match tokio::time::timeout(
                Duration::from_secs(SSE_KEEPALIVE_SECONDS),
                state.receiver.recv(),
            )
            .await
            {
                Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) | Err(_) => {}
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    state.done = true;
                    continue;
                }
            }
            let authorized =
                crate::repo::auth::resolve_auth_identity_by_key(&state.app, &state.user_key)
                    .ok()
                    .flatten()
                    .is_some_and(|identity| {
                        identity.principal_id == state.scope.owner_principal_id
                    });
            if !authorized {
                state.done = true;
                continue;
            }
            match load_events(&state.app, &state.scope, state.cursor) {
                Ok(events) => {
                    state.replay_may_continue = events.len() >= EVENT_PAGE_SIZE as usize;
                    state.pending = VecDeque::from(events);
                }
                Err(_) => {
                    state.done = true;
                    continue;
                }
            }
        }
    })
}

fn load_events(
    state: &AppState,
    scope: &OwnedConversationInputScope,
    cursor: u64,
) -> Result<Vec<ConversationInputEventRecord>, Response> {
    crate::repo::conversation_inputs::list_conversation_input_events(
        &state.core.db,
        scope,
        cursor,
        EVENT_PAGE_SIZE,
    )
    .map_err(|error| {
        tracing::error!(%error, "conversation_input_event_stream_replay_failed");
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "conversation_input_event_replay_failed",
        )
    })
}

fn requested_cursor(headers: &HeaderMap, query_cursor: Option<u64>) -> Result<u64, ()> {
    if let Some(cursor) = query_cursor {
        return Ok(cursor);
    }
    headers
        .get("last-event-id")
        .map(|value| value.to_str().map_err(|_| ())?.parse().map_err(|_| ()))
        .transpose()
        .map(|cursor| cursor.unwrap_or(0))
}

fn sse_event(record: &ConversationInputEventRecord) -> Event {
    Event::default()
        .id(record.event_seq.to_string())
        .event(record.event_kind.clone())
        .json_data(record)
        .unwrap_or_else(|_| Event::default().event("serialization_error"))
}

fn api_error(status: StatusCode, code: &'static str) -> Response {
    crate::api_err::<serde_json::Value>(status, code).into_response()
}

fn default_agent_id() -> String {
    "main".to_string()
}

#[cfg(test)]
#[path = "conversation_input_events_tests.rs"]
mod tests;
