use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrePlannerExitKind {
    BoundarySafety,
    MachineFactFastPath,
    CompatTrace,
    OrdinarySemantic,
}

impl PrePlannerExitKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::BoundarySafety => "boundary_safety",
            Self::MachineFactFastPath => "machine_fact_fast_path",
            Self::CompatTrace => "compat_trace",
            Self::OrdinarySemantic => "ordinary_semantic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PrePlannerExitInventoryItem {
    pub(super) reason_code: &'static str,
    pub(super) kind: PrePlannerExitKind,
    pub(super) migration_target: &'static str,
    pub(super) migration_stage: &'static str,
    pub(super) migration_order: u8,
    pub(super) nl_gate_refs: &'static [&'static str],
    pub(super) deletion_gate: &'static str,
    pub(super) owner_layer: &'static str,
}

impl PrePlannerExitInventoryItem {
    pub(super) fn trace_context(self) -> Value {
        json!({
            "schema_version": 1,
            "pre_planner_exit_kind": self.kind.as_str(),
            "pre_planner_exit_reason_code": self.reason_code,
            "migration_target": self.migration_target,
            "migration_stage": self.migration_stage,
            "migration_order": self.migration_order,
            "nl_gate_refs": self.nl_gate_refs,
            "deletion_gate": self.deletion_gate,
            "owner_layer": self.owner_layer,
        })
    }
}

pub(super) const PRE_PLANNER_EXIT_INVENTORY: &[PrePlannerExitInventoryItem] = &[
    PrePlannerExitInventoryItem {
        reason_code: "self_extension_boundary",
        kind: PrePlannerExitKind::BoundarySafety,
        migration_target: "self_extension_boundary_keep_outside_planner",
        migration_stage: "keep_boundary",
        migration_order: 0,
        nl_gate_refs: &[],
        deletion_gate: "keep_boundary",
        owner_layer: "ask_flow_self_extension",
    },
    PrePlannerExitInventoryItem {
        reason_code: "structural_alias_binding_ack",
        kind: PrePlannerExitKind::MachineFactFastPath,
        migration_target: "task_context_machine_fact_or_task_control_capability",
        migration_stage: "selected_local_observe_or_direct_scalar",
        migration_order: 10,
        nl_gate_refs: &["nl_alias_binding_followup_zh"],
        deletion_gate: "keep_machine_fact_fast_path",
        owner_layer: "ask_flow_context_fast_path",
    },
    PrePlannerExitInventoryItem {
        reason_code: "active_ordered_entries_count_direct_answer",
        kind: PrePlannerExitKind::MachineFactFastPath,
        migration_target: "task_context_machine_fact_or_task_control_capability",
        migration_stage: "selected_local_observe_or_direct_scalar",
        migration_order: 10,
        nl_gate_refs: &["nl_active_ordered_entries_count_zh"],
        deletion_gate: "keep_machine_fact_fast_path",
        owner_layer: "ask_flow_context_fast_path",
    },
    PrePlannerExitInventoryItem {
        reason_code: "recent_count_comparison_direct_answer",
        kind: PrePlannerExitKind::MachineFactFastPath,
        migration_target: "task_context_machine_fact_or_task_control_capability",
        migration_stage: "selected_local_observe_or_direct_scalar",
        migration_order: 10,
        nl_gate_refs: &["nl_recent_count_comparison_zh"],
        deletion_gate: "keep_machine_fact_fast_path",
        owner_layer: "ask_flow_context_fast_path",
    },
    PrePlannerExitInventoryItem {
        reason_code: "runtime_approval_wait_status_direct_answer",
        kind: PrePlannerExitKind::BoundarySafety,
        migration_target: "permission_wait_status_capability",
        migration_stage: "keep_boundary",
        migration_order: 0,
        nl_gate_refs: &[],
        deletion_gate: "keep_boundary",
        owner_layer: "ask_flow_permission_boundary",
    },
    PrePlannerExitInventoryItem {
        reason_code: "session_alias_target_direct_answer",
        kind: PrePlannerExitKind::MachineFactFastPath,
        migration_target: "task_context_machine_fact_or_task_control_capability",
        migration_stage: "selected_local_observe_or_direct_scalar",
        migration_order: 10,
        nl_gate_refs: &["nl_session_alias_target_en"],
        deletion_gate: "keep_machine_fact_fast_path",
        owner_layer: "ask_flow_context_fast_path",
    },
    PrePlannerExitInventoryItem {
        reason_code: "normalizer_runtime_fact_direct_answer",
        kind: PrePlannerExitKind::CompatTrace,
        migration_target: "planner_observe_runtime_fact",
        migration_stage: "selected_local_observe_or_direct_scalar",
        migration_order: 10,
        nl_gate_refs: &["nl_runtime_fact_direct_scalar_en"],
        deletion_gate: "delete_after_agent_loop_default",
        owner_layer: "ask_flow_normalizer_compat",
    },
    PrePlannerExitInventoryItem {
        reason_code: "active_file_basename_direct_answer",
        kind: PrePlannerExitKind::MachineFactFastPath,
        migration_target: "task_context_machine_fact_or_task_control_capability",
        migration_stage: "selected_local_observe_or_direct_scalar",
        migration_order: 10,
        nl_gate_refs: &["nl_active_file_basename_zh"],
        deletion_gate: "keep_machine_fact_fast_path",
        owner_layer: "ask_flow_context_fast_path",
    },
    PrePlannerExitInventoryItem {
        reason_code: "runtime_scalar_path_direct_answer",
        kind: PrePlannerExitKind::MachineFactFastPath,
        migration_target: "task_context_machine_fact_or_task_control_capability",
        migration_stage: "selected_local_observe_or_direct_scalar",
        migration_order: 10,
        nl_gate_refs: &["nl_runtime_scalar_path_zh"],
        deletion_gate: "keep_machine_fact_fast_path",
        owner_layer: "ask_flow_context_fast_path",
    },
    PrePlannerExitInventoryItem {
        reason_code: "normalizer_chat_direct_answer_candidate",
        kind: PrePlannerExitKind::CompatTrace,
        migration_target: "planner_or_chat_model_answer",
        migration_stage: "chat_respond_agent_loop",
        migration_order: 20,
        nl_gate_refs: &["nl_chat_answer_general_zh"],
        deletion_gate: "delete_after_agent_loop_default",
        owner_layer: "ask_flow_normalizer_compat",
    },
    PrePlannerExitInventoryItem {
        reason_code: "normalizer_chat_direct_answer_candidate_with_context_summary",
        kind: PrePlannerExitKind::CompatTrace,
        migration_target: "planner_or_chat_model_answer",
        migration_stage: "active_task_followup_or_chat_rewrite",
        migration_order: 20,
        nl_gate_refs: &["nl_active_task_followup_rewrite_zh"],
        deletion_gate: "delete_after_agent_loop_default",
        owner_layer: "ask_flow_normalizer_compat",
    },
    PrePlannerExitInventoryItem {
        reason_code: "inline_json_transform_promoted_to_planner",
        kind: PrePlannerExitKind::OrdinarySemantic,
        migration_target: "planner_capability_transform_json",
        migration_stage: "inline_transform_or_structured_repair",
        migration_order: 30,
        nl_gate_refs: &[
            "nl_inline_json_transform_strict_zh",
            "nl_inline_json_transform_table_en",
        ],
        deletion_gate: "delete_after_selected_class_release_gate",
        owner_layer: "ask_flow_planner_promotion",
    },
    PrePlannerExitInventoryItem {
        reason_code: "contract_test_hint_promoted_to_planner",
        kind: PrePlannerExitKind::CompatTrace,
        migration_target: "planner_loop_contract_fixture",
        migration_stage: "test_fixture_compat",
        migration_order: 0,
        nl_gate_refs: &[],
        deletion_gate: "test_fixture_only",
        owner_layer: "ask_flow_planner_promotion",
    },
    PrePlannerExitInventoryItem {
        reason_code: "pure_chat_agent_loop_submode",
        kind: PrePlannerExitKind::OrdinarySemantic,
        migration_target: "agent_loop_authority",
        migration_stage: "chat_respond_agent_loop",
        migration_order: 20,
        nl_gate_refs: &["nl_chat_answer_general_en"],
        deletion_gate: "delete_after_selected_class_release_gate",
        owner_layer: "ask_flow_planner_promotion",
    },
    PrePlannerExitInventoryItem {
        reason_code: "direct_answer_gate_recent_count_comparison",
        kind: PrePlannerExitKind::MachineFactFastPath,
        migration_target: "task_context_machine_fact_or_task_control_capability",
        migration_stage: "selected_local_observe_or_direct_scalar",
        migration_order: 10,
        nl_gate_refs: &["nl_recent_count_comparison_zh"],
        deletion_gate: "keep_machine_fact_fast_path",
        owner_layer: "direct_answer_gate",
    },
    PrePlannerExitInventoryItem {
        reason_code: "direct_answer_gate_clarify",
        kind: PrePlannerExitKind::BoundarySafety,
        migration_target: "planner_clarify_with_structured_slots",
        migration_stage: "clarify_repair_boundary_to_planner_slots",
        migration_order: 40,
        nl_gate_refs: &["nl_clarify_missing_target_zh"],
        deletion_gate: "keep_boundary",
        owner_layer: "direct_answer_gate",
    },
    PrePlannerExitInventoryItem {
        reason_code: "direct_answer_gate_promoted_to_planner",
        kind: PrePlannerExitKind::OrdinarySemantic,
        migration_target: "planner_loop_authority",
        migration_stage: "selected_local_observe_or_direct_scalar",
        migration_order: 10,
        nl_gate_refs: &[
            "nl_selected_local_observe_en",
            "nl_direct_scalar_runtime_status_zh",
        ],
        deletion_gate: "delete_after_selected_class_release_gate",
        owner_layer: "direct_answer_gate",
    },
    PrePlannerExitInventoryItem {
        reason_code: "direct_answer_gate_chat_fallback",
        kind: PrePlannerExitKind::OrdinarySemantic,
        migration_target: "agent_loop_respond_or_chat_model_answer",
        migration_stage: "active_task_followup_or_chat_rewrite",
        migration_order: 20,
        nl_gate_refs: &[
            "nl_active_task_followup_rewrite_zh",
            "nl_chat_answer_general_zh",
        ],
        deletion_gate: "delete_after_selected_class_release_gate",
        owner_layer: "direct_answer_gate",
    },
    PrePlannerExitInventoryItem {
        reason_code: "router_selected_clarify",
        kind: PrePlannerExitKind::BoundarySafety,
        migration_target: "planner_clarify_with_structured_slots",
        migration_stage: "clarify_repair_boundary_to_planner_slots",
        migration_order: 40,
        nl_gate_refs: &["nl_clarify_missing_slot_en"],
        deletion_gate: "keep_boundary",
        owner_layer: "ask_flow_clarify_boundary",
    },
];

pub(super) fn pre_planner_exit_for_reason(
    reason_code: &str,
) -> Option<PrePlannerExitInventoryItem> {
    let reason_code = reason_code.trim();
    PRE_PLANNER_EXIT_INVENTORY
        .iter()
        .copied()
        .find(|item| item.reason_code == reason_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn inventory_reason_codes_are_unique() {
        let mut seen = BTreeSet::new();
        for item in PRE_PLANNER_EXIT_INVENTORY {
            assert!(
                seen.insert(item.reason_code),
                "duplicate {}",
                item.reason_code
            );
        }
    }

    #[test]
    fn ordinary_semantic_exits_have_migration_targets() {
        for item in PRE_PLANNER_EXIT_INVENTORY {
            if item.kind == PrePlannerExitKind::OrdinarySemantic {
                assert!(!item.migration_target.trim().is_empty());
                assert!(!item.migration_stage.trim().is_empty());
                assert!(item.deletion_gate.starts_with("delete_after_"));
                assert!(item.migration_order > 0);
                assert!((1..=3).contains(&item.nl_gate_refs.len()));
                for case_ref in item.nl_gate_refs {
                    assert!(case_ref
                        .chars()
                        .all(|ch| { ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' }));
                }
            }
        }
    }

    #[test]
    fn migration_order_prioritizes_observe_then_followup_then_transform() {
        let selected_local =
            pre_planner_exit_for_reason("direct_answer_gate_promoted_to_planner").unwrap();
        let active_followup =
            pre_planner_exit_for_reason("direct_answer_gate_chat_fallback").unwrap();
        let inline_transform =
            pre_planner_exit_for_reason("inline_json_transform_promoted_to_planner").unwrap();

        assert_eq!(
            selected_local.migration_stage,
            "selected_local_observe_or_direct_scalar"
        );
        assert_eq!(
            active_followup.migration_stage,
            "active_task_followup_or_chat_rewrite"
        );
        assert_eq!(
            inline_transform.migration_stage,
            "inline_transform_or_structured_repair"
        );
        assert!(selected_local.migration_order < active_followup.migration_order);
        assert!(active_followup.migration_order < inline_transform.migration_order);
    }

    #[test]
    fn trace_context_exposes_machine_fields() {
        let item = pre_planner_exit_for_reason("inline_json_transform_promoted_to_planner")
            .expect("inventory item");
        let trace = item.trace_context();
        assert_eq!(
            trace.get("pre_planner_exit_kind").and_then(Value::as_str),
            Some("ordinary_semantic")
        );
        assert_eq!(
            trace
                .get("pre_planner_exit_reason_code")
                .and_then(Value::as_str),
            Some("inline_json_transform_promoted_to_planner")
        );
        assert!(trace
            .get("migration_target")
            .and_then(Value::as_str)
            .is_some());
        assert_eq!(
            trace.get("migration_stage").and_then(Value::as_str),
            Some("inline_transform_or_structured_repair")
        );
        assert_eq!(
            trace.get("migration_order").and_then(Value::as_u64),
            Some(30)
        );
        assert!(trace
            .get("nl_gate_refs")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()));
    }
}
