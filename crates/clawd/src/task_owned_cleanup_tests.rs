use super::*;
use crate::task_result_retention::test_support::Fixture;

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn cleanup_removes_all_owned_copies_and_retains_every_other_task_and_shared_root() {
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    let other = uuid::Uuid::new_v4().to_string();
    let owned = task_roots(&fixture.root, &fixture.task_id).unwrap();
    for root in &owned {
        write(&root.join("nested/media.bin"), b"downloaded-media");
    }
    let shared = [
        fixture
            .root
            .join("data/skills/private/browser-profile/state"),
        fixture.root.join("models/model.bin"),
        fixture.root.join("cache/retained"),
    ];
    for path in &shared {
        write(path, b"shared-data");
    }
    for root in task_roots(&fixture.root, &other).unwrap() {
        write(&root.join("ordinary.bin"), b"ordinary-data");
    }
    let job = uuid::Uuid::new_v4().to_string();
    let async_dir = claw_core::workspace_state::workspace_state_root(&fixture.root)
        .join("async_jobs")
        .join(&job);
    write(
        &async_dir.join("host_artifact_output_directory"),
        owned[1]
            .join("fixture/invocation")
            .to_str()
            .unwrap()
            .as_bytes(),
    );
    write(&async_dir.join("job_kind"), b"skill_runner");
    write(&async_dir.join("exit_code"), b"0");
    write(&async_dir.join("stdout"), b"private-result");
    write(&async_dir.join("stderr"), b"private-error");
    crate::task_result_retention::record_durable_job(
        &fixture.state,
        &fixture.task_id,
        &format!("local_process:{job}"),
    )
    .unwrap();
    let plan = prepare(&fixture.root, &fixture.task_id).unwrap();
    assert_eq!(plan.durable_jobs, vec![job]);
    execute(&fixture.root, &plan).unwrap();
    execute(&fixture.root, &plan).unwrap();
    assert!(!async_dir.exists());
    assert!(owned.iter().all(|p| !p.exists()));
    for path in shared {
        assert_eq!(std::fs::read(path).unwrap(), b"shared-data");
    }
    for root in task_roots(&fixture.root, &other).unwrap() {
        assert_eq!(
            std::fs::read(root.join("ordinary.bin")).unwrap(),
            b"ordinary-data"
        );
    }
}

#[test]
fn invalid_ids_and_foreign_async_job_are_not_deletion_authority() {
    let fixture = Fixture::new();
    assert!(prepare(&fixture.root, "../outside").is_err());
    let bad = TaskOwnedCleanupPlan {
        task_id: fixture.task_id.clone(),
        durable_jobs: vec!["../outside".to_string()],
    };
    assert!(execute(&fixture.root, &bad).is_err());
    let job = uuid::Uuid::new_v4().to_string();
    let job_dir = claw_core::workspace_state::workspace_state_root(&fixture.root)
        .join("async_jobs")
        .join(&job);
    write(&job_dir.join("stdout"), b"another-task");
    let plan = prepare(&fixture.root, &fixture.task_id).unwrap();
    assert!(plan.durable_jobs.is_empty());
    execute(&fixture.root, &plan).unwrap();
    assert_eq!(
        std::fs::read(job_dir.join("stdout")).unwrap(),
        b"another-task"
    );
}

#[cfg(unix)]
#[test]
fn symlink_roots_are_rejected_and_descendant_symlinks_never_followed() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = fixture.root.join("unrelated");
    write(&outside.join("keep"), b"do-not-delete");
    let root = task_roots(&fixture.root, &fixture.task_id).unwrap()[0].clone();
    std::fs::create_dir_all(root.parent().unwrap()).unwrap();
    symlink(&outside, &root).unwrap();
    assert!(prepare(&fixture.root, &fixture.task_id).is_err());
    std::fs::remove_file(&root).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    symlink(&outside, root.join("linked")).unwrap();
    let plan = prepare(&fixture.root, &fixture.task_id).unwrap();
    execute(&fixture.root, &plan).unwrap();
    assert_eq!(
        std::fs::read(outside.join("keep")).unwrap(),
        b"do-not-delete"
    );
}

#[test]
fn durable_monitor_blocks_cleanup_until_it_finishes_and_restart_needs_no_release_marker() {
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    let job = uuid::Uuid::new_v4().to_string();
    let path = claw_core::workspace_state::workspace_state_root(&fixture.root)
        .join("async_jobs")
        .join(&job);
    let invocation =
        task_roots(&fixture.root, &fixture.task_id).unwrap()[1].join("fixture/invocation");
    write(
        &path.join("host_artifact_output_directory"),
        invocation.to_str().unwrap().as_bytes(),
    );
    write(&path.join("job_kind"), b"skill_runner");
    write(&path.join("exit_code"), b"0");
    write(&path.join("resource_lease.json"), b"{}");
    write(&path.join("stdout"), b"retained-until-monitor-finished");
    crate::task_result_retention::record_durable_job(
        &fixture.state,
        &fixture.task_id,
        &format!("local_process:{job}"),
    )
    .unwrap();
    let guard = crate::task_result_retention::monitors::DurableJobMonitorGuard::register(&path);
    assert!(prepare(&fixture.root, &fixture.task_id).is_err());
    assert!(path.join("stdout").exists());
    drop(guard);
    let plan = prepare(&fixture.root, &fixture.task_id).unwrap();
    execute(&fixture.root, &plan).unwrap();
    assert!(!path.exists());
}

#[test]
fn killed_process_without_exit_record_cleans_after_stable_loss_but_live_or_unknown_never_authorizes(
) {
    use crate::local_process_job::ProcessIdentityState::*;
    for state in [AliveVerified, Unknown] {
        assert!(!terminal_process_is_stopped(true, state, true));
        assert!(!terminal_process_is_stopped(false, state, true));
    }
    assert!(!terminal_process_is_stopped(false, Missing, false));
    assert!(terminal_process_is_stopped(false, Missing, true));
    assert!(terminal_process_is_stopped(false, IdentityMismatch, true));
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    let job = uuid::Uuid::new_v4().to_string();
    let root = claw_core::workspace_state::workspace_state_root(&fixture.root)
        .join("async_jobs")
        .join(&job);
    let invocation =
        task_roots(&fixture.root, &fixture.task_id).unwrap()[1].join("fixture/invocation");
    write(
        &root.join("host_artifact_output_directory"),
        invocation.to_str().unwrap().as_bytes(),
    );
    write(&root.join("job_kind"), b"skill_runner");
    write(&root.join("stdout"), b"failed partial output");
    let mut child = std::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    write(&root.join("pid"), pid.to_string().as_bytes());
    crate::task_result_retention::record_durable_job(
        &fixture.state,
        &fixture.task_id,
        &format!("local_process:{job}"),
    )
    .unwrap();
    assert!(prepare(&fixture.root, &fixture.task_id).is_err());
    write(
        &root.join("process_missing_observed_at"),
        crate::now_ts_u64().saturating_sub(6).to_string().as_bytes(),
    );
    let plan = prepare(&fixture.root, &fixture.task_id).unwrap();
    execute(&fixture.root, &plan).unwrap();
    assert!(!root.exists());
}
