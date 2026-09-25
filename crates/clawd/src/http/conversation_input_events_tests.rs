use axum::http::{HeaderMap, HeaderValue};
use claw_core::conversation_input::{
    ConversationInputContent, ConversationInputDeliveryMode, ConversationInputSource,
    ConversationInputSubmission,
};
use futures_util::StreamExt;

use super::*;

#[test]
fn cursor_prefers_query_and_accepts_last_event_id() {
    let mut headers = HeaderMap::new();
    headers.insert("last-event-id", HeaderValue::from_static("17"));

    assert_eq!(requested_cursor(&headers, Some(23)), Ok(23));
    assert_eq!(requested_cursor(&headers, None), Ok(17));
}

#[test]
fn invalid_last_event_id_is_rejected() {
    let mut headers = HeaderMap::new();
    headers.insert("last-event-id", HeaderValue::from_static("not-a-cursor"));

    assert_eq!(requested_cursor(&headers, None), Err(()));
}

#[test]
fn conversation_event_notifier_wakes_subscribers() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut receiver = state
        .metrics
        .conversation_input_event_notifier
        .subscribe(crate::conversation_input_event_transport::GLOBAL_CONVERSATION_EVENT_KEY);

    crate::conversation_input_event_transport::notify(&state);

    assert!(receiver.try_recv().is_ok());
}

fn state_with_event() -> (
    crate::AppState,
    String,
    OwnedConversationInputScope,
    Vec<ConversationInputEventRecord>,
) {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_key = crate::repo::auth::create_auth_key(&state, "admin").expect("create key");
    let identity = crate::repo::auth::resolve_auth_identity_by_key(&state, &user_key)
        .expect("resolve identity")
        .expect("identity");
    let scope = OwnedConversationInputScope {
        owner_principal_id: identity.principal_id.clone(),
        conversation: ConversationInputScopeRef {
            conversation_id: "conversation-event-stream-test".to_string(),
            agent_id: "main".to_string(),
            channel: "ui".to_string(),
            channel_account_id: "browser-session".to_string(),
        },
    };
    crate::repo::conversation_inputs::accept_conversation_input(
        &state.core.db,
        &crate::repo::conversation_inputs::AcceptConversationInput {
            owner_principal_id: identity.principal_id,
            submission: ConversationInputSubmission {
                schema_version: 1,
                client_message_id: "event-stream-message-1".to_string(),
                scope: scope.conversation.clone(),
                content: vec![ConversationInputContent::Text {
                    text: "Persist this input.".to_string(),
                }],
                delivery_mode: ConversationInputDeliveryMode::Defer,
                expected_task_id: None,
                expected_instruction_revision: None,
                source: ConversationInputSource::default(),
            },
            preparation_state:
                claw_core::conversation_input::ConversationInputPreparationState::Ready,
        },
    )
    .expect("accept input");
    let events = load_events(&state, &scope, 0).expect("load events");
    (state, user_key, scope, events)
}

#[tokio::test]
async fn stream_replays_durable_events_without_waiting_for_a_notification() {
    let (state, user_key, scope, events) = state_with_event();
    assert!(!events.is_empty());
    let receiver = state
        .metrics
        .conversation_input_event_notifier
        .subscribe(crate::conversation_input_event_transport::GLOBAL_CONVERSATION_EVENT_KEY);
    let stream = event_stream(EventStreamState {
        app: state,
        user_key,
        scope,
        receiver,
        pending: events.clone().into(),
        cursor: 0,
        replay_may_continue: events.len() >= EVENT_PAGE_SIZE as usize,
        follow: false,
        done: false,
    });
    futures_util::pin_mut!(stream);

    for _ in &events {
        assert!(stream.next().await.is_some());
    }
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn finite_replay_drains_more_than_one_event_page_without_waiting() {
    let (state, user_key, scope, _) = state_with_event();
    for index in 2..=101 {
        crate::repo::conversation_inputs::accept_conversation_input(
            &state.core.db,
            &crate::repo::conversation_inputs::AcceptConversationInput {
                owner_principal_id: scope.owner_principal_id.clone(),
                submission: ConversationInputSubmission {
                    schema_version: 1,
                    client_message_id: format!("event-stream-message-{index}"),
                    scope: scope.conversation.clone(),
                    content: vec![ConversationInputContent::Text {
                        text: format!("Persist input {index}."),
                    }],
                    delivery_mode: ConversationInputDeliveryMode::Defer,
                    expected_task_id: None,
                    expected_instruction_revision: None,
                    source: ConversationInputSource::default(),
                },
                preparation_state:
                    claw_core::conversation_input::ConversationInputPreparationState::Ready,
            },
        )
        .expect("accept input");
    }
    let events = load_events(&state, &scope, 0).expect("load first event page");
    assert_eq!(events.len(), EVENT_PAGE_SIZE as usize);
    let receiver = state
        .metrics
        .conversation_input_event_notifier
        .subscribe(crate::conversation_input_event_transport::GLOBAL_CONVERSATION_EVENT_KEY);
    let stream = event_stream(EventStreamState {
        app: state,
        user_key,
        scope,
        receiver,
        pending: events.into(),
        cursor: 0,
        replay_may_continue: true,
        follow: false,
        done: false,
    });
    futures_util::pin_mut!(stream);

    let replayed = tokio::time::timeout(Duration::from_secs(1), async {
        let mut count = 0;
        while stream.next().await.is_some() {
            count += 1;
        }
        count
    })
    .await
    .expect("finite replay must not wait for another notification");
    assert_eq!(replayed, 101);
}

#[tokio::test]
async fn live_stream_stops_after_its_auth_key_is_disabled() {
    let (state, user_key, scope, _) = state_with_event();
    let receiver = state
        .metrics
        .conversation_input_event_notifier
        .subscribe(crate::conversation_input_event_transport::GLOBAL_CONVERSATION_EVENT_KEY);
    let stream = event_stream(EventStreamState {
        app: state.clone(),
        user_key: user_key.clone(),
        scope,
        receiver,
        pending: VecDeque::new(),
        cursor: u64::MAX,
        replay_may_continue: false,
        follow: true,
        done: false,
    });
    futures_util::pin_mut!(stream);
    state
        .core
        .db
        .get()
        .expect("db")
        .execute(
            "UPDATE auth_keys SET enabled = 0 WHERE user_key = ?1",
            [&user_key],
        )
        .expect("disable key");
    crate::conversation_input_event_transport::notify(&state);

    let next = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("stream wake");
    assert!(next.is_none());
}
