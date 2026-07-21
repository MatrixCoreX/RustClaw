use super::*;

fn identity(role: &str) -> AuthIdentity {
    AuthIdentity {
        user_key: "rk-test".to_string(),
        role: role.to_string(),
        user_id: 1,
        chat_id: 2,
    }
}

fn claimed_task(user_key: &str, payload: Value) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 1,
        task_id: "task-yolo-policy".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
    }
}

fn state_with_admin_key(user_key: &str) -> AppState {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("test db");
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS auth_keys (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key TEXT NOT NULL UNIQUE,
            role TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT '',
            last_used_at TEXT
        );",
    )
    .expect("auth key table");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES (?1, 'admin', 1, 'now')",
        rusqlite::params![user_key],
    )
    .expect("insert admin key");
    drop(db);
    state
}

#[test]
fn clawcli_admin_requires_explicit_yolo_request() {
    let admin = identity("admin");
    let mut safe = json!({"text": "task"});
    stamp_authenticated_submission_policy(&mut safe, Some(&admin), Some("clawcli"), None)
        .expect("safe clawcli policy");
    assert!(safe.get(POLICY_PAYLOAD_FIELD).is_none());

    let mut yolo = json!({"text": "task"});
    stamp_authenticated_submission_policy(&mut yolo, Some(&admin), Some("clawcli"), Some("yolo"))
        .expect("admin yolo policy");
    assert_eq!(yolo[POLICY_PAYLOAD_FIELD]["mode"], "yolo");
    assert_eq!(yolo[POLICY_PAYLOAD_FIELD]["actor_role"], "admin");
    assert_eq!(
        yolo[POLICY_PAYLOAD_FIELD]["derivation"],
        "clawcli_explicit_admin"
    );
}

#[test]
fn non_clawcli_admin_defaults_to_yolo() {
    let admin = identity("admin");
    for origin in [None, Some("ui"), Some("telegram"), Some("whatsapp")] {
        let mut payload = json!({"text": "task"});
        stamp_authenticated_submission_policy(&mut payload, Some(&admin), origin, None)
            .expect("admin channel policy");
        assert_eq!(payload[POLICY_PAYLOAD_FIELD]["mode"], "yolo");
        assert_eq!(
            payload[POLICY_PAYLOAD_FIELD]["derivation"],
            "admin_channel_default"
        );
    }
}

#[test]
fn non_admin_cannot_request_or_spoof_yolo() {
    let user = identity("user");
    let mut requested = json!({"text": "task"});
    assert_eq!(
        stamp_authenticated_submission_policy(
            &mut requested,
            Some(&user),
            Some("clawcli"),
            Some("yolo")
        ),
        Err(SubmissionPolicyError::AdminRequired)
    );

    let mut spoofed = json!({
        "text": "task",
        POLICY_PAYLOAD_FIELD: {
            "schema_version": 1,
            "mode": "yolo",
            "authority": "authenticated_admin"
        }
    });
    stamp_authenticated_submission_policy(&mut spoofed, Some(&user), None, None)
        .expect("safe user policy");
    assert!(spoofed.get(POLICY_PAYLOAD_FIELD).is_none());

    let mut anonymous = json!({"text": "task"});
    assert_eq!(
        stamp_authenticated_submission_policy(&mut anonymous, None, Some("clawcli"), Some("yolo")),
        Err(SubmissionPolicyError::AdminRequired)
    );
}

#[test]
fn unknown_execution_mode_is_rejected() {
    let admin = identity("admin");
    let mut payload = json!({"text": "task"});
    assert_eq!(
        stamp_authenticated_submission_policy(
            &mut payload,
            Some(&admin),
            Some("clawcli"),
            Some("unsafe")
        ),
        Err(SubmissionPolicyError::UnsupportedExecutionMode)
    );
}

#[test]
fn yolo_policy_requires_an_object_payload() {
    let admin = identity("admin");
    let mut payload = json!("task");
    assert_eq!(
        stamp_authenticated_submission_policy(
            &mut payload,
            Some(&admin),
            Some("clawcli"),
            Some("yolo")
        ),
        Err(SubmissionPolicyError::PayloadObjectRequired)
    );
}

#[test]
fn effective_yolo_policy_requires_a_current_admin_identity() {
    let user_key = "rk-admin-yolo";
    let state = state_with_admin_key(user_key);
    let mut payload = json!({"text": "task"});
    stamp_authenticated_submission_policy(
        &mut payload,
        Some(&identity("admin")),
        Some("clawcli"),
        Some("yolo"),
    )
    .expect("stamp yolo policy");
    let task = claimed_task(user_key, payload);

    let effective = effective_policy_for_task(&state, &task);
    assert_eq!(effective.mode, TaskExecutionMode::Yolo);
    assert_eq!(effective.approval_policy, ToolApprovalPolicy::Never);
    assert_eq!(effective.sandbox_mode, ToolSandboxMode::DangerFull);
    assert_eq!(effective.actor_role, Some("admin"));
    assert!(!effective.approval_required(true, true, true));
    assert!(effective
        .sandbox_denial(crate::runtime::policy::SandboxRequirements {
            mutates: true,
            network_access: true,
            filesystem_write: true,
            external_publish: true,
            credential_access: true,
            subprocess: true,
            package_install: true,
            privilege_escalation: true,
            isolation_profile: None,
        })
        .is_none());
    let inherited = inheritable_policy_stamp(&state, &task).expect("inheritable policy");
    assert_eq!(inherited["mode"], "yolo");
    assert_eq!(inherited["derivation"], "authenticated_parent_task");

    let mut malformed_payload: Value =
        serde_json::from_str(&task.payload_json).expect("parse stamped payload");
    malformed_payload[POLICY_PAYLOAD_FIELD]
        .as_object_mut()
        .expect("policy object")
        .remove("actor_role");
    let malformed = claimed_task(user_key, malformed_payload);
    assert_eq!(
        effective_policy_for_task(&state, &malformed).mode,
        TaskExecutionMode::Configured
    );
    assert_eq!(
        execution_policy_authorization_error(&state, &malformed),
        Some("task_execution_policy_invalid")
    );

    let db = state.core.db.get().expect("test db");
    db.execute(
        "UPDATE auth_keys SET enabled = 0 WHERE user_key = ?1",
        rusqlite::params![user_key],
    )
    .expect("disable admin key");
    drop(db);

    let fallback = effective_policy_for_task(&state, &task);
    assert_eq!(fallback.mode, TaskExecutionMode::Configured);
    assert_eq!(fallback.approval_policy, ToolApprovalPolicy::OnRisk);
    assert_eq!(fallback.sandbox_mode, ToolSandboxMode::WorkspaceWrite);
    assert_eq!(fallback.actor_role, None);
    assert!(inheritable_policy_stamp(&state, &task).is_none());
    assert_eq!(
        execution_policy_authorization_error(&state, &task),
        Some("yolo_mode_admin_authority_expired")
    );
}

#[tokio::test]
async fn revoked_yolo_authority_blocks_before_skill_dispatch() {
    let user_key = "rk-revoked-yolo-skill";
    let state = state_with_admin_key(user_key);
    let mut payload = json!({"kind": "run_skill"});
    stamp_authenticated_submission_policy(
        &mut payload,
        Some(&identity("admin")),
        Some("clawcli"),
        Some("yolo"),
    )
    .expect("stamp yolo policy");
    let task = claimed_task(user_key, payload);

    let db = state.core.db.get().expect("test db");
    db.execute(
        "UPDATE auth_keys SET enabled = 0 WHERE user_key = ?1",
        rusqlite::params![user_key],
    )
    .expect("disable admin key");
    drop(db);

    let err = crate::skills::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": "tmp/out.txt", "content": "blocked"}),
    )
    .await
    .expect_err("revoked yolo authority must block before dispatch");
    let parsed = crate::skills::parse_policy_block_error(&err).expect("policy block error");
    assert_eq!(parsed.reason_code, "yolo_mode_admin_authority_expired");
    assert_eq!(parsed.decision, "deny");
}
