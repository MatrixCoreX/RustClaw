use super::*;

fn resource_snapshot(total_mib: u64, available_mib: u64) -> HostResourceSnapshot {
    HostResourceSnapshot {
        schema_version: 1,
        sampled_at_epoch_ms: 1,
        physical_memory_bytes: Some(total_mib * 1024 * 1024),
        effective_memory_limit_bytes: Some(total_mib * 1024 * 1024),
        memory_available_bytes: Some(available_mib * 1024 * 1024),
        memory_current_bytes: Some((total_mib - available_mib) * 1024 * 1024),
        swap_used_bytes: Some(0),
        cpu_parallelism: 4,
        cgroup_version: Some(2),
        cgroup_events: claw_core::host_resources::CgroupMemoryEvents::default(),
        memory_pressure: claw_core::host_resources::MemoryPressureSample::default(),
        memory_source: "fixture",
        pressure_source: "fixture",
    }
}

#[test]
fn host_grant_never_exceeds_worker_safety_ceiling() {
    let request = SkillResourceRequest {
        class: SkillResourceClass::Cpu,
        cpu_cores: 2,
        memory_mb: 512,
        ..SkillResourceRequest::default()
    };
    let grant = host_grant(Some(&request), 3);
    assert!(grant.admitted);
    assert!((1..=3).contains(&grant.max_concurrency));
}

#[test]
fn missing_gpu_uses_declared_fallback_or_waits() {
    if gpu_device_available() {
        return;
    }
    let mut request = SkillResourceRequest {
        class: SkillResourceClass::Gpu,
        gpu_slots: 1,
        ..SkillResourceRequest::default()
    };
    assert!(!host_grant(Some(&request), 4).admitted);
    request.allow_cpu_fallback = true;
    assert!(host_grant(Some(&request), 4).admitted);
}

#[test]
fn impossible_memory_request_is_not_started() {
    let request = SkillResourceRequest {
        class: SkillResourceClass::Memory,
        memory_mb: u64::MAX,
        ..SkillResourceRequest::default()
    };
    let grant = host_grant(Some(&request), 4);
    assert!(!grant.admitted);
    assert_eq!(grant.wait_reason, Some("memory_unavailable"));
}

#[test]
fn host_policy_floor_prevents_resource_request_underreporting() {
    let general = SkillResourceRequest {
        memory_mb: 64,
        ..SkillResourceRequest::default()
    };
    let cpu = SkillResourceRequest {
        class: SkillResourceClass::Cpu,
        memory_mb: 64,
        ..SkillResourceRequest::default()
    };
    let memory = SkillResourceRequest {
        class: SkillResourceClass::Memory,
        memory_mb: 128,
        ..SkillResourceRequest::default()
    };
    let local_model = SkillResourceRequest {
        class: SkillResourceClass::LocalModel,
        memory_mb: 128,
        ..SkillResourceRequest::default()
    };
    assert_eq!(requested_memory_mb(&general), DEFAULT_SKILL_MEMORY_MIB);
    assert_eq!(requested_memory_mb(&cpu), 256);
    assert_eq!(requested_memory_mb(&memory), 512);
    assert_eq!(requested_memory_mb(&local_model), 1024);
}

#[test]
fn low_memory_host_serializes_runtime_work_and_disables_warm_pool() {
    let plan = runtime_concurrency_plan_for_host(4, 3, 2, true, 4, Some(1024));
    assert_eq!(plan.worker_concurrency, 1);
    assert_eq!(plan.skill_concurrency, 1);
    assert_eq!(plan.memory_background_concurrency, 1);
    assert!(!plan.runner_warm_pool_enabled);
}

#[test]
fn constrained_host_keeps_two_foreground_slots_but_one_background_slot() {
    let plan = runtime_concurrency_plan_for_host(4, 4, 4, true, 4, Some(4096));
    assert_eq!(plan.worker_concurrency, 2);
    assert_eq!(plan.skill_concurrency, 2);
    assert_eq!(plan.memory_background_concurrency, 1);
    assert!(!plan.runner_warm_pool_enabled);
}

#[test]
fn capable_host_preserves_configured_limits() {
    let plan = runtime_concurrency_plan_for_host(3, 2, 2, true, 8, Some(16 * 1024));
    assert_eq!(plan.worker_concurrency, 3);
    assert_eq!(plan.skill_concurrency, 2);
    assert_eq!(plan.memory_background_concurrency, 2);
    assert!(plan.runner_warm_pool_enabled);
}

#[test]
fn cpu_count_remains_a_hard_concurrency_ceiling() {
    let plan = runtime_concurrency_plan_for_host(8, 8, 8, true, 2, None);
    assert_eq!(plan.worker_concurrency, 2);
    assert_eq!(plan.skill_concurrency, 2);
    assert_eq!(plan.memory_background_concurrency, 2);
}

#[test]
fn broker_reserves_memory_atomically_and_releases_it_on_drop() {
    let broker = ResourceBroker::default();
    let request = SkillResourceRequest {
        class: SkillResourceClass::Memory,
        memory_mb: 700,
        ..SkillResourceRequest::default()
    };
    let first = broker
        .try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(8192, 2048), false)
        .expect("first lease");
    assert_eq!(broker.reserved_memory_mib(), 700);
    let second =
        broker.try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(8192, 2048), false);
    assert_eq!(
        second.expect_err("second lease must wait").wait_reason,
        Some("memory_unavailable")
    );
    let status = broker.status();
    assert_eq!(
        status.recent_admission_refusal_reason,
        Some("memory_unavailable")
    );
    assert!(status.recent_admission_refusal_at_epoch.is_some());
    drop(first);
    assert_eq!(broker.reserved_memory_mib(), 0);
    assert!(broker
        .try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(8192, 2048), false)
        .is_ok());
}

#[test]
fn materialized_process_memory_is_not_counted_twice() {
    let broker = ResourceBroker::new(ResourcePressurePolicy::default(), Some(192));
    let process_request = SkillResourceRequest {
        class: SkillResourceClass::Memory,
        memory_mb: 512,
        browser_slots: 1,
        ..SkillResourceRequest::default()
    };
    let process_lease = broker
        .try_acquire_for_snapshot(
            Some(&process_request),
            4,
            resource_snapshot(1536, 1000),
            false,
        )
        .expect("process lease");

    assert_eq!(broker.reserved_memory_mib(), 512);
    process_lease.refresh_materialized_memory(400);
    assert_eq!(broker.reserved_memory_mib(), 112);

    let control_request = SkillResourceRequest {
        class: SkillResourceClass::ProviderQuota,
        cpu_cores: 1,
        memory_mb: 64,
        network_slots: 1,
        provider_slots: 1,
        ..SkillResourceRequest::default()
    };
    let control_lease = broker
        .try_acquire_for_snapshot(
            Some(&control_request),
            4,
            resource_snapshot(1536, 500),
            false,
        )
        .expect("control lease after process memory materializes");
    assert_eq!(broker.reserved_memory_mib(), 240);
    drop(control_lease);

    process_lease.refresh_materialized_memory(100);
    assert_eq!(broker.reserved_memory_mib(), 412);
    assert_eq!(
        broker
            .try_acquire_for_snapshot(
                Some(&control_request),
                4,
                resource_snapshot(1536, 500),
                false,
            )
            .expect_err("unmaterialized headroom must stay reserved")
            .wait_reason,
        Some("memory_unavailable")
    );
}

#[test]
fn concurrent_broker_requests_cannot_spend_the_same_memory_budget() {
    let broker = ResourceBroker::default();
    let request = SkillResourceRequest {
        class: SkillResourceClass::Memory,
        memory_mb: 700,
        ..SkillResourceRequest::default()
    };
    let start = Arc::new(std::sync::Barrier::new(3));
    let hold = Arc::new(std::sync::Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let broker = broker.clone();
        let request = request.clone();
        let start = start.clone();
        let hold = hold.clone();
        workers.push(std::thread::spawn(move || {
            start.wait();
            let lease = broker.try_acquire_for_snapshot(
                Some(&request),
                4,
                resource_snapshot(8192, 2048),
                false,
            );
            hold.wait();
            lease.is_ok()
        }));
    }
    start.wait();
    hold.wait();
    let admitted = workers
        .into_iter()
        .map(|worker| worker.join().expect("worker"))
        .filter(|admitted| *admitted)
        .count();
    assert_eq!(admitted, 1);
}

#[test]
fn broker_lease_is_released_when_the_owner_panics() {
    let broker = ResourceBroker::default();
    let request = SkillResourceRequest {
        class: SkillResourceClass::Memory,
        memory_mb: 700,
        ..SkillResourceRequest::default()
    };
    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
        let broker = broker.clone();
        let request = request.clone();
        move || {
            let _lease = broker
                .try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(8192, 2048), false)
                .expect("lease before panic");
            panic!("intentional resource lease owner panic");
        }
    }));
    assert!(panic_result.is_err());
    assert_eq!(broker.reserved_memory_mib(), 0);
}

#[tokio::test]
async fn broker_lease_is_released_when_the_owner_is_cancelled() {
    let broker = ResourceBroker::default();
    let lease = broker
        .try_acquire_for_snapshot(
            Some(&SkillResourceRequest {
                class: SkillResourceClass::Memory,
                memory_mb: 700,
                ..SkillResourceRequest::default()
            }),
            4,
            resource_snapshot(8192, 2048),
            false,
        )
        .expect("lease before cancellation");
    let cancellation = tokio_util::sync::CancellationToken::new();
    let child_cancellation = cancellation.clone();
    let owner = tokio::spawn(async move {
        child_cancellation.cancelled().await;
        drop(lease);
    });
    cancellation.cancel();
    owner.await.expect("cancelled owner exits cleanly");
    assert_eq!(broker.reserved_memory_mib(), 0);
}

#[test]
fn broker_reserves_cpu_atomically() {
    let broker = ResourceBroker::default();
    let request = SkillResourceRequest {
        class: SkillResourceClass::Cpu,
        cpu_cores: 3,
        memory_mb: 256,
        ..SkillResourceRequest::default()
    };
    let first = broker
        .try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(8192, 4096), false)
        .expect("first lease");
    let second =
        broker.try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(8192, 4096), false);
    assert_eq!(
        second.expect_err("cpu must be reserved").wait_reason,
        Some("cpu_unavailable")
    );
    drop(first);
    assert!(broker
        .try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(8192, 4096), false)
        .is_ok());
}

#[test]
fn broker_reserves_network_and_provider_slots_atomically() {
    let broker = ResourceBroker::default();
    let request = SkillResourceRequest {
        network_slots: 2,
        provider_slots: 1,
        ..SkillResourceRequest::default()
    };
    let first = broker
        .try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(1024, 900), false)
        .expect("first lease");
    let second =
        broker.try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(1024, 900), false);
    assert_eq!(
        second
            .expect_err("network slots must be reserved")
            .wait_reason,
        Some("network_slot_unavailable")
    );
    drop(first);
    assert!(broker
        .try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(1024, 900), false)
        .is_ok());
}

#[test]
fn browser_request_receives_a_smaller_grant_on_constrained_hosts() {
    let broker = ResourceBroker::default();
    let request = SkillResourceRequest {
        class: SkillResourceClass::Memory,
        memory_mb: 384,
        browser_slots: 3,
        ..SkillResourceRequest::default()
    };
    let lease = broker
        .try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(2048, 1536), false)
        .expect("degraded browser lease");
    assert_eq!(lease.grant().projection["request"]["browser_slots"], 3);
    assert_eq!(lease.grant().projection["grant"]["browser_slots"], 1);
    assert_eq!(lease.grant().projection["grant"]["memory_mb"], 512);
    assert_eq!(broker.status().reserved_browser_slots, 1);
}

#[test]
fn concurrent_browser_requests_cannot_spend_the_same_slot() {
    let broker = ResourceBroker::default();
    let request = SkillResourceRequest {
        class: SkillResourceClass::Memory,
        memory_mb: 512,
        browser_slots: 2,
        ..SkillResourceRequest::default()
    };
    let first = broker
        .try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(8192, 4096), false)
        .expect("first browser lease");
    let second =
        broker.try_acquire_for_snapshot(Some(&request), 4, resource_snapshot(8192, 4096), false);
    assert_eq!(
        second
            .expect_err("browser slot must be reserved")
            .wait_reason,
        Some("browser_slot_unavailable")
    );
    drop(first);
}

#[test]
fn critical_pressure_blocks_heavy_work_but_keeps_light_control_work_available() {
    let broker = ResourceBroker::default();
    let mut pressured = resource_snapshot(8192, 4096);
    pressured.memory_pressure.full_avg10 = Some(25.0);
    broker.observe_snapshot(pressured.clone());
    assert_eq!(
        broker.observe_snapshot(pressured.clone()).pressure_state,
        claw_core::host_resources::ResourcePressureState::Critical
    );

    let heavy = SkillResourceRequest {
        class: SkillResourceClass::Cpu,
        memory_mb: 256,
        ..SkillResourceRequest::default()
    };
    let refusal = broker
        .try_acquire_for_snapshot(Some(&heavy), 4, pressured.clone(), false)
        .expect_err("heavy work must wait");
    assert_eq!(refusal.wait_reason, Some("resource_pressure"));
    assert_eq!(refusal.projection["admitted"], false);
    assert_eq!(refusal.projection["wait_reason"], "resource_pressure");
    assert!(broker
        .try_acquire_for_snapshot(None, 4, pressured, false)
        .is_ok());
}

#[test]
fn critical_pressure_refusal_p95_stays_bounded_and_control_work_remains_available() {
    let broker = ResourceBroker::default();
    let mut pressured = resource_snapshot(8192, 4096);
    pressured.memory_pressure.full_avg10 = Some(25.0);
    broker.observe_snapshot(pressured.clone());
    broker.observe_snapshot(pressured.clone());

    let heavy = SkillResourceRequest {
        class: SkillResourceClass::Cpu,
        memory_mb: 256,
        ..SkillResourceRequest::default()
    };
    let mut refusal_latencies = Vec::with_capacity(128);
    for _ in 0..128 {
        let started = std::time::Instant::now();
        let refusal = broker
            .try_acquire_for_snapshot(Some(&heavy), 4, pressured.clone(), false)
            .expect_err("critical pressure must reject new heavy work");
        refusal_latencies.push(started.elapsed());
        assert_eq!(refusal.wait_reason, Some("resource_pressure"));
    }
    refusal_latencies.sort_unstable();
    let p95_index = (refusal_latencies.len() * 95).div_ceil(100) - 1;
    assert!(refusal_latencies[p95_index] <= std::time::Duration::from_secs(2));

    let control_lease = broker
        .try_acquire_for_snapshot(None, 4, pressured, false)
        .expect("status and cancellation controls must remain available");
    drop(control_lease);
}

#[test]
fn compact_pressure_serializes_only_heavy_background_work() {
    let broker = ResourceBroker::default();
    let compact = resource_snapshot(3072, 2500);
    broker.observe_snapshot(compact.clone());
    assert_eq!(
        broker.observe_snapshot(compact.clone()).pressure_state,
        ResourcePressureState::Compact
    );
    let request = SkillResourceRequest {
        class: SkillResourceClass::Cpu,
        cpu_cores: 1,
        memory_mb: 256,
        ..SkillResourceRequest::default()
    };
    let background = broker
        .try_acquire_for_snapshot_and_key(Some(&request), 4, compact.clone(), false, None, true)
        .expect("first background lease");
    assert_eq!(background.grant().projection["resource_background"], true);
    assert_eq!(
        broker
            .try_acquire_for_snapshot_and_key(
                Some(&request),
                4,
                compact.clone(),
                false,
                None,
                true,
            )
            .expect_err("second heavy background lease must wait")
            .wait_reason,
        Some("resource_pressure")
    );
    assert!(broker
        .try_acquire_for_snapshot_and_key(Some(&request), 4, compact, false, None, false,)
        .is_ok());
}

#[test]
fn broker_projection_exposes_machine_resource_state_without_user_text() {
    let broker = ResourceBroker::default();
    let lease = broker
        .try_acquire_for_snapshot(None, 2, resource_snapshot(4096, 2048), false)
        .expect("default lease");
    let projection = &lease.grant().projection;
    assert_eq!(projection["resource_pressure_state"], "normal");
    assert_eq!(projection["host_memory_available_mb"], 2048);
    assert_eq!(projection["host_safety_reserve_mb"], 409);
    assert_eq!(projection["request"]["declared_memory_mb"], 0);
    assert_eq!(
        projection["request"]["host_policy_floor_memory_mb"],
        DEFAULT_SKILL_MEMORY_MIB
    );
    assert!(projection.get("text").is_none());
}

#[test]
fn observed_peak_estimate_is_versioned_and_converges_after_new_samples() {
    let broker = ResourceBroker::default();
    let request = SkillResourceRequest {
        memory_mb: 64,
        ..SkillResourceRequest::default()
    };
    let old_key = ResourceEstimateKey {
        scope: "fixture_skill".to_string(),
        action: "inspect".to_string(),
        registry_generation: 1,
        registry_generation_digest: Some("generation-one".to_string()),
    };
    broker.record_observed_peak_memory(old_key, 700);
    let new_key = ResourceEstimateKey {
        scope: "fixture_skill".to_string(),
        action: "inspect".to_string(),
        registry_generation: 2,
        registry_generation_digest: Some("generation-two".to_string()),
    };

    let inherited = broker
        .try_acquire_for_snapshot_and_key(
            Some(&request),
            4,
            resource_snapshot(8192, 4096),
            false,
            Some(&new_key),
            false,
        )
        .expect("new version should inherit conservative estimate");
    assert_eq!(
        inherited.grant().projection["request"]["declared_memory_mb"],
        64
    );
    assert_eq!(
        inherited.grant().projection["request"]["observed_estimate_memory_mb"],
        700
    );
    assert_eq!(inherited.grant().projection["grant"]["memory_mb"], 700);
    drop(inherited);

    for _ in 0..RESOURCE_ESTIMATE_MIN_SAMPLES {
        broker.record_observed_peak_memory(new_key.clone(), 300);
    }
    let converged = broker
        .try_acquire_for_snapshot_and_key(
            Some(&request),
            4,
            resource_snapshot(8192, 4096),
            false,
            Some(&new_key),
            false,
        )
        .expect("new version estimate should converge");
    assert_eq!(
        converged.grant().projection["request"]["observed_estimate_memory_mb"],
        300
    );
    assert_eq!(converged.grant().projection["grant"]["memory_mb"], 300);
}

#[test]
fn observed_peak_estimate_persists_in_private_runtime_cache() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-resource-estimate-test-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    let path = root.join("resource-estimates-v1.json");
    let key = ResourceEstimateKey {
        scope: "fixture_skill".to_string(),
        action: "run".to_string(),
        registry_generation: 4,
        registry_generation_digest: None,
    };
    let broker = ResourceBroker::default().with_estimate_store_path(path.clone());
    broker.record_observed_peak_memory(key.clone(), 640);
    assert!(path.is_file());

    let restored = ResourceBroker::default().with_estimate_store_path(path);
    let lease = restored
        .try_acquire_for_snapshot_and_key(
            None,
            2,
            resource_snapshot(8192, 4096),
            false,
            Some(&key),
            false,
        )
        .expect("restored estimate should be admissible");
    assert_eq!(lease.grant().projection["grant"]["memory_mb"], 640);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pressure_transition_log_is_bounded_structured_and_separate() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-resource-log-test-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    let broker = ResourceBroker::default();
    let status = broker.observe_snapshot(resource_snapshot(2048, 1024));
    append_resource_pressure_event(&root, &status, 1).expect("append pressure transition");
    let path = claw_core::workspace_state::workspace_state_root(&root)
        .join("logs/resource-pressure.jsonl");
    let raw = std::fs::read_to_string(&path).expect("read pressure log");
    let line = raw.lines().next().expect("pressure record");
    let record: serde_json::Value = serde_json::from_str(line).expect("parse pressure record");
    assert_eq!(record["record_type"], "resource_pressure_transition");
    assert_eq!(record["reclaimed_browsers"], 1);
    assert!(record.get("request").is_none());
    assert!(record.get("prompt").is_none());
    assert!(record.get("credential").is_none());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn runtime_process_memory_tracks_current_peak_roles_and_warning() {
    let broker = ResourceBroker::default();
    let first = broker.record_runtime_process_memory(
        crate::runtime_process_memory::RuntimeProcessMemorySample {
            measurement: "fixture".to_string(),
            process_count: 3,
            resident_and_swap_bytes: 700,
            roles: std::collections::BTreeMap::from([
                ("core".to_string(), 500),
                ("channel".to_string(), 200),
            ]),
        },
        Some(1_000),
    );
    assert!(!first.warning);
    assert_eq!(first.peak_bytes, 700);

    let second = broker.record_runtime_process_memory(
        crate::runtime_process_memory::RuntimeProcessMemorySample {
            measurement: "fixture".to_string(),
            process_count: 2,
            resident_and_swap_bytes: 800,
            roles: std::collections::BTreeMap::from([
                ("core".to_string(), 600),
                ("web_gateway".to_string(), 200),
            ]),
        },
        Some(1_000),
    );
    assert!(second.warning);
    assert_eq!(second.current_bytes, 800);
    assert_eq!(second.peak_bytes, 800);
    assert_eq!(second.roles_peak_bytes.get("core"), Some(&600));
    assert_eq!(second.roles_peak_bytes.get("channel"), Some(&200));
    assert_eq!(second.roles_peak_bytes.get("web_gateway"), Some(&200));
    assert_eq!(
        broker
            .status()
            .runtime_process_memory
            .expect("process memory status")
            .peak_bytes,
        800
    );
}

#[test]
fn broker_uses_validated_safety_reserve_override() {
    let broker = ResourceBroker::new(
        claw_core::host_resources::ResourcePressurePolicy::default(),
        Some(512),
    );
    let lease = broker
        .try_acquire_for_snapshot(None, 2, resource_snapshot(8192, 2048), false)
        .expect("default lease");
    assert_eq!(lease.grant().projection["host_safety_reserve_mb"], 512);
}

#[tokio::test]
async fn durable_identity_handoff_accepts_an_already_terminal_job() {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "agent-runtime-terminal-lease-handoff-test-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&root).expect("job directory");
    std::fs::write(root.join("exit_code"), "0").expect("terminal marker");

    assert!(!await_durable_process_identity_or_terminal(
        &root,
        u32::MAX,
        std::time::Duration::from_millis(25),
    )
    .await
    .expect("terminal jobs do not require a live process"));

    std::fs::remove_dir_all(root).expect("remove fixture");
}

#[cfg(unix)]
#[tokio::test]
async fn durable_identity_handoff_waits_for_a_valid_process_marker() {
    use std::process::Command;

    let mut root = std::env::temp_dir();
    root.push(format!(
        "agent-runtime-delayed-lease-handoff-test-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&root).expect("job directory");
    let executable = std::path::Path::new("/bin/sleep");
    let mut child = Command::new(executable)
        .arg("30")
        .spawn()
        .expect("spawn process");
    let marker_path = root.join("process_command_marker");
    std::fs::write(&marker_path, "marker-not-visible-yet").expect("initial marker");
    let delayed_marker_path = marker_path.clone();
    let marker_writer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(delayed_marker_path, executable.display().to_string())
            .expect("valid marker");
    });

    let verified = await_durable_process_identity_or_terminal(
        &root,
        child.id(),
        std::time::Duration::from_secs(1),
    )
    .await
    .expect("identity becomes valid within the handoff window");
    marker_writer.join().expect("marker writer");
    let _ = child.kill();
    let _ = child.wait();

    assert!(verified);
    std::fs::remove_dir_all(root).expect("remove fixture");
}

#[cfg(unix)]
#[tokio::test]
async fn durable_resource_lease_is_rebuilt_and_released_from_machine_markers() {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "agent-runtime-resource-lease-test-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    let job_dir = claw_core::workspace_state::workspace_state_root(&root)
        .join("async_jobs")
        .join("job-a");
    std::fs::create_dir_all(&job_dir).expect("job directory");
    let executable = std::env::current_exe().expect("current executable");
    let marker = executable
        .file_name()
        .expect("executable name")
        .to_string_lossy();
    std::fs::write(job_dir.join("pid"), std::process::id().to_string()).expect("pid");
    std::fs::write(job_dir.join("process_command_marker"), marker.as_bytes()).expect("marker");
    std::fs::write(job_dir.join("lease_ready"), "1").expect("lease ready");
    std::fs::write(job_dir.join("resource_lease_heartbeat_at"), "1")
        .expect("stale heartbeat fixture");
    std::fs::write(
        job_dir.join("resource_lease.json"),
        serde_json::to_vec(&DurableResourceLeaseMetadata {
            schema_version: 1,
            lease_token: uuid::Uuid::new_v4().to_string(),
            memory_mib: 384,
            cpu_cores: 1,
            network_slots: 1,
            provider_slots: 0,
            browser_slots: 1,
            heavy: true,
            background: true,
            estimate_key: None,
            pid: std::process::id(),
            created_at_epoch: crate::now_ts_u64(),
        })
        .expect("lease metadata"),
    )
    .expect("write lease metadata");

    let orphan_dir = job_dir.parent().expect("jobs root").join("job-orphan");
    std::fs::create_dir_all(&orphan_dir).expect("orphan job directory");
    let orphan_pid = u32::MAX;
    std::fs::write(orphan_dir.join("pid"), orphan_pid.to_string()).expect("orphan pid");
    std::fs::write(orphan_dir.join("process_command_marker"), "missing-process")
        .expect("orphan process marker");
    std::fs::write(orphan_dir.join("lease_ready"), "1").expect("orphan lease ready");
    std::fs::write(
        orphan_dir.join("resource_lease.json"),
        serde_json::to_vec(&DurableResourceLeaseMetadata {
            schema_version: 1,
            lease_token: uuid::Uuid::new_v4().to_string(),
            memory_mib: 2048,
            cpu_cores: 2,
            network_slots: 0,
            provider_slots: 0,
            browser_slots: 0,
            heavy: true,
            background: true,
            estimate_key: None,
            pid: orphan_pid,
            created_at_epoch: 1,
        })
        .expect("orphan lease metadata"),
    )
    .expect("write orphan lease metadata");

    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    assert_eq!(restore_durable_resource_leases(&state), 1);
    let broker = state.skill_rt.skill_concurrency_gates.resource_broker();
    assert!(broker.reserved_memory_mib() < 384);

    std::fs::write(job_dir.join("exit_code"), "0").expect("terminal marker");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(broker.reserved_memory_mib(), 0);
    std::fs::remove_dir_all(root).expect("remove fixture");
}
