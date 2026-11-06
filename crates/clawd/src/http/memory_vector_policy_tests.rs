use super::*;

fn policy_fixture() -> (AppState, crate::AuthIdentity, HeaderMap) {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let key = crate::repo::auth::create_auth_key(&state, "user").expect("policy fixture key");
    let identity = crate::resolve_auth_identity_by_key(&state, &key)
        .expect("policy identity lookup")
        .expect("policy identity");
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        key.parse().unwrap(),
    );
    {
        let db = state.core.db.get().unwrap();
        memory::embedding_jobs::initialize_embedding_runtime(&db, &state.policy.memory).unwrap();
    }
    (state, identity, headers)
}

#[tokio::test]
async fn index_status_returns_the_account_policy_not_a_conversation_policy() {
    let (state, identity, headers) = policy_fixture();
    let (account, conversation) = {
        let db = state.core.db.get().unwrap();
        (
            memory::settings::resolve_principal_memory_settings(
                &db,
                &identity.principal_id,
                state.policy.memory.long_term_enabled,
            )
            .unwrap(),
            memory::settings::resolve_memory_settings(
                &db,
                &identity.principal_id,
                Some("selected-conversation"),
                state.policy.memory.long_term_enabled,
            )
            .unwrap(),
        )
    };
    assert_ne!(account.policy_digest, conversation.policy_digest);
    let (code, Json(reply)) = vector_status(State(state), headers).await;
    assert_eq!(code, StatusCode::OK);
    let status = reply.data.unwrap();
    assert_eq!(status.policy_digest, account.policy_digest);
    assert_eq!(
        serde_json::to_value(status).unwrap()["policy_digest"],
        account.policy_digest
    );
}

#[tokio::test]
async fn index_policy_snapshot_keeps_stale_rejection_for_every_action() {
    let (state, identity, headers) = policy_fixture();
    let original = load_vector_status(&state, &identity.principal_id)
        .unwrap()
        .policy_digest;
    let update = serde_json::from_value(json!({
        "scope": "principal", "expected_revision": 0, "use_mode": "disabled",
    }))
    .unwrap();
    {
        let db = state.core.db.get().unwrap();
        memory::settings::update_memory_settings(
            &db,
            &identity,
            &update,
            state.policy.memory.long_term_enabled,
        )
        .unwrap();
    }
    let current = load_vector_status(&state, &identity.principal_id)
        .unwrap()
        .policy_digest;
    assert_ne!(original, current);
    let request = || MemoryVectorMutationRequest {
        expected_policy_digest: original.clone(),
    };
    let (code, Json(reply)) =
        vector_reindex(State(state.clone()), headers.clone(), Json(request())).await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(
        reply.error.as_deref(),
        Some("memory_settings_revision_conflict")
    );
    for action in ["paused", "active", "cancelled"] {
        let (code, Json(reply)) = vector_control(state.clone(), headers.clone(), request(), action);
        assert_eq!(code, StatusCode::CONFLICT);
        assert_eq!(
            reply.error.as_deref(),
            Some("memory_settings_revision_conflict")
        );
    }
    for action in ["paused", "active", "cancelled"] {
        let (code, Json(reply)) = vector_control(
            state.clone(),
            headers.clone(),
            MemoryVectorMutationRequest {
                expected_policy_digest: current.clone(),
            },
            action,
        );
        assert_eq!(code, StatusCode::OK);
        assert!(reply.ok);
    }
}

#[tokio::test]
async fn index_policy_status_still_requires_authentication() {
    let (state, _, _) = policy_fixture();
    let (code, Json(reply)) = vector_status(State(state), HeaderMap::new()).await;
    assert_eq!(code, StatusCode::UNAUTHORIZED);
    assert!(reply.data.is_none());
}
