use super::*;

#[test]
fn config_risk_guard_rewrite_uses_capability_ref_without_semantic_kind() {
    let mut route = base_route_result();
    route.route_reason = "capability_ref=config.guard_after_change".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let actions = vec![AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args: json!({
            "action": "guard_config",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    }];

    let normalized = rewrite_rustclaw_config_risk_assessment_to_guard(Some(&route), None, actions);

    let args = expect_planned_call(&normalized[0], "config_basic", "guard_rustclaw_config");
    assert_eq!(
        args.get("path").and_then(Value::as_str),
        Some("configs/config.toml")
    );
}
