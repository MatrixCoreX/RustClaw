use super::*;

fn select_context(
    source: &TaskDeliveryRecord,
    request: &RecallRequest,
) -> Result<Value, &'static str> {
    super::select_context(source, request, "fixture-binding", 100)
}

fn record() -> TaskDeliveryRecord {
    TaskDeliveryRecord {
        task: crate::ClaimedTask {
            claim_attempt: 1,
            task_id: Uuid::new_v4().to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: Some("fixture".into()),
            channel: "ui".into(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".into(),
            payload_json: json!({"text": "fixture request", "conversation_id": "one"}).to_string(),
        },
        owner_principal_id: Some("owner".into()),
        status: "succeeded".into(),
        error_text: None,
        result_json: Some(json!({"task_journal":{"trace":{"capability_results":[{
            "capability":"fixture.read", "status":"ok", "provenance":{"continuation_args":{"url":"https://example.test/item", "topics":["fixture"], "api_key":"hidden"}},
            "data":{"count":4,"authorization":"hidden"}
        }]}}})),
    }
}

#[test]
fn cross_turn_recall_requires_same_owner_channel_and_conversation_even_for_admin() {
    let a = record();
    let mut b = record();
    assert!(same_scope(&a, &b));
    b.owner_principal_id = Some("other".into());
    assert!(!same_scope(&a, &b));
    b = record();
    b.task.chat_id = 3;
    assert!(!same_scope(&a, &b));
    b = record();
    b.task.channel = "wechat".into();
    assert!(!same_scope(&a, &b));
    b = record();
    b.task.payload_json = json!({"conversation_id":"two"}).to_string();
    assert!(!same_scope(&a, &b));
    b = record();
    b.task.user_key = None;
    assert!(!same_scope(&a, &b));
    b = record();
    b.task.user_key = Some("rotated-key-same-principal".into());
    assert!(same_scope(&a, &b));
}

#[test]
fn selected_context_preserves_target_but_never_credentials_or_current_success_evidence() {
    let source = record();
    let mut request = RecallRequest {
        context_ref: context_ref(&source.task.task_id),
        field_paths: vec!["/capabilities/0/args".into()],
        ..Default::default()
    };
    let value = select_context(&source, &request).unwrap();
    assert_eq!(
        value["fields"]["/capabilities/0/args"]["url"],
        "https://example.test/item"
    );
    assert!(value["fields"]["/capabilities/0/args"]
        .get("api_key")
        .is_none());
    assert_eq!(value["current_execution_evidence"], false);
    assert_eq!(value["instruction_authority"], "none");
    request.field_paths = vec!["/capabilities/0/args/api_key".into()];
    assert_eq!(
        select_context(&source, &request),
        Err("context_field_forbidden")
    );
    request.field_paths = vec!["/missing".into()];
    assert_eq!(
        select_context(&source, &request),
        Err("context_field_unavailable")
    );
}

#[test]
fn context_recall_rejects_unbounded_selection_instead_of_silent_loss() {
    let mut source = record();
    source.result_json.as_mut().unwrap()["task_journal"]["trace"]["capability_results"][0]
        ["data"] = json!({"large":"x".repeat(25*1024)});
    let request = RecallRequest {
        context_ref: context_ref(&source.task.task_id),
        field_paths: vec!["/capabilities/0/data".into()],
        ..Default::default()
    };
    assert_eq!(
        select_context(&source, &request),
        Err("context_value_too_large")
    );
}

#[test]
fn capability_index_discovers_exact_argument_refs_without_copying_large_results() {
    let mut source = record();
    source.result_json.as_mut().unwrap()["task_journal"]["trace"]["capability_results"][0]
        ["data"] = json!({"large":"x".repeat(1024*1024)});
    let request = RecallRequest {
        context_ref: context_ref(&source.task.task_id),
        field_paths: vec!["/capabilities".into()],
        ..Default::default()
    };
    let value = select_context(&source, &request).unwrap();
    let index = &value["fields"]["/capabilities"][0];
    assert_eq!(index["args_ref"], "/capabilities/0/args");
    assert!(index.get("data").is_none());
    assert!(value.to_string().len() < 1024);
}

#[tokio::test]
async fn context_api_rechecks_current_auth_and_denies_cross_conversation_and_revoked_keys() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity("context-owner", "admin");
    let identity = crate::resolve_auth_identity_by_key(&state, "context-owner")
        .unwrap()
        .unwrap();
    let source = record();
    let current_id = Uuid::new_v4();
    let db = state.core.db.get().unwrap();
    for (id, status, result) in [
        (
            source.task.task_id.clone(),
            "succeeded",
            source.result_json.clone(),
        ),
        (current_id.to_string(), "running", None),
    ] {
        db.execute("INSERT INTO tasks (task_id,user_id,chat_id,user_key,principal_id,channel,kind,payload_json,status,result_json,created_at,updated_at)
            VALUES (?1,1,2,'context-owner',?2,'ui','ask',?3,?4,?5,'1','1')",
            rusqlite::params![id,identity.principal_id,source.task.payload_json,status,result.map(|v| v.to_string())]).unwrap();
    }
    drop(db);
    let request = || RecallRequest {
        context_ref: context_ref(&source.task.task_id),
        field_paths: vec!["/capabilities/0/args/topics".into()],
        ..Default::default()
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        "context-owner".parse().unwrap(),
    );
    let (status, Json(reply)) = recall(
        State(state.clone()),
        headers.clone(),
        Path(current_id),
        Json(request()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(reply.ok);
    assert_eq!(
        reply.data.unwrap()["fields"]["/capabilities/0/args/topics"],
        json!(["fixture"])
    );
    let db = state.core.db.get().unwrap();
    db.execute(
        "UPDATE tasks SET payload_json=?1 WHERE task_id=?2",
        rusqlite::params![
            json!({"conversation_id":"other"}).to_string(),
            source.task.task_id
        ],
    )
    .unwrap();
    drop(db);
    let (_, Json(reply)) = recall(
        State(state.clone()),
        headers.clone(),
        Path(current_id),
        Json(request()),
    )
    .await;
    assert!(!reply.ok);
    assert_eq!(
        reply.error.as_deref(),
        Some("context_reference_unauthorized")
    );
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET enabled=0 WHERE user_key='context-owner'",
            [],
        )
        .unwrap();
    let (_, Json(reply)) = recall(State(state), headers, Path(current_id), Json(request())).await;
    assert!(!reply.ok);
    assert_eq!(reply.error.as_deref(), Some("auth_key_invalid"));
}

#[path = "task_context_pagination_tests.rs"]
mod pagination;
