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

#[test]
fn large_diagnostics_retain_host_verified_file_bindings_and_conversion_continuation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("media-fixture.mp4");
    std::fs::write(&path, b"media fixture").unwrap();
    let mut result = CapabilityResultEnvelope::ok(
        "fixture.export",
        Some("export".to_string()),
        json!({
            "extra":{"diagnostics":vec!["large fixture diagnostic".repeat(400);8],
            "followup_policy":{"capability":"fixture.transform","input_field":"input_path",
                "input_value":path.to_string_lossy(),"next_action":"continue_requested_work"}}
        }),
    );
    result.artifacts.push(
        serde_json::from_value(json!({"path":path,"filename":"media-fixture.mp4",
        "visibility":"user_delivery","artifact_role":"video","metadata":{}}))
        .unwrap(),
    );
    crate::capability_result::bind_artifacts_to_task(&mut result, "fixture-task", "step-1", None);
    let expected_ref = result.artifacts[0].artifact_ref.clone().unwrap();
    let mut state = LoopState::default();
    state.capability_results.push(result);
    let observed = planner_last_observation(&state);
    let raw = observed
        .strip_prefix("capability_result_observation=")
        .unwrap();
    let value: Value = serde_json::from_str(raw).unwrap();
    assert!(raw.chars().count() <= 24 * 1024);
    assert_eq!(value["artifact_bindings"][0]["artifact_ref"], expected_ref);
    assert_eq!(
        value["artifact_bindings"][0]["verification"]["status"],
        "verified"
    );
    assert_eq!(value["artifact_binding_summary"]["complete"], true);
    assert_eq!(
        value["machine_continuation"]["capability"],
        "fixture.transform"
    );
    assert!(!raw.contains(root.path().to_str().unwrap()));
}

#[test]
fn large_artifact_sets_expose_explicit_counts_instead_of_silent_loss() {
    let mut result = CapabilityResultEnvelope::ok("fixture.batch", None, json!({}));
    for i in 0..20 {
        result.artifacts.push(
            serde_json::from_value(
                json!({"artifact_ref":format!("artifact:task/fixture-task/file-{i}"),
            "filename":format!("file-{i}.txt"),"metadata":{}}),
            )
            .unwrap(),
        );
    }
    let mut state = LoopState::default();
    state.capability_results.push(result);
    let observed = planner_last_observation(&state);
    let value: Value = serde_json::from_str(
        observed
            .strip_prefix("capability_result_observation=")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(value["artifact_bindings"].as_array().unwrap().len(), 16);
    assert_eq!(value["artifact_binding_summary"]["total_count"], 20);
    assert_eq!(value["artifact_binding_summary"]["complete"], false);
}
