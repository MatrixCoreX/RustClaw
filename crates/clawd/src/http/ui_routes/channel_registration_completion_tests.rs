use super::support::Fixture;
use super::*;

fn apply_during_start(fixture: &Fixture, change: &str) {
    if change == "reset" {
        reset_channel_binding_state_for_user_key(
            &fixture.state,
            fixture.platform.channel(),
            ADMIN_KEY,
        )
        .unwrap();
        return;
    }
    let mut db = fixture.state.core.db.get().unwrap();
    let id = fixture.session_id;
    match change {
        "pending" => {}
        "detected" | "bound" => {
            mark_pending_channel_bind_session_detected(&mut db, id, "fixture-user", "fixture-chat")
                .unwrap();
            if change == "bound" {
                finalize_pending_channel_bind_session(&mut db, id).unwrap();
            }
        }
        "failed" => {
            mark_pending_channel_bind_session_failed(&mut db, id, "fixture_explicit_failure")
                .unwrap();
        }
        "expired" => {
            db.execute(
                "UPDATE pending_channel_bind_sessions SET expires_at = '1' WHERE id = ?1",
                [id],
            )
            .unwrap();
        }
        "flow_replaced" => {
            db.execute(
                "UPDATE pending_channel_bind_sessions SET install_device_code = 'fixture-replaced-code' WHERE id = ?1",
                [id],
            )
            .unwrap();
        }
        "revoked" => {
            db.execute(
                "UPDATE auth_keys SET enabled = 0 WHERE user_key = ?1",
                [ADMIN_KEY],
            )
            .unwrap();
        }
        _ => unreachable!(),
    }
}

async fn finish_with(
    fixture: &Fixture,
    change: &str,
    fail_start: bool,
) -> anyhow::Result<PendingChannelBindSession> {
    let session = fixture.latest_session();
    fixture.release_response();
    maybe_complete_official_scan_with_start(&fixture.state, fixture.platform, session, async {
        apply_during_start(fixture, change);
        if fail_start {
            anyhow::bail!("fixture_service_start_failed");
        }
        Ok(())
    })
    .await
}

#[tokio::test]
async fn channel_registration_completion_preserves_normal_pending_and_failed_start() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for fail_start in [false, true] {
            let fixture = Fixture::new(platform, false).await;
            let result = finish_with(&fixture, "pending", fail_start).await.unwrap();
            assert_eq!(result.status, if fail_start { "failed" } else { "pending" });
            assert_eq!(fixture.latest_session().status, result.status);
            assert_eq!(fixture.requests(), 1);
            assert!(!fixture.root.join("logs").exists());
        }
    }
}

#[tokio::test]
async fn channel_registration_completion_returns_finalized_binding_after_either_start_result() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut failures = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for fail_start in [false, true] {
            let fixture = Fixture::new(platform, false).await;
            let result = finish_with(&fixture, "bound", fail_start).await;
            let bound = result.as_ref().ok().is_some_and(|session| {
                session.status == "bound"
                    && session.external_user_id.as_deref() == Some("fixture-user")
                    && session.external_chat_id.as_deref() == Some("fixture-chat")
                    && session.error_text.is_none()
            });
            assert_eq!(fixture.latest_session().status, "bound");
            assert_eq!(fixture.requests(), 1);
            if !bound {
                failures.push(json!({"platform":platform.channel(),"startup_failed":fail_start,"returned_error":result.is_err()}));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "registration_completed_binding={failures:?}"
    );
}

#[tokio::test]
async fn channel_registration_completion_preserves_current_session_transitions() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut failures = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        for change in [
            "detected",
            "failed",
            "expired",
            "flow_replaced",
            "reset",
            "revoked",
        ] {
            for fail_start in [false, true] {
                let fixture = Fixture::new(platform, false).await;
                let result = finish_with(&fixture, change, fail_start).await;
                let matched = match change {
                    "reset" => result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_ref::<ChannelRegistrationRejection>())
                        .is_some_and(|err| {
                            matches!(err, ChannelRegistrationRejection::SessionNotFound)
                        }),
                    "revoked" => result
                        .as_ref()
                        .err()
                        .and_then(|err| err.downcast_ref::<ChannelRegistrationRejection>())
                        .is_some_and(|err| {
                            matches!(err, ChannelRegistrationRejection::OwnerUnavailable)
                        }),
                    "failed" => result.as_ref().ok().is_some_and(|session| {
                        session.status == "failed"
                            && session.error_text.as_deref() == Some("fixture_explicit_failure")
                    }),
                    "expired" => result
                        .as_ref()
                        .ok()
                        .is_some_and(|session| session.status == "expired"),
                    "flow_replaced" => result.as_ref().ok().is_some_and(|session| {
                        session.status == "pending"
                            && session.install_device_code.as_deref()
                                == Some("fixture-replaced-code")
                    }),
                    "detected" => result.as_ref().ok().is_some_and(|session| {
                        session.status == if fail_start { "failed" } else { "detected" }
                    }),
                    _ => unreachable!(),
                };
                if !matched {
                    failures.push(json!({"platform":platform.channel(),"change":change,"startup_failed":fail_start,"returned_status":result.as_ref().ok().map(|session|session.status.as_str()),"returned_error":result.is_err()}));
                }
                assert_eq!(fixture.requests(), 1);
                assert!(!fixture.root.join("logs").exists());
            }
        }
    }
    assert!(
        failures.is_empty(),
        "registration_completion_state={failures:?}"
    );
}
