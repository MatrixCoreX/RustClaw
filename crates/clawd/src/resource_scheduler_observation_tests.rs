use super::*;
use crate::local_process_job::observation_test_support::LiveJobFixture;
use crate::local_process_job::{process_identity_state, ProcessIdentityState};

fn acquire_lease(broker: &ResourceBroker) -> ResourceLease {
    broker
        .try_acquire_for_snapshot(
            Some(&SkillResourceRequest {
                class: SkillResourceClass::Memory,
                memory_mb: 512,
                ..SkillResourceRequest::default()
            }),
            4,
            resource_snapshot(8192, 8192),
            false,
        )
        .expect("owned fixture lease")
}

async fn wait_for_release(broker: &ResourceBroker) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while broker.status().active_leases != 0 {
        assert!(tokio::time::Instant::now() < deadline, "lease_not_released");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn restore_job(fixture: &LiveJobFixture, name: &str) -> PathBuf {
    let job_dir = claw_core::workspace_state::workspace_state_root(fixture.path())
        .join("async_jobs")
        .join(name);
    std::fs::create_dir_all(&job_dir).unwrap();
    for file in ["pid", "process_command_marker"] {
        std::fs::copy(fixture.path().join(file), job_dir.join(file)).unwrap();
    }
    std::fs::write(job_dir.join("lease_ready"), "1").unwrap();
    std::fs::write(job_dir.join("exit_code"), "0").unwrap();
    std::fs::write(
        job_dir.join("resource_lease.json"),
        serde_json::to_vec(&DurableResourceLeaseMetadata {
            schema_version: 1,
            lease_token: uuid::Uuid::new_v4().to_string(),
            memory_mib: 512,
            cpu_cores: 1,
            network_slots: 0,
            provider_slots: 0,
            browser_slots: 0,
            heavy: false,
            background: true,
            estimate_key: None,
            pid: fixture.pid(),
            created_at_epoch: crate::now_ts_u64(),
        })
        .unwrap(),
    )
    .unwrap();
    job_dir
}

#[tokio::test]
async fn resource_observation_handoff_retains_live_process_with_early_exit_file() {
    let mut fixture = LiveJobFixture::new();
    fixture.write("exit_code", "0");
    let broker = ResourceBroker::default();
    let result = acquire_lease(&broker)
        .hold_until_durable_job_terminal(fixture.path().to_owned())
        .await;
    tokio::time::sleep(durable_resource_monitor_interval() * 3).await;
    let retained = broker.status().active_leases;
    let published = fixture.path().join("resource_lease.json").is_file();
    let released = fixture
        .path()
        .join("resource_lease_release_reason")
        .exists();
    let live = process_identity_state(fixture.path(), fixture.pid());
    fixture.finish();
    wait_for_release(&broker).await;

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(live, ProcessIdentityState::AliveVerified);
    assert_eq!(retained, 1, "early_exit_must_not_drop_admitted_reservation");
    assert!(published);
    assert!(!released);
}

#[derive(Clone, Copy)]
enum MonitorRecord {
    Live,
    MissingPid,
    ChangedPid,
}

async fn monitor_retains_existing_lease(record: MonitorRecord) {
    let mut fixture = LiveJobFixture::new();
    let mut other = LiveJobFixture::new();
    let broker = ResourceBroker::default();
    acquire_lease(&broker)
        .hold_until_durable_job_terminal(fixture.path().to_owned())
        .await
        .unwrap();
    let reserved_before = broker.reserved_memory_mib();
    match record {
        MonitorRecord::Live => {}
        MonitorRecord::MissingPid => std::fs::remove_file(fixture.path().join("pid")).unwrap(),
        MonitorRecord::ChangedPid => fixture.write("pid", &other.pid().to_string()),
    }
    fixture.write("exit_code", "0");
    tokio::time::sleep(durable_resource_monitor_interval() * 3).await;
    let retained = broker.status().active_leases;
    let reserved_after = broker.reserved_memory_mib();
    let released = fixture
        .path()
        .join("resource_lease_release_reason")
        .exists();
    let live = process_identity_state(fixture.path(), fixture.pid());
    fixture.write("pid", &fixture.pid().to_string());
    fixture.finish();
    other.finish();
    wait_for_release(&broker).await;

    assert_eq!(live, ProcessIdentityState::AliveVerified);
    assert_eq!(
        retained, 1,
        "live_or_unknown_must_keep_existing_reservation"
    );
    assert!(!released);
    if !matches!(record, MonitorRecord::Live) {
        assert_eq!(
            reserved_after, reserved_before,
            "unverified_pid_must_not_be_sampled"
        );
    }
}

#[tokio::test]
async fn resource_observation_monitor_retains_live_process_with_early_exit_file() {
    monitor_retains_existing_lease(MonitorRecord::Live).await;
}

#[tokio::test]
async fn resource_observation_monitor_retains_lease_when_pid_record_is_missing() {
    monitor_retains_existing_lease(MonitorRecord::MissingPid).await;
}

#[tokio::test]
async fn resource_observation_monitor_does_not_adopt_a_changed_pid_record() {
    monitor_retains_existing_lease(MonitorRecord::ChangedPid).await;
}

#[tokio::test]
async fn resource_observation_restore_retains_matching_live_process_despite_exit_file() {
    let mut fixture = LiveJobFixture::new();
    let job_dir = restore_job(&fixture, "live");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = fixture.path().to_owned();
    let broker = state.skill_rt.skill_concurrency_gates.resource_broker();
    assert_eq!(
        process_identity_state(&job_dir, fixture.pid()),
        ProcessIdentityState::AliveVerified
    );
    let restored = restore_durable_resource_leases(&state);
    tokio::time::sleep(durable_resource_monitor_interval() * 3).await;
    let retained = broker.status().active_leases;
    let released = job_dir.join("resource_lease_release_reason").exists();
    fixture.finish();
    wait_for_release(broker).await;

    assert_eq!(restored, 1);
    assert_eq!(retained, 1);
    assert!(!released);
}

#[tokio::test]
async fn resource_observation_restore_does_not_admit_missing_or_mismatched_pid_records() {
    let mut fixture = LiveJobFixture::new();
    let mut other = LiveJobFixture::new();
    let missing = restore_job(&fixture, "missing");
    std::fs::remove_file(missing.join("pid")).unwrap();
    let changed = restore_job(&fixture, "changed");
    std::fs::write(changed.join("pid"), other.pid().to_string()).unwrap();
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = fixture.path().to_owned();
    let restored = restore_durable_resource_leases(&state);
    let leases = state
        .skill_rt
        .skill_concurrency_gates
        .resource_broker()
        .status()
        .active_leases;
    fixture.finish();
    other.finish();

    assert_eq!(restored, 0);
    assert_eq!(leases, 0);
}

async fn observe_initial_mismatch(
    fixture: &LiveJobFixture,
    broker: &ResourceBroker,
) -> (String, i64) {
    acquire_lease(broker)
        .hold_until_durable_job_terminal(fixture.path().to_owned())
        .await
        .unwrap();
    let original_marker =
        std::fs::read_to_string(fixture.path().join("process_command_marker")).unwrap();
    fixture.write("process_command_marker", "not-the-owned-process-command");
    assert_eq!(
        process_identity_state(fixture.path(), fixture.pid()),
        ProcessIdentityState::IdentityMismatch
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let first_observed_at = loop {
        if let Some(value) =
            crate::local_process_job::read_i64(fixture.path(), "process_missing_observed_at")
        {
            break value;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "mismatch_not_observed"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    // A new heartbeat must come from the next live/unknown monitor observation.
    let heartbeat = fixture.path().join("resource_lease_heartbeat_at");
    if heartbeat.exists() {
        std::fs::remove_file(heartbeat).unwrap();
    }
    (original_marker, first_observed_at)
}

async fn wait_for_monitor_heartbeat(fixture: &LiveJobFixture) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !fixture.path().join("resource_lease_heartbeat_at").is_file() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "heartbeat_not_observed"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn resource_observation_grace_restarts_after_verified_liveness_returns() {
    let mut fixture = LiveJobFixture::new();
    let broker = ResourceBroker::default();
    let (original_marker, first_observed_at) = observe_initial_mismatch(&fixture, &broker).await;
    fixture.write("process_command_marker", &original_marker);
    wait_for_monitor_heartbeat(&fixture).await;
    let cleared = !fixture.path().join("process_missing_observed_at").exists();

    // Let the first real observation age past the production grace without
    // altering timestamps. The process remains held and actually live.
    let aging_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while crate::now_ts_u64() < first_observed_at as u64 + 6 {
        assert!(
            tokio::time::Instant::now() < aging_deadline,
            "fixture_wall_clock_did_not_advance"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let live = process_identity_state(fixture.path(), fixture.pid());
    let retained_while_live = broker.status().active_leases;
    let exit_observed_at = crate::now_ts_u64() as i64;
    fixture.finish();
    let observation_started = tokio::time::Instant::now();
    tokio::time::sleep(durable_resource_monitor_interval() * 3).await;
    let observation_elapsed = observation_started.elapsed();
    let retained_during_new_grace = broker.status().active_leases;
    let next_observed_at =
        crate::local_process_job::read_i64(fixture.path(), "process_missing_observed_at");
    let released = fixture
        .path()
        .join("resource_lease_release_reason")
        .exists();
    fixture.write("exit_code", "0");
    wait_for_release(&broker).await;

    assert_eq!(live, ProcessIdentityState::AliveVerified);
    assert_eq!(retained_while_live, 1);
    assert!(
        observation_elapsed < Duration::from_secs(5),
        "fixture_missed_grace_window"
    );
    assert!(
        cleared,
        "verified_liveness_must_clear_prior_loss_observation"
    );
    assert_eq!(
        retained_during_new_grace, 1,
        "old_loss_must_not_skip_new_grace"
    );
    assert!(next_observed_at.is_some_and(|value| value >= exit_observed_at));
    assert!(!released);
}

#[tokio::test]
async fn resource_observation_grace_unknown_does_not_claim_verified_recovery() {
    let mut fixture = LiveJobFixture::new();
    let broker = ResourceBroker::default();
    let (original_marker, first_observed_at) = observe_initial_mismatch(&fixture, &broker).await;
    std::fs::remove_file(fixture.path().join("pid")).unwrap();
    wait_for_monitor_heartbeat(&fixture).await;
    let retained = broker.status().active_leases;
    let observation =
        crate::local_process_job::read_i64(fixture.path(), "process_missing_observed_at");
    let released = fixture
        .path()
        .join("resource_lease_release_reason")
        .exists();
    fixture.write("pid", &fixture.pid().to_string());
    fixture.write("process_command_marker", &original_marker);
    fixture.finish();
    fixture.write("exit_code", "0");
    wait_for_release(&broker).await;

    assert_eq!(retained, 1);
    assert_eq!(observation, Some(first_observed_at));
    assert!(!released);
}
