use super::*;
use crate::memory::availability::{ByteMeasurement, GrowthDecision};

#[tokio::test]
async fn ordinary_http_response_has_manual_decision_without_global_limit_or_credentials() {
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;
    let (state, identity, _, key) = super::tests::fixture("user");
    let response = super::super::router()
        .with_state(state)
        .oneshot(
            Request::builder()
                .uri("/memory/availability")
                .header(claw_core::product_identity::AUTH_KEY_HEADER, key.as_str())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let data: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(
        data["data"]["settings"]["target_principal_id"],
        identity.principal_id
    );
    assert_eq!(data["data"]["manual_growth"]["decision"], "available");
    assert_eq!(
        data["data"]["manual_growth"]["configured_limit_bytes"]["status"],
        "admin_only"
    );
    assert_eq!(
        data["data"]["measurements"]["memory_domain_assigned_btree_bytes"]["status"],
        "admin_only"
    );
    assert!(!data.to_string().contains(&key));
}

#[tokio::test]
async fn absent_budget_remains_absent_after_authenticated_availability_read() {
    let (state, _, headers, _) = super::tests::fixture("admin");
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("DELETE FROM memory_storage_budget", [])
        .unwrap();
    let (status, Json(reply)) = get(
        State(state.clone()),
        headers,
        Query(CapacityQuery::default()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let report = reply.data.unwrap();
    assert_eq!(
        report.manual_growth.decision,
        GrowthDecision::StoragePolicyMissing
    );
    assert!(matches!(
        report.manual_growth.configured_limit_bytes,
        ByteMeasurement::Unavailable { .. }
    ));
    assert_eq!(
        state
            .core
            .db
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM memory_storage_budget", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn missing_source_quota_does_not_repair_policy_or_hide_manual_domain_state() {
    let (state, identity, headers, _) = super::tests::fixture("user");
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
            [&identity.principal_id],
        )
        .unwrap();
    let (status, Json(reply)) = get(
        State(state.clone()),
        headers,
        Query(CapacityQuery::default()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let report = reply.data.unwrap();
    assert_eq!(
        report.capacity_decision,
        crate::memory::capacity::GenerationAvailability::PrincipalQuotaMissing
    );
    assert_eq!(report.manual_growth.decision, GrowthDecision::Available);
    assert_eq!(
        report.manual_growth.configured_limit_bytes,
        ByteMeasurement::AdminOnly
    );
    assert_eq!(
        state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM memory_principal_quotas WHERE principal_id=?1",
                [&identity.principal_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn invalid_budget_is_visible_but_not_normalized_by_http_read() {
    let (state, _, headers, _) = super::tests::fixture("admin");
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE memory_storage_budget SET configured_limit_bytes='broken'",
            [],
        )
        .unwrap();
    let (status, Json(reply)) = get(
        State(state.clone()),
        headers,
        Query(CapacityQuery::default()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reply.data.unwrap().manual_growth.decision,
        GrowthDecision::StoragePolicyInvalid
    );
    assert_eq!(
        state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT configured_limit_bytes FROM memory_storage_budget",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "broken"
    );
}
