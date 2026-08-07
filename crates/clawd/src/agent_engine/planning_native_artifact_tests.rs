use super::*;
use claw_core::capability_result::{ArtifactRef, ArtifactVisibility, CapabilityResultEnvelope};

#[test]
fn planner_projects_artifact_bound_machine_continuation_without_internal_path() {
    let mut loop_state = LoopState::default();
    let mut result = CapabilityResultEnvelope::ok(
        "media.transform",
        Some("extract".to_string()),
        json!({
            "extra": {
                "followup_policy": {
                    "capability": "audio.inspect",
                    "input_field": "audio_path",
                    "input_value": "/private/runtime/extracted.wav",
                    "input_value_artifact_ref": "artifact:task/task-1/audio-1",
                    "fallback_capability": "audio.local",
                    "fallback_input_field": "input_path",
                    "fallback_input_value": "/private/runtime/extracted.wav",
                    "fallback_input_value_artifact_ref": "artifact:task/task-1/audio-1"
                }
            }
        }),
    );
    result.artifacts.push(ArtifactRef {
        artifact_ref: Some("artifact:task/task-1/audio-1".to_string()),
        id: Some("audio-1".to_string()),
        path: Some("/private/runtime/extracted.wav".to_string()),
        uri: None,
        media_type: Some("audio/wav".to_string()),
        filename: Some("extracted.wav".to_string()),
        artifact_role: Some("extracted_audio".to_string()),
        size_bytes: Some(1024),
        sha256: Some("a".repeat(64)),
        visibility: Some(ArtifactVisibility::InternalProcessing),
        owner_task_id: Some("task-1".to_string()),
        producer: None,
        lease: None,
        metadata: json!({}),
    });
    loop_state.capability_results.push(result);

    let observation = planner_last_observation(&loop_state);
    let projection: Value = serde_json::from_str(
        observation
            .strip_prefix("capability_result_observation=")
            .expect("generic observation prefix"),
    )
    .expect("structured observation");

    assert_eq!(
        projection["machine_continuation"]["capability"],
        "audio.inspect"
    );
    assert_eq!(
        projection["machine_continuation"]["input_field"],
        "audio_path"
    );
    assert_eq!(
        projection["machine_continuation"]["input_value_artifact_ref"],
        "artifact:task/task-1/audio-1"
    );
    assert_eq!(
        projection["artifact_bindings"][0]["artifact_role"],
        "extracted_audio"
    );
    assert_eq!(
        projection["data"]["extra"]["followup_policy"]["input_value"],
        "artifact:task/task-1/audio-1"
    );
    assert!(!projection
        .to_string()
        .contains("/private/runtime/extracted.wav"));
}
