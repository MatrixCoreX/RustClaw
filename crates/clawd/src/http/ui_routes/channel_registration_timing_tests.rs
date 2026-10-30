use super::{support::Fixture, *};

fn begin_payload(interval: Value, expiry: u64) -> Value {
    json!({
        "device_code":"fixture-timing-device",
        "verification_uri_complete":"https://registration.example.test/timing",
        "interval":interval,"expire_in":expiry,
    })
}

async fn finish_begin(
    fixture: &Fixture,
) -> (StatusCode, ApiResponse<FeishuBindSessionStatusResponse>) {
    let begin = fixture.begin();
    fixture.wait_for_response_gate().await;
    fixture.release_response();
    let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(15), begin)
        .await
        .unwrap()
        .unwrap();
    (status, body)
}

#[tokio::test]
async fn channel_registration_timing_rejects_unrepresentable_poll_intervals() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut failures = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for interval in [
            json!(2_147_484_u64),
            json!(i64::MAX as u64),
            json!(i64::MAX as u64 + 1),
            json!(u64::MAX),
            json!(-1),
            json!(1.5),
            json!("5"),
        ] {
            let fixture = Fixture::for_begin(platform, begin_payload(interval.clone(), 600)).await;
            let before = fixture.snapshot();
            let (status, body) = finish_begin(&fixture).await;
            let stored = fixture.latest_session();
            if status != StatusCode::BAD_GATEWAY
                || body.ok
                || body.data.is_some()
                || body.error.as_deref() != Some("channel_bind_registration_response_invalid")
                || stored.install_device_code.is_some()
                || stored.install_poll_interval_seconds.is_some()
            {
                failures.push(json!({"platform":platform.channel(),"interval":interval,
                    "http_status":status.as_u16(),"stored_interval":stored.install_poll_interval_seconds}));
            }
            assert_eq!(fixture.requests(), 2);
            assert_eq!(fixture.snapshot(), before);
            assert!(!fixture.root.join("logs").exists());
        }
    }
    assert!(failures.is_empty(), "unsafe_intervals={failures:?}");
}

#[tokio::test]
async fn channel_registration_timing_preserves_valid_provider_spacing_and_defaults() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for (interval, expected) in [
            (Value::Null, 5_u64),
            (json!(0), 1),
            (json!(1), 1),
            (json!(1801), 1801),
            (json!(2_147_483), 2_147_483),
        ] {
            let mut fixture = Fixture::for_begin(platform, begin_payload(interval, 600)).await;
            let before = fixture.snapshot();
            let (status, body) = finish_begin(&fixture).await;
            assert_eq!(status, StatusCode::OK);
            let public = body.data.unwrap();
            assert_eq!(public.status, "pending");
            assert_eq!(public.poll_interval_seconds, Some(expected));
            fixture.reopen_database();
            let stored = fixture.latest_session();
            assert_eq!(stored.install_poll_interval_seconds, Some(expected as i64));
            assert_eq!(
                stored.install_device_code.as_deref(),
                Some("fixture-timing-device")
            );
            assert_eq!(fixture.requests(), 2);
            assert_eq!(fixture.snapshot(), before);
        }
    }
}

#[tokio::test]
async fn channel_registration_timing_begin_never_restarts_the_host_session_lifetime() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::for_begin(platform, begin_payload(json!(5), u64::MAX)).await;
        let before = fixture.snapshot();
        let begin = fixture.begin();
        fixture.wait_for_response_gate().await;
        let id = fixture.latest_session().id;
        let host_deadline = current_unix_ts() + 120;
        {
            let db = fixture.state.core.db.get().unwrap();
            assert_eq!(
                db.execute(
                    "UPDATE pending_channel_bind_sessions SET expires_at=?1 WHERE id=?2",
                    rusqlite::params![host_deadline.to_string(), id],
                )
                .unwrap(),
                1
            );
        }
        fixture.release_response();
        let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(15), begin)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, StatusCode::OK);
        let session = body.data.unwrap();
        assert_eq!(session.session_id, id);
        assert!(session.expires_at.parse::<i64>().unwrap() <= host_deadline);
        assert!(fixture.latest_session().expires_at.parse::<i64>().unwrap() <= host_deadline);
        assert_eq!(fixture.requests(), 2);
        assert_eq!(fixture.snapshot(), before);
    }
}

#[tokio::test]
async fn channel_registration_timing_slow_down_is_cumulative_and_survives_reopen() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for http_status in [StatusCode::OK, StatusCode::BAD_REQUEST] {
            let mut fixture =
                Fixture::with_status_result(platform, http_status, json!({"error":"slow_down"}))
                    .await;
            let before = fixture.snapshot();
            let expiry = fixture.latest_session().expires_at;
            for (index, expected) in [6, 11, 16].into_iter().enumerate() {
                let poll = fixture.poll();
                fixture.wait_for_poll().await;
                fixture.finish_poll();
                let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(15), poll)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(status, StatusCode::OK);
                let session = body.data.unwrap();
                assert_eq!(session.status, "pending");
                assert_eq!(session.poll_interval_seconds, Some(expected));
                assert!(session.error_text.is_none());
                fixture.reopen_database();
                let stored = fixture.latest_session();
                assert_eq!(stored.install_poll_interval_seconds, Some(expected as i64));
                assert_eq!(stored.expires_at, expiry);
                assert_eq!(fixture.requests(), index + 1);
                assert_eq!(fixture.snapshot(), before);
                assert!(!fixture.root.join("logs").exists());
            }
        }
    }
}

#[tokio::test]
async fn channel_registration_timing_late_slow_down_cannot_change_a_new_attempt() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for mutation in ["replacement", "bound", "expired"] {
            let fixture = Fixture::with_result(platform, json!({"error":"slow_down"})).await;
            let before = fixture.snapshot();
            let poll = fixture.poll();
            fixture.wait_for_poll().await;
            {
                let db = fixture.state.core.db.get().unwrap();
                let sql = match mutation {
                    "replacement" => "UPDATE pending_channel_bind_sessions SET install_device_code='fixture-new-attempt' WHERE id=?1",
                    "bound" => "UPDATE pending_channel_bind_sessions SET status='bound' WHERE id=?1",
                    _ => "UPDATE pending_channel_bind_sessions SET expires_at='1' WHERE id=?1",
                };
                assert_eq!(db.execute(sql, [fixture.session_id]).unwrap(), 1);
            }
            fixture.finish_poll();
            let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(15), poll)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(status, StatusCode::OK);
            assert!(body.ok);
            assert_eq!(
                fixture.latest_session().install_poll_interval_seconds,
                Some(1)
            );
            assert_eq!(fixture.requests(), 1);
            assert_eq!(fixture.snapshot(), before);
        }
    }
}

#[tokio::test]
async fn channel_registration_timing_overflowing_slow_down_is_a_public_machine_failure() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::with_result(platform, json!({"error":"slow_down"})).await;
        let before = fixture.snapshot();
        {
            let db = fixture.state.core.db.get().unwrap();
            db.execute(
                "UPDATE pending_channel_bind_sessions SET install_poll_interval_seconds=?1 WHERE id=?2",
                rusqlite::params![2_147_483_i64, fixture.session_id],
            ).unwrap();
        }
        let poll = fixture.poll();
        fixture.wait_for_poll().await;
        fixture.finish_poll();
        let (status, Json(body)) = tokio::time::timeout(Duration::from_secs(15), poll)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(!body.ok && body.data.is_none());
        assert_eq!(
            body.error.as_deref(),
            Some("channel_registration_poll_interval_invalid")
        );
        let stored = fixture.latest_session();
        assert_eq!(stored.install_poll_interval_seconds, Some(2_147_483));
        assert_eq!(stored.status, "pending");
        assert!(stored.error_text.is_none());
        assert_eq!(fixture.requests(), 1);
        assert_eq!(fixture.snapshot(), before);
    }
}

#[tokio::test]
async fn channel_registration_timing_provider_expiry_does_not_allow_an_upstream_poll() {
    let _lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::for_begin(platform, begin_payload(json!(5), 0)).await;
        let before = fixture.snapshot();
        let (status, body) = finish_begin(&fixture).await;
        assert_eq!(status, StatusCode::OK);
        let id = body.data.unwrap().session_id;
        let (status, Json(body)) = get_channel_bind_session(
            fixture.state.clone(),
            Fixture::headers(support::ADMIN_KEY),
            id,
            platform,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.data.unwrap().status, "expired");
        assert_eq!(fixture.latest_session().status, "expired");
        assert_eq!(fixture.requests(), 2);
        assert_eq!(fixture.snapshot(), before);
        assert!(!fixture.root.join("logs").exists());
    }
}
