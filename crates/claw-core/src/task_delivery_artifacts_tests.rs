use std::fs;

use super::*;

fn temp_workspace(name: &str) -> PathBuf {
    let unique = format!(
        "{}_{}_{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    );
    let root = std::env::temp_dir().join(unique);
    fs::create_dir_all(&root).expect("create temp workspace");
    root
}

fn result_with_artifact(
    task_id: &str,
    artifact_id: &str,
    filename: &str,
    kind: &str,
    mime_type: &str,
    size_bytes: u64,
    deliver_to_user: Option<bool>,
) -> Value {
    let mut result = serde_json::json!({
        "artifacts": [{
            "schema_version": 1,
            "id": artifact_id,
            "filename": filename,
            "kind": kind,
            "mime_type": mime_type,
            "size_bytes": size_bytes,
            "sha256": "a".repeat(64),
            "download_url": format!("/v1/tasks/{task_id}/artifacts/{artifact_id}/content")
        }]
    });
    if let Some(deliver_to_user) = deliver_to_user {
        result["task_journal"] = serde_json::json!({
            "trace": {
                "capability_results": [{
                    "data": {"extra": {"delivery": {"deliver_to_user": deliver_to_user}}}
                }]
            }
        });
    }
    result
}

fn write_delivery_artifact(
    workspace: &Path,
    task_id: &str,
    artifact_id: &str,
    filename: &str,
    body: &[u8],
) -> PathBuf {
    let path = crate::workspace_state::workspace_artifacts_root(workspace)
        .join("delivery")
        .join(task_id)
        .join(artifact_id)
        .join(filename);
    fs::create_dir_all(path.parent().expect("artifact parent")).expect("create artifact parent");
    fs::write(&path, body).expect("write artifact");
    path
}

#[test]
fn canonical_artifact_reference_is_stable_and_rejects_unsafe_components() {
    assert_eq!(
        canonical_task_artifact_ref("task-1", "artifact-1").as_deref(),
        Some("artifact:task/task-1/artifact-1")
    );
    assert!(canonical_task_artifact_ref("../task", "artifact-1").is_none());
    assert!(canonical_task_artifact_ref("task-1", "artifact/1").is_none());
}

#[test]
fn structured_image_replaces_legacy_token_and_keeps_caption() {
    let workspace = temp_workspace("task_delivery_image");
    let task_id = "task-1";
    let artifact_id = "artifact-1";
    let path = write_delivery_artifact(
        &workspace,
        task_id,
        artifact_id,
        "photo.jpg",
        b"image-bytes",
    );
    let result = result_with_artifact(
        task_id,
        artifact_id,
        "photo.jpg",
        "image",
        "image/jpeg",
        11,
        None,
    );

    let messages = merge_task_artifact_delivery_messages(
        task_id,
        Some(&result),
        &workspace,
        vec!["下载完成\nIMAGE_FILE:/old/source/photo.jpg".to_string()],
    );

    assert_eq!(messages.len(), 1);
    let canonical_path = path.canonicalize().expect("canonical artifact path");
    assert_eq!(
        messages[0],
        format!("下载完成\nIMAGE_FILE:{}", canonical_path.display())
    );
    fs::remove_dir_all(workspace).ok();
}

#[test]
fn structured_image_is_added_when_model_only_returns_prose() {
    let workspace = temp_workspace("task_delivery_prose");
    let task_id = "task-2";
    let artifact_id = "artifact-2";
    let path = write_delivery_artifact(&workspace, task_id, artifact_id, "photo.png", b"png");
    let result = result_with_artifact(
        task_id,
        artifact_id,
        "photo.png",
        "image",
        "image/png",
        3,
        Some(true),
    );

    let messages = merge_task_artifact_delivery_messages(
        task_id,
        Some(&result),
        &workspace,
        vec!["已下载并发送。".to_string()],
    );

    let canonical_path = path.canonicalize().expect("canonical artifact path");
    assert_eq!(
        messages,
        vec![format!(
            "已下载并发送。\nIMAGE_FILE:{}",
            canonical_path.display()
        )]
    );
    fs::remove_dir_all(workspace).ok();
}

#[test]
fn explicit_no_delivery_removes_legacy_tokens() {
    let workspace = temp_workspace("task_delivery_disabled");
    let task_id = "task-3";
    let artifact_id = "artifact-3";
    let result = result_with_artifact(
        task_id,
        artifact_id,
        "photo.jpg",
        "image",
        "image/jpeg",
        4,
        Some(false),
    );

    let messages = merge_task_artifact_delivery_messages(
        task_id,
        Some(&result),
        &workspace,
        vec!["保存在本地。\nIMAGE_FILE:/old/source/photo.jpg".to_string()],
    );

    assert_eq!(messages, vec!["保存在本地。"]);
    fs::remove_dir_all(workspace).ok();
}

#[test]
fn unavailable_structured_artifact_keeps_legacy_delivery_message() {
    let workspace = temp_workspace("task_delivery_missing");
    let task_id = "task-4";
    let result = result_with_artifact(
        task_id,
        "artifact-4",
        "photo.jpg",
        "image",
        "image/jpeg",
        4,
        None,
    );
    let original = vec!["IMAGE_FILE:/old/source/photo.jpg".to_string()];

    let messages =
        merge_task_artifact_delivery_messages(task_id, Some(&result), &workspace, original.clone());

    assert_eq!(messages, original);
    fs::remove_dir_all(workspace).ok();
}

#[test]
fn ui_fallback_only_accepts_files_in_managed_delivery_tree() {
    let workspace = temp_workspace("task_delivery_fallback");
    let managed =
        write_delivery_artifact(&workspace, "task-5", "artifact-5", "report.bin", b"managed");
    let unmanaged = workspace.join("outside.bin");
    fs::write(&unmanaged, b"unmanaged").expect("write unmanaged file");

    assert!(is_managed_task_delivery_artifact_path(&workspace, &managed));
    assert!(!is_managed_task_delivery_artifact_path(
        &workspace, &unmanaged
    ));
    assert!(!is_managed_task_delivery_artifact_path(
        &workspace,
        &workspace.join("missing.bin")
    ));
    fs::remove_dir_all(workspace).ok();
}
