use super::*;

fn fixture_binding(actor: &str, task: &str) -> BrowserSessionBinding {
    BrowserSessionBinding {
        actor_ref: actor.to_string(),
        task_id: task.to_string(),
        registry_generation: 7,
        registry_digest: "registry-a".to_string(),
        policy_digest: "policy-a".to_string(),
    }
}

#[test]
fn binding_separates_actor_task_registry_and_policy() {
    let base = fixture_binding("actor-a", "task-a");
    let mut changed = base.clone();
    changed.actor_ref = "actor-b".to_string();
    assert_ne!(base, changed);
    changed = base.clone();
    changed.task_id = "task-b".to_string();
    assert_ne!(base, changed);
    changed = base.clone();
    changed.registry_generation = 8;
    assert_ne!(base, changed);
    changed = base.clone();
    changed.policy_digest = "policy-b".to_string();
    assert_ne!(base, changed);
}

#[test]
fn error_contract_uses_stable_machine_fields() {
    let error = BrowserSessionError::new("BROWSER_SESSION_LOST", true, json!({}));
    assert_eq!(error.message_key, "browser_session.browser_session_lost");
    assert!(error.retryable);
}

#[test]
fn detected_session_capacity_is_bounded() {
    assert!((1..=4).contains(&detect_session_capacity()));
}

#[test]
fn screenshot_contract_segments_only_when_host_resources_require_it() {
    let low_memory = screenshot_contract_for_resources(Some(2048), ResourcePressureState::Normal);
    assert_eq!(low_memory["mode"], "segmented_full_page");
    assert_eq!(low_memory["preserve_full_page"], true);
    assert_eq!(
        low_memory["max_tile_pixels"],
        LOW_MEMORY_SCREENSHOT_MAX_TILE_PIXELS
    );

    let pressured =
        screenshot_contract_for_resources(Some(8192), ResourcePressureState::Constrained);
    assert_eq!(pressured["mode"], "segmented_full_page");

    let normal = screenshot_contract_for_resources(Some(8192), ResourcePressureState::Normal);
    assert_eq!(normal["mode"], "single_full_page");
    assert_eq!(normal["preserve_full_page"], true);
    assert!(normal.get("max_tile_pixels").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_service_enforces_binding_and_recovers_from_bridge_loss() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace");
    let broker = crate::resource_scheduler::ResourceBroker::default();
    let service = BrowserSessionService::new_with_resource_broker(&workspace, broker.clone());
    let binding = fixture_binding("actor-a", "task-live-browser-session");
    let opened = service
        .open(binding.clone(), json!({"locale": "en-US"}), None)
        .await
        .expect("browser_fixture_open");
    let session_id = opened["session_id"].as_str().expect("session id");
    let page_id = opened["result"]["page_id"].as_str().expect("page id");
    let page_generation = opened["result"]["page_generation"]
        .as_u64()
        .expect("page generation");
    assert!(opened["result"].get("snapshot").is_none());
    assert_eq!(broker.status().active_heavy_leases, 1);
    let session = service
        .inner
        .sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("live session");
    assert!(session
        .last_safe_snapshot
        .lock()
        .expect("snapshot lock")
        .is_some());
    drop(session);

    let mut other_actor = binding.clone();
    other_actor.actor_ref = "actor-b".to_string();
    let mismatch = service
        .request(
            session_id,
            &other_actor,
            "snapshot",
            json!({"page_id": page_id, "expected_page_generation": page_generation}),
            None,
        )
        .await
        .expect_err("browser_cross_actor");
    assert_eq!(mismatch.code, "BROWSER_SESSION_BINDING_MISMATCH");

    let session = service
        .inner
        .sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("live session before lease refresh");
    let lease_before_request = unix_now().saturating_add(1);
    *session.lease_expires_at.lock().expect("lease expiry lock") = lease_before_request;
    drop(session);

    let snapshot = service
        .request(
            session_id,
            &binding,
            "snapshot",
            json!({"page_id": page_id, "expected_page_generation": page_generation}),
            None,
        )
        .await
        .expect("bound actor snapshot");
    assert_eq!(snapshot["session_id"], session_id);
    assert_eq!(snapshot["result"]["page_id"], page_id);
    assert!(snapshot["heartbeat_at"].as_u64().is_some());
    assert!(snapshot["lease_expires_at"]
        .as_u64()
        .is_some_and(|value| value > lease_before_request));

    let session = service
        .inner
        .sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("live session");
    kill_bridge(&session).await;
    drop(session);
    let lost = service
        .request(
            session_id,
            &binding,
            "snapshot",
            json!({"page_id": page_id, "expected_page_generation": page_generation}),
            None,
        )
        .await
        .expect_err("browser_bridge_dead");
    assert_eq!(lost.code, "BROWSER_SESSION_LOST");
    assert!(lost.details.get("last_safe_snapshot").is_some());
    assert!(!service.inner.sessions.lock().await.contains_key(session_id));
    assert_eq!(broker.status().active_heavy_leases, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_accepts_dispatch_lease_without_acquiring_a_second_browser_slot() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace");
    let broker = crate::resource_scheduler::ResourceBroker::default();
    let request = claw_core::skill_registry::SkillResourceRequest {
        class: claw_core::skill_registry::SkillResourceClass::Memory,
        cpu_cores: 1,
        memory_mb: 512,
        network_slots: 1,
        browser_slots: 1,
        ..claw_core::skill_registry::SkillResourceRequest::default()
    };
    let dispatch_lease = broker
        .try_acquire(Some(&request), 1)
        .expect("dispatch browser lease");
    assert_eq!(broker.status().reserved_browser_slots, 1);

    let service = BrowserSessionService::new_with_resource_broker(&workspace, broker.clone());
    let binding = fixture_binding("actor-dispatch-lease", "task-dispatch-lease");
    let opened = service
        .open_with_resource_lease(
            binding.clone(),
            json!({}),
            None,
            Some(dispatch_lease),
            Some(&request),
        )
        .await
        .expect("open must reuse dispatch lease");
    assert_eq!(broker.status().reserved_browser_slots, 1);

    service
        .close(
            opened["session_id"].as_str().expect("session id"),
            &binding,
            None,
        )
        .await
        .expect("close browser session");
    assert_eq!(broker.status().reserved_browser_slots, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_and_restarted_sessions_fail_closed() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace");
    let service = BrowserSessionService::new(&workspace);
    let binding = fixture_binding("actor-expired", "task-expired-browser-session");
    let opened = service
        .open(binding.clone(), json!({}), None)
        .await
        .expect("browser_fixture_open");
    let session_id = opened["session_id"].as_str().expect("session id");
    let page_id = opened["result"]["page_id"].as_str().expect("page id");
    let page_generation = opened["result"]["page_generation"]
        .as_u64()
        .expect("page generation");
    let session = service
        .inner
        .sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .expect("live session");
    *session.lease_expires_at.lock().expect("lease lock") = unix_now();
    drop(session);
    let expired = service
        .request(
            session_id,
            &binding,
            "snapshot",
            json!({"page_id": page_id, "expected_page_generation": page_generation}),
            None,
        )
        .await
        .expect_err("browser_expired");
    assert_eq!(expired.code, "BROWSER_SESSION_EXPIRED");
    assert!(!service.inner.sessions.lock().await.contains_key(session_id));

    let restarted = BrowserSessionService::new(&workspace);
    let lost = restarted
        .request(
            session_id,
            &binding,
            "snapshot",
            json!({"page_id": page_id, "expected_page_generation": page_generation}),
            None,
        )
        .await
        .expect_err("browser_restart_lost");
    assert_eq!(lost.code, "BROWSER_SESSION_LOST");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_cleanup_releases_only_the_matching_session_lease() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace");
    let service = BrowserSessionService::new(&workspace);
    let first_binding = fixture_binding("actor-cleanup", "task-cleanup-first");
    let second_binding = fixture_binding("actor-cleanup", "task-cleanup-second");
    let first = service
        .open(first_binding.clone(), json!({}), None)
        .await
        .expect("first browser session");
    let second = service
        .open(second_binding.clone(), json!({}), None)
        .await
        .expect("second browser session");
    let first_id = first["session_id"].as_str().expect("first id");
    let second_id = second["session_id"].as_str().expect("second id");

    assert_eq!(service.close_task_sessions("task-cleanup-first").await, 1);
    let first_lost = service
        .request(first_id, &first_binding, "observe_debug", json!({}), None)
        .await
        .expect_err("cleaned session must be gone");
    assert_eq!(first_lost.code, "BROWSER_SESSION_LOST");
    service
        .request(second_id, &second_binding, "observe_debug", json!({}), None)
        .await
        .expect("unrelated session remains available");
    assert_eq!(service.close_task_sessions("task-cleanup-second").await, 1);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_host_service_reaps_live_bridge_process() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace");
    let service = BrowserSessionService::new(&workspace);
    let binding = fixture_binding("actor-drop", "task-drop-browser-session");
    let opened = service
        .open(binding, json!({}), None)
        .await
        .expect("browser_fixture_open");
    let session_id = opened["session_id"].as_str().expect("session id");
    let process_id = {
        let session = service
            .inner
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .expect("live session");
        let process_id = session.bridge.lock().await.child.id().expect("child pid");
        process_id
    };
    assert!(Path::new(&format!("/proc/{process_id}")).exists());
    assert!(!linux_process_group_members(process_id).is_empty());
    let descendants = linux_process_descendants(process_id);
    assert!(!descendants.is_empty(), "browser_child_missing");
    let other_owner = service.clone();
    drop(service);
    other_owner
        .request(
            session_id,
            &fixture_binding("actor-drop", "task-drop-browser-session"),
            "observe_debug",
            json!({}),
            None,
        )
        .await
        .expect("another owner retains its bridge");
    drop(other_owner);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        if linux_process_group_members(process_id).is_empty()
            && descendants
                .iter()
                .all(|pid| !Path::new(&format!("/proc/{pid}")).exists())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "browser_bridge_survived:{process_id}; group={:?}; descendants={:?}",
        linux_process_group_members(process_id),
        descendants
            .iter()
            .filter_map(|pid| std::fs::read_to_string(format!("/proc/{pid}/stat")).ok())
            .collect::<Vec<_>>()
    );
}

#[cfg(target_os = "linux")]
fn linux_process_group_members(group_id: u32) -> Vec<u32> {
    linux_process_table()
        .into_iter()
        .filter_map(|(process_id, _, process_group)| {
            (process_group == group_id).then_some(process_id)
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_process_descendants(parent_id: u32) -> Vec<u32> {
    let table = linux_process_table();
    let mut parents = vec![parent_id];
    let mut descendants = Vec::new();
    while let Some(parent) = parents.pop() {
        for (process_id, candidate_parent, _) in &table {
            if *candidate_parent == parent && !descendants.contains(process_id) {
                descendants.push(*process_id);
                parents.push(*process_id);
            }
        }
    }
    descendants
}

#[cfg(target_os = "linux")]
fn linux_process_table() -> Vec<(u32, u32, u32)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
        .filter_map(|process_id| {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{process_id}/stat")) else {
                return None;
            };
            let Some(after_name) = stat.rsplit_once(')').map(|(_, suffix)| suffix.trim()) else {
                return None;
            };
            let fields = after_name.split_whitespace().collect::<Vec<_>>();
            Some((
                process_id,
                fields.get(1)?.parse::<u32>().ok()?,
                fields.get(2)?.parse::<u32>().ok()?,
            ))
        })
        .collect()
}
