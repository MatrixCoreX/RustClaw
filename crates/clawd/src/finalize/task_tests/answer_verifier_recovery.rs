#[test]
fn answer_verifier_recovery_terminal_marker_skips_finalize_reverification() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "summarize evidence");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_stop_signal(
        crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL,
    );
    journal.answer_verifier_summary = None;

    assert!(super::super::answer_verifier_recovery_already_terminal(
        &journal
    ));

    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: "still checking".to_string(),
        should_retry: true,
        retry_instruction: "retry".to_string(),
        confidence: 0.9,
    });

    assert!(!super::super::answer_verifier_recovery_already_terminal(
        &journal
    ));
}

#[test]
fn answer_verifier_retry_preserves_only_verified_current_task_delivery_tokens() {
    use claw_core::capability_result::{ArtifactRef, ArtifactVisibility, CapabilityResultEnvelope};

    fn artifact(task_id: &str, artifact_id: &str, visibility: ArtifactVisibility) -> ArtifactRef {
        ArtifactRef {
            artifact_ref: claw_core::task_delivery_artifacts::canonical_task_artifact_ref(
                task_id,
                artifact_id,
            ),
            id: Some(artifact_id.to_string()),
            path: Some(format!("/private/{artifact_id}")),
            uri: None,
            media_type: Some("image/webp".to_string()),
            filename: Some(format!("{artifact_id}.webp")),
            artifact_role: Some("original_image".to_string()),
            size_bytes: Some(12),
            sha256: Some("a".repeat(64)),
            visibility: Some(visibility),
            owner_task_id: Some(task_id.to_string()),
            producer: None,
            lease: None,
            metadata: serde_json::json!({}),
        }
    }

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "process the current media");
    let mut result = CapabilityResultEnvelope::ok("media.download", None, serde_json::json!({}));
    result.artifacts.push(artifact(
        "task-1",
        "image-1",
        ArtifactVisibility::UserDelivery,
    ));
    result.artifacts.push(artifact(
        "task-1",
        "image-2",
        ArtifactVisibility::UserDelivery,
    ));
    result.artifacts.push(artifact(
        "task-1",
        "internal-1",
        ArtifactVisibility::InternalProcessing,
    ));
    result.artifacts.push(artifact(
        "other-task",
        "foreign-1",
        ArtifactVisibility::UserDelivery,
    ));
    journal.capability_results.push(result);

    let rejected = concat!(
        "Result\n",
        "- image: IMAGE_FILE:artifact:task/task-1/image-1\n",
        "- internal: FILE:artifact:task/task-1/internal-1\n",
        "- foreign: FILE:artifact:task/other-task/foreign-1"
    );
    let rewritten = super::super::preserve_verified_delivery_tokens_after_retry(
        &journal,
        rejected,
        "rewritten result with paths only".to_string(),
    );

    assert_eq!(
        rewritten,
        concat!(
            "rewritten result with paths only\n",
            "- image: IMAGE_FILE:artifact:task/task-1/image-1\n",
            "IMAGE_FILE:artifact:task/task-1/image-1\n",
            "IMAGE_FILE:artifact:task/task-1/image-2"
        )
    );
}

#[test]
fn answer_verifier_retry_does_not_duplicate_retained_delivery_token() {
    use claw_core::capability_result::{ArtifactRef, ArtifactVisibility, CapabilityResultEnvelope};

    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "deliver file");
    let mut result = CapabilityResultEnvelope::ok("file.create", None, serde_json::json!({}));
    result.artifacts.push(ArtifactRef {
        artifact_ref: Some("artifact:task/task-1/file-1".to_string()),
        id: Some("file-1".to_string()),
        path: Some("/private/file-1.txt".to_string()),
        uri: None,
        media_type: Some("text/plain".to_string()),
        filename: Some("file-1.txt".to_string()),
        artifact_role: None,
        size_bytes: Some(4),
        sha256: Some("b".repeat(64)),
        visibility: Some(ArtifactVisibility::UserDelivery),
        owner_task_id: Some("task-1".to_string()),
        producer: None,
        lease: None,
        metadata: serde_json::json!({}),
    });
    journal.capability_results.push(result);

    let token = "FILE:artifact:task/task-1/file-1";
    let rewritten = super::super::preserve_verified_delivery_tokens_after_retry(
        &journal,
        token,
        token.to_string(),
    );

    assert_eq!(rewritten, token);
}

#[test]
fn answer_verifier_retry_keeps_detail_and_adds_standalone_delivery_line() {
    use claw_core::capability_result::{ArtifactRef, ArtifactVisibility, CapabilityResultEnvelope};

    let token = "IMAGE_FILE:artifact:task/task-1/image-1";
    let detail = format!("- image.webp · image/webp · 12 bytes: {token}");
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "deliver image");
    let mut result = CapabilityResultEnvelope::ok("image.create", None, serde_json::json!({}));
    result.artifacts.push(ArtifactRef {
        artifact_ref: Some("artifact:task/task-1/image-1".to_string()),
        id: Some("image-1".to_string()),
        path: Some("/private/image-1.webp".to_string()),
        uri: None,
        media_type: Some("image/webp".to_string()),
        filename: Some("image-1.webp".to_string()),
        artifact_role: Some("original_image".to_string()),
        size_bytes: Some(12),
        sha256: Some("c".repeat(64)),
        visibility: Some(ArtifactVisibility::UserDelivery),
        owner_task_id: Some("task-1".to_string()),
        producer: None,
        lease: None,
        metadata: serde_json::json!({}),
    });
    journal.capability_results.push(result);

    let rewritten = super::super::preserve_verified_delivery_tokens_after_retry(
        &journal,
        &detail,
        detail.clone(),
    );

    assert_eq!(rewritten, format!("{detail}\n{token}"));
}
