use serde_json::json;

use super::*;

fn task(payload: serde_json::Value) -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 1,
        task_id: "eligibility-task".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("synthetic-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
    }
}

fn settings(policy: ExternalContextPolicy) -> MemoryEffectiveSettings {
    MemoryEffectiveSettings {
        schema_version: 1,
        scope: super::super::settings::MemorySettingScope::Conversation,
        target_principal_id: "principal-test".to_string(),
        conversation_id: Some("conversation-test".to_string()),
        requested: super::super::settings::MemoryRequestedSettings {
            use_mode: super::super::settings::MemorySettingMode::Enabled,
            generate_mode: super::super::settings::MemorySettingMode::Enabled,
            external_context_policy: policy,
        },
        use_memory: true,
        generate_memory: true,
        external_context_policy: policy,
        use_source: "fixture".to_string(),
        generate_source: "fixture".to_string(),
        external_context_source: "fixture".to_string(),
        managed_deny_reason: None,
        revision: 1,
        policy_digest: "fixture-policy".to_string(),
        restart_required: false,
    }
}

#[test]
fn default_policy_keeps_external_and_attachment_bodies_out_of_durable_candidates() {
    let payload = json!({
        "attachments": [{"kind": "image"}],
        "web_context": {"ref": "web:1"},
        "mcp_context": {"ref": "mcp:1"},
    });
    let eligibility = build_turn_eligibility(
        &task(payload.clone()),
        &payload,
        &settings(ExternalContextPolicy::Exclude),
    );
    assert!(eligibility.durable_candidate_allowed);
    assert_eq!(eligibility.items.len(), 11);
    for category in [
        MemorySourceCategory::AttachmentOcrStt,
        MemorySourceCategory::Web,
        MemorySourceCategory::McpPrivateConnector,
    ] {
        assert!(eligibility.items.iter().any(|item| {
            item.category == category && item.disposition == MemoryEligibilityDisposition::Excluded
        }));
    }
}

#[test]
fn scheduled_subagent_and_group_sources_require_owner_verification_or_are_excluded() {
    let payload = json!({
        "chat_type": "group",
        "scheduled_job_id": "job-1",
        "parent_task_id": "parent-1",
    });
    let mut source = task(payload.clone());
    source.kind = "scheduled".to_string();
    let eligibility =
        build_turn_eligibility(&source, &payload, &settings(ExternalContextPolicy::Allow));
    assert!(!eligibility.durable_candidate_allowed);
    assert!(eligibility.items.iter().any(|item| {
        item.category == MemorySourceCategory::GroupChannel
            && item.disposition == MemoryEligibilityDisposition::Excluded
    }));
    assert!(eligibility.items.iter().any(|item| {
        item.category == MemorySourceCategory::Subagent
            && item.disposition == MemoryEligibilityDisposition::ParentVerifierRequired
    }));
}
