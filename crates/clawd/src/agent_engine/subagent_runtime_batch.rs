use super::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SUBAGENT_FINDINGS: usize = 16;
const MAX_SUBAGENT_FINDING_KEYS: usize = 16;

pub(super) fn record_subagent_batch_action_from_args_with_config(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
    config: &SubagentRuntimeConfig,
) -> Option<Option<&'static str>> {
    let invocation_policy = SubagentInvocationPolicy::from_args(args);
    let children = subagent_child_actions_from_args(args)?;
    Some(record_subagent_batch_action_with_config(
        loop_state,
        global_step,
        step_in_round,
        children,
        config,
        invocation_policy,
    ))
}

fn record_subagent_batch_action_with_config(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    children: Vec<SubagentChildAction>,
    config: &SubagentRuntimeConfig,
    invocation_policy: SubagentInvocationPolicy,
) -> Option<&'static str> {
    let parallel_batch_id = format!("subagent-batch:{}:{}", loop_state.round_no, step_in_round);
    let max_parallel_readonly = config.max_parallel_readonly as usize;
    let requested_child_count = children.len();
    let mut child_results = Vec::new();
    let mut child_requests = Vec::new();
    let mut child_summaries = Vec::new();
    let mut team_children = Vec::new();
    let mut team_lifecycle_events = vec![team_lifecycle_event(
        "agent_team_started",
        parallel_batch_id.as_str(),
        None,
        "started",
        None,
        None,
        None,
    )];
    let mut completed_count = 0usize;
    let mut rejected_count = 0usize;
    let mut skipped_count = 0usize;
    let mut required_failed_count = 0usize;
    let mut optional_failed_count = 0usize;
    let mut aggregated_evidence_refs = Vec::new();
    let mut aggregated_finding_refs = Vec::new();
    let mut aggregated_context_evidence_items = Vec::new();
    let mut finding_signals = AggregatedFindingSignals::default();

    for (child_index, child) in children.into_iter().enumerate() {
        let child_run_id = format!(
            "{}:{}:{}",
            parallel_batch_id,
            child_index + 1,
            normalize_machine_token(child.role.as_str())
        );
        let role_token = child.role.trim();
        team_lifecycle_events.push(team_lifecycle_event(
            "subagent_started",
            parallel_batch_id.as_str(),
            Some(child_run_id.as_str()),
            "started",
            Some(machine_ref_or_empty(role_token)),
            Some(child.required),
            None,
        ));
        let Some(role) = SubagentRole::parse_token(role_token) else {
            rejected_count += 1;
            if child.required {
                required_failed_count += 1;
            } else {
                optional_failed_count += 1;
            }
            let child_result = rejected_child_result(
                child_run_id.as_str(),
                role_token,
                child.required,
                "subagent_role_not_allowed",
            );
            team_children.push(agent_team_child_spec(
                child_run_id.as_str(),
                machine_ref_or_empty(role_token),
                "unresolved",
                child.required,
                None,
                "rejected",
            ));
            team_lifecycle_events.push(team_lifecycle_event(
                "subagent_failed",
                parallel_batch_id.as_str(),
                Some(child_run_id.as_str()),
                "rejected",
                Some(machine_ref_or_empty(role_token)),
                Some(child.required),
                Some("subagent_role_not_allowed"),
            ));
            child_summaries.push(child_summary_from_result(&child_result));
            child_results.push(child_result);
            continue;
        };
        if !config.role_allowed(role) {
            rejected_count += 1;
            if child.required {
                required_failed_count += 1;
            } else {
                optional_failed_count += 1;
            }
            let child_result = rejected_child_result(
                child_run_id.as_str(),
                role.as_token(),
                child.required,
                "subagent_role_disabled_by_config",
            );
            team_children.push(agent_team_child_spec(
                child_run_id.as_str(),
                role.as_token(),
                role.default_scope_token(),
                child.required,
                None,
                "rejected",
            ));
            team_lifecycle_events.push(team_lifecycle_event(
                "subagent_failed",
                parallel_batch_id.as_str(),
                Some(child_run_id.as_str()),
                "rejected",
                Some(role.as_token()),
                Some(child.required),
                Some("subagent_role_disabled_by_config"),
            ));
            child_summaries.push(child_summary_from_result(&child_result));
            child_results.push(child_result);
            continue;
        }
        if completed_count >= max_parallel_readonly {
            skipped_count += 1;
            if child.required {
                required_failed_count += 1;
            } else {
                optional_failed_count += 1;
            }
            let child_result = rejected_child_result(
                child_run_id.as_str(),
                role.as_token(),
                child.required,
                "subagent_parallel_limit_exceeded",
            );
            team_children.push(agent_team_child_spec(
                child_run_id.as_str(),
                role.as_token(),
                role.default_scope_token(),
                child.required,
                None,
                "skipped",
            ));
            team_lifecycle_events.push(team_lifecycle_event(
                "subagent_failed",
                parallel_batch_id.as_str(),
                Some(child_run_id.as_str()),
                "skipped",
                Some(role.as_token()),
                Some(child.required),
                Some("subagent_parallel_limit_exceeded"),
            ));
            child_summaries.push(child_summary_from_result(&child_result));
            child_results.push(child_result);
            continue;
        }

        let context_refs = safe_context_refs(&child.context_refs);
        let allowed_capabilities = safe_machine_token_list(&child.options.allowed_capabilities);
        let context_ref_count = context_refs.len();
        let allowed_capability_count = allowed_capabilities.len();
        let budget_summary = subagent_budget_summary(child.options.budget.as_ref(), config);
        let timeout_policy = subagent_timeout_policy(&budget_summary);
        let cancellation_policy = subagent_cancellation_policy(&timeout_policy);
        let timeout_ms = timeout_policy.get("timeout_ms").and_then(Value::as_u64);
        let context_evidence = context_evidence_summary(&context_refs, &child.options, config);
        let content_excerpt = context_evidence_combined_excerpt(&context_evidence);
        let content_paths = context_evidence_paths(&context_evidence);
        let primary_content_path = content_paths.first().cloned().unwrap_or_default();
        let content_excerpt_present = context_evidence_has_available_excerpt(&context_evidence);
        if let Some(items) = context_evidence.get("items").and_then(Value::as_array) {
            aggregated_context_evidence_items.extend(items.iter().cloned());
        }
        let findings = sanitized_findings(child.findings.as_ref());
        let finding_count = findings.len();
        finding_signals.observe_child_findings(child_run_id.as_str(), &findings);
        let child_request = child_request_envelope(
            child_run_id.as_str(),
            role,
            context_ref_count,
            allowed_capability_count,
            child.options.budget.as_ref(),
            config,
        );
        let child_result = json!({
            "schema_version": 1,
            "output_format": "machine_json",
            "child_run_id": child_run_id.as_str(),
            "status": "completed",
            "result_status": "completed",
            "outcome_code": "subagent_inline_readonly_completed",
            "role": role.as_token(),
            "role_family": role.family_token(),
            "required": child.required,
            "context_ref_count": context_ref_count,
            "allowed_capability_count": allowed_capability_count,
            "result_contract_present": child.options.result_contract.is_some(),
            "result_contract_required": role.result_contract_required(),
            "content_excerpt_present": content_excerpt_present,
            "findings": findings,
            "finding_count": finding_count,
            "write_enabled": false,
            "external_publish_enabled": false,
            "failure_isolated": true,
        });
        completed_count += 1;
        team_children.push(agent_team_child_spec(
            child_run_id.as_str(),
            role.as_token(),
            role.default_scope_token(),
            child.required,
            timeout_ms,
            "completed",
        ));
        team_lifecycle_events.push(team_lifecycle_event(
            "subagent_finished",
            parallel_batch_id.as_str(),
            Some(child_run_id.as_str()),
            "completed",
            Some(role.as_token()),
            Some(child.required),
            None,
        ));
        aggregated_evidence_refs.push(child_run_id.clone());
        if finding_count > 0 {
            aggregated_finding_refs.push(child_run_id.clone());
        }
        child_requests.push(json!({
            "child_run_id": child_run_id.as_str(),
            "request": child_request,
            "context_refs": context_refs,
            "path": primary_content_path.as_str(),
            "paths": content_paths,
            "excerpt": content_excerpt.as_str(),
            "content_excerpt": content_excerpt.as_str(),
            "context_evidence": context_evidence,
            "allowed_capabilities": allowed_capabilities,
            "budget": budget_summary,
            "timeout_policy": timeout_policy,
            "cancellation_policy": cancellation_policy,
            "context_slice": context_slice_summary(child.options.context_slice.as_ref()),
            "result_contract": result_contract_summary(child.options.result_contract.as_ref()),
            "objective_present": !child.objective.trim().is_empty(),
            "objective_char_count": child.objective.chars().count(),
            "required": child.required,
        }));
        child_summaries.push(child_summary_from_result(&child_result));
        child_results.push(child_result);
    }

    let aggregate_status = if required_failed_count > 0 {
        "failed_required_child"
    } else if rejected_count > 0 || skipped_count > 0 {
        "partial"
    } else {
        "completed"
    };
    let scheduler_status = if required_failed_count > 0 {
        "failed_required_child"
    } else if skipped_count > 0 {
        "bounded_completed_with_skips"
    } else if rejected_count > 0 {
        "partial_completed"
    } else {
        "bounded_parallel_completed"
    };
    let expected_failure_delivery =
        invocation_policy.expected_failure_delivery(required_failed_count);
    let parent_failure_isolated = expected_failure_delivery || required_failed_count == 0;
    let parent_status = if required_failed_count > 0 && !expected_failure_delivery {
        "failed"
    } else {
        "accepted"
    };
    let parent_result_status = if expected_failure_delivery {
        "completed_expected_failure"
    } else {
        aggregate_status
    };
    let parent_outcome_code = if expected_failure_delivery {
        "subagent_expected_required_child_failure_observed"
    } else if required_failed_count > 0 {
        "subagent_required_child_failed"
    } else if rejected_count > 0 || skipped_count > 0 {
        "subagent_parallel_partial_completed"
    } else {
        "subagent_parallel_readonly_completed"
    };
    let parent_scheduler_status = if expected_failure_delivery {
        "expected_required_child_failure_observed"
    } else {
        scheduler_status
    };
    let conflict_summary = finding_signals.conflict_summary();
    let conflict_count = conflict_summary
        .get("conflict_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let confidence_summary = finding_signals.confidence_summary();
    let main_thread_decision = aggregation_main_thread_decision(
        required_failed_count,
        conflict_count,
        expected_failure_delivery,
    );
    let recommended_next_action = main_thread_decision
        .get("recommended_next_action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if conflict_count > 0 {
        team_lifecycle_events.push(team_lifecycle_event(
            "agent_team_conflict_detected",
            parallel_batch_id.as_str(),
            None,
            "needs_conflict_resolution",
            None,
            None,
            Some("subagent_conflict_detected"),
        ));
    }
    team_lifecycle_events.push(team_lifecycle_event(
        "agent_team_aggregated",
        parallel_batch_id.as_str(),
        None,
        parent_result_status,
        None,
        None,
        None,
    ));
    let team_spec = agent_team_spec(
        parallel_batch_id.as_str(),
        invocation_policy.parent_task_id.as_deref(),
        config.max_parallel_readonly,
        &team_children,
    );
    let parent_context_evidence =
        context_evidence_summary_from_items(aggregated_context_evidence_items);
    let parent_content_excerpt = context_evidence_combined_excerpt(&parent_context_evidence);
    let parent_content_paths = context_evidence_paths(&parent_context_evidence);
    let parent_primary_content_path = parent_content_paths.first().cloned().unwrap_or_default();
    let child_result = json!({
        "schema_version": 1,
        "output_format": "machine_json",
        "status": aggregate_status,
        "result_status": aggregate_status,
        "outcome_code": if required_failed_count > 0 {
            "subagent_required_child_failed"
        } else if rejected_count > 0 || skipped_count > 0 {
            "subagent_parallel_partial_completed"
        } else {
            "subagent_parallel_readonly_completed"
        },
        "parallel_batch_id": parallel_batch_id.as_str(),
        "child_count": requested_child_count,
        "completed_count": completed_count,
        "rejected_count": rejected_count,
        "skipped_count": skipped_count,
        "required_failed_count": required_failed_count,
        "optional_failed_count": optional_failed_count,
        "conflict_count": conflict_count,
        "content_excerpt_present": context_evidence_has_available_excerpt(&parent_context_evidence),
        "write_enabled": false,
        "external_publish_enabled": false,
        "failure_isolated": required_failed_count == 0,
    });
    let mut observation = json!({
        "schema_version": 1,
        "owner_layer": "subagent_runtime",
        "status": parent_status,
        "result_status": parent_result_status,
        "outcome_code": parent_outcome_code,
        "execution_mode": "bounded_parallel_readonly_child_runs",
        "parallel_batch_id": parallel_batch_id.as_str(),
        "children_requested": requested_child_count,
        "children_scheduled": completed_count,
        "children_rejected": rejected_count,
        "children_skipped": skipped_count,
        "dry_run": invocation_policy.dry_run,
        "expected_failure": invocation_policy.expected_failure,
        "expected_failure_delivery": expected_failure_delivery,
        "actual_required_child_failed": required_failed_count > 0,
        "actual_failure_isolated": required_failed_count == 0,
        "runtime_config": config.trace_summary(),
        "team_spec": team_spec,
        "team_lifecycle_events": team_lifecycle_events,
        "scheduler": {
            "status": parent_scheduler_status,
            "reason_code": "bounded_parallel_readonly_execution",
            "lease_required": false,
            "checkpoint_required": false,
            "max_parallel_readonly": config.max_parallel_readonly,
            "requested_child_count": requested_child_count,
            "scheduled_child_count": completed_count,
            "skipped_child_count": skipped_count,
            "dry_run": invocation_policy.dry_run,
            "expected_failure": invocation_policy.expected_failure,
            "expected_failure_delivery": expected_failure_delivery,
        },
        "merge_contract": {
            "strategy": "merge_child_structured_findings",
            "parent_trace_event_type": "subagent",
            "child_trace_merge_status": "merged",
            "result_status": aggregate_status,
            "parent_result_status": parent_result_status,
            "failure_isolated": parent_failure_isolated,
            "actual_failure_isolated": required_failed_count == 0,
            "expected_failure_delivery": expected_failure_delivery,
        },
        "aggregation": {
            "schema_version": 1,
            "execution_mode": "bounded_parallel_readonly_child_runs",
            "status": aggregate_status,
            "strategy": "merge_child_machine_findings",
            "child_count": requested_child_count,
            "completed_count": completed_count,
            "rejected_count": rejected_count,
            "skipped_count": skipped_count,
            "required_failed_count": required_failed_count,
            "optional_failed_count": optional_failed_count,
            "evidence_refs": aggregated_evidence_refs,
            "finding_refs": aggregated_finding_refs,
            "finding_count": finding_signals.finding_count,
            "confidence_summary": confidence_summary,
            "conflict_summary": conflict_summary,
            "conflict_count": conflict_count,
            "main_thread_decision": main_thread_decision,
            "recommended_next_action": recommended_next_action,
            "expected_failure_delivery": expected_failure_delivery,
        },
        "child_requests": child_requests,
        "child_run_summaries": child_summaries,
        "child_results": child_results,
        "child_run_summary": {
            "parallel_batch_id": parallel_batch_id.as_str(),
            "status": aggregate_status,
            "result_status": aggregate_status,
            "trace_merge_status": "merged",
            "child_count": requested_child_count,
            "completed_count": completed_count,
            "rejected_count": rejected_count,
            "skipped_count": skipped_count,
            "conflict_count": conflict_count,
        },
        "child_result": child_result,
        "write_enabled": false,
        "external_publish_enabled": false,
        "failure_isolated": parent_failure_isolated,
        "global_step": global_step,
        "step_in_round": step_in_round,
        "round_no": loop_state.round_no,
    });
    if let Some(object) = observation.as_object_mut() {
        object.insert("output_format".to_string(), json!("machine_json"));
        object.insert(
            "action".to_string(),
            json!(context_evidence_action(&parent_context_evidence)),
        );
        object.insert(
            "path".to_string(),
            json!(parent_primary_content_path.as_str()),
        );
        object.insert("paths".to_string(), json!(parent_content_paths));
        object.insert(
            "excerpt".to_string(),
            json!(parent_content_excerpt.as_str()),
        );
        object.insert(
            "content_excerpt".to_string(),
            json!(parent_content_excerpt.as_str()),
        );
        object.insert("context_evidence".to_string(), parent_context_evidence);
    }
    loop_state.task_observations.push(observation);

    (required_failed_count > 0 && !expected_failure_delivery)
        .then_some(SUBAGENT_STOP_SIGNAL_REQUIRED_CHILD_FAILED)
}

#[derive(Debug, Clone, Default)]
struct SubagentInvocationPolicy {
    dry_run: bool,
    expected_failure: bool,
    parent_task_id: Option<String>,
}

impl SubagentInvocationPolicy {
    fn from_args(args: &Value) -> Self {
        Self {
            dry_run: args.get("dry_run").and_then(Value::as_bool) == Some(true),
            expected_failure: args.get("expected_failure").and_then(Value::as_bool) == Some(true),
            parent_task_id: args
                .get("parent_task_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .map(machine_ref_or_empty)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        }
    }

    fn expected_failure_delivery(&self, required_failed_count: usize) -> bool {
        self.dry_run && self.expected_failure && required_failed_count > 0
    }
}

struct SubagentChildAction {
    role: String,
    objective: String,
    context_refs: Vec<String>,
    options: SubagentActionOptions,
    findings: Option<Value>,
    required: bool,
}

#[derive(Default)]
struct AggregatedFindingSignals {
    finding_count: usize,
    confidence_count: usize,
    confidence_min: Option<f64>,
    confidence_max: Option<f64>,
    statuses_by_conflict_key: BTreeMap<String, BTreeSet<String>>,
    child_refs_by_conflict_key: BTreeMap<String, BTreeSet<String>>,
}

impl AggregatedFindingSignals {
    fn observe_child_findings(&mut self, child_run_id: &str, findings: &[Value]) {
        for finding in findings {
            let Some(finding) = finding.as_object() else {
                continue;
            };
            self.finding_count += 1;
            if let Some(confidence) = finding
                .get("confidence")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
            {
                self.confidence_count += 1;
                self.confidence_min = Some(
                    self.confidence_min
                        .map(|current| current.min(confidence))
                        .unwrap_or(confidence),
                );
                self.confidence_max = Some(
                    self.confidence_max
                        .map(|current| current.max(confidence))
                        .unwrap_or(confidence),
                );
            }
            let Some(conflict_key) = finding_conflict_key(finding) else {
                continue;
            };
            if let Some(status) = finding.get("status").and_then(Value::as_str) {
                let status = machine_ref_or_empty(status.trim());
                if !status.is_empty() {
                    self.statuses_by_conflict_key
                        .entry(conflict_key.clone())
                        .or_default()
                        .insert(status.to_string());
                }
            }
            self.child_refs_by_conflict_key
                .entry(conflict_key)
                .or_default()
                .insert(machine_ref_or_empty(child_run_id).to_string());
        }
    }

    fn conflict_summary(&self) -> Value {
        let mut conflict_groups = Vec::new();
        for (conflict_key, statuses) in &self.statuses_by_conflict_key {
            if statuses.len() < 2 {
                continue;
            }
            let child_run_ids = self
                .child_refs_by_conflict_key
                .get(conflict_key)
                .into_iter()
                .flat_map(|items| items.iter())
                .map(|item| json!(item))
                .collect::<Vec<_>>();
            conflict_groups.push(json!({
                "group_ref": conflict_key,
                "status_count": statuses.len(),
                "statuses": statuses.iter().map(|item| json!(item)).collect::<Vec<_>>(),
                "child_run_ids": child_run_ids,
            }));
        }
        json!({
            "schema_version": 1,
            "conflict_count": conflict_groups.len(),
            "conflict_groups": conflict_groups,
        })
    }

    fn confidence_summary(&self) -> Value {
        json!({
            "schema_version": 1,
            "reported_count": self.confidence_count,
            "missing_count": self.finding_count.saturating_sub(self.confidence_count),
            "min": self.confidence_min,
            "max": self.confidence_max,
        })
    }
}

fn finding_conflict_key(finding: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(group) = finding
        .get("conflict_group")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(machine_ref_or_empty)
        .filter(|value| !value.is_empty())
    {
        return Some(group.to_string());
    }
    let kind = finding
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(machine_ref_or_empty)
        .filter(|value| !value.is_empty());
    let code = finding
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| finding.get("error_code").and_then(Value::as_str))
        .map(str::trim)
        .map(machine_ref_or_empty)
        .filter(|value| !value.is_empty());
    match (kind, code) {
        (Some(kind), Some(code)) => Some(format!("{kind}:{code}")),
        (Some(kind), None) => Some(kind.to_string()),
        (None, Some(code)) => Some(code.to_string()),
        (None, None) => None,
    }
}

fn aggregation_main_thread_decision(
    required_failed_count: usize,
    conflict_count: u64,
    expected_failure_delivery: bool,
) -> Value {
    let decision_status = if expected_failure_delivery {
        "expected_failure_delivered"
    } else if required_failed_count > 0 {
        "blocked_required_child_failure"
    } else if conflict_count > 0 {
        "needs_conflict_resolution"
    } else {
        "ready_to_synthesize"
    };
    json!({
        "schema_version": 1,
        "decision_owner": "parent_agent_loop",
        "decision_required": required_failed_count > 0 || conflict_count > 0,
        "decision_status": decision_status,
        "recommended_next_action": if expected_failure_delivery {
            "deliver_expected_failure_evidence"
        } else if required_failed_count > 0 {
            "repair_required_child_failure"
        } else if conflict_count > 0 {
            "resolve_child_conflicts"
        } else {
            "synthesize_from_child_findings"
        },
        "input_refs": [
            "child_results",
            "aggregation.conflict_summary",
            "aggregation.confidence_summary"
        ],
    })
}

fn agent_team_spec(
    team_id: &str,
    parent_task_id: Option<&str>,
    max_parallel: u64,
    children: &[Value],
) -> Value {
    json!({
        "schema_version": 1,
        "spec_kind": "agent_team_spec",
        "team_id": team_id,
        "parent_task_id": parent_task_id.unwrap_or_default(),
        "child_task_ids": children
            .iter()
            .filter_map(|child| child.get("child_task_id").and_then(Value::as_str))
            .collect::<Vec<_>>(),
        "max_parallel": max_parallel,
        "write_permission": "read_only",
        "conflict_policy": "parent_loop_resolution_required",
        "children": children,
    })
}

fn agent_team_child_spec(
    child_run_id: &str,
    role: &str,
    scope: &str,
    required: bool,
    timeout_ms: Option<u64>,
    status: &str,
) -> Value {
    json!({
        "schema_version": 1,
        "child_task_id": child_run_id,
        "child_run_id": child_run_id,
        "role": role,
        "scope": scope,
        "required": required,
        "timeout_ms": timeout_ms,
        "write_permission": "read_only",
        "status": status,
    })
}

fn team_lifecycle_event(
    event_type: &'static str,
    team_id: &str,
    child_run_id: Option<&str>,
    status: &str,
    role: Option<&str>,
    required: Option<bool>,
    reason_code: Option<&str>,
) -> Value {
    json!({
        "schema_version": 1,
        "event_type": event_type,
        "team_id": team_id,
        "child_run_id": child_run_id,
        "status": status,
        "role": role,
        "required": required,
        "reason_code": reason_code,
        "write_permission": "read_only",
    })
}

fn subagent_child_actions_from_args(args: &Value) -> Option<Vec<SubagentChildAction>> {
    let children = args
        .get("children")
        .or_else(|| args.get("child_requests"))
        .or_else(|| args.get("subagents"))
        .and_then(Value::as_array)?;
    Some(
        children
            .iter()
            .filter_map(|child| {
                if !child.is_object() {
                    return None;
                }
                let (role, objective, context_refs, options) =
                    subagent_action_parts_from_args(child);
                Some(SubagentChildAction {
                    role,
                    objective,
                    context_refs,
                    options,
                    findings: child.get("findings").cloned(),
                    required: child
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect(),
    )
}

fn rejected_child_result(
    child_run_id: &str,
    role_token: &str,
    required: bool,
    error_code: &str,
) -> Value {
    json!({
        "schema_version": 1,
        "child_run_id": child_run_id,
        "status": "rejected",
        "result_status": "rejected",
        "outcome_code": error_code,
        "error_code": error_code,
        "role": machine_ref_or_empty(role_token),
        "required": required,
        "write_enabled": false,
        "external_publish_enabled": false,
        "failure_isolated": !required,
    })
}

fn child_summary_from_result(child_result: &Value) -> Value {
    json!({
        "child_run_id": child_result
            .get("child_run_id")
            .and_then(Value::as_str)
            .map(machine_ref_or_empty)
            .unwrap_or_default(),
        "status": child_result.get("status").and_then(Value::as_str),
        "result_status": child_result.get("result_status").and_then(Value::as_str),
        "outcome_code": child_result.get("outcome_code").and_then(Value::as_str),
        "role": child_result.get("role").and_then(Value::as_str),
        "required": child_result.get("required").and_then(Value::as_bool),
        "finding_count": child_result.get("finding_count").and_then(Value::as_u64),
        "write_enabled": false,
        "external_publish_enabled": false,
        "failure_isolated": child_result
            .get("failure_isolated")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

fn sanitized_findings(findings: Option<&Value>) -> Vec<Value> {
    let Some(items) = findings.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .take(MAX_SUBAGENT_FINDINGS)
        .filter_map(sanitized_finding)
        .collect()
}

fn sanitized_finding(finding: &Value) -> Option<Value> {
    let map = finding.as_object()?;
    let keys = map
        .keys()
        .filter(|key| key.as_str() != "text" && key.as_str() != "error_text")
        .take(MAX_SUBAGENT_FINDING_KEYS)
        .map(|key| {
            json!({
                "key": machine_ref_or_empty(key),
            })
        })
        .collect::<Vec<_>>();
    Some(json!({
        "schema_version": 1,
        "kind": machine_token_field(map.get("kind")),
        "status": machine_token_field(map.get("status")),
        "code": machine_token_field(map.get("code").or_else(|| map.get("error_code"))),
        "message_key": machine_token_field(map.get("message_key")),
        "conflict_group": machine_token_field(map.get("conflict_group")),
        "confidence": normalized_confidence(map.get("confidence")),
        "evidence_refs": machine_token_array_field(map.get("evidence_refs")),
        "key_count": map.len(),
        "keys": keys,
    }))
}

fn machine_token_field(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(machine_ref_or_empty)
        .filter(|value| !value.is_empty())
}

fn machine_token_array_field(value: Option<&Value>) -> Vec<&str> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .map(machine_ref_or_empty)
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn normalized_confidence(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0.0, 1.0))
}
