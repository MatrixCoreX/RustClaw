use std::time::Duration;

use super::*;

#[path = "channel_registration_fixture_tests.rs"]
mod support;
use support::{Fixture, ADMIN_KEY};

#[path = "channel_registration_begin_tests.rs"]
mod begin_tests;

#[path = "channel_registration_completion_tests.rs"]
mod completion_tests;

#[path = "channel_registration_application_tests.rs"]
mod application_tests;

#[path = "channel_registration_recovery_tests.rs"]
mod recovery_tests;

#[path = "channel_registration_protocol_tests.rs"]
mod protocol_tests;

#[path = "channel_registration_diagnostics_tests.rs"]
mod diagnostics_tests;

#[tokio::test]
async fn channel_registration_pending_control_keeps_the_session_and_configuration() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::new(platform, true).await;
        let before = fixture.snapshot();
        let poll = fixture.poll();
        fixture.wait_for_poll().await;
        fixture.finish_poll();
        let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(30), poll)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(body.ok);
        assert_eq!(body.data.unwrap().status, "pending");
        assert_eq!(fixture.requests(), 1);
        assert_eq!(fixture.snapshot(), before);
    }
}

#[tokio::test]
async fn channel_registration_poll_rejects_anonymous_user_and_other_owner_without_network() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::new(platform, true).await;
        let before = fixture.snapshot();
        for (key, expected) in [
            (None, StatusCode::UNAUTHORIZED),
            (Some("fixture-channel-user"), StatusCode::FORBIDDEN),
            (Some("fixture-channel-user"), StatusCode::NOT_FOUND),
        ] {
            if expected == StatusCode::NOT_FOUND {
                let db = fixture.state.core.db.get().unwrap();
                db.execute(
                    "UPDATE auth_keys SET role = 'user' WHERE user_key = ?1",
                    [ADMIN_KEY],
                )
                .unwrap();
                db.execute(
                    "UPDATE auth_keys SET role = 'admin' WHERE user_key = 'fixture-channel-user'",
                    [],
                )
                .unwrap();
            }
            let headers = key.map(Fixture::headers).unwrap_or_default();
            let (status, Json(body)) = get_channel_bind_session(
                fixture.state.clone(),
                headers,
                fixture.session_id,
                platform,
            )
            .await;
            assert_eq!(status, expected);
            assert!(!body.ok);
        }
        assert_eq!(fixture.requests(), 0);
        assert_eq!(fixture.snapshot(), before);
    }
}

#[tokio::test]
async fn channel_registration_late_result_cannot_rewrite_reset_deleted_expired_or_revoked_setup() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut failures = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for mutation in [
            "reset",
            "deleted",
            "expired",
            "revoked",
            "role_changed",
            "key_deleted",
            "bound",
            "flow_replaced",
            "owner_changed",
        ] {
            let fixture = Fixture::new(platform, false).await;
            let before = fixture.snapshot();
            let poll = fixture.poll();
            fixture.wait_for_poll().await;
            if mutation == "reset" {
                // Exercise the real binding-state reset, without the route's
                // process-wide service termination on a developer machine.
                reset_channel_binding_state_for_user_key(
                    &fixture.state,
                    platform.channel(),
                    ADMIN_KEY,
                )
                .unwrap();
            } else {
                let db = fixture.state.core.db.get().unwrap();
                match mutation {
                    "deleted" => {
                        db.execute(
                            "DELETE FROM pending_channel_bind_sessions WHERE id = ?1",
                            [fixture.session_id],
                        )
                        .unwrap();
                    }
                    "expired" => {
                        db.execute("UPDATE pending_channel_bind_sessions SET expires_at = '1' WHERE id = ?1", [fixture.session_id]).unwrap();
                    }
                    "revoked" => {
                        db.execute(
                            "UPDATE auth_keys SET enabled = 0 WHERE user_key = ?1",
                            [ADMIN_KEY],
                        )
                        .unwrap();
                    }
                    "role_changed" => {
                        db.execute(
                            "UPDATE auth_keys SET role = 'user' WHERE user_key = ?1",
                            [ADMIN_KEY],
                        )
                        .unwrap();
                    }
                    "key_deleted" => {
                        db.execute("DELETE FROM auth_keys WHERE user_key = ?1", [ADMIN_KEY])
                            .unwrap();
                    }
                    "bound" => {
                        db.execute("UPDATE pending_channel_bind_sessions SET status = 'bound', external_user_id = 'fixture-user', external_chat_id = 'fixture-chat' WHERE id = ?1", [fixture.session_id]).unwrap();
                    }
                    "flow_replaced" => {
                        db.execute("UPDATE pending_channel_bind_sessions SET install_device_code = 'fixture-replaced-code' WHERE id = ?1", [fixture.session_id]).unwrap();
                    }
                    "owner_changed" => {
                        db.execute("UPDATE pending_channel_bind_sessions SET user_key = 'fixture-channel-user' WHERE id = ?1", [fixture.session_id]).unwrap();
                    }
                    _ => unreachable!(),
                }
            }
            fixture.finish_poll();
            let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(30), poll)
                .await
                .unwrap()
                .unwrap();
            let (expected_status, expected_error, expected_state) = match mutation {
                "reset" | "deleted" | "owner_changed" => (
                    StatusCode::NOT_FOUND,
                    Some("channel_bind_session_not_found"),
                    None,
                ),
                "revoked" | "key_deleted" => (
                    StatusCode::UNAUTHORIZED,
                    Some("channel_bind_owner_unavailable"),
                    None,
                ),
                "role_changed" => (
                    StatusCode::FORBIDDEN,
                    Some("channel_bind_admin_required"),
                    None,
                ),
                "expired" => (StatusCode::OK, None, Some("expired")),
                "bound" => (StatusCode::OK, None, Some("bound")),
                "flow_replaced" => (StatusCode::OK, None, Some("pending")),
                _ => unreachable!(),
            };
            assert_eq!(status, expected_status, "{}:{mutation}", platform.channel());
            assert_eq!(body.error.as_deref(), expected_error);
            assert_eq!(body.ok, expected_status == StatusCode::OK);
            if let Some(expected) = expected_state {
                let current = body.data.unwrap();
                assert_eq!(current.status, expected);
                assert_eq!(current.session_id, fixture.session_id);
                assert_eq!(current.channel, platform.channel());
            } else {
                assert!(body.data.is_none());
            }
            let unchanged = fixture.snapshot() == before;
            let service_launch_attempt = fixture
                .root
                .join("logs")
                .join(format!("{}.log", platform.service()))
                .exists();
            if !unchanged || service_launch_attempt {
                failures.push(serde_json::json!({"platform":platform.channel(),"mutation":mutation,
                    "configuration_unchanged":unchanged,"service_launch_attempt":service_launch_attempt,
                    "status":status.as_u16(),"registration_requests":fixture.requests()}));
            }
            eprintln!(
                "registration_case={}",
                serde_json::json!({"platform":platform.channel(),"mutation":mutation,
                "status":status.as_u16(),"error_code":expected_error,"state":expected_state,
                "configuration_unchanged":unchanged,"service_launch_attempt":service_launch_attempt,"requests":fixture.requests()})
            );
            assert_eq!(fixture.requests(), 1);
        }
    }
    assert!(
        failures.is_empty(),
        "late_registration_effects={failures:?}"
    );
}

#[tokio::test]
async fn channel_registration_upstream_expiry_cannot_replace_an_already_bound_session() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture =
            Fixture::with_result(platform, serde_json::json!({"error":"expired_token"})).await;
        let before = fixture.snapshot();
        let poll = fixture.poll();
        fixture.wait_for_poll().await;
        {
            let db = fixture.state.core.db.get().unwrap();
            db.execute("UPDATE pending_channel_bind_sessions SET status = 'bound', external_user_id = 'fixture-user', external_chat_id = 'fixture-chat' WHERE id = ?1", [fixture.session_id]).unwrap();
        }
        fixture.finish_poll();
        let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(30), poll)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(body.ok);
        assert_eq!(body.data.unwrap().status, "bound");
        assert_eq!(fixture.requests(), 1);
        assert_eq!(fixture.snapshot(), before);
    }
}
