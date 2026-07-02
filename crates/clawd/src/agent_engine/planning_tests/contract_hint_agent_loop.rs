use super::*;

#[test]
fn direct_answer_contract_hint_capability_ref_uses_deterministic_guard_action() {
    let state = test_state_with_enabled_skills(&["config_basic", "config_edit"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::direct_answer();
    route.route_reason =
        "structured_contract_hint_fast_path; contract_hint_fast_path; capability_ref=config.guard_config".into();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    let plan = contract_hint_preferred_action_deterministic_plan_result(
        &state,
        "guard config",
        Some(&route),
        &LoopState::new(1),
        "sanitized request without hint block",
        None,
    )
    .expect("direct-answer compatibility trace should use contract hint capability_ref");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].skill, "config_basic");
    assert_eq!(
        plan.steps[0].args.get("action").and_then(Value::as_str),
        Some("guard_rustclaw_config")
    );
}
