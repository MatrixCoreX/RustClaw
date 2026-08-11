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

fn append_artifact(
    result: &mut Value,
    task_id: &str,
    artifact_id: &str,
    filename: &str,
    kind: &str,
    mime_type: &str,
    size_bytes: u64,
) {
    result["artifacts"]
        .as_array_mut()
        .expect("artifacts array")
        .push(serde_json::json!({
            "schema_version": 1,
            "id": artifact_id,
            "filename": filename,
            "kind": kind,
            "mime_type": mime_type,
            "size_bytes": size_bytes,
            "sha256": "b".repeat(64),
            "download_url": format!("/v1/tasks/{task_id}/artifacts/{artifact_id}/content")
        }));
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
fn async_completion_delivery_flag_controls_model_independent_video_delivery() {
    let workspace = temp_workspace("task_delivery_async_video");
    let task_id = "task-async-video";
    let artifact_id = "artifact-video";
    let path = write_delivery_artifact(&workspace, task_id, artifact_id, "clip.mp4", b"video");
    let mut result = result_with_artifact(
        task_id,
        artifact_id,
        "clip.mp4",
        "video",
        "video/mp4",
        5,
        None,
    );
    result["task_journal"] = serde_json::json!({"trace": {
        "capability_results": [{"status": "waiting"}],
        "task_checkpoint": {"boundary_context": {
            "async_job_terminal_observation": {
                "schema_version": 1,
                "source": "async_job_completion_checkpoint",
                "status": "succeeded",
                "final_result_json": {"extra": {
                    "delivery": {"deliver_to_user": true, "intent": "artifact"}
                }}
            }
        }}
    }});

    let messages = merge_task_artifact_delivery_messages(
        task_id,
        Some(&result),
        &workspace,
        vec!["download complete".to_string()],
    );
    assert_eq!(
        messages,
        vec![format!(
            "download complete\nVIDEO_FILE:{}",
            path.canonicalize().expect("canonical video").display()
        )]
    );

    result["task_journal"]["trace"]["task_checkpoint"]["boundary_context"]
        ["async_job_terminal_observation"]["final_result_json"]["extra"]["delivery"]
        ["deliver_to_user"] = serde_json::json!(false);
    let messages = merge_task_artifact_delivery_messages(
        task_id,
        Some(&result),
        &workspace,
        vec!["download complete\nVIDEO_FILE:/old/clip.mp4".to_string()],
    );
    assert_eq!(messages, vec!["download complete"]);
    fs::remove_dir_all(workspace).ok();
}

#[test]
fn explicit_user_file_excludes_internal_skill_output_artifact() {
    let workspace = temp_workspace("task_delivery_explicit_selection");
    let task_id = "task-explicit-selection";
    let internal_id = "skill-output:search-result";
    let user_id = "transcript-result";
    write_delivery_artifact(
        &workspace,
        task_id,
        internal_id,
        "search-results.json",
        b"internal-json",
    );
    let user_path = write_delivery_artifact(
        &workspace,
        task_id,
        user_id,
        "image_text_ai.txt",
        b"user transcript",
    );
    let mut result = result_with_artifact(
        task_id,
        internal_id,
        "search-results.json",
        "file",
        "application/json",
        13,
        None,
    );
    append_artifact(
        &mut result,
        task_id,
        user_id,
        "image_text_ai.txt",
        "file",
        "text/plain",
        15,
    );

    let messages = merge_task_artifact_delivery_messages(
        task_id,
        Some(&result),
        &workspace,
        vec!["转写完成\nFILE:/source/image_text_ai.txt".to_string()],
    );

    assert_eq!(
        messages,
        vec![format!(
            "转写完成\nFILE:{}",
            user_path
                .canonicalize()
                .expect("canonical user file")
                .display()
        )]
    );
    assert!(!messages[0].contains("search-results.json"));
    fs::remove_dir_all(workspace).ok();
}

#[test]
fn legacy_internal_skill_output_token_is_removed_without_a_manifest() {
    let workspace = temp_workspace("task_delivery_internal_legacy_token");
    let internal = crate::workspace_state::workspace_artifacts_root(&workspace)
        .join("skill-output")
        .join("task-legacy")
        .join("system_basic.json");
    fs::create_dir_all(internal.parent().expect("internal artifact parent"))
        .expect("create internal artifact parent");
    fs::write(&internal, b"internal evidence").expect("write internal evidence");

    let messages = merge_task_artifact_delivery_messages(
        "task-legacy",
        None,
        &workspace,
        vec![format!(
            "处理完成\nFILE:{}",
            internal
                .canonicalize()
                .expect("canonical internal")
                .display()
        )],
    );

    assert_eq!(messages, vec!["处理完成"]);
    fs::remove_dir_all(workspace).ok();
}

#[test]
fn long_text_is_preserved_when_the_text_artifact_is_delivered() {
    let workspace = temp_workspace("task_delivery_long_text_and_artifact");
    let task_id = "task-long-text";
    let artifact_id = "artifact-transcript";
    let filename = "transcript.txt";
    let transcript = "多语言 transcript line\n"
        .repeat(600)
        .trim_end()
        .to_string();
    let artifact_path = write_delivery_artifact(
        &workspace,
        task_id,
        artifact_id,
        filename,
        transcript.as_bytes(),
    );
    let result = result_with_artifact(
        task_id,
        artifact_id,
        filename,
        "file",
        "text/plain",
        transcript.len() as u64,
        Some(true),
    );

    let messages = merge_task_artifact_delivery_messages(
        task_id,
        Some(&result),
        &workspace,
        vec![format!("{transcript}\nFILE:{filename}")],
    );

    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with(&transcript));
    assert_eq!(
        messages[0]
            .chars()
            .take(transcript.chars().count())
            .collect::<String>(),
        transcript
    );
    assert!(messages[0].ends_with(&format!(
        "FILE:{}",
        artifact_path
            .canonicalize()
            .expect("canonical text artifact")
            .display()
    )));
    fs::remove_dir_all(workspace).ok();
}

#[test]
fn explicitly_referenced_internal_manifest_is_still_never_delivered() {
    let workspace = temp_workspace("task_delivery_internal_explicit");
    let task_id = "task-internal-explicit";
    let artifact_id = "skill-output:internal-explicit";
    write_delivery_artifact(
        &workspace,
        task_id,
        artifact_id,
        "system_basic.json",
        b"internal evidence",
    );
    let result = result_with_artifact(
        task_id,
        artifact_id,
        "system_basic.json",
        "file",
        "application/json",
        17,
        None,
    );

    let messages = merge_task_artifact_delivery_messages(
        task_id,
        Some(&result),
        &workspace,
        vec!["处理完成\nFILE:system_basic.json".to_string()],
    );

    assert_eq!(messages, vec!["处理完成"]);
    fs::remove_dir_all(workspace).ok();
}

#[test]
fn internal_skill_output_is_not_auto_delivered_with_prose() {
    let workspace = temp_workspace("task_delivery_internal_default");
    let task_id = "task-internal-default";
    let artifact_id = "skill-output:search-result";
    write_delivery_artifact(
        &workspace,
        task_id,
        artifact_id,
        "search-results.json",
        b"internal-json",
    );
    let result = result_with_artifact(
        task_id,
        artifact_id,
        "search-results.json",
        "file",
        "application/json",
        13,
        None,
    );

    let messages = merge_task_artifact_delivery_messages(
        task_id,
        Some(&result),
        &workspace,
        vec!["检索已完成。".to_string()],
    );

    assert_eq!(messages, vec!["检索已完成。"]);
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
