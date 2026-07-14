use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::Json;
use serde_json::{json, Value};

use super::{goal_by_task_id, GoalByTaskIdRequest};

const USER_KEY: &str = "goal-route-test-key";

fn state_with_goal_task(task_id: &str, payload: Value) -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("get db");
    db.execute_batch(
        "CREATE TABLE auth_keys (
            user_key TEXT PRIMARY KEY,
            role TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            last_used_at TEXT
        );
        CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            channel TEXT NOT NULL,
            external_user_id TEXT,
            external_chat_id TEXT,
            message_id INTEGER,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0,
            claim_attempt INTEGER NOT NULL DEFAULT 0,
            claimed_at INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create route test tables");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'admin', 1, '1', NULL)",
        rusqlite::params![USER_KEY],
    )
    .expect("insert auth key");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, ?2, 7, ?3, 'ui', 'ask', ?4, 'running', NULL, NULL, '1', '1')",
        rusqlite::params![
            task_id,
            crate::stable_i64_from_key(USER_KEY),
            USER_KEY,
            payload.to_string(),
        ],
    )
    .expect("insert task");
    drop(db);
    state
}

fn auth_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-rustclaw-key", HeaderValue::from_static(USER_KEY));
    headers
}

fn stored_payload(state: &crate::AppState, task_id: &str) -> Value {
    let db = state.core.db.get().expect("get db");
    let raw: String = db
        .query_row(
            "SELECT payload_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| row.get(0),
        )
        .expect("select payload");
    serde_json::from_str(&raw).expect("payload json")
}

#[tokio::test]
async fn goal_by_task_id_edits_goal_payload_through_authorized_route() {
    let task_id = "goal-route-edit";
    let state = state_with_goal_task(
        task_id,
        json!({
            "text": "task",
            "user_key": "rk-secret-in-payload",
            "goal_spec": {
                "objective": "old",
                "done_conditions": ["old_done"],
                "metadata": {"access_token": "tok-secret-in-goal"}
            }
        }),
    );

    let (status, Json(resp)) = goal_by_task_id(
        State(state.clone()),
        auth_headers(),
        Json(GoalByTaskIdRequest {
            task_id: task_id.to_string(),
            operation: "edit".to_string(),
            goal: Some(json!({
                "objective": "updated",
                "constraints": ["scope=workspace"]
            })),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(resp.ok);
    let data = resp.data.expect("response data");
    assert_eq!(data["status"], "task_goal_control_updated");
    assert_eq!(data["operation"], "edit");
    assert_eq!(data["goal"]["objective"], "updated");
    assert!(data["goal"].get("text").is_none());
    assert!(data["goal"].get("error_text").is_none());
    assert_eq!(data["payload_json"]["user_key"], "[REDACTED]");
    assert_eq!(
        data["payload_json"]["goal"]["metadata"]["access_token"],
        "[REDACTED]"
    );

    let payload = stored_payload(&state, task_id);
    assert_eq!(payload["goal"]["objective"], "updated");
    assert_eq!(payload["goal"]["done_conditions"][0], "old_done");
    assert_eq!(payload["user_key"], "rk-secret-in-payload");
    assert_eq!(
        payload["goal"]["metadata"]["access_token"],
        "tok-secret-in-goal"
    );
    assert!(payload.get("goal_spec").is_none());
}

#[tokio::test]
async fn goal_by_task_id_clears_goal_payload_through_authorized_route() {
    let task_id = "goal-route-clear";
    let state = state_with_goal_task(
        task_id,
        json!({
            "text": "task",
            "goal": {"objective": "old"},
            "task_goal": {"objective": "legacy"}
        }),
    );

    let (status, Json(resp)) = goal_by_task_id(
        State(state.clone()),
        auth_headers(),
        Json(GoalByTaskIdRequest {
            task_id: task_id.to_string(),
            operation: "clear".to_string(),
            goal: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(resp.ok);
    let data = resp.data.expect("response data");
    assert_eq!(data["status"], "task_goal_control_updated");
    assert_eq!(data["operation"], "clear");
    assert!(data["goal"].is_null());
    assert_eq!(data["payload_json"]["goal_cleared"], true);

    let payload = stored_payload(&state, task_id);
    assert!(payload.get("goal").is_none());
    assert!(payload.get("task_goal").is_none());
    assert_eq!(payload["goal_cleared"], true);
}
