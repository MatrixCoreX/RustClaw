use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use super::support::Fixture;
use super::*;

async fn startup_request(counter: Arc<AtomicUsize>) -> anyhow::Result<()> {
    counter.fetch_add(1, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(25)).await;
    Ok(())
}

async fn controlled_poll(
    fixture: &Fixture,
    starts: Arc<AtomicUsize>,
) -> anyhow::Result<PendingChannelBindSession> {
    let session = fixture.latest_session();
    fixture.release_response();
    maybe_complete_official_scan_with_start(
        &fixture.state,
        fixture.platform,
        session,
        startup_request(starts),
    )
    .await
}

fn assert_credentials_are_enabled(fixture: &Fixture) {
    for raw in fixture.snapshot() {
        let value: toml::Value = toml::from_str(std::str::from_utf8(&raw).unwrap()).unwrap();
        let channel = &value[fixture.platform.channel()];
        assert_eq!(channel["enabled"].as_bool(), Some(true));
        assert_eq!(channel["app_id"].as_str(), Some("fixture-app-id"));
        assert_eq!(channel["app_secret"].as_str(), Some("fixture-app-secret"));
    }
    assert!(!fixture.root.join("logs").exists());
}

#[tokio::test]
async fn channel_registration_application_control_writes_credentials_without_real_launch() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::new(platform, false).await;
        let starts = Arc::new(AtomicUsize::new(0));
        let session = controlled_poll(&fixture, starts.clone()).await.unwrap();
        assert_eq!(session.status, "pending");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.requests(), 1);
        assert_credentials_are_enabled(&fixture);
    }
}

#[tokio::test]
async fn channel_registration_repeated_application_cannot_undo_explicit_stop() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut failures = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::new(platform, false).await;
        let starts = Arc::new(AtomicUsize::new(0));
        controlled_poll(&fixture, starts.clone()).await.unwrap();
        persist_channel_service_enabled(&fixture.state, platform.service(), false).unwrap();
        let disabled = fixture.snapshot();
        let session = controlled_poll(&fixture, starts.clone()).await.unwrap();
        let preserved = fixture.snapshot() == disabled;
        let detail = serde_json::json!({"platform":platform.channel(),
            "explicit_stop_preserved":preserved,"startup_requests":starts.load(Ordering::SeqCst),
            "upstream_requests":fixture.requests(),"status":session.status});
        eprintln!("registration_apply_repeat={detail}");
        if !preserved || starts.load(Ordering::SeqCst) != 1 || fixture.requests() != 1 {
            failures.push(detail);
        }
    }
    assert!(
        failures.is_empty(),
        "repeated_registration_effects={failures:?}"
    );
}

#[tokio::test]
async fn channel_registration_concurrent_responses_apply_only_once() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut failures = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::new(platform, false).await;
        let starts = Arc::new(AtomicUsize::new(0));
        let session = fixture.latest_session();
        let release_both = async {
            tokio::time::timeout(Duration::from_secs(15), async {
                while fixture.requests() < 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            fixture.release_response();
            fixture.release_response();
        };
        let (first, (), second) = tokio::join!(
            maybe_complete_official_scan_with_start(
                &fixture.state,
                platform,
                session.clone(),
                startup_request(starts.clone())
            ),
            release_both,
            maybe_complete_official_scan_with_start(
                &fixture.state,
                platform,
                session,
                startup_request(starts.clone())
            ),
        );
        assert_eq!(first.unwrap().status, "pending");
        assert_eq!(second.unwrap().status, "pending");
        assert_credentials_are_enabled(&fixture);
        let detail = serde_json::json!({"platform":platform.channel(),
            "startup_requests":starts.load(Ordering::SeqCst),"upstream_requests":fixture.requests()});
        eprintln!("registration_apply_concurrent={detail}");
        if starts.load(Ordering::SeqCst) != 1 || fixture.requests() != 2 {
            failures.push(detail);
        }
    }
    assert!(
        failures.is_empty(),
        "concurrent_registration_effects={failures:?}"
    );
}

#[tokio::test]
async fn channel_registration_partial_config_failure_is_not_automatically_replayed() {
    let _env_lock = crate::fixture_replay_e2e::fixture_env_lock();
    let mut failures = Vec::new();
    for platform in [AgentAppChannel::Feishu, AgentAppChannel::Lark] {
        let fixture = Fixture::new(platform, false).await;
        let starts = Arc::new(AtomicUsize::new(0));
        let mounted = fixture
            .root
            .join("docker/config/channels")
            .join(format!("{}.toml", platform.channel()));
        let original = std::fs::read(&mounted).unwrap();
        fixture.release_response();
        assert!(maybe_complete_official_scan_with_effects(
            &fixture.state,
            platform,
            fixture.latest_session(),
            startup_request(starts.clone()),
            |state, platform, client_id, client_secret| {
                std::fs::remove_file(&mounted)?;
                std::fs::create_dir(&mounted)?;
                write_generated_credentials(state, platform, client_id, client_secret)
            },
        )
        .await
        .is_err());
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        let active: toml::Value = toml::from_str(
            &std::fs::read_to_string(
                fixture
                    .root
                    .join("configs/channels")
                    .join(format!("{}.toml", platform.channel())),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            active[platform.channel()]["app_id"].as_str(),
            Some("fixture-app-id")
        );
        let ready = match platform {
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
        };
        assert!(!ready);
        std::fs::remove_dir(&mounted).unwrap();
        std::fs::write(&mounted, original).unwrap();
        let _ = controlled_poll(&fixture, starts.clone()).await;
        let detail = serde_json::json!({"platform":platform.channel(),
            "startup_requests":starts.load(Ordering::SeqCst),"upstream_requests":fixture.requests(),
            "stored_status":fixture.latest_session().status});
        eprintln!("registration_apply_partial={detail}");
        if starts.load(Ordering::SeqCst) != 0 || fixture.requests() != 1 {
            failures.push(detail);
        }
    }
    assert!(
        failures.is_empty(),
        "uncertain_registration_replayed={failures:?}"
    );
}
