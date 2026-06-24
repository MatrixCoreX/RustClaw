use super::*;
use std::collections::BTreeSet;

#[test]
fn repair_inventory_reason_codes_are_unique() {
    let mut seen = BTreeSet::new();
    for item in REPAIR_BOUNDARY_INVENTORY {
        assert!(
            seen.insert(item.reason_code),
            "duplicate repair reason code {}",
            item.reason_code
        );
    }
}

#[test]
fn repair_inventory_covers_required_boundary_buckets() {
    let required = [
        "normalizer_schema_contract_repair",
        "contract_repair_judge_schema_boundary",
        "current_turn_missing_locator_boundary_repair",
        "active_text_followup_route_repair",
        "post_route_legacy_semantic_repair_deferral",
        "plan_repair_loop_recovery",
        "answer_verifier_missing_evidence_repair",
        "plan_verifier_issue_repair",
        "permission_contract_repair",
        "provider_blocker_recovery",
        "checkpoint_resume_recovery",
    ];

    for reason_code in required {
        assert!(
            REPAIR_BOUNDARY_INVENTORY
                .iter()
                .any(|item| item.reason_code == reason_code),
            "missing repair inventory item {}",
            reason_code
        );
    }
}

#[test]
fn repair_inventory_items_are_machine_field_bounded() {
    for item in REPAIR_BOUNDARY_INVENTORY {
        assert!(!item.owner_layer.trim().is_empty());
        assert!(!item.runtime_scope.trim().is_empty());
        assert!(!item.source_files.is_empty());
        assert!(!item.allowed_input_fields.is_empty());
        assert!(!item.forbidden_input_fields.is_empty());
        assert!(!item.migration_target.trim().is_empty());
        assert!(!item.next_recovery_kind.trim().is_empty());
        assert!(!item.deletion_gate.trim().is_empty());
        assert!(
            item.deletion_gate
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
            "{} deletion_gate must be a machine token",
            item.reason_code
        );
        assert!(
            item.deletion_gate.starts_with("keep_")
                || item.deletion_gate.starts_with("delete_after_"),
            "{} deletion_gate must be keep_* or delete_after_*",
            item.reason_code
        );

        assert!(
            item.forbidden_input_fields.contains(&"user_prompt_phrase"),
            "{} must explicitly reject user prompt phrase matching",
            item.reason_code
        );
        assert!(
            item.forbidden_input_fields
                .contains(&"localized_reply_text"),
            "{} must explicitly reject localized reply text matching",
            item.reason_code
        );
        assert!(
            item.forbidden_input_fields.contains(&"text"),
            "{} must explicitly reject user-visible text fields as repair input",
            item.reason_code
        );
        assert!(
            item.forbidden_input_fields.contains(&"error_text"),
            "{} must explicitly reject user-visible error_text fields as repair input",
            item.reason_code
        );
    }
}

#[test]
fn ordinary_semantic_repair_is_marked_for_loop_migration() {
    let ordinary: Vec<_> = REPAIR_BOUNDARY_INVENTORY
        .iter()
        .filter(|item| item.repair_class == RepairBoundaryClass::OrdinarySemanticRepair)
        .collect();

    assert!(
        !ordinary.is_empty(),
        "inventory should expose ordinary semantic repair debt instead of hiding it"
    );

    for item in ordinary {
        assert!(
            item.migration_target.starts_with("migrate_to_agent_loop"),
            "{} must have explicit agent-loop migration target",
            item.reason_code
        );
        assert_eq!(
            item.next_recovery_kind, "replan",
            "{} should recover through planner replan, not pre-route execution",
            item.reason_code
        );
        assert!(
            item.forbidden_input_fields
                .contains(&"language_phrase_array"),
            "{} must forbid language phrase arrays",
            item.reason_code
        );
        assert!(
            item.deletion_gate.starts_with("delete_after_"),
            "{} must have an explicit deletion gate",
            item.reason_code
        );
    }
}

#[test]
fn signal_lookup_prefers_reason_then_owner_then_source() {
    let by_reason = repair_inventory_for_signal(
        Some("verify_risk_budget_exceeded"),
        Some("missing_required_arg"),
        Some("answer_verifier"),
        Some("verifier"),
    )
    .expect("reason lookup");
    assert_eq!(by_reason.reason_code, "permission_contract_repair");

    let by_owner = repair_inventory_for_signal(
        None,
        Some("missing_required_arg"),
        Some("plan_verifier"),
        Some("answer_verifier"),
    )
    .expect("owner alias lookup");
    assert_eq!(by_owner.reason_code, "plan_verifier_issue_repair");

    let by_source = repair_inventory_for_signal(None, None, None, Some("answer_verifier"))
        .expect("source fallback lookup");
    assert_eq!(
        by_source.reason_code,
        "answer_verifier_missing_evidence_repair"
    );
}

#[test]
fn verifier_reason_aliases_split_replan_from_permission() {
    let missing_arg =
        repair_inventory_for_signal(Some("verify_missing_required_arg"), None, None, None)
            .expect("missing arg verifier inventory");
    assert_eq!(missing_arg.reason_code, "plan_verifier_issue_repair");
    assert_eq!(
        missing_arg.repair_class,
        RepairBoundaryClass::LoopBoundedRecovery
    );
    assert_eq!(missing_arg.next_recovery_kind, "replan");

    let risk = repair_inventory_for_signal(Some("verify_risk_budget_exceeded"), None, None, None)
        .expect("risk verifier inventory");
    assert_eq!(risk.reason_code, "permission_contract_repair");
    assert_eq!(
        risk.repair_class,
        RepairBoundaryClass::PermissionContractRepair
    );
    assert_eq!(risk.next_recovery_kind, "needs_user");
}

#[test]
fn executor_status_aliases_split_contract_from_provider_blocker() {
    let contract = repair_inventory_for_signal(
        Some("executor_step_failed"),
        Some("contract_action_rejected"),
        None,
        Some("executor"),
    )
    .expect("contract executor inventory");
    assert_eq!(contract.reason_code, "permission_contract_repair");
    assert_eq!(
        contract.repair_class,
        RepairBoundaryClass::PermissionContractRepair
    );

    let provider = repair_inventory_for_signal(
        Some("executor_step_failed"),
        Some("quota_exceeded"),
        None,
        Some("executor"),
    )
    .expect("provider executor inventory");
    assert_eq!(provider.reason_code, "provider_blocker_recovery");
    assert_eq!(
        provider.repair_class,
        RepairBoundaryClass::LoopBoundedRecovery
    );
    assert_eq!(provider.next_recovery_kind, "wait_background");
}

#[test]
fn trace_value_exposes_auditable_machine_fields() {
    let item = REPAIR_BOUNDARY_INVENTORY
        .iter()
        .copied()
        .find(|item| item.reason_code == "permission_contract_repair")
        .expect("permission inventory item");
    let value = item.trace_value();

    assert_eq!(value.get("schema_version").and_then(Value::as_u64), Some(1));
    assert_eq!(
        value.get("repair_class").and_then(Value::as_str),
        Some("permission_contract_repair")
    );
    assert_eq!(
        value.get("next_recovery_kind").and_then(Value::as_str),
        Some("needs_user")
    );
    assert_eq!(
        value.get("deletion_gate").and_then(Value::as_str),
        Some("keep_policy_boundary")
    );
    assert!(value
        .get("forbidden_input_fields")
        .and_then(Value::as_array)
        .is_some_and(|fields| fields
            .iter()
            .any(|field| field.as_str() == Some("permission_bypass_action"))));
}
