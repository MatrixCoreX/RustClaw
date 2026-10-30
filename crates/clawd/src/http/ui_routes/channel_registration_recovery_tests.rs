use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use super::{support::Fixture, *};

fn outcome<T>(
    (status, Json(body)): (StatusCode, Json<ApiResponse<T>>),
) -> (StatusCode, Option<String>) {
    (status, body.error)
}

async fn update_credentials(
    fixture: &Fixture,
    app_id: &str,
    app_secret: &str,
) -> (StatusCode, Option<String>) {
    let state = State(fixture.state.clone());
    let headers = Fixture::headers(ADMIN_KEY);
    match fixture.platform {
        AgentAppChannel::Feishu => outcome(
            update_feishu_config(
                state,
                headers,
                Json(UpdateFeishuConfigRequest {
                    app_id: app_id.into(),
                    app_secret: app_secret.into(),
                }),
            )
            .await,
        ),
        AgentAppChannel::Lark => outcome(
            update_lark_config(
                state,
                headers,
                Json(UpdateLarkConfigRequest {
                    app_id: app_id.into(),
                    app_secret: app_secret.into(),
                }),
            )
            .await,
        ),
    }
}

fn ready(fixture: &Fixture) -> bool {
    match fixture.platform {
        AgentAppChannel::Feishu => {
            load_feishu_config_response(&fixture.state, None)
                .unwrap()
                .bind_ready
        }
        AgentAppChannel::Lark => {
            load_lark_config_response(&fixture.state, None)
                .unwrap()
                .bind_ready
        }
    }
}

#[tokio::test]
async fn channel_registration_cancelled_start_survives_reopen_and_requires_explicit_recovery() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let mut fixture = Fixture::new(platform, false).await;
        let started = Arc::new(tokio::sync::Notify::new());
        let state = fixture.state.clone();
        let session = fixture.latest_session();
        let signal = started.clone();
        fixture.release_response();
        let operation = tokio::spawn(async move {
            maybe_complete_official_scan_with_start(&state, platform, session, async move {
                signal.notify_one();
                std::future::pending::<anyhow::Result<()>>().await
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(15), started.notified())
            .await
            .unwrap();
        let written = fixture.snapshot();
        operation.abort();
        assert!(operation.await.unwrap_err().is_cancelled());
        fixture.reopen_database();
        let starts = Arc::new(AtomicUsize::new(0));
        let count = starts.clone();
        let error = maybe_complete_official_scan_with_start(
            &fixture.state,
            platform,
            fixture.latest_session(),
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ChannelRegistrationRejection>(),
            Some(ChannelRegistrationRejection::ApplicationRecoveryRequired)
        ));
        assert_eq!(fixture.requests(), 1);
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.snapshot(), written);
        assert!(
            !ready(&fixture),
            "uncertain_application_must_not_appear_ready"
        );
        assert_eq!(
            update_credentials(&fixture, "fixture-manual-app", "fixture-manual-secret").await,
            (StatusCode::OK, None)
        );
        assert!(ready(&fixture));
        assert_eq!(fixture.requests(), 1);
        assert!(!fixture.root.join("logs").exists());
    }
}

#[tokio::test]
async fn channel_registration_lost_ack_reopens_without_replay_and_preserves_bot_handshake() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let mut fixture = Fixture::new(platform, false).await;
        fixture.release_response();
        let starts = AtomicUsize::new(0);
        let acknowledged = maybe_complete_official_scan_with_start(
            &fixture.state,
            platform,
            fixture.latest_session(),
            async {
                starts.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(acknowledged.status, "pending");
        drop(acknowledged);
        fixture.reopen_database();
        let pending = maybe_complete_official_scan_with_start(
            &fixture.state,
            platform,
            fixture.latest_session(),
            async {
                starts.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(pending.status, "pending");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert!(ready(&fixture));
        {
            let mut db = fixture.state.core.db.get().unwrap();
            mark_pending_channel_bind_session_detected(
                &mut db,
                pending.id,
                "fixture-bot-user",
                "fixture-bot-chat",
            )
            .unwrap();
            finalize_pending_channel_bind_session(&mut db, pending.id).unwrap();
        }
        let (status, Json(body)) = get_channel_bind_session(
            fixture.state.clone(),
            Fixture::headers(ADMIN_KEY),
            pending.id,
            platform,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.data.unwrap().status, "bound");
        assert_eq!(fixture.requests(), 1);
        assert!(!fixture.root.join("logs").exists());
    }
}

#[tokio::test]
async fn channel_registration_held_poll_cannot_overwrite_manual_configuration() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::new(platform, false).await;
        let state = fixture.state.clone();
        let session = fixture.latest_session();
        let starts = Arc::new(AtomicUsize::new(0));
        let count = starts.clone();
        let operation = tokio::spawn(async move {
            maybe_complete_official_scan_with_start(&state, platform, session, async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
        });
        fixture.wait_for_poll().await;
        assert_eq!(
            update_credentials(&fixture, "fixture-manual-app", "fixture-manual-secret").await,
            (StatusCode::OK, None)
        );
        let manual = fixture.snapshot();
        fixture.release_response();
        let error = operation.await.unwrap().unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ChannelRegistrationRejection>(),
            Some(ChannelRegistrationRejection::RegistrationSuperseded)
        ));
        assert_eq!(fixture.snapshot(), manual);
        assert_eq!(fixture.requests(), 1);
        assert_eq!(starts.load(Ordering::SeqCst), 0);
    }
}

async fn mutation_request(fixture: &Fixture, operation: &str) -> (StatusCode, Option<String>) {
    match operation {
        "update" => {
            update_credentials(fixture, "fixture-queued-app", "fixture-queued-secret").await
        }
        "begin" => outcome(
            start_channel_bind_session(
                fixture.state.clone(),
                Fixture::headers(ADMIN_KEY),
                StartFeishuBindSessionRequest {
                    expires_in_seconds: Some(600),
                },
                fixture.platform,
            )
            .await,
        ),
        "dedicated_reset" => match fixture.platform {
            AgentAppChannel::Feishu => outcome(
                reset_feishu_config_handler(
                    State(fixture.state.clone()),
                    Fixture::headers(ADMIN_KEY),
                )
                .await,
            ),
            AgentAppChannel::Lark => outcome(
                reset_lark_config_handler(
                    State(fixture.state.clone()),
                    Fixture::headers(ADMIN_KEY),
                )
                .await,
            ),
        },
        action => outcome(
            control_service(
                State(fixture.state.clone()),
                Fixture::headers(ADMIN_KEY),
                AxumPath((fixture.platform.service().into(), action.into())),
            )
            .await,
        ),
    }
}

#[tokio::test]
async fn channel_registration_actual_mutations_wait_and_recheck_actor_before_effects() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for operation in [
            "update",
            "begin",
            "dedicated_reset",
            "start",
            "stop",
            "restart",
            "reset",
        ] {
            let fixture = Fixture::new(platform, true).await;
            let before = fixture.snapshot();
            let held = lock_channel_configuration(&fixture.state, platform)
                .await
                .unwrap();
            let mut request = Box::pin(mutation_request(&fixture, operation));
            assert!(
                futures_util::poll!(request.as_mut()).is_pending(),
                "{operation}"
            );
            {
                let db = fixture.state.core.db.get().unwrap();
                db.execute(
                    "UPDATE auth_keys SET enabled = 0 WHERE user_key = ?1",
                    [ADMIN_KEY],
                )
                .unwrap();
            }
            drop(held);
            let (status, error) = request.await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{}:{operation}",
                platform.channel()
            );
            assert_eq!(error.as_deref(), Some("channel_bind_owner_unavailable"));
            assert_eq!(fixture.snapshot(), before);
            assert_eq!(fixture.requests(), 0);
            assert_eq!(fixture.latest_session().id, fixture.session_id);
            assert!(!fixture.root.join("logs").exists());
            let db = fixture.state.core.db.get().unwrap();
            let revision: i64 = db
                .query_row(
                    "SELECT revision FROM channel_configuration_revisions WHERE channel = ?1",
                    [platform.channel()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(revision, 1);
        }
    }
}

#[tokio::test]
async fn channel_registration_begin_attachment_is_atomic_and_explicit_restart_recovers() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut failures = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let mut fixture = Fixture::for_begin(
            platform,
            json!({
                "device_code": "fixture-begin-code",
                "verification_uri_complete": "https://registration.example.test/new",
                "interval": 1, "expire_in": 600,
            }),
        )
        .await;
        let before = fixture.snapshot();
        {
            let db = fixture.state.core.db.get().unwrap();
            db.execute_batch(
                "CREATE TRIGGER reject_install_attachment
                BEFORE UPDATE OF install_device_code ON pending_channel_bind_sessions
                BEGIN SELECT RAISE(ABORT, 'fixture_attach_rejected'); END;",
            )
            .unwrap();
        }
        let request = fixture.begin();
        fixture.wait_for_response_gate().await;
        fixture.release_response();
        let (status, Json(body)) = request.await.unwrap();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!body.ok);
        assert!(body.data.is_none());
        fixture.reopen_database();
        let session = fixture.latest_session();
        assert!(session.install_device_code.is_none());
        {
            let db = fixture.state.core.db.get().unwrap();
            let (phase, device): (String, Option<String>) = db.query_row(
                "SELECT phase, device_digest FROM channel_registration_applications WHERE session_id = ?1",
                [session.id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(phase, "prepared");
            if device.is_some() {
                failures.push(platform.channel());
            }
            db.execute_batch("DROP TRIGGER reject_install_attachment")
                .unwrap();
        }
        let replacement = fixture.begin();
        fixture.wait_for_response_gate().await;
        fixture.release_response();
        let (status, Json(body)) = replacement.await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(body.ok);
        assert!(body.data.unwrap().session_id > session.id);
        let fresh = fixture.latest_session();
        assert_eq!(
            fresh.install_device_code.as_deref(),
            Some("fixture-begin-code")
        );
        let db = fixture.state.core.db.get().unwrap();
        assert!(matches!(
            crate::repo::channel_registration::registration_gate(&db, &fresh).unwrap(),
            crate::repo::channel_registration::RegistrationGate::Ready { .. }
        ));
        assert_eq!(fixture.requests(), 4);
        assert_eq!(fixture.snapshot(), before);
        assert!(!fixture.root.join("logs").exists());
    }
    assert!(
        failures.is_empty(),
        "registration_begin_partial_attachment={failures:?}"
    );
}
