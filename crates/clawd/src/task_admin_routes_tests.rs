use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::Json;
use serde_json::{json, Value};

use super::{
    cancel_task_by_id, close_child_task_by_id, goal_by_task_id, list_active_tasks,
    list_approval_scope_grants, resume_task_by_id, retry_child_task_by_id,
    revoke_approval_scope_grant, stop_child_tasks_by_parent, ActiveTasksRequest,
    CancelTaskByIdRequest, CloseChildTaskByIdRequest, GoalByTaskIdRequest, ResumeTaskByIdRequest,
    RetryChildTaskByIdRequest, RevokeApprovalScopeGrantRequest, StopChildTasksByParentRequest,
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
    crate::repo::child_task_graph::ensure_child_task_graph_schema(&db)
        .expect("child_task_graph_schema_test");
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
    crate::repo::ensure_principal_ownership_schema(&db)
        .expect("bind route fixture to stable principal");
    drop(db);
    state
}

fn auth_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-agent-key", HeaderValue::from_static(USER_KEY));
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
async fn admin_active_task_list_uses_the_same_system_scope_as_health() {
    let state = state_with_goal_task("admin-task", json!({"text": "admin work"}));
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
         ) VALUES (
            'other-task', 99, 9, 'other-key', 'ui', 'ask', ?1,
            'running', NULL, NULL, '2', '2'
         )",
        rusqlite::params![json!({"text": "other work"}).to_string()],
    )
    .expect("insert other owner task");
    drop(db);

    let (status, Json(response)) = list_active_tasks(
        State(state),
        auth_headers(),
        Json(ActiveTasksRequest {
            user_id: 0,
            chat_id: 0,
            exclude_task_id: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let data = response.data.expect("active tasks response");
    assert_eq!(data["count"], 2);
    let tasks = data["tasks"].as_array().expect("active task array");
    assert!(tasks.iter().all(|task| task["channel"] == "ui"));
    assert!(tasks
        .iter()
        .all(|task| task["source_user_id"].as_str().is_some()));
}

#[tokio::test]
async fn cancel_task_by_id_is_idempotent_after_the_task_is_cancelled() {
    let task_id = "cancel-route-idempotent";
    let state = state_with_goal_task(task_id, json!({"text": "long task"}));

    let (first_status, Json(first_response)) = cancel_task_by_id(
        State(state.clone()),
        auth_headers(),
        Json(CancelTaskByIdRequest {
            task_id: task_id.to_string(),
        }),
    )
    .await;
    assert_eq!(first_status, StatusCode::OK);
    let first = first_response.data.expect("first cancel response");
    assert_eq!(first["status"], "task_cancelled");
    assert_eq!(first["canceled"], 1);

    let (second_status, Json(second_response)) = cancel_task_by_id(
        State(state),
        auth_headers(),
        Json(CancelTaskByIdRequest {
            task_id: task_id.to_string(),
        }),
    )
    .await;
    assert_eq!(second_status, StatusCode::OK);
    let second = second_response.data.expect("second cancel response");
    assert_eq!(second["status"], "task_already_cancelled");
    assert_eq!(second["canceled"], 0);
    assert_eq!(second["already_terminal"], true);
    assert_eq!(second["task_id"], task_id);
}

#[tokio::test]
async fn child_stop_all_and_close_routes_are_parent_scoped_and_idempotent() {
    let parent_task_id = "control-route-parent";
    let active_child_id = "control-route-active-child";
    let done_child_id = "control-route-done-child";
    let state = state_with_goal_task(parent_task_id, json!({"text": "parent"}));
    insert_child_task(&state, parent_task_id, active_child_id, "running");
    insert_child_task(&state, parent_task_id, done_child_id, "succeeded");
    {
        let db = state.core.db.get().expect("get db");
        db.execute(
            "INSERT INTO child_task_graphs (
                parent_task_id, schema_version, status, max_parallel,
                session_ref, session_open_capacity, created_at, updated_at
             ) VALUES (?1, 2, 'active', 2, 'route-session', 2, '1', '1')",
            rusqlite::params![parent_task_id],
        )
        .expect("insert child graph");
        for (child_task_id, readiness) in
            [(active_child_id, "running"), (done_child_id, "succeeded")]
        {
            db.execute(
                "INSERT INTO child_task_graph_nodes (
                    parent_task_id, child_task_id, role, required, readiness,
                    permission_profile, merge_policy, owned_paths_json,
                    budget_json, model_policy_json, tool_policy_json,
                    result_contract_json, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, 'review', 1, ?3, 'read_only',
                    'structured_findings', '[]', '{}', '{}', '{}', '{}', '1', '1'
                 )",
                rusqlite::params![parent_task_id, child_task_id, readiness],
            )
            .expect("insert child graph node");
        }
    }

    let (first_status, Json(first_response)) = stop_child_tasks_by_parent(
        State(state.clone()),
        auth_headers(),
        Json(StopChildTasksByParentRequest {
            parent_task_id: parent_task_id.to_string(),
        }),
    )
    .await;
    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(
        first_response.data.expect("stop all data")["cancelled_child_count"],
        1
    );

    let (repeat_status, Json(repeat_response)) = stop_child_tasks_by_parent(
        State(state.clone()),
        auth_headers(),
        Json(StopChildTasksByParentRequest {
            parent_task_id: parent_task_id.to_string(),
        }),
    )
    .await;
    assert_eq!(repeat_status, StatusCode::OK);
    assert_eq!(
        repeat_response.data.expect("repeat stop data")["cancelled_child_count"],
        0
    );

    let (close_status, Json(close_response)) = close_child_task_by_id(
        State(state.clone()),
        auth_headers(),
        Json(CloseChildTaskByIdRequest {
            parent_task_id: parent_task_id.to_string(),
            child_task_id: done_child_id.to_string(),
        }),
    )
    .await;
    assert_eq!(close_status, StatusCode::OK);
    let close_data = close_response.data.expect("close data");
    assert_eq!(close_data["status"], "child_thread_closed");
    assert!(close_data["child_task_graph"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .any(|node| {
            node["child_task_id"] == done_child_id && node["thread_state"] == "closed"
        }));

    let (repeat_close_status, Json(repeat_close_response)) = close_child_task_by_id(
        State(state),
        auth_headers(),
        Json(CloseChildTaskByIdRequest {
            parent_task_id: parent_task_id.to_string(),
            child_task_id: done_child_id.to_string(),
        }),
    )
    .await;
    assert_eq!(repeat_close_status, StatusCode::OK);
    assert_eq!(
        repeat_close_response.data.expect("repeat close data")["status"],
        "child_thread_closed"
    );
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
