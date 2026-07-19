use serde_json::json;

use super::*;
use crate::approval_grant::{ApprovalScopeBinding, ApprovalScopeEntry};

fn scope() -> ApprovalScopeBinding {
    ApprovalScopeBinding {
        scope_kind: "session".to_string(),
        scope_fingerprint: "sha256:scope".to_string(),
        entries: vec![ApprovalScopeEntry {
            capability: "filesystem.remove_path".to_string(),
            action: "remove_path".to_string(),
            effect: "mutate".to_string(),
            resource_kind: "workspace_path".to_string(),
            resources: vec!["run/example.txt".to_string()],
        }],
    }
}

fn task(user_key: &str, chat_id: i64) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: "task-match".to_string(),
        user_id: 42,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "web".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({}).to_string(),
    }
}

fn state_with_scope_db() -> AppState {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("db");
    ensure_approval_scope_grant_schema(&db).expect("scope schema");
    state
}

#[test]
fn signed_scope_grant_matches_only_exact_actor_session_and_scope() {
    let state = state_with_scope_db();
    let binding = ApprovalBinding {
        action_fingerprint: "sha256:action".to_string(),
        arguments_hash: "sha256:args".to_string(),
        action_count: 1,
        targets: vec!["fs_basic".to_string()],
        scope: Some(scope()),
    };
    let db = state.core.db.get().expect("db");
    let grant = insert_approval_scope_grant(
        &db,
        "task-source",
        42,
        7,
        "web",
        "actor-key",
        binding.scope.as_ref().expect("scope"),
        crate::now_ts_u64() as i64,
    )
    .expect("insert scope grant");
    drop(db);

    let matched = match_approval_scope_grant(&state, &task("actor-key", 7), &binding)
        .expect("match scope")
        .expect("scope grant");
    assert_eq!(matched.grant_id, grant.grant_id);
    assert!(
        match_approval_scope_grant(&state, &task("other-key", 7), &binding)
            .expect("other actor lookup")
            .is_none()
    );
    assert!(
        match_approval_scope_grant(&state, &task("actor-key", 8), &binding)
            .expect("other session lookup")
            .is_none()
    );
}

#[test]
fn tampered_scope_grant_signature_fails_closed_and_revocation_is_immediate() {
    let state = state_with_scope_db();
    let binding = ApprovalBinding {
        action_fingerprint: "sha256:action".to_string(),
        arguments_hash: "sha256:args".to_string(),
        action_count: 1,
        targets: vec!["fs_basic".to_string()],
        scope: Some(scope()),
    };
    let db = state.core.db.get().expect("db");
    let grant = insert_approval_scope_grant(
        &db,
        "task-source",
        42,
        7,
        "web",
        "actor-key",
        binding.scope.as_ref().expect("scope"),
        crate::now_ts_u64() as i64,
    )
    .expect("insert scope grant");
    db.execute(
        "UPDATE approval_scope_grants SET signature = 'hmac-sha256:00' WHERE grant_id = ?1",
        rusqlite::params![grant.grant_id],
    )
    .expect("tamper signature");
    drop(db);
    assert!(
        match_approval_scope_grant(&state, &task("actor-key", 7), &binding)
            .expect("tampered lookup")
            .is_none()
    );

    let db = state.core.db.get().expect("db");
    let second = insert_approval_scope_grant(
        &db,
        "task-source-2",
        42,
        7,
        "web",
        "actor-key",
        binding.scope.as_ref().expect("scope"),
        crate::now_ts_u64() as i64,
    )
    .expect("insert second grant");
    drop(db);
    assert!(
        revoke_approval_scope_grant(&state, "actor-key", &second.grant_id).expect("revoke scope")
    );
    assert!(
        match_approval_scope_grant(&state, &task("actor-key", 7), &binding)
            .expect("revoked lookup")
            .is_none()
    );
}

#[test]
fn expired_scope_grant_does_not_match() {
    let state = state_with_scope_db();
    let binding = ApprovalBinding {
        action_fingerprint: "sha256:action".to_string(),
        arguments_hash: "sha256:args".to_string(),
        action_count: 1,
        targets: vec!["fs_basic".to_string()],
        scope: Some(scope()),
    };
    let db = state.core.db.get().expect("db");
    insert_approval_scope_grant(
        &db,
        "task-expired",
        42,
        7,
        "web",
        "actor-key",
        binding.scope.as_ref().expect("scope"),
        crate::now_ts_u64() as i64 - APPROVAL_SCOPE_GRANT_TTL_SECONDS,
    )
    .expect("insert expired scope grant");
    drop(db);

    assert!(
        match_approval_scope_grant(&state, &task("actor-key", 7), &binding)
            .expect("expired lookup")
            .is_none()
    );
}
