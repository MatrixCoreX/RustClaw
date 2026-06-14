use super::*;
use crate::{OutputLocatorKind, OutputSemanticKind};

#[test]
fn recent_artifacts_judgment_rejects_structured_field_substitute() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs".to_string(),
        ..IntentOutputContract::default()
    };

    let field_policy = action_policy_for_output_contract(
        Some(&contract),
        "config_basic",
        &serde_json::json!({
            "action": "read_field",
            "path": "configs/agent_guard.toml",
            "field_path": "agent_guard.semantic_route_authority"
        }),
    )
    .expect("config field policy");
    assert_eq!(
        field_policy.decision,
        ActionPolicyDecision::RejectedNotAllowed,
        "{field_policy:?}"
    );
    assert_eq!(field_policy.action_key, "config_basic.read_field");
    assert_eq!(field_policy.contract_match, "recent_artifacts_judgment");

    let fields_policy = action_policy_for_output_contract(
        Some(&contract),
        "config_basic",
        &serde_json::json!({
            "action": "read_fields",
            "path": "configs/config.toml",
            "field_paths": ["skills.skills_list"]
        }),
    )
    .expect("config fields policy");
    assert_eq!(
        fields_policy.decision,
        ActionPolicyDecision::RejectedNotAllowed,
        "{fields_policy:?}"
    );
    assert_eq!(fields_policy.action_key, "config_basic.read_fields");
    assert_eq!(fields_policy.contract_match, "recent_artifacts_judgment");

    let tree_policy = action_policy_for_output_contract(
        Some(&contract),
        "system_basic",
        &serde_json::json!({
            "action": "tree_summary",
            "path": ".",
            "max_depth": 2,
            "max_children_per_dir": 8
        }),
    )
    .expect("directory tree policy");
    assert!(tree_policy.is_allowed(), "{tree_policy:?}");
    assert_eq!(tree_policy.action_key, "system_basic.tree_summary");
    assert_eq!(tree_policy.contract_match, "recent_artifacts_judgment");
}
