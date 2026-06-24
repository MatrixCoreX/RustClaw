use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepairBoundaryClass {
    SchemaCompatRepair,
    BoundarySafetyRepair,
    LoopBoundedRecovery,
    LegacyLogFallback,
    OrdinarySemanticRepair,
    PermissionContractRepair,
    CheckpointResumeRepair,
}

impl RepairBoundaryClass {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::SchemaCompatRepair => "schema_compat_repair",
            Self::BoundarySafetyRepair => "boundary_safety_repair",
            Self::LoopBoundedRecovery => "loop_bounded_recovery",
            Self::LegacyLogFallback => "legacy_log_fallback",
            Self::OrdinarySemanticRepair => "ordinary_semantic_repair",
            Self::PermissionContractRepair => "permission_contract_repair",
            Self::CheckpointResumeRepair => "checkpoint_resume_repair",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RepairBoundaryInventoryItem {
    pub(crate) reason_code: &'static str,
    pub(crate) repair_class: RepairBoundaryClass,
    pub(crate) owner_layer: &'static str,
    pub(crate) runtime_scope: &'static str,
    pub(crate) source_files: &'static [&'static str],
    pub(crate) allowed_input_fields: &'static [&'static str],
    pub(crate) forbidden_input_fields: &'static [&'static str],
    pub(crate) migration_target: &'static str,
    pub(crate) next_recovery_kind: &'static str,
    pub(crate) deletion_gate: &'static str,
}

impl RepairBoundaryInventoryItem {
    pub(crate) fn trace_value(self) -> Value {
        json!({
            "schema_version": 1,
            "reason_code": self.reason_code,
            "repair_class": self.repair_class.as_token(),
            "owner_layer": self.owner_layer,
            "runtime_scope": self.runtime_scope,
            "source_files": self.source_files,
            "allowed_input_fields": self.allowed_input_fields,
            "forbidden_input_fields": self.forbidden_input_fields,
            "migration_target": self.migration_target,
            "next_recovery_kind": self.next_recovery_kind,
            "deletion_gate": self.deletion_gate,
        })
    }
}

pub(crate) const REPAIR_BOUNDARY_INVENTORY: &[RepairBoundaryInventoryItem] = &[
    RepairBoundaryInventoryItem {
        reason_code: "normalizer_schema_contract_repair",
        repair_class: RepairBoundaryClass::SchemaCompatRepair,
        owner_layer: "intent_router_normalizer_model",
        runtime_scope: "pre_route_normalizer",
        source_files: &[
            "crates/clawd/src/intent_router_normalizer_model.rs",
            "crates/clawd/src/intent_router_contract_repair_report.rs",
            "crates/clawd/src/intent_router_contract_repair_judge.rs",
        ],
        allowed_input_fields: &[
            "llm_json_parse_error",
            "schema_field_path",
            "contract_repair_report",
            "machine_reason_code",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "skill_name_phrase",
        ],
        migration_target: "keep_schema_compat_only",
        next_recovery_kind: "schema_repair",
        deletion_gate: "keep_schema_compat_boundary",
    },
    RepairBoundaryInventoryItem {
        reason_code: "contract_repair_judge_schema_boundary",
        repair_class: RepairBoundaryClass::SchemaCompatRepair,
        owner_layer: "contract_repair_judge",
        runtime_scope: "pre_route_normalizer",
        source_files: &[
            "crates/clawd/src/intent_router_contract_repair_judge.rs",
            "prompts/layers/overlays/contract_repair_judge_prompt.md",
        ],
        allowed_input_fields: &[
            "normalized_contract_json",
            "schema_violation_code",
            "machine_override_marker",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "ordinary_semantic_route",
            "skill_selection_phrase",
        ],
        migration_target: "limit_to_schema_and_boundary_repair",
        next_recovery_kind: "schema_repair",
        deletion_gate: "keep_schema_compat_boundary",
    },
    RepairBoundaryInventoryItem {
        reason_code: "current_turn_missing_locator_boundary_repair",
        repair_class: RepairBoundaryClass::BoundarySafetyRepair,
        owner_layer: "intent_router_normalizer_run",
        runtime_scope: "pre_route_normalizer",
        source_files: &[
            "crates/clawd/src/intent_router_normalizer_run.rs",
            "crates/clawd/src/intent_router_current_turn_structural_repair.rs",
            "crates/clawd/src/intent_router_observation_repair.rs",
        ],
        allowed_input_fields: &[
            "current_turn_locator",
            "field_path",
            "target_kind",
            "route_reason_token",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "guessed_path_from_phrase",
        ],
        migration_target: "planner_clarify_or_structured_locator_recovery",
        next_recovery_kind: "clarify",
        deletion_gate: "keep_boundary_safety",
    },
    RepairBoundaryInventoryItem {
        reason_code: "active_text_followup_route_repair",
        repair_class: RepairBoundaryClass::OrdinarySemanticRepair,
        owner_layer: "intent_router_active_task_repair",
        runtime_scope: "pre_route_normalizer",
        source_files: &["crates/clawd/src/intent_router_active_task_repair.rs"],
        allowed_input_fields: &[
            "active_task_id",
            "observed_action_ref",
            "current_turn_locator",
            "machine_route_reason",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "language_phrase_array",
        ],
        migration_target: "migrate_to_agent_loop_followup_recovery",
        next_recovery_kind: "replan",
        deletion_gate: "delete_after_agent_loop_followup_gate",
    },
    RepairBoundaryInventoryItem {
        reason_code: "post_route_legacy_semantic_repair_deferral",
        repair_class: RepairBoundaryClass::LegacyLogFallback,
        owner_layer: "post_route_policy_legacy_semantic_repair",
        runtime_scope: "post_route_policy",
        source_files: &[
            "crates/clawd/src/post_route_policy.rs",
            "crates/clawd/src/post_route_policy/legacy_semantic_repair.rs",
        ],
        allowed_input_fields: &[
            "route_gate_kind",
            "route_reason_token",
            "semantic_route_authority",
            "journal_trace_token",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "ordinary_semantic_route",
        ],
        migration_target: "defer_selected_agent_loop_routes_to_loop",
        next_recovery_kind: "replan",
        deletion_gate: "delete_after_agent_loop_default",
    },
    RepairBoundaryInventoryItem {
        reason_code: "plan_repair_loop_recovery",
        repair_class: RepairBoundaryClass::LoopBoundedRecovery,
        owner_layer: "agent_engine_plan_repair",
        runtime_scope: "agent_loop",
        source_files: &[
            "crates/clawd/src/agent_engine/planning.rs",
            "crates/clawd/src/agent_engine/direct_observed_finalize_support.rs",
            "crates/clawd/src/agent_engine/session_alias_target_coverage.rs",
            "prompts/layers/overlays/plan_repair_prompt.md",
        ],
        allowed_input_fields: &[
            "repair_envelope",
            "attempt_ledger",
            "observed_action_refs",
            "evidence_gap",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "runtime_fixed_reply_template",
        ],
        migration_target: "keep_inside_agent_loop_bounded_recovery",
        next_recovery_kind: "replan",
        deletion_gate: "keep_loop_bounded_recovery",
    },
    RepairBoundaryInventoryItem {
        reason_code: "answer_verifier_missing_evidence_repair",
        repair_class: RepairBoundaryClass::LoopBoundedRecovery,
        owner_layer: "answer_verifier",
        runtime_scope: "agent_loop",
        source_files: &[
            "crates/clawd/src/answer_verifier.rs",
            "crates/clawd/src/answer_verifier_runtime.rs",
            "crates/clawd/src/agent_engine/loop_control_answer_recovery.rs",
            "crates/clawd/src/agent_engine/loop_control_local_health_recovery.rs",
            "crates/clawd/src/task_journal.rs",
        ],
        allowed_input_fields: &[
            "missing_evidence_fields",
            "should_retry",
            "verifier_confidence",
            "observed_field_path",
            "machine_issue_code",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "retry_reply_sentence",
        ],
        migration_target: "repair_envelope_to_loop_recovery",
        next_recovery_kind: "replan",
        deletion_gate: "keep_loop_bounded_recovery",
    },
    RepairBoundaryInventoryItem {
        reason_code: "plan_verifier_issue_repair",
        repair_class: RepairBoundaryClass::LoopBoundedRecovery,
        owner_layer: "plan_verifier",
        runtime_scope: "agent_loop",
        source_files: &[
            "crates/clawd/src/verifier.rs",
            "crates/clawd/src/repair_signal.rs",
            "crates/clawd/src/task_journal.rs",
        ],
        allowed_input_fields: &[
            "verify_issue_kind",
            "status_code",
            "message_key",
            "step_id",
            "missing_fields",
            "rejected_action",
            "suggested_contract_action",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "retry_reply_sentence",
        ],
        migration_target: "repair_envelope_to_loop_recovery",
        next_recovery_kind: "replan",
        deletion_gate: "keep_loop_bounded_recovery",
    },
    RepairBoundaryInventoryItem {
        reason_code: "permission_contract_repair",
        repair_class: RepairBoundaryClass::PermissionContractRepair,
        owner_layer: "permission_contract_boundary",
        runtime_scope: "execution_preflight",
        source_files: &[
            "crates/clawd/src/verifier.rs",
            "crates/clawd/src/contract_matrix_runtime.rs",
            "crates/clawd/src/agent_engine/skill_execution_preflight.rs",
            "configs/task_contract_matrix.toml",
        ],
        allowed_input_fields: &[
            "permission_decision",
            "risk_level",
            "action_effect",
            "requires_confirmation",
            "dry_run_required",
            "contract_failure_policy",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "permission_bypass_action",
            "risk_upgrade_repair",
        ],
        migration_target: "deny_or_require_confirmation_without_bypass",
        next_recovery_kind: "needs_user",
        deletion_gate: "keep_policy_boundary",
    },
    RepairBoundaryInventoryItem {
        reason_code: "provider_blocker_recovery",
        repair_class: RepairBoundaryClass::LoopBoundedRecovery,
        owner_layer: "execution_provider_blocker",
        runtime_scope: "agent_loop",
        source_files: &[
            "crates/clawd/src/execution_adapters.rs",
            "crates/clawd/src/agent_engine/loop_control.rs",
            "crates/clawd/src/worker/async_poll_executor.rs",
        ],
        allowed_input_fields: &[
            "provider_status",
            "status_code",
            "retry_after_seconds",
            "external_provider_blocked",
            "provider_supported",
            "unsupported_reason",
            "message_key",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "semantic_route_error",
        ],
        migration_target: "wait_background_or_structured_blocker",
        next_recovery_kind: "wait_background",
        deletion_gate: "keep_loop_bounded_recovery",
    },
    RepairBoundaryInventoryItem {
        reason_code: "checkpoint_resume_recovery",
        repair_class: RepairBoundaryClass::CheckpointResumeRepair,
        owner_layer: "task_lifecycle_resume",
        runtime_scope: "worker_resume",
        source_files: &[
            "crates/clawd/src/task_lifecycle.rs",
            "crates/clawd/src/repo/task_resume_execution.rs",
            "crates/clawd/src/worker/runtime_support.rs",
            "crates/clawd/src/worker/resume_replay_executor.rs",
            "crates/clawd/src/worker/async_poll_executor.rs",
            "crates/clawd/src/agent_engine/attempt_ledger.rs",
        ],
        allowed_input_fields: &[
            "checkpoint_id",
            "resume_entrypoint",
            "attempt_fingerprint",
            "side_effect_fingerprint",
            "completed_action_refs",
        ],
        forbidden_input_fields: &[
            "user_prompt_phrase",
            "localized_reply_text",
            "text",
            "error_text",
            "rerun_completed_side_effect",
        ],
        migration_target: "resume_from_checkpoint_without_replaying_side_effects",
        next_recovery_kind: "wait_background",
        deletion_gate: "keep_lifecycle_recovery",
    },
];

pub(crate) fn repair_inventory_for_signal(
    reason_code: Option<&str>,
    status_code: Option<&str>,
    owner_layer: Option<&str>,
    source_token: Option<&str>,
) -> Option<RepairBoundaryInventoryItem> {
    if let Some(reason_code) = reason_code.and_then(non_empty_trimmed) {
        if let Some(item) = REPAIR_BOUNDARY_INVENTORY
            .iter()
            .copied()
            .find(|item| item.reason_code == reason_code)
        {
            return Some(item);
        }
        if let Some(item) = inventory_for_reason_alias(reason_code) {
            return Some(item);
        }
    }

    if let Some(status_code) = status_code.and_then(non_empty_trimmed) {
        if let Some(item) = REPAIR_BOUNDARY_INVENTORY
            .iter()
            .copied()
            .find(|item| item.reason_code == status_code)
        {
            return Some(item);
        }
        if let Some(item) = inventory_for_reason_alias(status_code) {
            return Some(item);
        }
    }

    if let Some(owner_layer) = owner_layer.and_then(non_empty_trimmed) {
        if let Some(item) = REPAIR_BOUNDARY_INVENTORY
            .iter()
            .copied()
            .find(|item| item.owner_layer == owner_layer)
        {
            return Some(item);
        }
        if let Some(item) = inventory_for_owner_alias(owner_layer) {
            return Some(item);
        }
    }

    source_token.and_then(non_empty_trimmed).and_then(|source| {
        REPAIR_BOUNDARY_INVENTORY
            .iter()
            .copied()
            .find(|item| match source {
                "verifier" => item.reason_code == "plan_verifier_issue_repair",
                "answer_verifier" => item.reason_code == "answer_verifier_missing_evidence_repair",
                "executor" => item.reason_code == "provider_blocker_recovery",
                "runtime" => item.reason_code == "checkpoint_resume_recovery",
                _ => false,
            })
    })
}

fn inventory_for_owner_alias(owner_layer: &str) -> Option<RepairBoundaryInventoryItem> {
    let reason_code = match owner_layer {
        "answer_verifier_runtime" => "answer_verifier_missing_evidence_repair",
        "plan_repair" | "agent_engine_planning" => "plan_repair_loop_recovery",
        "task_lifecycle" | "resume_replay_executor" | "async_poll_executor" => {
            "checkpoint_resume_recovery"
        }
        _ => return None,
    };
    REPAIR_BOUNDARY_INVENTORY
        .iter()
        .copied()
        .find(|item| item.reason_code == reason_code)
}

fn inventory_for_reason_alias(reason_code: &str) -> Option<RepairBoundaryInventoryItem> {
    let inventory_reason = match reason_code {
        "verify_confirmation_required" | "verify_risk_budget_exceeded" => {
            "permission_contract_repair"
        }
        "verify_contract_action_rejected"
        | "verify_contract_missing"
        | "verify_contract_policy_violation"
        | "verify_contract_preferred_action_available" => "permission_contract_repair",
        "contract_action_rejected"
        | "contract_arg_rejected"
        | "unsafe_sql"
        | "invalid_credentials"
        | "credential_missing"
        | "auth_failed" => "permission_contract_repair",
        "not_found"
        | "exit_status"
        | "command_failed"
        | "timeout"
        | "provider_error"
        | "provider_retryable_response"
        | "rate_limited"
        | "quota_exhausted"
        | "quota_exceeded" => "provider_blocker_recovery",
        "verify_skill_not_visible"
        | "verify_capability_unavailable"
        | "verify_missing_required_arg"
        | "verify_default_creation_target_applied"
        | "verify_unresolved_template_arg"
        | "verify_invalid_depends_on"
        | "verify_primary_fallback_conflict"
        | "verify_route_clarify_required"
        | "verify_recipe_inspect_before_mutate_required"
        | "verify_recipe_validation_after_mutate_required"
        | "verify_recipe_target_scope_required" => "plan_verifier_issue_repair",
        _ => return None,
    };
    REPAIR_BOUNDARY_INVENTORY
        .iter()
        .copied()
        .find(|item| item.reason_code == inventory_reason)
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
#[path = "repair_boundary_inventory_tests.rs"]
mod tests;
