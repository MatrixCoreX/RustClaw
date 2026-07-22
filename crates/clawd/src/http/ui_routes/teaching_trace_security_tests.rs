use super::*;
use axum::http::HeaderValue;

fn identity(user_key: &str, role: &str) -> AuthIdentity {
    AuthIdentity {
        user_key: user_key.to_string(),
        role: role.to_string(),
        user_id: 1,
        chat_id: 1,
    }
}

#[test]
fn teaching_trace_requires_explicit_opt_in_and_exact_owner_or_admin() {
    let owner = identity("owner-key", "user");
    let other = identity("other-key", "user");
    let admin = identity("admin-key", "admin");

    assert_eq!(
        teaching_trace_access_scope(&owner, Some("owner-key"), false),
        Err("teaching_trace_opt_in_required")
    );
    assert_eq!(
        teaching_trace_access_scope(&owner, Some("owner-key"), true),
        Ok("task_owner")
    );
    assert_eq!(
        teaching_trace_access_scope(&other, Some("owner-key"), true),
        Err("teaching_trace_access_denied")
    );
    assert_eq!(
        teaching_trace_access_scope(&admin, Some("owner-key"), true),
        Ok("admin")
    );
    assert_eq!(
        teaching_trace_access_scope(&owner, None, true),
        Err("teaching_trace_access_denied")
    );
}

#[test]
fn teaching_trace_redacts_nested_and_free_text_secrets_without_hiding_prompt_structure() {
    let mut entries = vec![TaskDebugEntry {
        ts: Some(1),
        task_id: Some("task-redact".to_string()),
        parent_task_id: None,
        child_task_id: None,
        call_id: Some("call-1".to_string()),
        vendor: Some("fixture".to_string()),
        provider: Some("fixture".to_string()),
        provider_type: Some("fixture".to_string()),
        model: Some("fixture-model".to_string()),
        model_kind: Some("chat".to_string()),
        status: Some("ok".to_string()),
        mode: Some("planner".to_string()),
        prompt_source: Some("planner".to_string()),
        prompt_hash: None,
        prompt_file: None,
        prompt: Some("### MEMORY_USE_POLICY\nAuthorization: Bearer abcdefghijklmnop".to_string()),
        request_payload: Some(json!({
            "headers": {"authorization": "Bearer abcdefghijklmnop"},
            "messages": [{"content": "use tp-1234567890abcdefghijkl"}],
            "credential_state": "configured",
        })),
        response: Some("api_key=plain-secret".to_string()),
        raw_response: Some(r#"{"token":"plain-token","answer":"ok"}"#.to_string()),
        clean_response: Some("ok".to_string()),
        sanitized: Some(false),
        error: None,
        usage: None,
    }];

    let count = redact_task_debug_entries(&mut entries);
    let encoded = serde_json::to_string(&entries).expect("serialize entries");

    assert!(count >= 4);
    assert!(entries[0]
        .prompt
        .as_deref()
        .is_some_and(|prompt| prompt.contains("### MEMORY_USE_POLICY")));
    assert_eq!(entries[0].sanitized, Some(true));
    assert!(encoded.contains("credential_state"));
    assert!(encoded.contains("configured"));
    assert!(!encoded.contains("abcdefghijklmnop"));
    assert!(!encoded.contains("plain-secret"));
    assert!(!encoded.contains("plain-token"));
    assert!(!encoded.contains("tp-1234567890abcdefghijkl"));
}

#[tokio::test]
async fn teaching_trace_endpoint_rejects_cross_user_shared_channel_access() {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("db");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("auth schema");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_key TEXT,
            channel TEXT NOT NULL,
            status TEXT NOT NULL,
            result_json TEXT
        );",
    )
    .expect("task schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES ('trace-other', 'user', 1, '1', NULL)",
        [],
    )
    .expect("other identity");
    db.execute(
        "INSERT INTO tasks (task_id, user_key, channel, status, result_json)
         VALUES ('task-shared-channel', 'trace-owner', 'telegram', 'succeeded', NULL)",
        [],
    )
    .expect("task row");
    drop(db);

    let mut headers = HeaderMap::new();
    headers.insert("x-rustclaw-key", HeaderValue::from_static("trace-other"));
    let (status, Json(response)) = task_debug_detail(
        State(state),
        headers,
        AxumPath("task-shared-channel".to_string()),
        Query(TeachingTraceQuery {
            teaching: Some(true),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!response.ok);
    assert_eq!(
        response.error.as_deref(),
        Some("teaching_trace_access_denied")
    );
    assert_eq!(
        response.data.expect("machine error")["message_key"],
        "clawd.ui.teaching_trace.teaching_trace_access_denied"
    );
}

#[tokio::test]
async fn teaching_trace_endpoint_requires_query_opt_in() {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("db");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("auth schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES ('trace-admin', 'admin', 1, '1', NULL)",
        [],
    )
    .expect("admin identity");
    drop(db);

    let mut headers = HeaderMap::new();
    headers.insert("x-rustclaw-key", HeaderValue::from_static("trace-admin"));
    let (status, Json(response)) = task_debug_detail(
        State(state),
        headers,
        AxumPath("task-any".to_string()),
        Query(TeachingTraceQuery { teaching: None }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        response.error.as_deref(),
        Some("teaching_trace_opt_in_required")
    );
}

#[tokio::test]
async fn teaching_trace_endpoint_allows_exact_owner_and_labels_trace_layers() {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("db");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("auth schema");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_key TEXT,
            channel TEXT NOT NULL,
            status TEXT NOT NULL,
            result_json TEXT
        );",
    )
    .expect("task schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES ('trace-owner', 'user', 1, '1', NULL)",
        [],
    )
    .expect("owner identity");
    db.execute(
        "INSERT INTO tasks (task_id, user_key, channel, status, result_json)
         VALUES ('task-owned', 'trace-owner', 'ui', 'succeeded', NULL)",
        [],
    )
    .expect("task row");
    drop(db);

    let mut headers = HeaderMap::new();
    headers.insert("x-rustclaw-key", HeaderValue::from_static("trace-owner"));
    let (status, Json(response)) = task_debug_detail(
        State(state),
        headers,
        AxumPath("task-owned".to_string()),
        Query(TeachingTraceQuery {
            teaching: Some(true),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(response.ok);
    let data = response.data.expect("teaching trace");
    assert_eq!(data["trace_schema_version"], 2);
    assert_eq!(data["access"]["scope"], "task_owner");
    assert_eq!(
        data["trace_layers"]["provider_data"]["classification"],
        "redacted_provider_io"
    );
    assert_eq!(
        data["trace_layers"]["rustclaw_decisions"]["classification"],
        "parsed_machine_decisions"
    );
}
