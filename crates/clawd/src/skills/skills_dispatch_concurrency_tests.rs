use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

use super::{
    acquire_skill_dispatch_permits_with_serialization, action_scoped_planner_mapping,
    dispatch_queue_job_is_terminal, retain_dispatch_queue_permit_until_job_terminal, runner,
    skill_dispatch_queue_selection, skill_dispatch_serialization_key,
};

fn claimed_task(task_id: &str, user_id: i64, chat_id: i64) -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 1,
        task_id: task_id.to_string(),
        user_id,
        chat_id,
        user_key: Some(format!("user-{user_id}")),
        channel: "wechat".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "run_skill".to_string(),
        payload_json: "{}".to_string(),
    }
}

#[test]
fn registry_effect_selects_mutation_serialization_without_serializing_reads() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    *state.core.skill_views_snapshot.write().expect("snapshot") =
        Arc::new(crate::SkillViewsSnapshot {
            binding: Default::default(),
            registry: Some(Arc::new(registry)),
            skills_list: Arc::new(enabled),
        });
    assert_eq!(
        skill_dispatch_serialization_key(
            &state,
            "http_basic",
            &serde_json::json!({"action": "download"}),
        )
        .as_deref(),
        Some("__serial_skill__http_basic")
    );
    assert!(skill_dispatch_serialization_key(
        &state,
        "transform",
        &serde_json::json!({"action": "transform_data"}),
    )
    .is_none());
}

#[test]
fn declared_dispatch_queue_is_per_user_and_undeclared_skills_keep_existing_behavior() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    *state.core.skill_views_snapshot.write().expect("snapshot") =
        Arc::new(crate::SkillViewsSnapshot {
            binding: Default::default(),
            registry: Some(Arc::new(registry)),
            skills_list: Arc::new(enabled),
        });

    let first = skill_dispatch_queue_selection(
        &state,
        &claimed_task("media-1", 7, 10),
        "media_download",
        &serde_json::json!({"action": "download"}),
    )
    .expect("media queue selection");
    let same_user = skill_dispatch_queue_selection(
        &state,
        &claimed_task("media-2", 7, 11),
        "media_download",
        &serde_json::json!({"action": "transcribe"}),
    )
    .expect("same user queue selection");
    let other_user = skill_dispatch_queue_selection(
        &state,
        &claimed_task("media-3", 8, 10),
        "media_download",
        &serde_json::json!({"action": "ocr"}),
    )
    .expect("other user queue selection");

    assert_eq!(first.key, same_user.key);
    assert_ne!(first.key, other_user.key);
    assert!(skill_dispatch_queue_selection(
        &state,
        &claimed_task("media-capabilities", 7, 10),
        "media_download",
        &serde_json::json!({"action": "capabilities"}),
    )
    .is_none());
    assert!(skill_dispatch_queue_selection(
        &state,
        &claimed_task("ordinary", 7, 10),
        "transform",
        &serde_json::json!({"action": "transform_data"}),
    )
    .is_none());
}

#[test]
fn media_download_queue_is_owned_by_the_durable_runner() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    *state.core.skill_views_snapshot.write().expect("snapshot") =
        Arc::new(crate::SkillViewsSnapshot {
            binding: Default::default(),
            registry: Some(Arc::new(registry)),
            skills_list: Arc::new(enabled),
        });

    let mapping = action_scoped_planner_mapping(
        &state,
        "media_download",
        &serde_json::json!({"action": "download"}),
    );
    assert!(runner::local_process_durable_background_requested(
        mapping.as_ref()
    ));
}

#[tokio::test]
async fn serialized_mutation_waits_before_global_but_independent_read_continues() {
    let gates = Arc::new(crate::runtime::state::SkillConcurrencyGates::default());
    let global = Arc::new(Semaphore::new(2));
    let first = acquire_skill_dispatch_permits_with_serialization(
        &gates,
        &global,
        "mutation-first",
        "sample",
        None,
        Some("__serial_skill__sample"),
    )
    .await
    .expect("first mutation");
    let waiting_gates = gates.clone();
    let waiting_global = global.clone();
    let mut waiting = tokio::spawn(async move {
        acquire_skill_dispatch_permits_with_serialization(
            &waiting_gates,
            &waiting_global,
            "mutation-second",
            "sample",
            None,
            Some("__serial_skill__sample"),
        )
        .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut waiting)
            .await
            .is_err()
    );
    assert_eq!(global.available_permits(), 1);
    let read = acquire_skill_dispatch_permits_with_serialization(
        &gates, &global, "read", "sample", None, None,
    )
    .await
    .expect("independent read");
    drop(read);
    drop(first);
    drop(waiting.await.expect("join").expect("second mutation"));
}

#[tokio::test]
async fn user_scoped_queue_serializes_one_user_without_blocking_another() {
    let gates = Arc::new(crate::runtime::state::SkillConcurrencyGates::default());
    let global = Arc::new(Semaphore::new(2));
    let first = acquire_skill_dispatch_permits_with_serialization(
        &gates,
        &global,
        "first-user-first",
        "sample",
        None,
        Some("__dispatch_queue__sample__user__7"),
    )
    .await
    .expect("first user first task");

    let waiting_gates = gates.clone();
    let waiting_global = global.clone();
    let mut same_user = tokio::spawn(async move {
        acquire_skill_dispatch_permits_with_serialization(
            &waiting_gates,
            &waiting_global,
            "first-user-second",
            "sample",
            None,
            Some("__dispatch_queue__sample__user__7"),
        )
        .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut same_user)
            .await
            .is_err()
    );

    let other_user = acquire_skill_dispatch_permits_with_serialization(
        &gates,
        &global,
        "second-user-first",
        "sample",
        None,
        Some("__dispatch_queue__sample__user__8"),
    )
    .await
    .expect("other user remains parallel");
    drop(other_user);
    drop(first);
    drop(same_user.await.expect("join").expect("same user continues"));
}

#[tokio::test]
async fn durable_queue_permit_is_released_only_after_terminal_marker() {
    let semaphore = Arc::new(Semaphore::new(1));
    let permit = semaphore
        .clone()
        .acquire_owned()
        .await
        .expect("queue permit");
    let job_dir = std::env::temp_dir().join(format!(
        "agent-skill-queue-terminal-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&job_dir).expect("job dir");
    retain_dispatch_queue_permit_until_job_terminal(
        permit,
        job_dir.clone(),
        "queue-task".to_string(),
        "fixture".to_string(),
        "user",
    );
    assert_eq!(semaphore.available_permits(), 0);
    std::fs::write(job_dir.join("exit_code"), "1\n").expect("failed terminal marker");
    tokio::time::timeout(Duration::from_secs(2), async {
        while semaphore.available_permits() == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queue permit release");
    let _ = std::fs::remove_dir_all(job_dir);
}

#[test]
fn durable_queue_terminal_detection_handles_startup_failure_and_process_loss() {
    let root = std::env::temp_dir().join(format!(
        "agent-skill-queue-terminal-detection-{}",
        uuid::Uuid::new_v4()
    ));
    let startup_failed = root.join("startup-failed");
    std::fs::create_dir_all(&startup_failed).expect("startup failure dir");
    std::fs::write(startup_failed.join("startup_failed"), "spawn_failed\n")
        .expect("startup failure marker");
    assert!(dispatch_queue_job_is_terminal(&startup_failed, 100));

    let process_lost = root.join("process-lost");
    std::fs::create_dir_all(&process_lost).expect("process loss dir");
    std::fs::write(process_lost.join("pid"), "2147483647\n").expect("missing pid marker");
    std::fs::write(
        process_lost.join("process_command_marker"),
        "missing-runner\n",
    )
    .expect("command marker");
    assert!(!dispatch_queue_job_is_terminal(&process_lost, 100));
    assert!(dispatch_queue_job_is_terminal(&process_lost, 105));
    let _ = std::fs::remove_dir_all(root);
}
