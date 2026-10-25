use super::*;
use crate::task_result_retention::test_support::Fixture;

fn grant_authentication(fixture: &Fixture) {
    let path = fixture.root.join("data/nni/work/policy.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&serde_json::json!({
        "schema_version": 1, "principal_id": "fixture-principal",
        "capabilities": [fixture.enrollment.capability],
        "artifact_origins": [fixture.enrollment.origin], "max_file_bytes": 1024,
        "browser_authentication_grants": [{"origin":fixture.enrollment.origin,"capability":fixture.enrollment.capability}],
    })).unwrap()).unwrap();
}

#[test]
fn explicit_application_grant_retains_authentication_but_not_task_data() {
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(&fixture.task_id, "run_skill", &fixture.enrollment.idempotency_key(), &fixture.payload());
    grant_authentication(&fixture);
    let normal = fixture.state.core.skill_storage.directory_descriptor("fixture", 1).unwrap();
    let shared = std::path::PathBuf::from(normal.directory_path.as_ref().unwrap());
    let ordinary = shared.join("browser-profile");
    std::fs::create_dir_all(&ordinary).unwrap();
    std::fs::write(ordinary.join("keep"), b"ordinary-agent-login").unwrap();
    let scoped = descriptor_for_task(&fixture.state, &fixture.task_id, &fixture.payload(), Some(normal.clone())).unwrap().unwrap();
    let authentication = scoped.browser_authentication.as_ref().unwrap();
    let authentication_root = std::path::PathBuf::from(&authentication.directory_path);
    assert!(authentication_root.starts_with(shared.join("application-authentication")));
    std::fs::write(authentication_root.join("fixture.json"), b"authentication-only").unwrap();
    let temporary = std::path::PathBuf::from(scoped.directory_path.unwrap());
    std::fs::write(temporary.join("video.mp4"), b"temporary-private-video").unwrap();
    let plan = super::super::files::prepare(&fixture.root, &fixture.task_id).unwrap();
    super::super::files::execute(&fixture.root, &plan).unwrap();
    assert!(!temporary.exists());
    assert_eq!(std::fs::read(authentication_root.join("fixture.json")).unwrap(), b"authentication-only");
    assert_eq!(std::fs::read(ordinary.join("keep")).unwrap(), b"ordinary-agent-login");
    let id = uuid::Uuid::new_v4().to_string();
    let mut enrollment = fixture.enrollment.clone();
    enrollment.work_id = format!("nni-work-{}", uuid::Uuid::new_v4().simple());
    enrollment.job_id = uuid::Uuid::new_v4().to_string();
    let payload = serde_json::json!({"_task_source":"distributed_work","_requested_capability":enrollment.capability,"_delivery_cleanup":enrollment});
    super::super::register_enrollment(&fixture.root, &id, &enrollment).unwrap();
    fixture.insert(&id, "run_skill", &enrollment.idempotency_key(), &payload);
    let next = descriptor_for_task(&fixture.state, &id, &payload, Some(normal)).unwrap().unwrap();
    assert_eq!(next.browser_authentication.unwrap().directory_path, authentication.directory_path);
    assert_ne!(next.directory_path.as_deref(), Some(temporary.to_str().unwrap()));
    std::fs::remove_dir_all(shared).unwrap();
}

#[test]
fn ungranted_or_revoked_authentication_cannot_expose_persistent_credentials() {
    let fixture = Fixture::new();fixture.enroll();
    fixture.insert(&fixture.task_id,"run_skill",&fixture.enrollment.idempotency_key(),&fixture.payload());
    let normal = fixture.state.core.skill_storage.directory_descriptor("fixture",1).unwrap();
    let scoped = descriptor_for_task(&fixture.state,&fixture.task_id,&fixture.payload(),Some(normal.clone())).unwrap().unwrap();
    assert!(scoped.browser_authentication.is_none());
    grant_authentication(&fixture);
    let granted = descriptor_for_task(&fixture.state,&fixture.task_id,&fixture.payload(),Some(normal.clone())).unwrap().unwrap();
    assert!(granted.browser_authentication.is_some());
    std::fs::remove_file(fixture.root.join("data/nni/work/policy.json")).unwrap();
    let revoked = descriptor_for_task(&fixture.state,&fixture.task_id,&fixture.payload(),Some(normal.clone())).unwrap().unwrap();
    assert!(revoked.browser_authentication.is_none());
    std::fs::remove_dir_all(normal.directory_path.unwrap()).unwrap();
}

#[cfg(unix)]
#[test]
fn authentication_symlinks_never_expose_or_modify_unrelated_storage() {
    use std::os::unix::fs::symlink;
    let fixture=Fixture::new();fixture.enroll();
    fixture.insert(&fixture.task_id,"run_skill",&fixture.enrollment.idempotency_key(),&fixture.payload());
    grant_authentication(&fixture);
    let normal=fixture.state.core.skill_storage.directory_descriptor("fixture",1).unwrap();
    let base=std::path::PathBuf::from(normal.directory_path.as_ref().unwrap());
    let outside=fixture.root.join("unrelated-login");std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep"),b"unchanged").unwrap();
    symlink(&outside,base.join("application-authentication")).unwrap();
    assert!(descriptor_for_task(&fixture.state,&fixture.task_id,&fixture.payload(),Some(normal)).is_err());
    assert_eq!(std::fs::read(outside.join("keep")).unwrap(),b"unchanged");
    std::fs::remove_file(base.join("application-authentication")).unwrap();
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn ordinary_and_forged_tasks_keep_the_exact_original_storage_descriptor() {
    let fixture = Fixture::new();
    let payload = fixture.payload();
    fixture.insert(&fixture.task_id, "run_skill", "ordinary", &payload);
    let original = fixture
        .state
        .core
        .skill_storage
        .directory_descriptor("fixture", 1)
        .unwrap();
    let original_json = serde_json::to_value(&original).unwrap();
    let result = descriptor_for_task(&fixture.state, &fixture.task_id, &payload, Some(original))
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(result).unwrap(), original_json);
    assert!(
        !claw_core::workspace_state::workspace_artifacts_root(&fixture.root)
            .join("task-storage")
            .exists()
    );
}

#[test]
fn enrolled_profiles_are_task_scoped_and_cleanup_leaves_shared_profile_bytes_unchanged() {
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    let normal = fixture
        .state
        .core
        .skill_storage
        .directory_descriptor("fixture", 1)
        .unwrap();
    let shared = std::path::PathBuf::from(normal.directory_path.as_ref().unwrap())
        .join("public-browser-profile/Cache/video");
    std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
    std::fs::write(&shared, b"ordinary-browser-cache").unwrap();
    let scoped = descriptor_for_task(
        &fixture.state,
        &fixture.task_id,
        &fixture.payload(),
        Some(normal.clone()),
    )
    .unwrap()
    .unwrap();
    let storage = std::path::PathBuf::from(scoped.directory_path.unwrap());
    assert!(storage.starts_with(
        claw_core::workspace_state::workspace_artifacts_root(&fixture.root)
            .join("task-storage")
            .join(&fixture.task_id)
    ));
    assert_eq!(scoped.skill_name, normal.skill_name);
    assert_eq!(scoped.schema_version, normal.schema_version);
    assert_ne!(Some(storage.display().to_string()), normal.directory_path);
    let cache = storage.join("public-browser-profile/Default/Cache/video");
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, b"app-browser-cache").unwrap();
    let plan = super::super::files::prepare(&fixture.root, &fixture.task_id).unwrap();
    super::super::files::execute(&fixture.root, &plan).unwrap();
    assert!(!storage.exists());
    assert_eq!(std::fs::read(&shared).unwrap(), b"ordinary-browser-cache");
    // Fixture resolver storage is outside its workspace, so remove only our
    // own synthetic shared-profile fixture after verifying it was preserved.
    std::fs::remove_file(shared).unwrap();
}

#[test]
fn separate_enrolled_tasks_do_not_share_browser_profiles() {
    let first = Fixture::new();
    first.enroll();
    first.insert(
        &first.task_id,
        "run_skill",
        &first.enrollment.idempotency_key(),
        &first.payload(),
    );
    let second = Fixture::new();
    second.enroll();
    second.insert(
        &second.task_id,
        "run_skill",
        &second.enrollment.idempotency_key(),
        &second.payload(),
    );
    let one = descriptor_for_task(
        &first.state,
        &first.task_id,
        &first.payload(),
        Some(
            first
                .state
                .core
                .skill_storage
                .directory_descriptor("fixture", 1)
                .unwrap(),
        ),
    )
    .unwrap()
    .unwrap();
    let two = descriptor_for_task(
        &second.state,
        &second.task_id,
        &second.payload(),
        Some(
            second
                .state
                .core
                .skill_storage
                .directory_descriptor("fixture", 1)
                .unwrap(),
        ),
    )
    .unwrap()
    .unwrap();
    assert_ne!(one.directory_path, two.directory_path);
}

#[test]
fn enrolled_sqlite_storage_fails_without_touching_shared_database() {
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    let descriptor = fixture
        .state
        .core
        .skill_storage
        .descriptor("fixture", 1)
        .unwrap();
    let path = std::path::PathBuf::from(&descriptor.database_path);
    std::fs::write(&path, b"shared-sqlite-fixture").unwrap();
    assert!(descriptor_for_task(
        &fixture.state,
        &fixture.task_id,
        &fixture.payload(),
        Some(descriptor)
    )
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"shared-sqlite-fixture");
    std::fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[test]
fn enrolled_storage_rejects_child_symlink_without_changing_shared_files_or_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    let normal = fixture
        .state
        .core
        .skill_storage
        .directory_descriptor("fixture", 1)
        .unwrap();
    let shared = fixture.root.join("shared-profile");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o750)).unwrap();
    std::fs::write(shared.join("keep"), b"ordinary-private-profile").unwrap();
    let before = std::fs::metadata(&shared).unwrap().permissions().mode();
    let task = claw_core::workspace_state::workspace_artifacts_root(&fixture.root)
        .join("task-storage")
        .join(&fixture.task_id);
    std::fs::create_dir_all(&task).unwrap();
    symlink(&shared, task.join("fixture")).unwrap();
    assert!(descriptor_for_task(
        &fixture.state,
        &fixture.task_id,
        &fixture.payload(),
        Some(normal)
    )
    .is_err());
    assert_eq!(
        std::fs::read(shared.join("keep")).unwrap(),
        b"ordinary-private-profile"
    );
    assert_eq!(
        std::fs::metadata(&shared).unwrap().permissions().mode(),
        before
    );
}
