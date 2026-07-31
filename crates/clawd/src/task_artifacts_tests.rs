use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Value};

use super::*;

struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-runtime-task-artifacts-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn materializes_structured_capability_output_into_task_delivery_storage() {
    let workspace = TempWorkspace::new();
    let output = workspace.path().join("document").join("月报.pdf");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, b"pdf-fixture").unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "data": {"extra": {
                "output_path": output.display().to_string(),
                "media_type": "application/pdf"
            }},
            "artifacts": []
        }]}}
    });

    let materialized =
        materialize_task_result_artifacts(workspace.path(), "task-123", &result.to_string())
            .unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&value));

    assert_eq!(manifests.len(), 1);
    assert_eq!(value["text"], "done");
    assert_eq!(manifests[0].filename, "月报.pdf");
    assert_eq!(manifests[0].kind, "pdf");
    assert_eq!(manifests[0].schema_version, TASK_ARTIFACT_SCHEMA_VERSION);
    assert_eq!(
        manifests[0].artifact_ref,
        format!("artifact:task/task-123/{}", manifests[0].id)
    );
    assert!(manifests[0].preview_url.is_some());
    let delivered = delivery_artifact_path(
        workspace.path(),
        "task-123",
        &manifests[0].id,
        &manifests[0].filename,
    );
    assert_eq!(fs::read(delivered).unwrap(), b"pdf-fixture");
}

#[test]
fn legacy_manifest_is_normalized_only_by_the_central_decoder() {
    let value = json!({
        "artifacts": [{
            "schema_version": 1,
            "id": "artifact-legacy",
            "filename": "legacy.txt",
            "kind": "file",
            "mime_type": "text/plain",
            "size_bytes": 6,
            "sha256": "a".repeat(64),
            "download_url": "/v1/tasks/task-legacy/artifacts/artifact-legacy/content"
        }]
    });

    let manifests = manifests_from_result(Some(&value));
    assert_eq!(manifests.len(), 1);
    assert_eq!(
        manifests[0].artifact_ref,
        "artifact:task/task-legacy/artifact-legacy"
    );

    let mut mismatched = value;
    mismatched["artifacts"][0]["schema_version"] = json!(2);
    mismatched["artifacts"][0]["artifact_ref"] = json!("artifact:task/other/artifact-legacy");
    assert!(manifests_from_result(Some(&mismatched)).is_empty());
}

#[test]
fn skips_dry_run_and_workspace_escape_paths() {
    let workspace = TempWorkspace::new();
    let planned = workspace.path().join("document").join("planned.mp4");
    let outside = std::env::temp_dir().join(format!("outside-{}.txt", uuid::Uuid::new_v4()));
    fs::create_dir_all(planned.parent().unwrap()).unwrap();
    fs::write(&planned, b"should-not-publish").unwrap();
    fs::write(&outside, b"outside").unwrap();
    let result = json!({
        "text": "done",
        "artifacts": [{"path": outside.display().to_string()}],
        "extra": {"dry_run": true, "output_path": planned.display().to_string()}
    });

    let materialized =
        materialize_task_result_artifacts(workspace.path(), "task-escape", &result.to_string())
            .unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();

    assert!(manifests_from_result(Some(&value)).is_empty());
    fs::remove_file(outside).unwrap();
}

#[test]
fn delivery_lookup_rejects_manifest_filename_traversal() {
    let workspace = TempWorkspace::new();
    let manifest = TaskArtifactManifest {
        schema_version: 1,
        id: "artifact-1".to_string(),
        artifact_ref: String::new(),
        filename: "../../secret".to_string(),
        kind: "file".to_string(),
        mime_type: "application/octet-stream".to_string(),
        size_bytes: 1,
        sha256: "a".repeat(64),
        download_url: "/v1/tasks/task-1/artifacts/artifact-1/content".to_string(),
        preview_url: None,
    };

    assert!(validated_delivery_artifact_path(workspace.path(), "task-1", &manifest).is_none());
}

#[test]
fn cleanup_removes_only_delivery_directories_without_tasks() {
    let workspace = TempWorkspace::new();
    let db = Connection::open_in_memory().unwrap();
    db.execute("CREATE TABLE tasks (task_id TEXT PRIMARY KEY)", [])
        .unwrap();
    db.execute("INSERT INTO tasks (task_id) VALUES ('task-live')", [])
        .unwrap();
    let root = workspace.path().join(".agent-runtime/artifacts/delivery");
    fs::create_dir_all(root.join("task-live")).unwrap();
    fs::create_dir_all(root.join("task-gone")).unwrap();

    assert_eq!(
        cleanup_orphaned_delivery_artifacts(workspace.path(), &db).unwrap(),
        1
    );
    assert!(root.join("task-live").is_dir());
    assert!(!root.join("task-gone").exists());
}

#[test]
fn svg_and_html_are_never_inline_previewed() {
    assert!(!inline_preview_allowed("image/svg+xml"));
    assert!(!inline_preview_allowed("text/html"));
    assert!(inline_preview_allowed("image/png"));
    assert!(inline_preview_allowed("video/mp4"));
}
