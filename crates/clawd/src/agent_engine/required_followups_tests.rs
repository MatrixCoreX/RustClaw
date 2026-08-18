use super::*;
use claw_core::capability_result::{
    CapabilityResultEnvelope, CapabilityResultStatus, Continuation, ContinuationKind,
};

fn bundle_result() -> CapabilityResultEnvelope {
    bundle_result_with_activation(None)
}

fn bundle_result_with_activation(activation_requirement: Option<&str>) -> CapabilityResultEnvelope {
    let mut policy = json!({
        "completion_requirement": "all_components",
        "steps": [
            {
                "component_kind": "images",
                "capability": "image_vision.extract_text",
                "input_field": "images",
                "input_value": [{"artifact_ref": "artifact:task/task-1/image-1"}]
            },
            {
                "component_kind": "background_audio",
                "capability": "audio.preview_transcribe",
                "input_field": "audio_path",
                "input_value": "artifact:task/task-1/audio-1",
                "fallback_capability": "media_download.transcribe",
                "fallback_input_field": "input_path",
                "fallback_input_value": "artifact:task/task-1/audio-1",
                "completion_capabilities": ["audio.transcribe", "media_download.transcribe"],
                "recommended_capability_pointer": "/extra/recommended_capability"
            }
        ]
    });
    if let Some(value) = activation_requirement {
        policy["activation_requirement"] = json!(value);
    }
    CapabilityResultEnvelope::ok(
        "media_download.download",
        Some("download".to_string()),
        json!({
            "extra": {
                "content_bundle": {
                    "kind": "image_audio_article",
                    "followup_policy": policy
                }
            }
        }),
    )
}

fn selected_bundle_result() -> CapabilityResultEnvelope {
    CapabilityResultEnvelope::ok(
        "media_download.download",
        Some("download".to_string()),
        json!({
            "extra": {"content_bundle": {"followup_policy": {
                "activation_requirement": "required",
                "completion_requirement": "selected_components",
                "steps": [{
                    "component_kind": "video_audio",
                    "capability": "audio.preview_transcribe",
                    "input_field": "audio_path",
                    "input_value": "artifact:task/task-1/video-1",
                    "fallback_capability": "media_download.transcribe",
                    "fallback_input_field": "input_path",
                    "fallback_input_value": "artifact:task/task-1/video-1",
                    "completion_capabilities": ["audio.transcribe", "media_download.transcribe"],
                    "recommended_capability_pointer": "/extra/recommended_capability"
                }]
            }}}
        }),
    )
}

fn ok(capability: &str) -> CapabilityResultEnvelope {
    CapabilityResultEnvelope::ok(capability, None, json!({"status": "ok"}))
}

fn preview(recommended_capability: &str) -> CapabilityResultEnvelope {
    CapabilityResultEnvelope::ok(
        "audio.preview_transcribe",
        Some("preview_transcribe".to_string()),
        json!({
            "extra": {
                "recommended_capability": recommended_capability,
                "provider_location": if recommended_capability == "audio.transcribe" {
                    "remote"
                } else {
                    "local"
                }
            }
        }),
    )
}

fn failed(capability: &str) -> CapabilityResultEnvelope {
    let mut result = ok(capability);
    result.status = CapabilityResultStatus::Error;
    result.error = Some(claw_core::capability_result::StructuredError {
        code: "failed".to_string(),
        message_key: "test.failed".to_string(),
        retryable: false,
        details: json!({}),
    });
    result
}

#[test]
fn plain_download_does_not_activate_conditional_text_conversion() {
    assert!(next_required_followup(&[bundle_result()]).is_none());
}

#[test]
fn explicit_all_scope_activates_both_components_at_download_boundary() {
    let results = vec![bundle_result_with_activation(Some("required"))];
    let required = next_required_followup(&results).expect("first required component");
    assert_eq!(required.capability, "image_vision.extract_text");
}

#[test]
fn explicitly_selected_single_component_is_enforced() {
    let results = vec![selected_bundle_result()];
    let required = next_required_followup(&results).expect("selected component");
    assert_eq!(required.capability, "audio.preview_transcribe");
    assert_eq!(required.completion_requirement, "selected_components");
}

#[test]
fn image_conversion_requires_the_audio_component_before_responding() {
    let results = vec![bundle_result(), ok("image_vision.extract_text")];
    let required = next_required_followup(&results).expect("audio followup");
    assert_eq!(required.component_kind, "background_audio");
    assert_eq!(required.capability, "audio.preview_transcribe");
    assert_eq!(required.args["audio_path"], "artifact:task/task-1/audio-1");

    let (actions, observation) = enforce_required_followup(
        &[AgentAction::Respond {
            content: "premature".to_string(),
        }],
        &results,
    )
    .expect("terminal override");
    assert!(matches!(
        &actions[0],
        AgentAction::CallCapability { capability, .. }
            if capability == "audio.preview_transcribe"
    ));
    assert_eq!(
        observation["reason_code"],
        "required_component_not_completed"
    );
}

#[test]
fn audio_conversion_requires_the_image_component_before_responding() {
    let results = vec![bundle_result(), ok("audio.preview_transcribe")];
    let required = next_required_followup(&results).expect("image followup");
    assert_eq!(required.capability, "image_vision.extract_text");
    assert!(required.args["images"].is_array());
}

#[test]
fn unrelated_planner_action_cannot_displace_required_component() {
    let results = vec![bundle_result(), ok("image_vision.extract_text")];
    let (actions, _) = enforce_required_followup(
        &[AgentAction::CallCapability {
            capability: "filesystem.find_entries".to_string(),
            args: json!({"root": ".agent-runtime"}),
        }],
        &results,
    )
    .expect("required continuation override");

    assert_eq!(actions.len(), 1);
    assert!(matches!(
        &actions[0],
        AgentAction::CallCapability { capability, .. }
            if capability == "audio.preview_transcribe"
    ));
}

#[test]
fn successful_components_allow_terminal_response() {
    let results = vec![
        bundle_result(),
        ok("image_vision.extract_text"),
        preview("audio.transcribe"),
        ok("audio.transcribe"),
    ];
    assert!(next_required_followup(&results).is_none());
    assert!(enforce_required_followup(
        &[AgentAction::Respond {
            content: "complete".to_string(),
        }],
        &results,
    )
    .is_none());
}

#[test]
fn failed_primary_transcription_uses_declared_fallback_once() {
    let results = vec![
        bundle_result(),
        ok("image_vision.extract_text"),
        preview("audio.transcribe"),
        failed("audio.transcribe"),
    ];
    let required = next_required_followup(&results).expect("fallback");
    assert_eq!(required.capability, "media_download.transcribe");
    assert_eq!(required.args["input_path"], "artifact:task/task-1/audio-1");

    let exhausted = vec![
        bundle_result(),
        ok("image_vision.extract_text"),
        preview("audio.transcribe"),
        failed("audio.transcribe"),
        failed("media_download.transcribe"),
    ];
    assert!(next_required_followup(&exhausted).is_none());
}

#[test]
fn successful_preview_still_requires_the_selected_transcription() {
    let remote = vec![
        bundle_result(),
        ok("image_vision.extract_text"),
        preview("audio.transcribe"),
    ];
    let required = next_required_followup(&remote).expect("remote transcription");
    assert_eq!(required.capability, "audio.transcribe");
    assert_eq!(required.args["audio_path"], "artifact:task/task-1/audio-1");

    let local = vec![
        bundle_result(),
        ok("image_vision.extract_text"),
        preview("media_download.transcribe"),
    ];
    let required = next_required_followup(&local).expect("local transcription");
    assert_eq!(required.capability, "media_download.transcribe");
    assert_eq!(required.args["input_path"], "artifact:task/task-1/audio-1");
}

#[test]
fn in_flight_component_is_not_started_twice() {
    let mut waiting = ok("audio.preview_transcribe");
    waiting.status = CapabilityResultStatus::Waiting;
    waiting.continuation = Some(Continuation {
        kind: ContinuationKind::Poll,
        reference: Some("job-1".to_string()),
        poll_after_ms: Some(1_000),
        state: json!({}),
    });
    let results = vec![bundle_result(), ok("image_vision.extract_text"), waiting];
    assert!(next_required_followup(&results).is_none());
}
