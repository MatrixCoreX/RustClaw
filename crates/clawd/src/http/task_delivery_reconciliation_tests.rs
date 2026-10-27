use super::*;

#[tokio::test]
async fn only_current_channel_owner_can_request_reconciliation() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity("owner", "user");
    state.seed_test_auth_identity("other", "admin");
    let owner = crate::resolve_auth_identity_by_key(&state, "owner")
        .unwrap()
        .unwrap();
    let id = Uuid::new_v4();
    let payload = json!({"channel_ingress":{"schema_version":1,"channel":"telegram","adapter":"telegram_bot","account_id":"bot-main","external_user_id":"1","external_chat_id":"2","reply_target":{"kind":"chat","external_id":"2"}}});
    let db = state.core.db.get().unwrap();
    db.execute("INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,external_user_id,external_chat_id,kind,payload_json,status,created_at,updated_at) VALUES (?1,1,2,'owner',?2,'telegram','1','2','ask',?3,'succeeded','1','1')",rusqlite::params![id.to_string(),owner.principal_id,payload.to_string()]).unwrap();
    db.execute("INSERT INTO channel_bindings(channel,external_user_id,external_chat_id,user_key,bound_at,updated_at) VALUES ('telegram','1','2','owner','1','1')",[]).unwrap();
    drop(db);
    for key in [None, Some("other"), Some("owner")] {
        let mut headers = HeaderMap::new();
        if let Some(key) = key {
            headers.insert(
                claw_core::product_identity::AUTH_KEY_HEADER,
                key.parse().unwrap(),
            );
        }
        let decision=serde_json::from_value(json!({"operation":"retry_unknown","snapshot_digest":"0".repeat(64),"decision_id":Uuid::new_v4(),"confirmed_duplicate_risk":true})).unwrap();
        let (status, Json(reply)) =
            reconcile(State(state.clone()), headers, Path(id), Json(decision)).await;
        assert_eq!(
            status,
            if key == Some("owner") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::UNAUTHORIZED
            }
        );
        assert_eq!(
            reply.error.as_deref(),
            Some(if key == Some("owner") {
                "delivery_dispatch_missing"
            } else {
                "task_delivery_unauthorized"
            })
        );
    }
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("DELETE FROM channel_bindings", [])
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        "owner".parse().unwrap(),
    );
    let decision=serde_json::from_value(json!({"operation":"retry_failed","snapshot_digest":"0".repeat(64),"decision_id":Uuid::new_v4()})).unwrap();
    assert_eq!(
        reconcile(State(state), headers, Path(id), Json(decision))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn inspection_projects_delivery_without_overwriting_execution_or_granting_actions() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let mut task:TaskQueryResponse=serde_json::from_value(json!({"task_id":Uuid::new_v4(),"status":"running","result_json":{"task_journal":{"summary":{"execution_status":"succeeded","answer_verification":{"status":"unavailable"}}}},"error_text":null})).unwrap();
    attach_status(&state, &mut task, "ui", false);
    let summary = &task.result_json.as_ref().unwrap()["task_journal"]["summary"];
    assert_eq!(summary["execution_status"], "succeeded");
    assert_eq!(summary["answer_verification"]["status"], "unavailable");
    assert_eq!(summary["delivery"]["status"], "not_required");
    assert_eq!(summary["delivery"]["can_reconcile"], false);
}
