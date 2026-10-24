use super::*;
use crate::task_result_retention::test_support::Fixture;

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
