use super::*;
use crate::memory::availability::ByteMeasurement;

fn fixture(role: &str) -> (AppState, crate::AuthIdentity, HeaderMap, String) {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let key = crate::repo::auth::create_auth_key(&state, role).unwrap();
    let identity = crate::resolve_auth_identity_by_key(&state, &key)
        .unwrap()
        .unwrap();
    {
        let db = state.core.db.get().unwrap();
        crate::memory::retention::ensure_principal_quota(
            &db,
            &state.policy.memory,
            &identity.principal_id,
            1,
        )
        .unwrap();
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        key.parse().unwrap(),
    );
    (state, identity, headers, key)
}

#[tokio::test]
async fn capacity_api_requires_a_current_enabled_credential() {
    let (state, _, headers, key) = fixture("user");
    let (status, Json(reply)) = get(
        State(state.clone()),
        HeaderMap::new(),
        Query(CapacityQuery::default()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(reply.data.is_none());
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("UPDATE auth_keys SET enabled=0 WHERE user_key=?1", [&key])
        .unwrap();
    let (status, Json(reply)) = get(State(state), headers, Query(CapacityQuery::default())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(reply.data.is_none());
}

#[tokio::test]
async fn ordinary_capacity_response_is_own_scoped_and_has_no_global_numbers() {
    let (state, identity, headers, _) = fixture("user");
    let (status, Json(reply)) = get(State(state), headers, Query(CapacityQuery::default())).await;
    assert_eq!(status, StatusCode::OK);
    let report = reply.data.unwrap();
    assert_eq!(report.settings.target_principal_id, identity.principal_id);
    assert_eq!(report.settings.scope, MemorySettingScope::Principal);
    assert_eq!(
        report.measurements.main_sqlite_allocated_page_bytes,
        ByteMeasurement::AdminOnly
    );
    assert_eq!(
        report.measurements.database_path_filesystem_available_bytes,
        ByteMeasurement::AdminOnly
    );
}

#[tokio::test]
async fn administrator_observation_exposes_measurements_but_never_paths_or_keys() {
    let (state, _, headers, key) = fixture("admin");
    let (status, Json(reply)) = get(State(state), headers, Query(CapacityQuery::default())).await;
    assert_eq!(status, StatusCode::OK);
    let report = serde_json::to_value(reply.data.unwrap()).unwrap();
    assert_eq!(
        report["measurements"]["main_sqlite_allocated_page_bytes"]["status"],
        "measured"
    );
    assert_eq!(
        report["measurements"]["database_path_filesystem_available_bytes"]["status"],
        "unavailable"
    );
    let text = report.to_string();
    assert!(!text.contains(&key));
    assert!(!text.contains("database_path\":"));
}

#[tokio::test]
async fn capacity_role_is_fresh_for_each_request_not_cached_from_a_previous_admin_read() {
    let (state, _, headers, key) = fixture("admin");
    let (status, Json(reply)) = get(
        State(state.clone()),
        headers.clone(),
        Query(CapacityQuery::default()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(matches!(
        reply
            .data
            .unwrap()
            .measurements
            .main_sqlite_allocated_page_bytes,
        ByteMeasurement::Measured { .. }
    ));
    // Out-of-band host role change in this disposable database. The public
    // key-management API intentionally forbids demoting its sole administrator.
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("UPDATE auth_keys SET role='user' WHERE user_key=?1", [&key])
        .unwrap();
    let (status, Json(reply)) = get(State(state), headers, Query(CapacityQuery::default())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reply
            .data
            .unwrap()
            .measurements
            .main_sqlite_allocated_page_bytes,
        ByteMeasurement::AdminOnly
    );
}

#[tokio::test]
async fn selected_conversation_policy_and_account_policy_remain_distinct() {
    let (state, identity, headers, _) = fixture("user");
    let expected = {
        let db = state.core.db.get().unwrap();
        settings::resolve_memory_settings(
            &db,
            &identity.principal_id,
            Some("selected"),
            state.policy.memory.long_term_enabled,
        )
        .unwrap()
    };
    let (status, Json(reply)) = get(
        State(state.clone()),
        headers.clone(),
        Query(CapacityQuery {
            scope: Some(MemorySettingScope::Conversation),
            conversation_id: Some("selected".into()),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply.data.unwrap().settings, expected);
    let (_, Json(reply)) = get(State(state), headers, Query(CapacityQuery::default())).await;
    assert_ne!(
        reply.data.unwrap().settings.policy_digest,
        expected.policy_digest
    );
}

#[tokio::test]
async fn ambiguous_or_administrator_capacity_scopes_are_not_silently_reinterpreted() {
    let (state, _, headers, _) = fixture("admin");
    for query in [
        CapacityQuery {
            scope: Some(MemorySettingScope::Admin),
            conversation_id: None,
        },
        CapacityQuery {
            scope: Some(MemorySettingScope::Conversation),
            conversation_id: Some(" ".into()),
        },
        CapacityQuery {
            scope: None,
            conversation_id: Some("selected".into()),
        },
    ] {
        let (status, Json(reply)) = get(State(state.clone()), headers.clone(), Query(query)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(reply.data.is_none());
    }
}

#[tokio::test]
async fn capacity_query_rejects_cross_principal_selectors_at_the_actual_router() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    let (state, _, _, key) = fixture("admin");
    let router = super::super::router().with_state(state);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/memory/availability?target_principal_id=other")
                .header(claw_core::product_identity::AUTH_KEY_HEADER, key)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn schema_failure_is_safe_and_never_triggers_read_path_migration() {
    let (state, _, headers, _) = fixture("user");
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("DELETE FROM runtime_schema_migrations", [])
        .unwrap();
    let (status, Json(reply)) = get(
        State(state.clone()),
        headers,
        Query(CapacityQuery::default()),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error.as_deref(), Some("memory_capacity_unavailable"));
    assert!(reply.data.is_none());
    let count: i64 = state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM runtime_schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
