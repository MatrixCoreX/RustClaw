use std::sync::atomic::Ordering;

use axum::http::{Method, StatusCode};
use serde_json::json;

#[path = "model_control_plane_test_support.rs"]
mod support;
use support::{Fixture, INLINE_KEY, MANAGED_KEY};

#[tokio::test]
async fn model_connection_probe_denies_non_admin_before_credential_resolution() {
    let mut failures = Vec::new();
    for key in [None, Some(""), Some("  "), Some(INLINE_KEY)] {
        let fixture = Fixture::new().await;
        let before = fixture.snapshot();
        let (status, response) = fixture
            .api(
                Some("user"),
                Method::POST,
                "/llm/test",
                Some(fixture.llm_body(key)),
            )
            .await;
        let requests = fixture.requests.load(Ordering::SeqCst);
        let managed_seen = fixture.managed_seen.load(Ordering::SeqCst);
        if status != StatusCode::FORBIDDEN || requests != 0 {
            failures.push(json!({"inline_kind":if key.is_none(){"omitted"}else if key.unwrap().trim().is_empty(){"empty"}else{"fixture"},"status":status.as_u16(),"provider_requests":requests,"managed_fixture_received":managed_seen>0}));
        }
        if status == StatusCode::FORBIDDEN {
            assert_eq!(response["ok"], false);
            assert_eq!(response["error"], "admin_required");
        }
        assert_eq!(fixture.snapshot(), before);
        assert!(!response.to_string().contains(MANAGED_KEY));
    }
    assert!(
        failures.is_empty(),
        "unauthorized_probe_results={failures:?}"
    );
}

#[tokio::test]
async fn model_control_plane_writes_deny_non_admin_without_inline_keys() {
    let mut failures = Vec::new();
    for index in 0..3 {
        let fixture = Fixture::new().await;
        let (path, body) = fixture.writes().remove(index);
        let before = fixture.snapshot();
        let (status, response) = fixture
            .api(Some("user"), Method::POST, path, Some(body))
            .await;
        let unchanged = before == fixture.snapshot();
        if status != StatusCode::FORBIDDEN || !unchanged {
            failures
                .push(json!({"path":path,"status":status.as_u16(),"files_unchanged":unchanged}));
        }
        if status == StatusCode::FORBIDDEN {
            assert_eq!(response["ok"], false);
            assert_eq!(response["error"], "admin_required");
        }
        assert!(!response.to_string().contains(MANAGED_KEY));
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
    }
    assert!(
        failures.is_empty(),
        "unauthorized_write_results={failures:?}"
    );
}

#[tokio::test]
async fn model_control_plane_anonymous_requests_have_no_effects() {
    let fixture = Fixture::new().await;
    let before = fixture.snapshot();
    let mut operations = fixture.writes();
    operations.push(("/llm/test", fixture.llm_body(None)));
    for (path, body) in operations {
        let (status, response) = fixture.api(None, Method::POST, path, Some(body)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(response["ok"], false);
    }
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.snapshot(), before);
}

#[tokio::test]
async fn model_control_plane_administrator_writes_still_work() {
    for index in 0..3 {
        let fixture = Fixture::new().await;
        let (path, body) = fixture.writes().remove(index);
        let before = fixture.snapshot();
        let (status, response) = fixture
            .api(Some("admin"), Method::POST, path, Some(body))
            .await;
        assert_eq!(status, StatusCode::OK, "{path}: {response:?}");
        assert_eq!(response["ok"], true);
        assert_ne!(fixture.snapshot(), before);
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
        assert!(!response.to_string().contains(MANAGED_KEY));
    }
}

#[tokio::test]
async fn model_connection_probe_administrator_uses_draft_or_managed_fixture_without_saving() {
    for key in [None, Some(""), Some("  "), Some(INLINE_KEY)] {
        let fixture = Fixture::new().await;
        let before = fixture.snapshot();
        let (status, response) = fixture
            .api(
                Some("admin"),
                Method::POST,
                "/llm/test",
                Some(fixture.llm_body(key)),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{response:?}");
        assert_eq!(response["data"]["response_text"], "fixture-answer");
        assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            fixture.managed_seen.load(Ordering::SeqCst),
            usize::from(key != Some(INLINE_KEY))
        );
        assert_eq!(fixture.snapshot(), before);
        assert!(!response.to_string().contains(MANAGED_KEY));
    }
}

#[tokio::test]
async fn authenticated_model_readiness_metadata_stays_readable_and_secret_free() {
    let fixture = Fixture::new().await;
    for role in ["user", "admin"] {
        for path in ["/llm/config", "/models/catalog"] {
            let (status, response) = fixture.api(Some(role), Method::GET, path, None).await;
            assert_eq!(status, StatusCode::OK, "{path}: {response:?}");
            assert_eq!(response["ok"], true);
            assert!(!response.to_string().contains(MANAGED_KEY));
        }
    }
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn model_control_plane_ignores_claimed_administrator_fields() {
    let fixture = Fixture::new().await;
    let before = fixture.snapshot();
    let mut body = fixture.llm_body(None);
    body["role"] = json!("admin");
    body["isAdmin"] = json!(true);
    body["user_key"] = json!("fixture-model-admin");
    let (status, response) = fixture
        .api(Some("user"), Method::POST, "/llm/test", Some(body))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(response["error"], "admin_required");
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.snapshot(), before);
}
