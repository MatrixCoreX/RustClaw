use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepairBoundaryClass {
    LoopBoundedRecovery,
    PermissionContractRepair,
    CheckpointResumeRepair,
}

impl RepairBoundaryClass {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::LoopBoundedRecovery => "loop_bounded_recovery",
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
    pub(crate) entrypoints: &'static [&'static str],
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
            "entrypoints": self.entrypoints,
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
        reason_code: "plan_repair_loop_recovery",
        repair_class: RepairBoundaryClass::LoopBoundedRecovery,
        owner_layer: "agent_engine_plan_repair",
        runtime_scope: "agent_loop",
        source_files: &[
            "crates/clawd/src/agent_engine/planning.rs",
            "crates/clawd/src/agent_engine/planning_repair.rs",
            "crates/clawd/src/prompt_utils_json_repair.rs",
            "prompts/layers/overlays/plan_repair_prompt.md",
        ],
        entrypoints: &["plan_round_actions", "repair_plan_actions"],
        allowed_input_fields: &[
            "planner_parse_error",
            "attempt_ledger",
            "planner_output",
            "turn_boundary_envelope",
            "tool_spec",
            "skill_playbooks",
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
        entrypoints: &[
            "verify_answer_observe_only",
            "local_missing_evidence_verifier_gap",
            "answer_verifier_retry_summary",
            "try_recover_latest_synthesis_answer_verifier_gap",
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
        entrypoints: &["verify_plan"],
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
        entrypoints: &[
            "preflight_permission_decision",
            "handle_preflight_argument_failure",
            "action_policy_for_output_contract",
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
        entrypoints: &[
            "run_skill",
            "execute_async_poll_dispatch_result_with_state",
            "record_attempt",
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
        entrypoints: &[
            "task_query_lifecycle_projection",
            "checkpoint_resume_directive",
            "execute_seeded_agent_loop_dispatch_result",
            "record_claimed_handoff_paused_checkpoint_resume_dispatch_internal",
            "build_attempt_ledger_snapshot",
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
        | "verify_boundary_clarify_required"
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
