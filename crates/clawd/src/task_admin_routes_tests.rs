use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::Json;
use serde_json::{json, Value};

use super::{
    goal_by_task_id, list_approval_scope_grants, resume_task_by_id, retry_child_task_by_id,
    revoke_approval_scope_grant, GoalByTaskIdRequest, ResumeTaskByIdRequest,
    RetryChildTaskByIdRequest, RevokeApprovalScopeGrantRequest,
};

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

fn insert_child_task(
    state: &crate::AppState,
    parent_task_id: &str,
    child_task_id: &str,
    status: &str,
) {
    let payload = json!({
        "text": "original objective",
        "task_role": "subagent_child",
        "parent_task_id": parent_task_id,
        "child_task_id": child_task_id,
        "child_task_contract": {
            "schema_version": 1,
            "parent_task_id": parent_task_id,
            "child_task_id": child_task_id,
            "role": "writer",
            "scope": {
                "objective": "original objective",
                "allowed_capabilities": ["filesystem.read_text_range", "workspace.apply_patch"]
            },
            "permission_profile": "local_worktree",
            "required": true,
            "budget": {
                "max_rounds": 4,
                "max_tool_calls": 16,
                "timeout_ms": 300000
            },
            "result_contract": {
                "output_format": "machine_json"
            },
            "merge_policy": "structured_findings"
        }
    });
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, ?2, 7, ?3, 'ui', 'ask', ?4, ?5, ?6, NULL, '1', '1')",
        rusqlite::params![
            child_task_id,
            crate::stable_i64_from_key(USER_KEY),
            USER_KEY,
            payload.to_string(),
            status,
            json!({"status_code": "verification_failed"}).to_string(),
        ],
    )
    .expect("insert child task");
    db.execute(
        "UPDATE tasks
         SET result_json = ?2
         WHERE task_id = ?1",
        rusqlite::params![
            parent_task_id,
            json!({"child_task_ids": [child_task_id]}).to_string()
        ],
    )
    .expect("link child task");
}

fn set_pending_approval(state: &crate::AppState, task_id: &str, request_id: &str) {
    let expires_at = crate::now_ts_u64().saturating_add(300);
    let result = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "needs_user",
            "resume_reason": "confirmation_required",
            "checkpoint_id": "checkpoint-approval"
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": "checkpoint-approval",
            "boundary_context": {},
            "last_successful_round": null,
            "last_successful_step": null,
            "pending_action": null,
            "observations": [],
            "evidence_refs": [],
            "artifact_refs": [],
            "completed_side_effect_refs": [],
            "budget": {
                "round": 0,
                "step": 0,
                "llm_calls": 0,
                "tool_calls": 0,
                "elapsed_ms": 0,
                "llm_elapsed_ms": 0,
                "tool_elapsed_ms": 0
            },
            "resume_entrypoint": "await_user_input"
        },
        "resume_context": {
            "approval_request": {
                "schema_version": 1,
                "request_id": request_id,
                "task_id": task_id,
                "status": "pending",
                "action_fingerprint": "sha256:action",
                "arguments_hash": "sha256:args",
                "expires_at": expires_at,
            }
        }
    });
    let db = state.core.db.get().expect("get db");
    db.execute(
        "UPDATE tasks SET status = 'running', result_json = ?2 WHERE task_id = ?1",
        rusqlite::params![task_id, result.to_string()],
    )
    .expect("set pending approval");
}

fn set_pending_scope_approval(state: &crate::AppState, task_id: &str, request_id: &str) {
    let expires_at = crate::now_ts_u64().saturating_add(300);
    let result = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "needs_user",
            "resume_reason": "confirmation_required",
            "checkpoint_id": "checkpoint-approval"
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": "checkpoint-approval",
            "boundary_context": {},
            "last_successful_round": null,
            "last_successful_step": null,
            "pending_action": null,
            "observations": [],
            "evidence_refs": [],
            "artifact_refs": [],
            "completed_side_effect_refs": [],
            "budget": {
                "round": 0,
                "step": 0,
                "llm_calls": 0,
                "tool_calls": 0,
                "elapsed_ms": 0,
                "llm_elapsed_ms": 0,
                "tool_elapsed_ms": 0
            },
            "resume_entrypoint": "await_user_input"
        },
        "resume_context": {
            "approval_request": {
                "schema_version": 1,
                "request_id": request_id,
                "task_id": task_id,
                "status": "pending",
                "action_fingerprint": "sha256:action",
                "arguments_hash": "sha256:args",
                "expires_at": expires_at,
                "scope_grant": {
                    "available": true,
                    "scope_kind": "session",
                    "scope_fingerprint": "sha256:scope",
                    "entries": [{
                        "capability": "filesystem.remove_path",
                        "action": "remove_path",
                        "effect": "mutate",
                        "resource_kind": "workspace_path",
                        "resources": ["run/example.txt"]
                    }]
                }
            }
        }
    });
    let db = state.core.db.get().expect("get db");
    db.execute(
        "UPDATE tasks SET status = 'running', result_json = ?2 WHERE task_id = ?1",
        rusqlite::params![task_id, result.to_string()],
    )
    .expect("set pending scope approval");
}

#[tokio::test]
async fn retry_child_task_route_queues_revised_attempt_for_same_actor() {
    let parent_task_id = "retry-route-parent";
    let child_task_id = "retry-route-child";
    let state = state_with_goal_task(parent_task_id, json!({"text": "parent"}));
    insert_child_task(&state, parent_task_id, child_task_id, "failed");

    let (status, Json(response)) = retry_child_task_by_id(
        State(state.clone()),
        auth_headers(),
        Json(RetryChildTaskByIdRequest {
            parent_task_id: parent_task_id.to_string(),
            child_task_id: child_task_id.to_string(),
            revised_goal: "preserve the public contract while fixing verification".to_string(),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(response.ok);
    let data = response.data.expect("response data");
    assert_eq!(data["status"], "child_task_retry_queued");
    assert_eq!(data["previous_child_task_id"], child_task_id);
    assert_eq!(data["retry_index"], 1);
    let retry_task_id = data["child_task_id"].as_str().expect("retry task id");
    let payload = stored_payload(&state, retry_task_id);
    assert_eq!(
        payload["text"],
        "preserve the public contract while fixing verification"
    );
    assert_eq!(
        payload["child_task_contract"]["child_task_id"],
        retry_task_id
    );
    assert_eq!(
        payload["child_task_contract"]["permission_profile"],
        "local_worktree"
    );
}

#[tokio::test]
async fn retry_child_task_route_rejects_nonterminal_child() {
    let parent_task_id = "retry-route-active-parent";
    let child_task_id = "retry-route-active-child";
    let state = state_with_goal_task(parent_task_id, json!({"text": "parent"}));
    insert_child_task(&state, parent_task_id, child_task_id, "running");

    let (status, Json(response)) = retry_child_task_by_id(
        State(state),
        auth_headers(),
        Json(RetryChildTaskByIdRequest {
            parent_task_id: parent_task_id.to_string(),
            child_task_id: child_task_id.to_string(),
            revised_goal: "replacement objective".to_string(),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some("child_task_not_retryable"));
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

#[tokio::test]
async fn resume_needs_user_task_requires_and_applies_exact_approval_request() {
    let task_id = "approval-route-task";
    let request_id = "approval-route-1";
    let state = state_with_goal_task(task_id, json!({"text": "task"}));
    set_pending_approval(&state, task_id, request_id);

    let (missing_status, _) = resume_task_by_id(
        State(state.clone()),
        auth_headers(),
        Json(ResumeTaskByIdRequest {
            task_id: task_id.to_string(),
            checkpoint_id: None,
            resume_reason: None,
            user_message: None,
            new_constraints: None,
            approval_request_id: None,
            approval_decision: None,
        }),
    )
    .await;
    assert_eq!(missing_status, StatusCode::CONFLICT);

    let (invalid_status, _) = resume_task_by_id(
        State(state.clone()),
        auth_headers(),
        Json(ResumeTaskByIdRequest {
            task_id: task_id.to_string(),
            checkpoint_id: None,
            resume_reason: None,
            user_message: None,
            new_constraints: None,
            approval_request_id: Some(request_id.to_string()),
            approval_decision: Some("approve".to_string()),
        }),
    )
    .await;
    assert_eq!(invalid_status, StatusCode::BAD_REQUEST);

    let (status, Json(resp)) = resume_task_by_id(
        State(state.clone()),
        auth_headers(),
        Json(ResumeTaskByIdRequest {
            task_id: task_id.to_string(),
            checkpoint_id: None,
            resume_reason: None,
            user_message: None,
            new_constraints: None,
            approval_request_id: Some(request_id.to_string()),
            approval_decision: Some("approve_once".to_string()),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp.ok);
    assert_eq!(
        resp.data.expect("response data")["status"],
        "approval_grant_approved"
    );

    let db = state.core.db.get().expect("get db");
    let (stored_status, raw_result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select approved task");
    let stored_result: Value = serde_json::from_str(&raw_result).expect("result json");
    assert_eq!(stored_status, "running");
    assert_eq!(
        stored_result["resume_context"]["approval_request"]["status"],
        "approved"
    );
    assert_eq!(stored_result["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        stored_result["task_checkpoint"]["resume_entrypoint"],
        "next_planner_round"
    );
}

#[tokio::test]
async fn resume_needs_user_task_can_deny_the_exact_approval_request() {
    let task_id = "approval-route-deny";
    let request_id = "approval-route-deny-1";
    let state = state_with_goal_task(task_id, json!({"text": "task"}));
    set_pending_approval(&state, task_id, request_id);

    let (status, Json(resp)) = resume_task_by_id(
        State(state.clone()),
        auth_headers(),
        Json(ResumeTaskByIdRequest {
            task_id: task_id.to_string(),
            checkpoint_id: None,
            resume_reason: None,
            user_message: None,
            new_constraints: None,
            approval_request_id: Some(request_id.to_string()),
            approval_decision: Some("deny".to_string()),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(resp.ok);
    let data = resp.data.expect("response data");
    assert_eq!(data["status"], "approval_request_denied");
    assert_eq!(data["approval_decision"], "deny");

    let db = state.core.db.get().expect("get db");
    let (stored_status, raw_result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select denied task");
    let stored_result: Value = serde_json::from_str(&raw_result).expect("result json");
    assert_eq!(stored_status, "failed");
    assert_eq!(
        stored_result["resume_context"]["approval_request"]["status"],
        "denied"
    );
}

#[tokio::test]
async fn scoped_approval_can_be_listed_and_revoked_by_the_same_actor() {
    let task_id = "approval-route-scope";
    let request_id = "approval-route-scope-1";
    let state = state_with_goal_task(task_id, json!({"text": "task"}));
    set_pending_scope_approval(&state, task_id, request_id);

    let (status, Json(resp)) = resume_task_by_id(
        State(state.clone()),
        auth_headers(),
        Json(ResumeTaskByIdRequest {
            task_id: task_id.to_string(),
            checkpoint_id: None,
            resume_reason: None,
            user_message: None,
            new_constraints: None,
            approval_request_id: Some(request_id.to_string()),
            approval_decision: Some("always_for_scope".to_string()),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let data = resp.data.expect("scope response");
    assert_eq!(data["status"], "approval_scope_grant_created");
    assert_eq!(data["task_lifecycle"]["state"], "waiting");
    let grant_id = data["scope_grant"]["grant_id"]
        .as_str()
        .expect("grant id")
        .to_string();

    let (list_status, Json(list_resp)) =
        list_approval_scope_grants(State(state.clone()), auth_headers()).await;
    assert_eq!(list_status, StatusCode::OK);
    let list = list_resp.data.expect("grant list");
    assert_eq!(list["count"], 1);
    assert_eq!(list["grants"][0]["grant_id"], grant_id);

    let (revoke_status, Json(revoke_resp)) = revoke_approval_scope_grant(
        State(state.clone()),
        auth_headers(),
        Json(RevokeApprovalScopeGrantRequest {
            grant_id: grant_id.clone(),
        }),
    )
    .await;
    assert_eq!(revoke_status, StatusCode::OK);
    assert_eq!(
        revoke_resp.data.expect("revoke response")["status"],
        "approval_scope_grant_revoked"
    );

    let (_, Json(list_resp)) = list_approval_scope_grants(State(state), auth_headers()).await;
    assert!(list_resp.data.expect("grant list")["grants"][0]["revoked_at"].is_number());
}
