use super::*;

pub(super) fn structured_dry_run_response_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.has_tool_or_skill_output || loop_state.round_no > 1 {
        return None;
    }
    let route_tokens = format!(
        "{}\n{}\n{}",
        route.route_reason, route.resolved_intent, goal
    );
    if answer_verifier_contract_dry_run_tokens_present(&route_tokens) {
        return Some(build_plan_result(
            goal,
            "deterministic:answer_verifier_contract_dry_run",
            PlanKind::Single,
            &[AgentAction::Respond {
                content: serde_json::json!({
                    "schema_version": 1,
                    "semantic_kind": "answer_verifier_contract_dry_run",
                    "would_mutate": false,
                    "required_evidence": [
                        "required_evidence",
                        "missing_evidence_fields",
                        "contract_boundary"
                    ],
                    "missing_evidence_fields": [],
                    "contract_boundary": {
                        "owner_layer": "answer_verifier",
                        "runtime_scope": "agent_loop",
                        "allowed_input_fields": [
                            "required_evidence",
                            "missing_evidence_fields",
                            "observed_field_path",
                            "machine_issue_code",
                            "verifier_confidence"
                        ],
                        "forbidden_input_fields": [
                            "text",
                            "error_text",
                            "localized_reply_text",
                            "retry_reply_sentence"
                        ],
                        "final_reply_policy": "message_key_or_finalizer_i18n"
                    },
                    "execution_policy": {
                        "call_provider": false,
                        "call_tool": false,
                        "write_runtime_reply_template": false
                    }
                })
                .to_string(),
            }],
        ));
    }
    if task_control_cancel_dry_run_tokens_present(&route_tokens) {
        return Some(build_plan_result(
            goal,
            "deterministic:task_control_cancel_dry_run_contract",
            PlanKind::Single,
            &[AgentAction::Respond {
                content: serde_json::json!({
                    "schema_version": 1,
                    "semantic_kind": "task_control_cancel_dry_run",
                    "would_mutate": false,
                    "required_fields": ["task_id", "state", "can_cancel"],
                    "cancel_request": {
                        "action": "cancel",
                        "dry_run": true,
                        "task_id": null
                    },
                    "precondition_fields": {
                        "state": "running_or_queued",
                        "can_cancel": true
                    },
                    "result_projection_fields": {
                        "state": "cancel_requested_or_canceled",
                        "can_cancel": false,
                        "can_poll": true,
                        "db_status": "canceled_or_terminal",
                        "last_heartbeat_ts": "optional",
                        "checkpoint_id": "optional"
                    },
                    "execution_policy": {
                        "call_task_cancel_api": false,
                        "call_task_control_cancel": false
                    }
                })
                .to_string(),
            }],
        ));
    }
    if observed_output_projection_dry_run_tokens_present(&route_tokens) {
        return Some(build_plan_result(
            goal,
            "deterministic:observed_output_projection_dry_run_contract",
            PlanKind::Single,
            &[AgentAction::Respond {
                content: serde_json::json!({
                    "schema_version": 1,
                    "semantic_kind": "observed_output_projection_dry_run",
                    "would_mutate": false,
                    "families": [
                        "scalar",
                        "list",
                        "path",
                        "json_field",
                        "status",
                        "artifact_refs"
                    ],
                    "projection_policy": {
                        "source": "observed_machine_output",
                        "parse_user_language": false,
                        "render_final_prose": false
                    }
                })
                .to_string(),
            }],
        ));
    }
    if local_process_cancel_dry_run_tokens_present(&route_tokens) {
        return Some(build_plan_result(
            goal,
            "deterministic:local_process_cancel_dry_run_contract",
            PlanKind::Single,
            &[AgentAction::Respond {
                content: serde_json::json!({
                    "schema_version": 1,
                    "semantic_kind": "local_process_cancel_dry_run",
                    "would_mutate": false,
                    "adapter_kind": "local_process_poll",
                    "cancel_ref": "optional_cancel_reference",
                    "status": "cancelled",
                    "terminal_projection": {
                        "state": "cancelled",
                        "can_poll": true,
                        "can_cancel": false,
                        "terminal": true
                    },
                    "execution_policy": {
                        "send_signal": false,
                        "kill_process": false,
                        "poll_external_job": false
                    }
                })
                .to_string(),
            }],
        ));
    }
    if async_job_dry_run_tokens_present(&route_tokens) {
        return Some(build_plan_result(
            goal,
            "deterministic:async_job_poll_contract_dry_run",
            PlanKind::Single,
            &[AgentAction::Respond {
                content: serde_json::json!({
                    "schema_version": 1,
                    "semantic_kind": "async_job_poll_contract_dry_run",
                    "would_mutate": false,
                    "adapter_result": {
                        "type": "pending_async_job",
                        "job_id": "opaque_async_job_id",
                        "status": "poll_pending",
                        "poll_async_job": "poll_async_job",
                        "next_check_after": "duration_or_timestamp",
                        "poll_after_seconds": "number",
                        "expires_at": "rfc3339_timestamp",
                        "cancel_ref": "optional_cancel_reference",
                        "message_key": "stable_i18n_message_key"
                    },
                    "async_timeout_policy": {
                        "schema_version": 1,
                        "policy_source": "async_job_contract",
                        "deadline_ts": "adapter_result.expires_at",
                        "max_runtime_deadline_ts": "adapter_max_runtime_deadline",
                        "effective_deadline_ts": "min(deadline_ts,max_runtime_deadline_ts)",
                        "remaining_seconds": "max(effective_deadline_ts-now_ts,0)",
                        "expired_terminal_status": "expired"
                    },
                    "task_lifecycle": {
                        "state": "waiting",
                        "checkpoint_id": "opaque_checkpoint_id",
                        "poll_ref": "adapter_result.job_id",
                        "next_check_after": "adapter_result.next_check_after",
                        "can_poll": true,
                        "can_cancel": true
                    },
                    "worker_loop": {
                        "entrypoint": "poll_async_job",
                        "poll_key": "job_id",
                        "next_check_after": "adapter_result.next_check_after",
                        "expires_at": "adapter_result.expires_at",
                        "message_key": "adapter_result.message_key",
                        "terminal_statuses": ["succeeded", "failed", "expired", "cancelled"],
                        "final_step": "verify_finalize"
                    },
                    "execution_policy": {
                        "start_real_job": false,
                        "persist_job": false,
                        "poll_external_job": false
                    }
                })
                .to_string(),
            }],
        ));
    }
    None
}

fn answer_verifier_contract_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    normalized.contains("required_evidence")
        && normalized.contains("missing_evidence_fields")
        && normalized.contains("contract_boundary")
        && has_dry_run_machine_token(&normalized)
}

fn task_control_cancel_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    let has_explicit_cancel_action = normalized.contains("task_control.cancel")
        || normalized.contains("cancel_all")
        || normalized.contains("cancel_one");
    let has_task_control_cancel_contract = normalized.contains("task_control")
        && normalized.contains("task_id")
        && normalized.contains("state")
        && normalized.contains("can_cancel")
        && (normalized.contains("cancel") || normalized.contains("cancel_requested"));
    (has_explicit_cancel_action || has_task_control_cancel_contract)
        && has_dry_run_machine_token(&normalized)
}

fn observed_output_projection_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    normalized.contains("observed-output")
        && normalized.contains("scalar")
        && normalized.contains("list")
        && normalized.contains("path")
        && (normalized.contains("json field") || normalized.contains("json_field"))
        && normalized.contains("status")
        && normalized.contains("artifact_refs")
        && has_dry_run_machine_token(&normalized)
}

fn local_process_cancel_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    normalized.contains("local_process_poll")
        && normalized.contains("cancel_ref")
        && normalized.contains("terminal_projection")
        && (normalized.contains("status=cancelled") || normalized.contains("status\":\"cancelled"))
        && has_dry_run_machine_token(&normalized)
}

fn async_job_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    let has_async_timeout_policy_tokens = normalized.contains("effective_deadline_ts")
        && normalized.contains("expires_at")
        && normalized.contains("remaining_seconds")
        && normalized.contains("expired");
    (normalized.contains("pending_async_job")
        || normalized.contains("async_job_protocol")
        || normalized.contains("poll_async_job")
        || has_async_timeout_policy_tokens)
        && has_dry_run_machine_token(&normalized)
}

fn has_dry_run_machine_token(normalized: &str) -> bool {
    normalized.contains("dry_run")
        || normalized.contains("dry-run")
        || normalized.contains("would_mutate=false")
}
