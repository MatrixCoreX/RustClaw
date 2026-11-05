use super::*;
use axum::response::IntoResponse;
use std::time::{Duration, Instant};

#[path = "aipp_read_surfaces_tests/fixtures.rs"]
mod fixtures;
use fixtures::{BUNDLE, PREVIEW, SURFACES, Surface, SurfaceFixture};

fn auth_headers(key: Option<&'static str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(key) = key {
        headers.insert(
            claw_core::product_identity::AUTH_KEY_HEADER,
            axum::http::HeaderValue::from_static(key),
        );
    }
    headers
}

async fn request(
    surface: Surface,
    state: &AppState,
    key: Option<&'static str>,
    asset: &str,
) -> axum::response::Response {
    let headers = auth_headers(key);
    match surface {
        Surface::Catalog => get_aipp_catalog(State(state.clone()), headers)
            .await
            .into_response(),
        Surface::Preview => {
            get_aipp_media_preview(
                State(state.clone()),
                headers,
                AxumPath(("fixture_view".into(), 1)),
            )
            .await
        }
        Surface::Bundle => {
            get_aipp_bundle_asset(
                State(state.clone()),
                headers,
                AxumPath(("fixture_view".into(), asset.into())),
            )
            .await
        }
    }
}

async fn error_body(
    response: axum::response::Response,
    status: StatusCode,
    code: &str,
    auth: bool,
) {
    assert_eq!(response.status(), status);
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body,
        json!({
            "ok": false, "error": code,
            "data": if auth { Value::Null } else { json!({
                "error_code": code, "message_key": format!("clawd.ui.aipp.{code}"),
            }) },
        })
    );
}

async fn pool_wait(key: &'static str, status: StatusCode, code: Option<&str>) {
    for surface in SURFACES {
        let fixture = SurfaceFixture::new(surface);
        let held = fixture.state.core.db.get().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            drop(held);
        });
        let start = Instant::now();
        let (response, timer_delay) =
            tokio::join!(request(surface, &fixture.state, Some(key), ""), async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                start.elapsed()
            },);
        release.join().unwrap();
        assert_eq!(response.status(), status, "{surface:?}");
        eprintln!(
            "aipp_surface={surface:?} pool_hold_ms=500 timer_delay_ms={}",
            timer_delay.as_millis()
        );
        assert!(
            timer_delay < Duration::from_millis(400),
            "{surface:?}: {timer_delay:?}"
        );
        if let Some(code) = code {
            error_body(response, status, code, true).await;
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn all_aipp_read_surfaces_keep_the_executor_responsive_during_admin_auth_pool_wait() {
    pool_wait("aipp-view-admin", StatusCode::OK, None).await;
}

#[tokio::test(flavor = "current_thread")]
async fn all_aipp_read_surfaces_keep_the_executor_responsive_during_invalid_auth_pool_wait() {
    pool_wait(
        "aipp-view-unknown",
        StatusCode::UNAUTHORIZED,
        Some("auth_key_invalid"),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn all_aipp_read_surfaces_keep_the_executor_responsive_during_non_admin_auth_pool_wait() {
    pool_wait(
        "aipp-view-user",
        StatusCode::FORBIDDEN,
        Some("admin_required"),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn inline_aipp_catalog_control_demonstrates_executor_stall() {
    let fixture = SurfaceFixture::new(Surface::Catalog);
    let held = fixture.state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let start = Instant::now();
    let ((status, _), timer_delay) = tokio::join!(
        async { read_aipp_catalog(&fixture.state, &auth_headers(Some("aipp-view-admin"))) },
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            start.elapsed()
        },
    );
    release.join().unwrap();
    assert_eq!(status, StatusCode::OK);
    assert!(timer_delay >= Duration::from_millis(500), "{timer_delay:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn all_aipp_read_surfaces_reject_missing_keys_without_waiting_for_the_database() {
    for surface in SURFACES {
        let fixture = SurfaceFixture::new(surface);
        let _held = fixture.state.core.db.get().unwrap();
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            request(surface, &fixture.state, None, ""),
        )
        .await
        .unwrap();
        error_body(
            response,
            StatusCode::UNAUTHORIZED,
            "auth_key_required",
            true,
        )
        .await;
    }
}

#[tokio::test]
async fn all_aipp_read_surfaces_recheck_roles_and_key_revocation_on_every_request() {
    for surface in SURFACES {
        let fixture = SurfaceFixture::new(surface);
        assert_eq!(
            request(surface, &fixture.state, Some("aipp-view-admin"), "")
                .await
                .status(),
            StatusCode::OK
        );
        fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE auth_keys SET role='user' WHERE user_key='aipp-view-admin'",
                [],
            )
            .unwrap();
        error_body(
            request(surface, &fixture.state, Some("aipp-view-admin"), "").await,
            StatusCode::FORBIDDEN,
            "admin_required",
            true,
        )
        .await;
        fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE auth_keys SET role='admin', enabled=0 WHERE user_key='aipp-view-admin'",
                [],
            )
            .unwrap();
        error_body(
            request(surface, &fixture.state, Some("aipp-view-admin"), "").await,
            StatusCode::UNAUTHORIZED,
            "auth_key_invalid",
            true,
        )
        .await;
    }
}

#[tokio::test]
async fn all_aipp_read_surfaces_fail_closed_without_exposing_auth_database_errors() {
    for surface in SURFACES {
        let fixture = SurfaceFixture::new(surface);
        fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .execute("DROP TABLE auth_keys", [])
            .unwrap();
        error_body(
            request(surface, &fixture.state, Some("aipp-view-admin"), "").await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "auth_lookup_failed",
            true,
        )
        .await;
    }
}

#[path = "aipp_read_surfaces_tests/state.rs"]
mod state_and_streams;

#[path = "aipp_read_surfaces_tests/security.rs"]
mod security;
