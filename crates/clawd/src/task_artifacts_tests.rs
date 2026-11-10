use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Value};

use super::*;

struct TempWorkspace(PathBuf);

fn generated_revision(workspace: &Path, task_id: &str, text: &str, replaces: Vec<String>) -> Value {
    let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    let namespace = format!("generated-file-{}", &digest[..32]);
    let mut artifact = crate::skill_output_artifact::publish_task_text_artifact(
        workspace,
        task_id,
        &namespace,
        "result.txt",
        text,
        json!({"producer":"filesystem_text_delivery", "replaces_artifact_refs":replaces}),
    )
    .unwrap()
    .artifact_ref;
    artifact["owner_task_id"] = json!(task_id);
    artifact["id"] = json!(format!("a_{}", &digest[..32]));
    artifact["visibility"] = json!("user_delivery");
    artifact
}

#[test]
fn explicit_file_revision_replaces_old_review_but_preserves_unrelated_delivery() {
    let workspace = TempWorkspace::new();
    let task_id = "revision-task";
    let old = crate::skill_output_artifact::publish_task_text_artifact(
        workspace.path(),
        task_id,
        "transcript-review",
        "transcript.txt",
        "old translation",
        json!({}),
    )
    .unwrap()
    .artifact_ref;
    let image = workspace.path().join("image.png");
    fs::write(&image, b"image-fixture").unwrap();
    let old_ref = claw_core::task_delivery_artifacts::canonical_task_artifact_ref(
        task_id,
        old["id"].as_str().unwrap(),
    )
    .unwrap();
    let current = generated_revision(workspace.path(), task_id, "correct English", vec![old_ref]);
    let result = json!({"task_journal":{"trace":{"capability_results":[
        {"status":"ok","artifacts":[old, {"path":image}]},
        {"status":"ok","artifacts":[current]}
    ]}}});
    let materialized =
        materialize_task_result_artifacts(workspace.path(), task_id, &result.to_string()).unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&value));
    assert_eq!(manifests.len(), 2);
    assert!(manifests
        .iter()
        .any(|manifest| manifest.filename == "image.png"));
    let text = manifests
        .iter()
        .find(|manifest| manifest.filename == "result.txt")
        .unwrap();
    assert_eq!(
        fs::read_to_string(delivery_artifact_path(
            workspace.path(),
            task_id,
            &text.id,
            &text.filename
        ))
        .unwrap(),
        "correct English"
    );
    assert!(!manifests
        .iter()
        .any(|manifest| manifest.id.starts_with("transcript-review:")));
    // Re-materialization must not resurrect an old preserved delivery copy.
    let repeat =
        materialize_task_result_artifacts(workspace.path(), task_id, &materialized).unwrap();
    assert_eq!(
        manifests_from_result(Some(&serde_json::from_str::<Value>(&repeat).unwrap())),
        manifests
    );
}

#[test]
fn invalid_revision_never_suppresses_valid_prior_artifact() {
    for invalid in ["tampered", "missing", "cross_task"] {
        let workspace = TempWorkspace::new();
        let old_path = workspace.path().join("old.txt");
        fs::write(&old_path, b"old valid").unwrap();
        let mut current = generated_revision(
            workspace.path(),
            "task-a",
            "current",
            vec!["artifact:task/task-a/old".into()],
        );
        let path = workspace.path().join(current["path"].as_str().unwrap());
        match invalid {
            "tampered" => fs::write(path, b"tampered bytes").unwrap(),
            "missing" => fs::remove_file(path).unwrap(),
            _ => current["owner_task_id"] = json!("task-b"),
        }
        let result = json!({"task_journal":{"trace":{"capability_results":[
            {"status":"ok","artifacts":[{"id":"old","path":old_path}]},
            {"status":"ok","artifacts":[current]}
        ]}}});
        let output =
            materialize_task_result_artifacts(workspace.path(), "task-a", &result.to_string())
                .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert!(
            manifests_from_result(Some(&value))
                .iter()
                .any(|manifest| manifest.id == "old"),
            "{invalid}"
        );
    }
}

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
    assert_eq!(value["artifact_delivery"]["candidate_count"], 1);
    assert_eq!(value["artifact_delivery"]["delivered_count"], 1);
    assert_eq!(value["artifact_delivery"]["truncated"], false);
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
fn materialization_deduplicates_files_with_identical_content() {
    let workspace = TempWorkspace::new();
    let first = workspace.path().join("downloads").join("first.webp");
    let second = workspace.path().join("downloads").join("second.webp");
    fs::create_dir_all(first.parent().unwrap()).unwrap();
    fs::write(&first, b"identical-image").unwrap();
    fs::write(&second, b"identical-image").unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "data": {"extra": {
                "delivery": {"deliver_to_user": true},
                "artifacts": [
                    {"path": first.display().to_string(), "mime_type": "image/webp"},
                    {"path": second.display().to_string(), "mime_type": "image/webp"}
                ]
            }}
        }]}}
    });

    let materialized = materialize_task_result_artifacts(
        workspace.path(),
        "task-duplicate-content",
        &result.to_string(),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&value));

    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].filename, "first.webp");
    assert_eq!(value["artifact_delivery"]["candidate_count"], 2);
    assert_eq!(value["artifact_delivery"]["delivered_count"], 1);
    assert_eq!(value["artifact_delivery"]["truncated"], false);
}

#[test]
fn materializes_trusted_async_completion_artifact_without_model_file_token() {
    let workspace = TempWorkspace::new();
    let output = workspace.path().join("downloads").join("clip.mp4");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, b"video-fixture").unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {
            "capability_results": [{"status": "waiting"}],
            "task_checkpoint": {"boundary_context": {
                "async_job_terminal_observation": {
                    "schema_version": 1,
                    "source": "async_job_completion_checkpoint",
                    "status": "succeeded",
                    "final_result_json": {"extra": {
                        "delivery": {"deliver_to_user": true, "intent": "artifact"},
                        "artifacts": [{
                            "path": output.display().to_string(),
                            "filename": "clip.mp4",
                            "mime_type": "video/mp4"
                        }]
                    }}
                }
            }}
        }}
    });

    let materialized =
        materialize_task_result_artifacts(workspace.path(), "task-async", &result.to_string())
            .unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&value));

    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].filename, "clip.mp4");
    assert_eq!(manifests[0].kind, "video");
    assert_eq!(manifests[0].mime_type, "video/mp4");
}

#[test]
fn async_poll_completion_preserves_artifact_before_resume_projection() {
    let workspace = TempWorkspace::new();
    let output = workspace.path().join("downloads").join("clip.mp4");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, b"video-fixture").unwrap();
    let skill_result = json!({
        "status": "ok",
        "extra": {
            "delivery": {"deliver_to_user": true, "intent": "artifact"},
            "artifacts": [{
                "id": "video-1",
                "path": output.display().to_string(),
                "filename": "clip.mp4",
                "mime_type": "video/mp4",
                "size_bytes": 13
            }]
        }
    });
    let payload = json!({
        "executor_result_status": "async_poll_completed",
        "final_result_json": {
            "source": "local_process_async_job",
            "exit_code": 0,
            "output": skill_result.to_string()
        }
    });

    assert!(
        preserve_async_completion_artifacts(workspace.path(), "task-async-preserve", &payload,)
            .unwrap()
    );
    let delivered = delivery_artifact_path(
        workspace.path(),
        "task-async-preserve",
        "video-1",
        "clip.mp4",
    );
    assert_eq!(fs::read(delivered).unwrap(), b"video-fixture");
}

#[test]
fn final_projection_restores_every_artifact_preserved_before_journal_compaction() {
    let workspace = TempWorkspace::new();
    let downloads = workspace.path().join("downloads");
    fs::create_dir_all(&downloads).unwrap();
    let first = downloads.join("01.jpg");
    let second = downloads.join("02.jpg");
    let article = downloads.join("article.txt");
    fs::write(&first, b"first-image").unwrap();
    fs::write(&second, b"second-image").unwrap();
    fs::write(&article, b"article-body").unwrap();
    let first_digest = sha256_file(&first).unwrap();
    let skill_result = json!({
        "status": "ok",
        "extra": {
            "delivery": {"deliver_to_user": true, "intent": "artifact"},
            "artifacts": [
                {"path": first.display().to_string(), "filename": "01.jpg", "mime_type": "image/jpeg"},
                {"path": second.display().to_string(), "filename": "02.jpg", "mime_type": "image/jpeg"},
                {"path": article.display().to_string(), "filename": "article.txt", "mime_type": "text/plain"}
            ]
        }
    });
    let payload = json!({
        "executor_result_status": "async_poll_completed",
        "final_result_json": skill_result
    });
    let task_id = "task-compacted-artifacts";
    assert!(preserve_async_completion_artifacts(workspace.path(), task_id, &payload).unwrap());
    fs::remove_file(&first).unwrap();
    fs::remove_file(&second).unwrap();
    fs::remove_file(&article).unwrap();

    let compacted = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "artifacts": [{
                "id": "first-image",
                "path": first.display().to_string(),
                "filename": "01.jpg",
                "media_type": "image/jpeg",
                "sha256": first_digest,
                "visibility": "user_delivery"
            }],
            "data": {}
        }]}}
    });
    let materialized =
        materialize_task_result_artifacts(workspace.path(), task_id, &compacted.to_string())
            .unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&value));
    let mut filenames = manifests
        .iter()
        .map(|manifest| manifest.filename.as_str())
        .collect::<Vec<_>>();
    filenames.sort_unstable();

    assert_eq!(filenames, ["01.jpg", "02.jpg", "article.txt"]);
    assert_eq!(value["artifact_delivery"]["candidate_count"], 3);
    assert_eq!(value["artifact_delivery"]["delivered_count"], 3);
}

#[test]
fn rematerializes_from_durable_task_copy_after_async_source_cleanup() {
    let workspace = TempWorkspace::new();
    let output = workspace.path().join("downloads").join("clip.mp4");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, b"video-fixture").unwrap();
    let digest = sha256_file(&output).unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "artifacts": [{
                "id": "video-1",
                "path": output.display().to_string(),
                "filename": "clip.mp4",
                "media_type": "video/mp4",
                "size_bytes": 13,
                "sha256": digest,
                "visibility": "user_delivery"
            }],
            "data": {}
        }]}}
    });

    let first =
        materialize_task_result_artifacts(workspace.path(), "task-async-copy", &result.to_string())
            .unwrap();
    let first: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(manifests_from_result(Some(&first)).len(), 1);
    fs::remove_file(&output).unwrap();

    let second =
        materialize_task_result_artifacts(workspace.path(), "task-async-copy", &result.to_string())
            .unwrap();
    let second: Value = serde_json::from_str(&second).unwrap();
    let manifests = manifests_from_result(Some(&second));
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].id, "video-1");
    assert_eq!(manifests[0].sha256, digest);
}

#[test]
fn rematerializes_from_digest_when_durable_copy_used_a_derived_id() {
    let workspace = TempWorkspace::new();
    let output = workspace.path().join("downloads").join("clip.mp4");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, b"video-fixture").unwrap();
    let digest = sha256_file(&output).unwrap();
    let durable = delivery_artifact_path(
        workspace.path(),
        "task-digest-copy",
        "artifact-derived",
        "clip.mp4",
    );
    fs::create_dir_all(durable.parent().unwrap()).unwrap();
    fs::copy(&output, &durable).unwrap();
    fs::remove_file(&output).unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "artifacts": [{
                "id": "a_bound_video",
                "path": output.display().to_string(),
                "filename": "clip.mp4",
                "media_type": "video/mp4",
                "size_bytes": 13,
                "sha256": digest,
                "visibility": "user_delivery"
            }],
            "data": {}
        }]}}
    });

    let materialized = materialize_task_result_artifacts(
        workspace.path(),
        "task-digest-copy",
        &result.to_string(),
    )
    .unwrap();
    let materialized: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&materialized));
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].id, "a_bound_video");
    assert_eq!(manifests[0].sha256, digest);
}

#[test]
fn materializes_extracted_audio_from_processing_inputs() {
    let workspace = TempWorkspace::new();
    let audio = workspace
        .path()
        .join("downloads")
        .join("clip_video_audio.wav");
    fs::create_dir_all(audio.parent().unwrap()).unwrap();
    fs::write(&audio, b"audio-fixture").unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "data": {"extra": {
                "delivery": {"deliver_to_user": true, "intent": "artifact"},
                "processing_inputs": {
                    "video_audio": {
                        "status": "available",
                        "path": audio.display().to_string(),
                        "filename": "clip_video_audio.wav",
                        "mime_type": "audio/x-wav",
                        "size_bytes": 13
                    }
                }
            }}
        }]}}
    });

    let materialized = materialize_task_result_artifacts(
        workspace.path(),
        "task-audio-input",
        &result.to_string(),
    )
    .unwrap();
    let materialized: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&materialized));
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].filename, "clip_video_audio.wav");
    assert_eq!(manifests[0].kind, "audio");
}

#[test]
fn rejects_untrusted_or_failed_async_completion_artifacts() {
    let workspace = TempWorkspace::new();
    let output = workspace.path().join("downloads").join("clip.mp4");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, b"video-fixture").unwrap();

    for (source, status) in [
        ("untrusted_completion", "succeeded"),
        ("async_job_completion_checkpoint", "failed"),
    ] {
        let result = json!({
            "text": "done",
            "task_journal": {"trace": {"task_checkpoint": {"boundary_context": {
                "async_job_terminal_observation": {
                    "schema_version": 1,
                    "source": source,
                    "status": status,
                    "final_result_json": {"extra": {"artifacts": [{
                        "path": output.display().to_string(),
                        "mime_type": "video/mp4"
                    }]}}
                }
            }}}}
        });
        let materialized = materialize_task_result_artifacts(
            workspace.path(),
            "task-untrusted",
            &result.to_string(),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&materialized).unwrap();
        assert!(manifests_from_result(Some(&value)).is_empty());
    }
}

#[test]
fn never_materializes_internal_skill_output_records() {
    let workspace = TempWorkspace::new();
    let output = workspace
        .path()
        .join(".agent-runtime/artifacts/skill-output/task-1/system_basic.json");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, b"{\"internal\":true}").unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "artifacts": [{
                "id": "skill-output:invocation-1",
                "path": output.display().to_string(),
                "media_type": "application/json"
            }],
            "data": {}
        }]}}
    });

    let materialized =
        materialize_task_result_artifacts(workspace.path(), "task-internal", &result.to_string())
            .unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();

    assert!(manifests_from_result(Some(&value)).is_empty());
}

#[test]
fn typed_internal_and_evidence_artifacts_are_never_user_delivery() {
    let workspace = TempWorkspace::new();
    let internal = workspace.path().join("private").join("audio.wav");
    let evidence = workspace.path().join("private").join("trace.json");
    fs::create_dir_all(internal.parent().unwrap()).unwrap();
    fs::write(&internal, b"audio").unwrap();
    fs::write(&evidence, b"{}").unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "artifacts": [
                {
                    "artifact_ref": "artifact:task/task-private/a_audio",
                    "owner_task_id": "task-private",
                    "path": internal.display().to_string(),
                    "visibility": "internal_processing"
                },
                {
                    "artifact_ref": "artifact:task/task-private/a_trace",
                    "owner_task_id": "task-private",
                    "path": evidence.display().to_string(),
                    "visibility": "evidence"
                }
            ],
            "data": {}
        }]}}
    });

    let materialized =
        materialize_task_result_artifacts(workspace.path(), "task-private", &result.to_string())
            .unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();

    assert!(manifests_from_result(Some(&value)).is_empty());
}

#[test]
fn explicit_save_only_capability_never_materializes_its_files() {
    let workspace = TempWorkspace::new();
    let output = workspace.path().join("private").join("audio.wav");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, b"audio").unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "artifacts": [{
                "id": "private-audio",
                "path": output.display().to_string(),
                "media_type": "audio/wav"
            }],
            "data": {"extra": {
                "delivery": {"intent": "save_only", "deliver_to_user": false}
            }}
        }]}}
    });

    let materialized =
        materialize_task_result_artifacts(workspace.path(), "task-private", &result.to_string())
            .unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();

    assert!(manifests_from_result(Some(&value)).is_empty());
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
fn materializes_ocr_txt_from_invocation_digest_when_journal_path_is_truncated() {
    let workspace = TempWorkspace::new();
    let task_id = "task-ocr-truncated-path";
    let ocr = workspace
        .path()
        .join(".agent-runtime/artifacts/skill-invocations")
        .join(task_id)
        .join("image_vision")
        .join("594655cf-7fca-4159-966d-8d68c3de9734")
        .join("image_text_ai.txt");
    fs::create_dir_all(ocr.parent().unwrap()).unwrap();
    let ocr_bytes = "图转文识别结果\n";
    fs::write(&ocr, ocr_bytes).unwrap();
    let digest = sha256_file(&ocr).unwrap();
    let size_bytes = ocr_bytes.len() as u64;
    let truncated_path = format!(
        "{}...(truncated)",
        &ocr.to_string_lossy()[..ocr.to_string_lossy().chars().count().min(80)]
    );
    let artifact_id = format!("a_{}", &digest[..32]);
    let result = json!({
        "text": format!("FILE:artifact:task/{task_id}/{artifact_id}"),
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "capability": "image_vision.extract_text",
            "delivery": {"intent": "artifact", "constraints": {"deliver_to_user": true}},
            "artifacts": [{
                "id": artifact_id,
                "path": truncated_path,
                "filename": "image_text_ai.txt",
                "media_type": "text/plain; charset=utf-8",
                "size_bytes": size_bytes,
                "sha256": digest,
                "visibility": "user_delivery"
            }],
            "data": {"extra": {
                "delivery": {"deliver_to_user": true, "intent": "artifact"}
            }}
        }]}}
    });

    let materialized =
        materialize_task_result_artifacts(workspace.path(), task_id, &result.to_string()).unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&value));
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].filename, "image_text_ai.txt");
    assert_eq!(manifests[0].id, format!("a_{}", &digest[..32]));
    assert_eq!(value["artifact_delivery"]["candidate_count"], 1);
    assert_eq!(value["artifact_delivery"]["delivered_count"], 1);
    let delivered = delivery_artifact_path(
        workspace.path(),
        task_id,
        &manifests[0].id,
        &manifests[0].filename,
    );
    assert_eq!(fs::read(delivered).unwrap(), "图转文识别结果\n".as_bytes());
}

#[test]
fn materializes_published_transcript_txt_even_when_journal_omits_transcribe() {
    let workspace = TempWorkspace::new();
    let task_id = "task-transcript-only";
    let transcript = workspace
        .path()
        .join(".agent-runtime/artifacts/transcript-review")
        .join(task_id)
        .join("transcript.txt");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(&transcript, "杭州限狗令\n").unwrap();
    let image = workspace.path().join("downloads").join("post.webp");
    fs::create_dir_all(image.parent().unwrap()).unwrap();
    fs::write(&image, b"image-bytes").unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [{
            "status": "ok",
            "capability": "media_download.download",
            "data": {"extra": {
                "delivery": {"deliver_to_user": true},
                "artifacts": [{
                    "path": image.display().to_string(),
                    "filename": "post.webp",
                    "mime_type": "image/webp"
                }]
            }}
        }]}}
    });

    let materialized =
        materialize_task_result_artifacts(workspace.path(), task_id, &result.to_string()).unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&value));
    assert_eq!(manifests.len(), 2);
    assert!(manifests
        .iter()
        .any(|manifest| manifest.filename == "post.webp"));
    let transcript_manifest = manifests
        .iter()
        .find(|manifest| manifest.filename == "transcript.txt")
        .expect("transcript.txt");
    assert!(transcript_manifest.id.starts_with("transcript-review:"));
    assert_eq!(transcript_manifest.kind, "file");
    let delivered = delivery_artifact_path(
        workspace.path(),
        task_id,
        &transcript_manifest.id,
        &transcript_manifest.filename,
    );
    assert_eq!(fs::read(delivered).unwrap(), "杭州限狗令\n".as_bytes());
}

#[test]
fn materializes_image_text_ai_from_skill_invocation_when_journal_omits_extract_text() {
    let workspace = TempWorkspace::new();
    let task_id = "task-ocr-omitted-journal";
    let invocation = workspace
        .path()
        .join(".agent-runtime/artifacts/skill-invocations")
        .join(task_id)
        .join("image_vision")
        .join("39b145a9-91da-4aea-a9c3-f67fb464b567");
    fs::create_dir_all(&invocation).unwrap();
    let reviewed = invocation.join("image_text_ai.txt");
    let raw = invocation.join("image_text_ai_raw.txt");
    fs::write(&reviewed, "家庭如何养出优秀孩子\n").unwrap();
    fs::write(&raw, "raw-unreviewed\n").unwrap();
    let image = workspace.path().join("downloads").join("post.webp");
    fs::create_dir_all(image.parent().unwrap()).unwrap();
    fs::write(&image, b"image-bytes").unwrap();
    let result = json!({
        "text": "done",
        "task_journal": {"trace": {"capability_results": [
            {"status":"error","capability":"agent.subagent"},
            {"status":"ok","capability":"task.plan_set"},
            {"status":"ok","capability":"task.plan_update"},
            {
                "status":"ok",
                "capability":"media_download.download",
                "data":{"extra":{
                    "delivery":{"deliver_to_user":true},
                    "artifacts":[{
                        "path": image.display().to_string(),
                        "filename": "post.webp",
                        "mime_type": "image/webp"
                    }]
                }}
            }
        ]}}
    });

    let materialized =
        materialize_task_result_artifacts(workspace.path(), task_id, &result.to_string()).unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();
    let manifests = manifests_from_result(Some(&value));
    assert!(
        manifests
            .iter()
            .any(|manifest| manifest.filename == "post.webp"),
        "download image should still materialize"
    );
    let ocr = manifests
        .iter()
        .find(|manifest| manifest.filename == "image_text_ai.txt")
        .expect("image_text_ai.txt");
    assert_eq!(ocr.kind, "file");
    assert!(!manifests
        .iter()
        .any(|manifest| manifest.filename == "image_text_ai_raw.txt"));
    let delivered = delivery_artifact_path(workspace.path(), task_id, &ocr.id, &ocr.filename);
    assert_eq!(
        fs::read(delivered).unwrap(),
        "家庭如何养出优秀孩子\n".as_bytes()
    );
}

#[test]
fn svg_and_html_are_never_inline_previewed() {
    assert!(!inline_preview_allowed("image/svg+xml"));
    assert!(!inline_preview_allowed("text/html"));
    assert!(inline_preview_allowed("image/png"));
    assert!(inline_preview_allowed("video/mp4"));
}
