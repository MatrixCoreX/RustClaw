use serde_json::json;

fn task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 1,
        task_id: "async-recall-task".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("async-recall-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({"conversation_id":"async-recall-conversation"}).to_string(),
    }
}

#[tokio::test]
async fn no_opt_in_means_zero_remote_query_outbound() {
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.policy.memory.embedding_provider_kind = "remote_http".to_string();
    state.policy.memory.embedding_endpoint_ref = "MISSING_TEST_ENDPOINT".to_string();
    state.policy.memory.embedding_credential_ref = "MISSING_TEST_TOKEN".to_string();
    state.policy.memory.embedding_model = "fixture-embedding".to_string();
    state.policy.memory.embedding_dims = 3;
    let db = state.core.db.get().unwrap();
    db.execute(
        "INSERT OR IGNORE INTO auth_keys(user_key, role, enabled, created_at)
         VALUES ('async-recall-key', 'user', 1, '1')",
        [],
    )
    .unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    crate::repo::ensure_principal_ownership_schema(&db).unwrap();
    crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
    crate::repo::auth::ensure_principal_identity_schema(&db).unwrap();
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "async-recall-key")
        .unwrap()
        .unwrap();
    crate::memory::settings::update_memory_settings(
        &db,
        &claw_core::types::AuthIdentity {
            user_key: "async-recall-key".to_string(),
            principal_id,
            role: "user".to_string(),
            user_id: 1,
            chat_id: 2,
        },
        &crate::memory::settings::MemorySettingsUpdateRequest {
            scope: crate::memory::settings::MemorySettingScope::Principal,
            target_principal_id: None,
            conversation_id: None,
            use_mode: Some(crate::memory::settings::MemorySettingMode::Enabled),
            generate_mode: Some(crate::memory::settings::MemorySettingMode::Enabled),
            external_context_policy: Some(crate::memory::settings::ExternalContextPolicy::Exclude),
            expected_revision: Some(0),
            long_term_enabled: None,
        },
        state.policy.memory.long_term_enabled,
    )
    .unwrap();
    drop(db);

    let outcome = super::retrieve_for_task(&state, &task(), "ordinary query")
        .await
        .unwrap();
    assert_eq!(outcome.trace.remote_outbound_count, 0);
    assert_eq!(
        outcome.trace.fallback_code.as_deref(),
        Some("remote_embedding_consent_required")
    );
}

#[tokio::test]
async fn sensitive_query_is_never_sent_even_with_remote_consent() {
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.policy.memory.embedding_provider_kind = "remote_http".to_string();
    state.policy.memory.embedding_endpoint_ref = "MISSING_TEST_ENDPOINT".to_string();
    state.policy.memory.embedding_credential_ref = "MISSING_TEST_TOKEN".to_string();
    state.policy.memory.embedding_model = "fixture-embedding".to_string();
    state.policy.memory.embedding_dims = 3;
    let db = state.core.db.get().unwrap();
    db.execute(
        "INSERT OR IGNORE INTO auth_keys(user_key, role, enabled, created_at)
         VALUES ('async-recall-key', 'user', 1, '1')",
        [],
    )
    .unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    crate::repo::ensure_principal_ownership_schema(&db).unwrap();
    crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
    crate::repo::auth::ensure_principal_identity_schema(&db).unwrap();
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "async-recall-key")
        .unwrap()
        .unwrap();
    crate::memory::settings::update_memory_settings(
        &db,
        &claw_core::types::AuthIdentity {
            user_key: "async-recall-key".to_string(),
            principal_id,
            role: "user".to_string(),
            user_id: 1,
            chat_id: 2,
        },
        &crate::memory::settings::MemorySettingsUpdateRequest {
            scope: crate::memory::settings::MemorySettingScope::Principal,
            target_principal_id: None,
            conversation_id: None,
            use_mode: Some(crate::memory::settings::MemorySettingMode::Enabled),
            generate_mode: Some(crate::memory::settings::MemorySettingMode::Enabled),
            external_context_policy: Some(crate::memory::settings::ExternalContextPolicy::Allow),
            expected_revision: Some(0),
            long_term_enabled: None,
        },
        state.policy.memory.long_term_enabled,
    )
    .unwrap();
    drop(db);

    let outcome = super::retrieve_for_task(
        &state,
        &task(),
        "api_key=synthetic-secret-value ordinary query",
    )
    .await
    .unwrap();
    assert_eq!(outcome.trace.remote_outbound_count, 0);
    assert_eq!(
        outcome.trace.fallback_code.as_deref(),
        Some("remote_embedding_sensitive_query_blocked")
    );
}

#[test]
fn query_cache_is_byte_bounded_ttl_scoped_and_principal_invalidatable() {
    let now = crate::now_ts_u64() as i64;
    let principal_a = format!("principal-cache-a-{now}");
    let principal_b = format!("principal-cache-b-{now}");
    let key_a = super::query_cache_key(&principal_a, "profile", "policy", "query-a");
    let key_b = super::query_cache_key(&principal_b, "profile", "policy", "query-b");
    super::put_cached_vector(key_a.clone(), vec![1.0; 32], now + 60, 512);
    super::put_cached_vector(key_b.clone(), vec![2.0; 32], now + 60, 512);
    assert!(super::query_cache_bytes() <= 512);
    assert_eq!(super::get_cached_vector(&key_a, now), Some(vec![1.0; 32]));
    assert_eq!(super::get_cached_vector(&key_b, now), Some(vec![2.0; 32]));

    super::invalidate_principal_query_cache(&principal_a);
    assert_eq!(super::get_cached_vector(&key_a, now), None);
    assert_eq!(super::get_cached_vector(&key_b, now), Some(vec![2.0; 32]));

    let expired = super::query_cache_key(&principal_b, "profile", "policy", "expired");
    super::put_cached_vector(expired.clone(), vec![3.0; 8], now - 1, 512);
    assert_eq!(super::get_cached_vector(&expired, now), None);
    super::invalidate_principal_query_cache(&principal_b);
}
