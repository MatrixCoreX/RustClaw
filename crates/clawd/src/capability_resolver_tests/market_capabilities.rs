use super::*;

#[test]
fn registry_resolves_crypto_positions_capability() {
    let state = state_with_workspace_registry();
    let (action, record) =
        resolve_capability_action_with_record_for_state(&state, "crypto.positions", json!({}));
    let action = action.expect("registry crypto.positions capability should resolve");
    match action {
        AgentAction::CallSkill { skill, args } => {
            assert_eq!(skill, "crypto");
            assert_eq!(
                args.get("action").and_then(Value::as_str),
                Some("positions")
            );
        }
        other => panic!("unexpected resolved action: {other:?}"),
    }
    assert_eq!(
        record.reason_code,
        "capability_resolver_registry_mapping_resolved"
    );
    assert_eq!(record.source, "registry");
    assert_eq!(record.capability_ref, "crypto.positions");
}

#[test]
fn registry_does_not_expose_crypto_trading_or_order_actions_to_planner() {
    let state = state_with_workspace_registry();
    for capability in [
        "crypto.trade_preview",
        "crypto.trade_submit",
        "crypto.order_status",
        "crypto.cancel_order",
        "crypto.cancel_all_orders",
        "crypto.open_orders",
        "crypto.trade_history",
    ] {
        let (action, record) = resolve_capability_action_with_record_for_state(
            &state,
            capability,
            json!({"action": capability.trim_start_matches("crypto.")}),
        );
        assert!(action.is_none(), "{capability} must stay direct/admin-only");
        assert_eq!(record.reason_code, "capability_unavailable", "{capability}");
        assert_eq!(record.source, "none", "{capability}");
        assert_eq!(record.capability_ref, capability);
        assert!(record.resolved_ref.is_none(), "{capability}");
    }
}

#[test]
fn crypto_registry_separates_read_capability_policy_from_complete_runner_risk() {
    let state = state_with_workspace_registry();
    let manifest = state.skill_manifest("crypto").expect("crypto manifest");
    assert_eq!(
        manifest.risk_level,
        Some(claw_core::skill_registry::SkillRiskLevel::High)
    );
    assert_eq!(manifest.requires_confirmation, Some(true));
    assert_eq!(manifest.side_effect, Some(true));

    for action in ["quote", "multi_quote", "positions"] {
        assert!(
            !state.skill_invocation_requires_confirmation_policy(
                "crypto",
                Some(&json!({"action": action}))
            ),
            "{action} should remain a read-only planner path"
        );
    }
    for action in ["trade_submit", "cancel_order", "cancel_all_orders"] {
        assert!(
            state.skill_invocation_requires_confirmation_policy(
                "crypto",
                Some(&json!({"action": action}))
            ),
            "{action} must inherit complete-runner confirmation policy"
        );
    }

    let registry = state.get_skills_registry().expect("skills registry");
    for capability in ["crypto.quote", "crypto.multi_quote"] {
        let mapping = registry
            .planner_capabilities("crypto")
            .iter()
            .find(|mapping| mapping.name == capability)
            .unwrap_or_else(|| panic!("missing {capability}"));
        assert_eq!(mapping.network_access, Some(true), "{capability}");
        assert_eq!(mapping.credential_access, Some(false), "{capability}");
    }
    let positions = registry
        .planner_capabilities("crypto")
        .iter()
        .find(|mapping| mapping.name == "crypto.positions")
        .expect("crypto.positions");
    assert_eq!(positions.network_access, Some(true));
    assert_eq!(positions.credential_access, Some(true));
}

#[test]
fn registry_resolves_market_quote_capabilities_without_domain_contracts() {
    let state = state_with_workspace_registry();
    for (capability, expected_skill, symbol) in [
        ("crypto.quote", "crypto", "BTCUSDT"),
        ("stock.quote", "stock", "600519"),
    ] {
        let (action, record) = resolve_capability_action_with_record_for_state(
            &state,
            capability,
            json!({"symbol": symbol}),
        );
        let action = action.unwrap_or_else(|| panic!("{capability} should resolve"));
        let AgentAction::CallSkill { skill, args } = action else {
            panic!("unexpected resolved action for {capability}: {action:?}");
        };
        assert_eq!(skill, expected_skill);
        assert_eq!(args.get("action").and_then(Value::as_str), Some("quote"));
        assert_eq!(args.get("symbol").and_then(Value::as_str), Some(symbol));
        assert_eq!(
            record.reason_code,
            "capability_resolver_registry_mapping_resolved"
        );
        assert_eq!(record.source, "registry");
        assert_eq!(record.capability_ref, capability);
    }
}
