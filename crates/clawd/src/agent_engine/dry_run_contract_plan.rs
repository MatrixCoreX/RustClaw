use super::*;

pub(super) fn structured_dry_run_response_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    let route_only_tokens = route_result
        .map(|route| format!("{}\n{}", route.route_reason, route.resolved_intent))
        .unwrap_or_default();
    if loop_state.round_no <= 1
        && finalizer_language_policy_dry_run_tokens_present(&route_only_tokens)
    {
        return Some(finalizer_language_policy_dry_run_plan(goal));
    }
    if loop_state.has_tool_or_skill_output || loop_state.round_no > 1 {
        return None;
    }
    if answer_verifier_contract_dry_run_tokens_present(&route_only_tokens) {
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
    if task_control_cancel_dry_run_tokens_present(&route_only_tokens) {
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
    if let Some(intent) = task_control_lifecycle_dry_run_intent(&route_only_tokens) {
        let actions = task_control_lifecycle_dry_run_actions(intent);
        return Some(build_plan_result(
            goal,
            "deterministic:task_control_lifecycle_dry_run_contract",
            PlanKind::Single,
            &actions,
        ));
    }
    if observed_output_projection_dry_run_tokens_present(&route_only_tokens) {
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
    if local_process_cancel_dry_run_tokens_present(&route_only_tokens) {
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
    if async_job_dry_run_tokens_present(&route_only_tokens) {
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

fn finalizer_language_policy_dry_run_plan(goal: &str) -> PlanResult {
    build_plan_result(
        goal,
        "deterministic:finalizer_language_policy_dry_run_contract",
        PlanKind::Single,
        &[AgentAction::Respond {
            content: serde_json::json!({
                "schema_version": 1,
                "semantic_kind": "finalizer_language_policy_dry_run",
                "would_mutate": false,
                "message_key": "clawd.finalizer.language_policy",
                "runtime_allowed_outputs": [
                    "message_key",
                    "structured_evidence"
                ],
                "runtime_forbidden_outputs": [
                    "fixed_user_reply_template",
                    "localized_reply_text",
                    "language_phrase_branch"
                ],
                "structured_evidence": {
                    "owner_layer": "runtime",
                    "output_contract": "message_key_or_structured_evidence",
                    "final_reply_owner": "finalizer_llm_i18n"
                },
                "final_reply_policy": {
                    "owner_layer": "finalizer",
                    "renderer": "finalizer_llm_i18n",
                    "language_source": "request_language_hint",
                    "input_channel": "message_key_or_structured_evidence"
                }
            })
            .to_string(),
        }],
    )
}

fn answer_verifier_contract_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    if !has_dry_run_machine_token(&normalized) {
        return false;
    }
    let has_verifier_contract = contains_machine_kv_or_json_pair(
        &normalized,
        "verifier_contract",
        "answer_verifier_required_evidence",
    );
    let has_boundary_owner =
        contains_machine_kv_or_json_pair(&normalized, "owner_layer", "answer_verifier");
    let has_runtime_scope =
        contains_machine_kv_or_json_pair(&normalized, "runtime_scope", "agent_loop");
    let has_contract_fields = [
        "required_evidence",
        "missing_evidence_fields",
        "contract_boundary",
    ]
    .into_iter()
    .all(|field| normalized.contains(field));
    has_verifier_contract && has_boundary_owner && has_runtime_scope && has_contract_fields
}

fn task_control_cancel_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    let has_explicit_cancel_action =
        contains_machine_kv_or_json_pair(&normalized, "capability_ref", "task_control.cancel_one")
            || contains_machine_kv_or_json_pair(
                &normalized,
                "capability_ref",
                "task_control.cancel_all",
            );
    has_explicit_cancel_action && has_dry_run_machine_token(&normalized)
}

fn observed_output_projection_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    if !has_dry_run_machine_token(&normalized) {
        return false;
    }
    let has_projection_contract = contains_machine_kv_or_json_pair(
        &normalized,
        "projection_contract",
        "observed_output_projection",
    );
    let has_observed_source =
        contains_machine_kv_or_json_pair(&normalized, "source", "observed_machine_output");
    let has_projection_families = [
        "scalar",
        "list",
        "path",
        "json_field",
        "status",
        "artifact_refs",
    ]
    .into_iter()
    .all(|family| normalized.contains(family));
    has_projection_contract && has_observed_source && has_projection_families
}

fn finalizer_language_policy_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    if !has_dry_run_machine_token(&normalized) {
        return false;
    }
    let has_policy_message_key = contains_machine_kv_or_json_pair(
        &normalized,
        "message_key",
        "clawd.finalizer.language_policy",
    );
    let has_finalizer_renderer =
        contains_machine_kv_or_json_pair(&normalized, "renderer", "finalizer_llm_i18n");
    let has_output_contract = contains_machine_kv_or_json_pair(
        &normalized,
        "output_contract",
        "message_key_or_structured_evidence",
    );
    has_policy_message_key && has_finalizer_renderer && has_output_contract
}

fn local_process_cancel_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    contains_machine_kv_or_json_pair(&normalized, "adapter_kind", "local_process_poll")
        && normalized.contains("cancel_ref")
        && normalized.contains("terminal_projection")
        && contains_machine_kv_or_json_pair(&normalized, "status", "cancelled")
        && has_dry_run_machine_token(&normalized)
}

fn async_job_dry_run_tokens_present(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    if !has_dry_run_machine_token(&normalized) {
        return false;
    }
    let has_async_job_protocol_envelope = normalized.contains("async_job_protocol=version:");
    let has_pending_job_contract = normalized.contains("pending_async_job_contract");
    let has_poll_adapter_result = normalized.contains("async_poll_adapter_result");
    let has_async_timeout_policy_envelope = normalized.contains("async_timeout_policy")
        && contains_machine_kv_or_json_pair(&normalized, "policy_source", "async_job_contract");
    let has_async_timeout_policy_tokens = normalized.contains("effective_deadline_ts")
        && normalized.contains("expires_at")
        && normalized.contains("remaining_seconds")
        && normalized.contains("expired");
    has_async_job_protocol_envelope
        || has_pending_job_contract
        || has_poll_adapter_result
        || (has_async_timeout_policy_envelope && has_async_timeout_policy_tokens)
}

fn contains_machine_kv_or_json_pair(text: &str, key: &str, value: &str) -> bool {
    let kv_pair = format!("{key}={value}");
    let compact_text = text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let compact_json_pair = format!("\"{}\":\"{}\"", key, value);
    text.contains(&kv_pair) || compact_text.contains(&compact_json_pair)
}

fn has_dry_run_machine_token(normalized: &str) -> bool {
    normalized.contains("dry_run")
        || normalized.contains("dry-run")
        || normalized.contains("would_mutate=false")
}

#[derive(Clone)]
struct TaskControlLifecycleDryRunIntent {
    include_resume: bool,
    include_pause: bool,
    task_id: Option<String>,
    checkpoint_id: Option<String>,
    pause_seconds: Option<u64>,
}

fn task_control_lifecycle_dry_run_intent(text: &str) -> Option<TaskControlLifecycleDryRunIntent> {
    let normalized = text.to_ascii_lowercase();
    if !has_dry_run_machine_token(&normalized) {
        return None;
    }
    let include_resume =
        contains_machine_kv_or_json_pair(&normalized, "capability_ref", "task_control.resume");
    let include_pause =
        contains_machine_kv_or_json_pair(&normalized, "capability_ref", "task_control.pause");
    if !include_resume && !include_pause {
        return None;
    }
    Some(TaskControlLifecycleDryRunIntent {
        include_resume,
        include_pause,
        task_id: machine_assignment_value(text, "task_id"),
        checkpoint_id: machine_assignment_value(text, "checkpoint_id"),
        pause_seconds: machine_assignment_value(text, "pause_seconds")
            .and_then(|value| value.parse::<u64>().ok()),
    })
}

fn task_control_lifecycle_dry_run_actions(
    intent: TaskControlLifecycleDryRunIntent,
) -> Vec<AgentAction> {
    let mut actions = Vec::new();
    if intent.include_resume {
        actions.push(AgentAction::CallSkill {
            skill: "task_control".to_string(),
            args: task_control_lifecycle_dry_run_skill_args("resume", &intent),
        });
    }
    if intent.include_pause {
        actions.push(AgentAction::CallSkill {
            skill: "task_control".to_string(),
            args: task_control_lifecycle_dry_run_skill_args("pause", &intent),
        });
    }
    actions.push(AgentAction::Respond {
        content: task_control_lifecycle_dry_run_summary(&intent),
    });
    actions
}

fn task_control_lifecycle_dry_run_skill_args(
    action: &str,
    intent: &TaskControlLifecycleDryRunIntent,
) -> Value {
    let mut args = serde_json::Map::new();
    args.insert("action".to_string(), Value::String(action.to_string()));
    args.insert("dry_run".to_string(), Value::Bool(true));
    if let Some(task_id) = intent.task_id.as_deref() {
        args.insert("task_id".to_string(), Value::String(task_id.to_string()));
    }
    if action == "resume" {
        if let Some(checkpoint_id) = intent.checkpoint_id.as_deref() {
            args.insert(
                "checkpoint_id".to_string(),
                Value::String(checkpoint_id.to_string()),
            );
        }
    }
    if action == "pause" {
        if let Some(pause_seconds) = intent.pause_seconds {
            args.insert(
                "pause_seconds".to_string(),
                Value::Number(serde_json::Number::from(pause_seconds)),
            );
        }
    }
    Value::Object(args)
}

fn task_control_lifecycle_dry_run_summary(intent: &TaskControlLifecycleDryRunIntent) -> String {
    let mut summary_tokens = Vec::new();
    if intent.include_resume {
        summary_tokens.push("task_control.resume.dry_run".to_string());
    }
    if intent.include_pause {
        summary_tokens.push("task_control.pause.dry_run".to_string());
    }
    if let Some(checkpoint_id) = intent.checkpoint_id.as_deref() {
        summary_tokens.push(format!("checkpoint_id={checkpoint_id}"));
    }
    if let Some(task_id) = intent.task_id.as_deref() {
        summary_tokens.push(format!("task_id={task_id}"));
    }
    if let Some(pause_seconds) = intent.pause_seconds {
        summary_tokens.push(format!("pause_seconds={pause_seconds}"));
    }
    summary_tokens.push("would_mutate=false".to_string());
    summary_tokens.join(" ")
}

fn machine_assignment_value(text: &str, key: &str) -> Option<String> {
    let normalized = text.to_ascii_lowercase();
    for marker in [
        format!("{key}="),
        format!("{key}:"),
        format!("\"{key}\":\""),
        format!("\"{key}\": \""),
    ] {
        let Some(start) = normalized.find(&marker).map(|idx| idx + marker.len()) else {
            continue;
        };
        let value: String = text[start..]
            .chars()
            .skip_while(|ch| ch.is_ascii_whitespace())
            .take_while(|ch| {
                ch.is_ascii_alphanumeric() || matches!(*ch, '-' | '_' | '.' | '/' | ':' | '@')
            })
            .collect();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}
