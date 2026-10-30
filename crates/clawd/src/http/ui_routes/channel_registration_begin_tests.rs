use super::support::{Fixture, ADMIN_KEY};
use super::*;

fn begin_result() -> serde_json::Value {
    serde_json::json!({"device_code":"fixture-begin-code",
        "verification_uri_complete":"https://registration.example.test/new",
        "interval":5,"expire_in":600})
}

#[tokio::test]
async fn channel_registration_begin_control_attaches_only_the_new_session() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::for_begin(platform, begin_result()).await;
        let before = fixture.snapshot();
        let begin = fixture.begin();
        fixture.wait_for_response_gate().await;
        let created = fixture.latest_session();
        assert_ne!(created.id, fixture.session_id);
        assert!(created.install_device_code.is_none());
        fixture.release_response();
        let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(30), begin)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(body.ok);
        let response = body.data.unwrap();
        assert_eq!(response.session_id, created.id);
        assert_eq!(response.channel, platform.channel());
        assert_eq!(response.status, "pending");
        assert_eq!(
            response.entry_url.as_deref(),
            Some("https://registration.example.test/new")
        );
        assert_eq!(response.poll_interval_seconds, Some(5));
        assert_eq!(
            fixture.latest_session().install_device_code.as_deref(),
            Some("fixture-begin-code")
        );
        let db = fixture.state.core.db.get().unwrap();
        let prior = get_pending_channel_bind_session_by_id(&db, fixture.session_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            prior.install_device_code.as_deref(),
            Some("fixture-device-code")
        );
        assert_eq!(fixture.requests(), 2);
        assert_eq!(fixture.snapshot(), before);
    }
}

#[tokio::test]
async fn channel_registration_begin_late_results_preserve_current_authority_and_session() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut failures = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for (mutation, sql, expected_status, expected_error, expected_state) in [
            ("revoked", "UPDATE auth_keys SET enabled = 0 WHERE user_key = (SELECT user_key FROM pending_channel_bind_sessions WHERE id = ?1)", StatusCode::UNAUTHORIZED, Some("channel_bind_owner_unavailable"), None),
            ("key_deleted", "DELETE FROM auth_keys WHERE user_key = (SELECT user_key FROM pending_channel_bind_sessions WHERE id = ?1)", StatusCode::UNAUTHORIZED, Some("channel_bind_owner_unavailable"), None),
            ("role_changed", "UPDATE auth_keys SET role = 'user' WHERE user_key = (SELECT user_key FROM pending_channel_bind_sessions WHERE id = ?1)", StatusCode::FORBIDDEN, Some("channel_bind_admin_required"), None),
            ("deleted", "DELETE FROM pending_channel_bind_sessions WHERE id = ?1", StatusCode::NOT_FOUND, Some("channel_bind_session_not_found"), None),
            ("owner_changed", "UPDATE pending_channel_bind_sessions SET user_key = 'fixture-channel-user' WHERE id = ?1", StatusCode::NOT_FOUND, Some("channel_bind_session_not_found"), None),
            ("expired", "UPDATE pending_channel_bind_sessions SET expires_at = '1' WHERE id = ?1", StatusCode::OK, None, Some("expired")),
            ("bound", "UPDATE pending_channel_bind_sessions SET status = 'bound', external_user_id = 'fixture-user', external_chat_id = 'fixture-chat' WHERE id = ?1", StatusCode::OK, None, Some("bound")),
            ("flow_replaced", "UPDATE pending_channel_bind_sessions SET install_device_code = 'fixture-replaced-code', install_verification_url = 'https://registration.example.test/replaced' WHERE id = ?1", StatusCode::OK, None, Some("pending")),
            ("reset", "", StatusCode::NOT_FOUND, Some("channel_bind_session_not_found"), None),
        ] {
            let fixture = Fixture::for_begin(platform, begin_result()).await;
            let before = fixture.snapshot();
            let begin = fixture.begin();
            fixture.wait_for_response_gate().await;
            let id = fixture.latest_session().id;
            assert_ne!(id, fixture.session_id);
            if mutation == "reset" {
                reset_channel_binding_state_for_user_key(&fixture.state, platform.channel(), ADMIN_KEY).unwrap();
            } else {
                let db = fixture.state.core.db.get().unwrap();
                assert!(db.execute(sql, [id]).unwrap() > 0);
            }
            fixture.release_response();
            let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(30), begin).await.unwrap().unwrap();
            let db = fixture.state.core.db.get().unwrap();
            let stored = get_pending_channel_bind_session_by_id(&db, id).unwrap();
            let code_unchanged = stored.as_ref().is_none_or(|session| {
                session.install_device_code.as_deref() == if mutation == "flow_replaced" { Some("fixture-replaced-code") } else { None }
            });
            let state_matches = body.data.as_ref().map(|session| session.status.as_str()) == expected_state;
            let safe = status == expected_status && body.error.as_deref() == expected_error
                && body.ok == expected_status.is_success() && state_matches && code_unchanged;
            let detail = serde_json::json!({"platform":platform.channel(),"mutation":mutation,
                "status":status.as_u16(),"expected_status":expected_status.as_u16(),
                "state_matches":state_matches,"registration_unchanged":code_unchanged,"accepted":body.ok,
                "machine_error_matches":body.error.as_deref()==expected_error});
            eprintln!("registration_begin_case={detail}");
            if !safe { failures.push(detail); }
            assert_eq!(fixture.requests(), 2);
            assert_eq!(fixture.snapshot(), before);
            assert!(!fixture.root.join("logs").exists());
        }
    }
    assert!(failures.is_empty(), "late_begin_results={failures:?}");
}

#[tokio::test]
async fn channel_registration_begin_old_upstream_error_cannot_replace_bound_evidence() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::for_begin(platform, serde_json::json!({"device_code":""})).await;
        let before = fixture.snapshot();
        let begin = fixture.begin();
        fixture.wait_for_response_gate().await;
        let id = fixture.latest_session().id;
        {
            let db = fixture.state.core.db.get().unwrap();
            db.execute("UPDATE pending_channel_bind_sessions SET status = 'bound', external_user_id = 'fixture-user', external_chat_id = 'fixture-chat' WHERE id = ?1", [id]).unwrap();
        }
        fixture.release_response();
        let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(30), begin)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(body.ok);
        assert!(body.error.is_none());
        assert_eq!(body.data.unwrap().status, "bound");
        assert_eq!(fixture.latest_session().status, "bound");
        assert_eq!(fixture.requests(), 2);
        assert_eq!(fixture.snapshot(), before);
    }
}
