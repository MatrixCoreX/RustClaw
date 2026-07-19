use serde_json::json;

use super::{TurnBoundaryEnvelope, TurnInputMaterialization};

fn task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-boundary-1".to_string(),
        user_id: 7,
        chat_id: 8,
        user_key: Some("user-key".to_string()),
        channel: "web".to_string(),
        external_user_id: Some("external-user".to_string()),
        external_chat_id: Some("external-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

#[test]
fn envelope_uses_only_explicit_machine_fields_and_materialized_attachments() {
    let payload = json!({
        "text": "do not project this user text",
        "source": "ui_chat",
        "budget_profile": "long_task",
        "path": "README.md",
        "attachments": [{
            "kind": "file",
            "path": "data/ui/7/8/report.txt",
            "mime_type": "text/plain",
            "size": 42
        }],
        "ignored_freeform": "not part of the protocol"
    });
    let envelope = TurnBoundaryEnvelope::from_claimed_task(
        &task(),
        &payload,
        "do not project this user text",
        super::TurnInputMaterialization::RawText,
        Some("pwd".to_string()),
        false,
        false,
    );

    assert_eq!(envelope.raw_chars, 29);
    assert_eq!(envelope.explicit_machine_command.as_deref(), Some("pwd"));
    assert_eq!(
        envelope.structured_locator_facts,
        vec!["README.md", "data/ui/7/8/report.txt"]
    );
    assert_eq!(
        envelope
            .explicit_api_fields
            .get("budget_profile")
            .map(String::as_str),
        Some("long_task")
    );
    assert!(!envelope.explicit_api_fields.contains_key("text"));
    assert!(!envelope
        .explicit_api_fields
        .contains_key("ignored_freeform"));

    let prompt_line = envelope.compact_prompt_line();
    assert!(prompt_line.contains("turn_boundary_envelope"));
    assert!(prompt_line.contains("README.md"));
    assert!(!prompt_line.contains("do not project this user text"));
    assert!(!prompt_line.contains("external-user"));
    assert!(!prompt_line.contains("user-key"));
}

#[test]
fn envelope_derives_runtime_permission_boundary_without_semantic_routing() {
    let envelope = TurnBoundaryEnvelope::from_claimed_task(
        &task(),
        &json!({}),
        "anything",
        super::TurnInputMaterialization::RawText,
        None,
        true,
        false,
    );

    assert_eq!(envelope.permission_profile, "elevated_runtime_policy");
    assert_eq!(envelope.budget_profile, "adaptive");
    assert!(envelope.safety_context.task_identity_bound);
    assert!(envelope.safety_context.attachments_validated);
}

#[test]
fn input_materialization_is_classified_from_machine_state() {
    assert_eq!(
        TurnInputMaterialization::classify(false, true, 0),
        TurnInputMaterialization::RawText
    );
    assert_eq!(
        TurnInputMaterialization::classify(false, false, 1),
        TurnInputMaterialization::AttachmentOnly
    );
    assert_eq!(
        TurnInputMaterialization::classify(true, false, 1),
        TurnInputMaterialization::AudioTranscript
    );
    assert_eq!(
        TurnInputMaterialization::classify(true, true, 1),
        TurnInputMaterialization::TextAndAudioTranscript
    );
}
