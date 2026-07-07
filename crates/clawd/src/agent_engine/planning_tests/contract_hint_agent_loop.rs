use super::*;

#[test]
fn direct_answer_contract_hint_capability_ref_exposes_guard_policy() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::respond_trace();
    route.route_reason =
        "structured_contract_hint_fast_path; contract_hint_fast_path; capability_ref=config.guard_rustclaw_config".into();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "config_basic",
        &json!({
            "action": "guard_rustclaw_config",
            "path": "configs/config.toml",
        }),
    )
    .expect("contract hint capability_ref should expose config_basic guard policy");

    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}
