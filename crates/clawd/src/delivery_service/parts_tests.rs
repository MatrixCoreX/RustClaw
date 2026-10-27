use super::*;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn fixture() -> (AppState, ClaimedTask, Value, ChannelDeliveryEnvelope) {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let payload = json!({"channel":"whatsapp", "external_chat_id":"fixture@chat", "channel_ingress":{
        "schema_version":1,"channel":"whatsapp","adapter":"whatsapp_web","external_user_id":"one","external_chat_id":"fixture@chat",
        "reply_target":{"kind":"chat","external_id":"fixture@chat"},"locale":"en"}});
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: Uuid::new_v4().to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "whatsapp".into(),
        external_user_id: Some("one".into()),
        external_chat_id: Some("fixture@chat".into()),
        kind: "ask".into(),
        payload_json: payload.to_string(),
    };
    let envelope = build_daemon_delivery_envelope(
        &state,
        &task,
        &payload,
        "fixture",
        ChannelDeliverySource::ImmediateDaemon,
        ChannelTaskDeliveryContent::Full,
        None,
    )
    .unwrap();
    (state, task, payload, envelope)
}

#[test]
fn immutable_part_plan_binds_recipient_actor_and_content() {
    let (state, task, _, envelope) = fixture();
    let items = vec!["one".into(), "two".into()];
    let plan = load_or_create_plan(&state, &task, &envelope, &items).unwrap();
    let child = part_envelope(&envelope, &plan, 0, "one");
    assert_eq!(
        child.idempotency_key,
        part_envelope(&envelope, &plan, 0, "one").idempotency_key
    );
    assert_ne!(
        child.idempotency_key,
        part_envelope(&envelope, &plan, 1, "two").idempotency_key
    );
    let mut changed = envelope.clone();
    changed.reply_target.external_id = "other".into();
    assert!(load_or_create_plan(&state, &task, &changed, &items).is_err());
    assert!(
        load_or_create_plan(&state, &task, &envelope, &["changed".into(), "two".into()]).is_err()
    );
    let mut changed_task = task.clone();
    changed_task.user_id = 99;
    assert!(load_or_create_plan(&state, &changed_task, &envelope, &items).is_err());
}

#[test]
fn part_projection_separates_transcript_and_media_without_interpreting_prose() {
    let (state, _, _, mut envelope) = fixture();
    envelope.text_segments[0].text =
        "fixture prose\nFILE:/tmp/transcript.txt\nIMAGE_FILE:/tmp/one.jpg".into();
    assert_eq!(
        texts(&state, &envelope).unwrap(),
        vec![
            "fixture prose",
            "FILE:/tmp/transcript.txt",
            "IMAGE_FILE:/tmp/one.jpg"
        ]
    );
}

#[test]
fn wechat_part_plan_keeps_transcripts_ahead_of_slow_video_without_size_dependent_identity() {
    let (state, _, _, mut envelope) = fixture();
    envelope.channel = ChannelKind::Wechat;
    envelope.text_segments[0].text =
        "fixture\nVIDEO_FILE:/tmp/clip.mp4\nFILE:/tmp/transcript.txt\nIMAGE_FILE:/tmp/photo.jpg"
            .into();
    assert_eq!(
        texts(&state, &envelope).unwrap(),
        vec![
            "fixture",
            "FILE:/tmp/transcript.txt",
            "IMAGE_FILE:/tmp/photo.jpg",
            "VIDEO_FILE:/tmp/clip.mp4"
        ]
    );
}

#[derive(Clone, Default)]
struct Mock {
    text: Arc<AtomicUsize>,
    file: Arc<AtomicUsize>,
}

async fn accept_text(State(mock): State<Mock>) -> Json<Value> {
    let n = mock.text.fetch_add(1, Ordering::SeqCst);
    Json(json!({"ok":true,"message_ids":[format!("text-{n}")]}))
}
async fn flaky_file(State(mock): State<Mock>) -> (StatusCode, Json<Value>) {
    let n = mock.file.fetch_add(1, Ordering::SeqCst);
    if n == 0 {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":{"code":"rate_limit"}})),
        )
    } else {
        (
            StatusCode::OK,
            Json(json!({"ok":true,"message_ids":[format!("file-{n}")]})),
        )
    }
}

#[tokio::test]
async fn explicit_owner_retry_uses_the_original_plan_without_resending_accepted_text() {
    use crate::repo::channel_delivery_receipt::reconciliation::{self, Decision, Operation};
    let (mut state, mut task, payload, _) = fixture();
    state.seed_test_auth_identity("part-owner", "user");
    let identity = crate::resolve_auth_identity_by_key(&state, "part-owner")
        .unwrap()
        .unwrap();
    task.user_key = Some(identity.user_key.clone());
    {
        let db = state.core.db.get().unwrap();
        crate::repo::channel_delivery_outbox::ensure_channel_delivery_outbox_schema(&db).unwrap();
        db.execute("INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,external_user_id,external_chat_id,kind,payload_json,status,created_at,updated_at) VALUES (?1,1,2,'part-owner',?2,'whatsapp','one','fixture@chat','ask',?3,'succeeded','1','1')",rusqlite::params![task.task_id,identity.principal_id,task.payload_json]).unwrap();
        db.execute("INSERT INTO channel_bindings(channel,external_user_id,external_chat_id,user_key,bound_at,updated_at) VALUES ('whatsapp','one','fixture@chat','part-owner','1','1')",[]).unwrap();
    }
    let mock = Mock::default();
    let app = Router::new()
        .route("/v1/send-text", post(accept_text))
        .route("/v1/send-result", post(flaky_file))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.channels.whatsapp_web_enabled = true;
    state.channels.whatsapp_web_bridge_base_url =
        format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let file = std::env::temp_dir().join(format!("delivery-explicit-{}.txt", Uuid::new_v4()));
    std::fs::write(&file, b"fixture transcript").unwrap();
    let envelope = build_daemon_delivery_envelope(
        &state,
        &task,
        &payload,
        &format!("fixture text\nFILE:{}", file.display()),
        ChannelDeliverySource::ImmediateDaemon,
        ChannelTaskDeliveryContent::Full,
        None,
    )
    .unwrap();
    assert!(
        deliver(&state, &task, &payload, &envelope)
            .await
            .unwrap()
            .retryable
    );
    {
        let mut db = state.core.db.get().unwrap();
        let now = crate::now_ts_u64() as i64;
        let view = reconciliation::inspect(&db, &task.task_id, now).unwrap();
        let decision = Decision {
            operation: Operation::RetryFailed,
            snapshot_digest: view["snapshot_digest"].as_str().unwrap().into(),
            decision_id: Uuid::new_v4(),
            confirmed_duplicate_risk: false,
        };
        assert_eq!(
            reconciliation::reconcile(&mut db, &task.task_id, &identity, &decision, now).unwrap()
                ["status"],
            "queued"
        );
    }
    assert!(deliver(&state, &task, &payload, &envelope)
        .await
        .unwrap()
        .accepted());
    assert_eq!(mock.text.load(Ordering::SeqCst), 1);
    assert_eq!(mock.file.load(Ordering::SeqCst), 2);
    std::fs::remove_file(file).unwrap();
    server.abort();
}

#[tokio::test]
async fn partial_file_retry_survives_service_restart_and_never_resends_accepted_text() {
    let (mut state, task, payload, mut envelope) = fixture();
    let db_file = std::env::temp_dir().join(format!("delivery-restart-{}.sqlite", Uuid::new_v4()));
    let open_pool = || {
        r2d2::Pool::builder()
            .max_size(1)
            .build(r2d2_sqlite::SqliteConnectionManager::file(&db_file))
            .unwrap()
    };
    state.core.db = open_pool();
    state = state.with_seeded_db_schema();
    let mock = Mock::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/v1/send-text", post(accept_text))
        .route("/v1/send-result", post(flaky_file))
        .with_state(mock.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    state.channels.whatsapp_web_enabled = true;
    state.channels.whatsapp_web_bridge_base_url = format!("http://{address}");
    let file = std::env::temp_dir().join(format!("delivery-{}.txt", Uuid::new_v4()));
    std::fs::write(&file, b"fixture transcript").unwrap();
    envelope.text_segments[0].text = format!("fixture text\nFILE:{}", file.display());
    let first = deliver(&state, &task, &payload, &envelope).await.unwrap();
    assert_eq!(
        first.receipt.as_ref().unwrap().status,
        ChannelDeliveryStatus::Partial
    );
    assert!(first.retryable);
    assert_eq!(mock.text.load(Ordering::SeqCst), 1);
    // Close the original pool and reopen the on-disk database before retry.
    let mut restarted = state.clone();
    drop(state);
    restarted.core.db = open_pool();
    let (second, concurrent) = tokio::join!(
        deliver(&restarted, &task, &payload, &envelope),
        deliver(&restarted, &task, &payload, &envelope)
    );
    let second = second.unwrap();
    let concurrent = concurrent.unwrap();
    assert!(second.accepted() || concurrent.accepted());
    let last = deliver(&restarted, &task, &payload, &envelope)
        .await
        .unwrap();
    assert!(last.accepted());
    assert_eq!(mock.text.load(Ordering::SeqCst), 1);
    assert_eq!(mock.file.load(Ordering::SeqCst), 2);
    let plan = load_or_create_plan(
        &restarted,
        &task,
        &envelope,
        &texts(&restarted, &envelope).unwrap(),
    )
    .unwrap();
    let accepted_text = part_envelope(&envelope, &plan, 0, "fixture text");
    assert!(matches!(
        crate::repo::claim_channel_delivery_dispatch(&restarted.core.db, &accepted_text, 120)
            .unwrap(),
        ClaimChannelDeliveryDispatchOutcome::ExistingReceipt(_)
    ));
    std::fs::remove_file(file).unwrap();
    drop(restarted);
    std::fs::remove_file(db_file).unwrap();
    server.abort();
}

#[test]
fn transport_timeouts_and_invalid_acknowledgements_are_not_safe_to_retry() {
    use claw_core::channel_provider_error::{ChannelProviderError, ChannelProviderTransportKind};
    let timeout = ChannelProviderError::from_transport(
        "fixture",
        "send",
        ChannelProviderTransportKind::Timeout,
        "fixture",
    );
    assert!(!delivery_failure_fields(&timeout.to_string()).4);
    let bad = ChannelProviderError::from_machine_failure(
        "fixture",
        "send",
        claw_core::channel_provider_error::ChannelProviderFailureClass::InvalidResponse,
        Some(200),
        None,
        None,
        "fixture",
    );
    assert!(!delivery_failure_fields(&bad.to_string()).4);
}

#[tokio::test]
async fn canceled_dispatch_keeps_accepted_prefix_and_requires_query_after_lease_expiry() {
    let (mut state, task, payload, mut envelope) = fixture();
    let mock = Mock::default();
    let gate = Arc::new(tokio::sync::Notify::new());
    let file_gate = gate.clone();
    let app = Router::new()
        .route("/v1/send-text", post(accept_text))
        .route(
            "/v1/send-result",
            post(move |State(mock): State<Mock>| {
                let gate = file_gate.clone();
                async move {
                    mock.file.fetch_add(1, Ordering::SeqCst);
                    gate.notified().await;
                    Json(json!({"ok":true,"message_ids":["file-pending"]}))
                }
            }),
        )
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.channels.whatsapp_web_enabled = true;
    state.channels.whatsapp_web_bridge_base_url =
        format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let file = std::env::temp_dir().join(format!("canceled-delivery-{}.txt", Uuid::new_v4()));
    std::fs::write(&file, b"fixture transcript").unwrap();
    envelope.text_segments[0].text = format!("fixture text\nFILE:{}", file.display());
    let dispatch = {
        let (state, task, payload, envelope) = (
            state.clone(),
            task.clone(),
            payload.clone(),
            envelope.clone(),
        );
        tokio::spawn(async move { deliver(&state, &task, &payload, &envelope).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while mock.file.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    dispatch.abort();
    assert!(dispatch.await.unwrap_err().is_cancelled());
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE channel_delivery_dispatch_claims SET lease_expires_at_ts=1",
            [],
        )
        .unwrap();
    let result = deliver(&state, &task, &payload, &envelope).await.unwrap();
    assert_eq!(result.status, ChannelDeliveryServiceStatus::QueryRequired);
    assert!(!result.retryable);
    assert_eq!(mock.text.load(Ordering::SeqCst), 1);
    assert_eq!(mock.file.load(Ordering::SeqCst), 1);
    gate.notify_one();
    server.abort();
    std::fs::remove_file(file).unwrap();
}

#[test]
fn delivered_prefix_does_not_settle_or_disable_retry_of_an_unsent_remainder() {
    let (state, _, _, envelope) = fixture();
    let accepted = result_from_existing_receipt(accepted_delivery_receipt(
        &envelope,
        crate::channel_send::ChannelSendOutcome {
            provider_message_ids: vec!["accepted-prefix".into()],
        },
        crate::now_ts_u64(),
    ));
    let mut failure = accepted_delivery_receipt(&envelope, Default::default(), crate::now_ts_u64());
    failure.status = ChannelDeliveryStatus::Failed;
    failure.error_code = Some("channel.provider.rate_limited".into());
    failure.diagnostic_id = Some("fixture:rate".into());
    failure.retryable = true;
    let result = aggregate(
        &envelope,
        &[accepted, result_from_existing_receipt(failure)],
    );
    crate::repo::record_channel_delivery_receipt(&state.core.db, result.receipt.as_ref().unwrap())
        .unwrap();
    crate::repo::record_whatsapp_cloud_provider_status(
        &state.core.db,
        "accepted-prefix",
        claw_core::channel_whatsapp_cloud::WhatsappDeliveryEventStatus::Delivered,
        crate::now_ts_u64() + 1,
        None,
    )
    .unwrap();
    let db = state.core.db.get().unwrap();
    let stored = crate::repo::channel_delivery_receipt::load_channel_delivery_receipt_from_db(
        &db,
        &envelope.idempotency_key,
    )
    .unwrap()
    .unwrap();
    assert_eq!(stored.status, ChannelDeliveryStatus::Partial);
    assert!(stored.retryable);
}

#[tokio::test]
async fn changed_attachment_is_not_sent_and_parent_cleanup_removes_child_receipts() {
    let (state, task, payload, mut envelope) = fixture();
    let file = std::env::temp_dir().join(format!("changed-delivery-{}.txt", Uuid::new_v4()));
    std::fs::write(&file, b"original").unwrap();
    envelope.text_segments[0].text = format!("fixture text\nFILE:{}", file.display());
    let items = texts(&state, &envelope).unwrap();
    let plan = load_or_create_plan(&state, &task, &envelope, &items).unwrap();
    let text = part_envelope(&envelope, &plan, 0, &items[0]);
    let mut receipt = accepted_delivery_receipt(
        &text,
        crate::channel_send::ChannelSendOutcome {
            provider_message_ids: vec!["accepted-text".into()],
        },
        crate::now_ts_u64(),
    );
    receipt.provider_message_ids.clear();
    crate::repo::record_channel_delivery_receipt(&state.core.db, &receipt).unwrap();
    std::fs::write(&file, b"changed").unwrap();
    let result = deliver(&state, &task, &payload, &envelope).await.unwrap();
    let receipt = result.receipt.unwrap();
    assert_eq!(receipt.status, ChannelDeliveryStatus::Partial);
    assert_eq!(receipt.parts[0].status, ChannelDeliveryStatus::Accepted);
    assert_eq!(
        receipt.parts[1].error_code.as_deref(),
        Some("channel_delivery_artifact_changed_or_expired")
    );
    assert!(!receipt.retryable);
    let db = state.core.db.get().unwrap();
    db.execute(
        "DELETE FROM channel_delivery_receipts WHERE idempotency_key=?1",
        params![envelope.idempotency_key],
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM channel_delivery_part_plans",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM channel_delivery_receipts",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    std::fs::remove_file(file).unwrap();
}
