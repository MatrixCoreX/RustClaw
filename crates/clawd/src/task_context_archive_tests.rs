use super::*;
use crate::secure_workspace_fs::tests::TestWorkspace;

fn items(count: usize) -> Vec<Value> {
    (0..count).map(|index| json!({"capability":"fixture.read","status":"ok",
        "provenance":{"continuation_args":{"path":format!("file-{index}"),"api_key":"private-fixture"}},
        "data":{"index":index,"padding":"x".repeat(2048)}})).collect()
}

#[test]
fn immutable_history_keeps_130_entries_and_reads_only_selected_data() {
    let root = TestWorkspace::new("context-archive");
    let task_id = uuid::Uuid::new_v4().to_string();
    let original = items(130);
    let reference = publish(root.path(), &task_id, &original).unwrap();
    assert_eq!(reference["total"], 130);
    let index =
        read_selected(root.path(), &task_id, &reference, &["/capabilities".into()]).unwrap();
    assert_eq!(index.len(), 130);
    assert!(index.iter().all(|item| item.get("data").is_none()));
    let selected = read_selected(
        root.path(),
        &task_id,
        &reference,
        &["/capabilities/129/data/index".into()],
    )
    .unwrap();
    assert_eq!(selected[129]["data"]["index"], 129);
    assert_eq!(
        selected[129]["provenance"]["continuation_args"]["api_key"]["redacted"],
        true
    );
    assert!(!serde_json::to_string(&selected)
        .unwrap()
        .contains("private-fixture"));
    assert_eq!(
        publish(root.path(), &task_id, &original).unwrap(),
        reference
    );
}

#[test]
fn changed_source_and_digest_corruption_never_return_stale_history() {
    let root = TestWorkspace::new("context-archive-corrupt");
    let task_id = uuid::Uuid::new_v4().to_string();
    let reference = publish(root.path(), &task_id, &items(2)).unwrap();
    let paths = ["/capabilities/1/data".to_string()];
    assert!(read_selected(
        root.path(),
        &uuid::Uuid::new_v4().to_string(),
        &reference,
        &paths
    )
    .is_err());
    let manifest_path = object_path(
        root.path(),
        &task_id,
        reference["manifest_sha256"].as_str().unwrap(),
    )
    .unwrap();
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let item_path = object_path(root.path(), &task_id, &manifest.items[1].sha256).unwrap();
    std::fs::write(&item_path, b"modified").unwrap();
    assert_eq!(
        read_selected(root.path(), &task_id, &reference, &paths),
        Err("context_archive_corrupt")
    );
    std::fs::remove_file(&item_path).unwrap();
    assert_eq!(
        read_selected(root.path(), &task_id, &reference, &paths),
        Err("context_archive_unavailable")
    );
    // Index discovery does not require every large data object to be opened.
    assert_eq!(
        read_selected(root.path(), &task_id, &reference, &["/capabilities".into()])
            .unwrap()
            .len(),
        2
    );
    std::fs::write(manifest_path, b"modified").unwrap();
    assert_eq!(
        read_selected(root.path(), &task_id, &reference, &paths),
        Err("context_archive_corrupt")
    );
}

#[test]
fn over_budget_items_and_manifest_path_injection_fail_explicitly() {
    let root = TestWorkspace::new("context-archive-budget");
    let task_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        publish(
            root.path(),
            &task_id,
            &[json!({"data":"x".repeat(MAX_ITEM_BYTES)})]
        ),
        Err("context_archive_budget_exceeded")
    );
    assert!(object_path(root.path(), "..", &"0".repeat(64)).is_err());
    assert!(object_path(root.path(), &task_id, "../outside").is_err());
}

#[test]
fn task_owned_deletion_removes_history_and_cannot_be_read_from_old_reference() {
    let root = TestWorkspace::new("context-archive-delete");
    let task_id = uuid::Uuid::new_v4().to_string();
    let reference = publish(root.path(), &task_id, &items(2)).unwrap();
    let plan = crate::task_result_retention::files::prepare(root.path(), &task_id).unwrap();
    crate::task_result_retention::files::execute(root.path(), &plan).unwrap();
    assert_eq!(
        read_selected(root.path(), &task_id, &reference, &["/capabilities".into()]),
        Err("context_archive_unavailable")
    );
}

#[cfg(unix)]
#[test]
fn archive_read_never_follows_replaced_objects() {
    let root = TestWorkspace::new("context-archive-link");
    let outside = TestWorkspace::new("context-archive-link-outside");
    let task_id = uuid::Uuid::new_v4().to_string();
    let reference = publish(root.path(), &task_id, &items(1)).unwrap();
    let path = object_path(
        root.path(),
        &task_id,
        reference["manifest_sha256"].as_str().unwrap(),
    )
    .unwrap();
    let original = std::fs::read(&path).unwrap();
    let external = outside.path().join("object");
    std::fs::write(&external, original).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&external, &path).unwrap();
    assert_eq!(
        read_selected(root.path(), &task_id, &reference, &["/capabilities".into()]),
        Err("context_archive_unavailable")
    );
}

#[test]
fn stale_or_missing_claim_never_creates_an_archive_directory() {
    let root = TestWorkspace::new("context-archive-stale-claim");
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.skill_rt.workspace_root = root.path().to_path_buf();
    let task = crate::ClaimedTask {
        task_id: uuid::Uuid::new_v4().to_string(),
        claim_attempt: 1,
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    };
    let reference = capture(&state, &task, &[]);
    assert_eq!(reference["status"], "unavailable");
    assert_eq!(reference["error_code"], "context_archive_stale_claim");
    assert!(!root
        .path()
        .join(".agent-runtime/artifacts/canonical-evidence")
        .exists());
}

#[test]
fn unavailable_storage_never_publishes_an_available_reference() {
    let workspace = TestWorkspace::new("context-archive-storage-unavailable");
    let task_id = uuid::Uuid::new_v4().to_string();
    let blocked = claw_core::workspace_state::workspace_artifacts_root(workspace.path())
        .join("canonical-evidence");
    std::fs::create_dir_all(blocked.parent().unwrap()).unwrap();
    std::fs::write(&blocked, b"not a directory").unwrap();
    assert_eq!(
        publish(workspace.path(), &task_id, &items(1)).unwrap_err(),
        "context_archive_write_unavailable"
    );
    assert_eq!(std::fs::read(&blocked).unwrap(), b"not a directory");
}

#[test]
fn interrupted_object_sequence_has_no_manifest_and_retries_immutably() {
    let workspace = TestWorkspace::new("context-archive-partial-sequence");
    let task_id = uuid::Uuid::new_v4().to_string();
    let original = items(2);
    let mut interrupted = original.clone();
    interrupted[1] = json!({"data":"x".repeat(MAX_ITEM_BYTES)});
    assert_eq!(
        publish(workspace.path(), &task_id, &interrupted),
        Err("context_archive_budget_exceeded")
    );
    let directory = claw_core::workspace_state::workspace_artifacts_root(workspace.path())
        .join("canonical-evidence")
        .join(&task_id);
    let paths = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    let first_bytes = std::fs::read(&paths[0]).unwrap();
    assert!(serde_json::from_slice::<Manifest>(&first_bytes).is_err());
    let reference = publish(workspace.path(), &task_id, &original).unwrap();
    assert_eq!(std::fs::read(&paths[0]).unwrap(), first_bytes);
    let selected = read_selected(
        workspace.path(),
        &task_id,
        &reference,
        &["/capabilities/1/data/index".into()],
    )
    .unwrap();
    assert_eq!(selected[1]["data"]["index"], 1);
}

#[test]
fn manifest_publication_failure_does_not_claim_success_or_overwrite_corruption() {
    let workspace = TestWorkspace::new("context-archive-manifest-publication");
    let task_id = uuid::Uuid::new_v4().to_string();
    let original = items(2);
    let reference = publish(workspace.path(), &task_id, &original).unwrap();
    let manifest = object_path(
        workspace.path(),
        &task_id,
        reference["manifest_sha256"].as_str().unwrap(),
    )
    .unwrap();
    std::fs::write(&manifest, b"corrupt-manifest").unwrap();
    assert_eq!(
        publish(workspace.path(), &task_id, &original),
        Err("context_archive_write_unavailable")
    );
    assert_eq!(std::fs::read(&manifest).unwrap(), b"corrupt-manifest");
    assert_eq!(
        read_selected(
            workspace.path(),
            &task_id,
            &reference,
            &["/capabilities".into()]
        ),
        Err("context_archive_corrupt")
    );
    std::fs::remove_file(&manifest).unwrap();
    assert_eq!(
        publish(workspace.path(), &task_id, &original).unwrap(),
        reference
    );
}
