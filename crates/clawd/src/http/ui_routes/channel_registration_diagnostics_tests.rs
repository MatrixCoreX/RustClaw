use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use super::{support::Fixture, *};

const PRIVATE_DETAIL: &str =
    "fixture-private-description credential=fixture-secret /fixture/private";

#[tokio::test]
async fn channel_registration_diagnostics_non_authoritative_shapes_do_not_control_state() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for (code, status) in [
            ("authorization_pending", "pending"),
            ("access_denied", "failed"),
        ] {
            for detail in [
                serde_json::Value::Null,
                json!({"decision":"allow","secret":PRIVATE_DETAIL}),
                json!([PRIVATE_DETAIL]),
                json!(123),
            ] {
                let fixture = Fixture::with_status_result(
                    platform,
                    StatusCode::BAD_REQUEST,
                    json!({
                        "error":code,"error_description":detail,
                    }),
                )
                .await;
                let before = fixture.snapshot();
                let (http_status, body) = finish_poll(&fixture).await;
                assert_eq!(http_status, StatusCode::OK);
                assert_eq!(body.data.unwrap().status, status);
                assert_no_launch(&fixture, &before);
            }
        }
    }
}

#[tokio::test]
async fn channel_registration_diagnostics_existing_records_are_only_projected_not_modified() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::new(platform, true).await;
        let before = fixture.snapshot();
        for (status, stored, expected) in [
            (
                "failed",
                Some(PRIVATE_DETAIL),
                Some("channel_bind_registration_rejected"),
            ),
            ("failed", None, Some("channel_bind_registration_rejected")),
            (
                "failed",
                Some("channel_bind_registration_request_timeout"),
                Some("channel_bind_registration_request_timeout"),
            ),
            (
                "failed",
                Some("channel_bind_registration_start_failed"),
                Some("channel_bind_registration_start_failed"),
            ),
            (
                "failed",
                Some("channel_bind_registration_access_denied credential=fixture-secret"),
                Some("channel_bind_registration_rejected"),
            ),
            (
                "failed",
                Some("expired"),
                Some("channel_bind_registration_rejected"),
            ),
            ("expired", Some(PRIVATE_DETAIL), Some("expired")),
            ("bound", Some(PRIVATE_DETAIL), None),
        ] {
            {
                let db = fixture.state.core.db.get().unwrap();
                db.execute(
                    "UPDATE pending_channel_bind_sessions SET status=?1,error_text=?2 WHERE id=?3",
                    rusqlite::params![status, stored, fixture.session_id],
                )
                .unwrap();
            }
            let (http_status, Json(body)) = get_channel_bind_session(
                fixture.state.clone(),
                Fixture::headers(ADMIN_KEY),
                fixture.session_id,
                platform,
            )
            .await;
            assert_eq!(http_status, StatusCode::OK);
            let public = body.data.unwrap();
            assert_eq!(public.status, status);
            assert_eq!(public.error_text.as_deref(), expected);
            assert_eq!(fixture.latest_session().error_text.as_deref(), stored);
            assert_eq!(fixture.snapshot(), before);
            assert_eq!(fixture.requests(), 0);
        }
    }
}

async fn finish_poll(
    fixture: &Fixture,
) -> (StatusCode, ApiResponse<FeishuBindSessionStatusResponse>) {
    let poll = fixture.poll();
    fixture.wait_for_poll().await;
    fixture.finish_poll();
    let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(15), poll)
        .await
        .unwrap()
        .unwrap();
    (status, body)
}

fn assert_no_launch(fixture: &Fixture, before: &[Vec<u8>]) {
    assert_eq!(fixture.snapshot(), before);
    assert!(!fixture.root.join("logs").exists());
    assert_eq!(fixture.requests(), 1);
}

#[tokio::test]
async fn channel_registration_diagnostics_terminal_errors_store_only_owned_tokens() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for status in [StatusCode::OK, StatusCode::BAD_REQUEST] {
            for (code, expected) in [
                ("access_denied", "channel_bind_registration_access_denied"),
                (
                    "fixture-private-error credential=fixture-secret",
                    "channel_bind_registration_rejected",
                ),
            ] {
                let mut fixture = Fixture::with_status_result(
                    platform,
                    status,
                    json!({"error":code,"error_description":PRIVATE_DETAIL}),
                )
                .await;
                let before = fixture.snapshot();
                let (status, body) = finish_poll(&fixture).await;
                assert_eq!(status, StatusCode::OK);
                assert!(body.ok && body.error.is_none());
                let session = body.data.unwrap();
                assert_eq!(session.status, "failed");
                assert_eq!(session.error_text.as_deref(), Some(expected));
                let public = serde_json::to_string(&session).unwrap();
                assert!(!public.contains(PRIVATE_DETAIL));
                assert!(!public.contains("fixture-private-error"));
                fixture.reopen_database();
                assert_eq!(
                    fixture.latest_session().error_text.as_deref(),
                    Some(expected)
                );
                let (status, Json(body)) = get_channel_bind_session(
                    fixture.state.clone(),
                    Fixture::headers(ADMIN_KEY),
                    fixture.session_id,
                    platform,
                )
                .await;
                assert_eq!(status, StatusCode::OK);
                assert_eq!(body.data.unwrap().error_text.as_deref(), Some(expected));
                assert_no_launch(&fixture, &before);
            }
        }
    }
}

#[tokio::test]
async fn channel_registration_diagnostics_preserves_polling_and_expiry_controls() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for http_status in [StatusCode::OK, StatusCode::BAD_REQUEST] {
            for (code, status, error) in [
                ("authorization_pending", "pending", None),
                ("slow_down", "pending", None),
                ("expired_token", "expired", Some("expired")),
            ] {
                let fixture = Fixture::with_status_result(
                    platform,
                    http_status,
                    json!({"error":code,"error_description":PRIVATE_DETAIL}),
                )
                .await;
                let before = fixture.snapshot();
                let (http_status, body) = finish_poll(&fixture).await;
                assert_eq!(http_status, StatusCode::OK);
                let session = body.data.unwrap();
                assert_eq!(session.status, status);
                assert_eq!(session.error_text.as_deref(), error);
                assert_no_launch(&fixture, &before);
            }
        }
    }
}

#[tokio::test]
async fn channel_registration_diagnostics_errors_never_authorize_mixed_credentials() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for (code, expected_status) in [
            ("authorization_pending", "pending"),
            ("access_denied", "failed"),
            ("fixture-private-error", "failed"),
        ] {
            let fixture = Fixture::with_result(
                platform,
                json!({
                    "client_id":"fixture-app-id","client_secret":"fixture-app-secret",
                    "user_info":{"open_id":"fixture-open-id"},
                    "error":code,"error_description":PRIVATE_DETAIL,
                }),
            )
            .await;
            let before = fixture.snapshot();
            let writes = Arc::new(AtomicUsize::new(0));
            let starts = Arc::new(AtomicUsize::new(0));
            fixture.release_response();
            let session = maybe_complete_official_scan_with_effects(
                &fixture.state,
                platform,
                fixture.latest_session(),
                async {
                    starts.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                |_, _, _, _| {
                    writes.fetch_add(1, Ordering::SeqCst);
                    anyhow::bail!("fixture_unexpected_credential_write")
                },
            )
            .await;
            assert_eq!(writes.load(Ordering::SeqCst), 0, "error={code}");
            assert_eq!(starts.load(Ordering::SeqCst), 0);
            assert_eq!(session.unwrap().status, expected_status);
            assert_no_launch(&fixture, &before);
        }
    }
}

#[tokio::test]
async fn channel_registration_diagnostics_failed_status_never_reaches_application() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            let fixture = Fixture::with_status_result(
                platform,
                status,
                json!({
                    "client_id":"fixture-app-id","client_secret":"fixture-app-secret",
                    "user_info":{"open_id":"fixture-open-id"},
                    "error":"access_denied","error_description":PRIVATE_DETAIL,
                }),
            )
            .await;
            let before = fixture.snapshot();
            let (status, body) = finish_poll(&fixture).await;
            assert_eq!(status, StatusCode::BAD_GATEWAY);
            assert!(!body.ok && body.data.is_none());
            assert_eq!(
                body.error.as_deref(),
                Some("channel_bind_registration_http_status_rejected")
            );
            let session = fixture.latest_session();
            assert_eq!(session.status, "pending");
            assert!(session.error_text.is_none());
            assert_no_launch(&fixture, &before);
        }
    }
}

#[tokio::test]
async fn channel_registration_diagnostics_start_failure_is_machine_only_and_durable() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let mut fixture = Fixture::new(platform, false).await;
        let starts = Arc::new(AtomicUsize::new(0));
        fixture.release_response();
        let session = maybe_complete_official_scan_with_start(
            &fixture.state,
            platform,
            fixture.latest_session(),
            async {
                starts.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!(PRIVATE_DETAIL)
            },
        )
        .await
        .unwrap();
        assert_eq!(session.status, "failed");
        assert_eq!(
            session.error_text.as_deref(),
            Some("channel_bind_registration_start_failed")
        );
        fixture.reopen_database();
        assert_eq!(fixture.latest_session().error_text, session.error_text);
        let (status, Json(body)) = get_channel_bind_session(
            fixture.state.clone(),
            Fixture::headers(ADMIN_KEY),
            fixture.session_id,
            platform,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.data.unwrap().error_text, session.error_text);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.requests(), 1);
        assert!(!fixture.root.join("logs").exists());
    }
}
