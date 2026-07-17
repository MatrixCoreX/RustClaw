use serde_json::{json, Value};

use super::*;

pub(super) fn rollout_attribution_json(attribution: &TaskJournalRolloutAttribution) -> Value {
    json!({
        "switch_name": attribution.switch_name.as_str(),
        "event": attribution.event.as_str(),
        "outcome": attribution.outcome.as_str(),
        "reason_code": attribution.reason_code.as_deref(),
        "owner_layer": attribution.owner_layer.as_deref(),
        "decision": attribution.decision.as_deref(),
        "skill": attribution.skill.as_deref(),
        "action": attribution.action.as_deref(),
        "capability_ref": attribution.capability_ref.as_deref(),
        "dedup_scope": attribution.dedup_scope.as_deref(),
        "fingerprint": attribution.fingerprint.as_deref(),
        "repeat_count": attribution.repeat_count,
        "limit": attribution.limit,
        "failure_attribution": attribution.failure_attribution.as_deref(),
        "missing_slots": &attribution.missing_slots,
        "required_evidence": &attribution.required_evidence,
        "missing_evidence_fields": &attribution.missing_evidence_fields,
        "confidence": attribution.confidence,
        "risk_level": attribution.risk_level.as_deref(),
        "output_contract_ref": attribution.output_contract_ref.as_deref(),
        "old_first_layer_decision": attribution.old_first_layer_decision.as_deref(),
        "agent_decision": attribution.agent_decision.as_deref(),
        "decision_delta": attribution.decision_delta.as_deref(),
        "route_layer_that_disagreed": attribution.route_layer_that_disagreed.as_deref(),
        "old_required_evidence": &attribution.old_required_evidence,
        "agent_required_evidence": &attribution.agent_required_evidence,
        "capability_delta": attribution.capability_delta.as_deref(),
        "risk_delta": attribution.risk_delta.as_deref(),
        "output_contract_delta": attribution.output_contract_delta.as_deref(),
        "final_outcome": attribution.final_outcome.as_deref(),
        "verifier_pass": attribution.verifier_pass,
        "llm_call_count": attribution.llm_call_count,
        "tool_call_count": attribution.tool_call_count,
        "external_tool_call_count": attribution.external_tool_call_count,
        "latency_ms": attribution.latency_ms,
        "budget_profile": attribution.budget_profile.as_deref(),
        "boundary_context": attribution.boundary_context.as_ref(),
        "decision_envelope": attribution.decision_envelope.as_ref(),
    })
}

pub(super) fn verify_summary_json(verify: &TaskJournalVerifySummary) -> Value {
    let first_issue = verify.issues.first();
    json!({
        "approved": verify.approved,
        "mode": verify.mode.as_str(),
        "owner_layer": "plan_verifier",
        "blocked_reason": verify.blocked_reason.as_deref().map(crate::truncate_for_log),
        "blocked_reason_code": first_issue.map(|issue| issue.kind.reason_code()),
        "blocked_status_code": first_issue.map(|issue| issue.kind.status_code()),
        "blocked_message_key": first_issue.map(|issue| issue.kind.message_key()),
        "shadow_blocked_reason": verify.shadow_blocked_reason.as_deref().map(crate::truncate_for_log),
        "permission_decision": &verify.permission_decision,
        "needs_confirmation": verify.needs_confirmation,
        "issue_count": verify.issues.len(),
    })
}

pub(super) fn verify_trace_json(
    verify: &TaskJournalVerifySummary,
    plan: Option<&crate::PlanResult>,
) -> Value {
    let first_issue = verify.issues.first();
    json!({
        "approved": verify.approved,
        "mode": verify.mode.as_str(),
        "owner_layer": "plan_verifier",
        "blocked_reason": verify.blocked_reason.as_deref().map(crate::truncate_for_log),
        "blocked_reason_code": first_issue.map(|issue| issue.kind.reason_code()),
        "blocked_status_code": first_issue.map(|issue| issue.kind.status_code()),
        "blocked_message_key": first_issue.map(|issue| issue.kind.message_key()),
        "shadow_blocked_reason": verify.shadow_blocked_reason.as_deref().map(crate::truncate_for_log),
        "permission_decision": &verify.permission_decision,
        "needs_confirmation": verify.needs_confirmation,
        "issues": verify.issues.iter().map(|issue| {
            verifier_issue_repair_signal_json(issue, plan)
        }).collect::<Vec<_>>(),
    })
}

pub(super) fn finalizer_summary_json(
    summary: &TaskJournalFinalizerSummary,
    output_contract: Option<&crate::IntentOutputContract>,
    journal: &TaskJournal,
) -> Value {
    let evidence_coverage =
        output_contract.map(|contract| evidence_coverage_for_output_contract(contract, journal));
    let final_answer_shape =
        output_contract.and_then(crate::evidence_policy::final_answer_shape_for_output_contract);
    json!({
        "stage": summary.stage.map(TaskJournalFinalizerStage::as_str),
        "disposition": summary.disposition.map(crate::finalize::FinalizerDisposition::as_str),
        "fallback": summary.fallback.map(TaskJournalFinalizerFallback::as_str),
        "final_answer_shape": final_answer_shape.map(crate::evidence_policy::FinalAnswerShape::as_str),
        "final_answer_shape_class": final_answer_shape.map(|shape| shape.class().as_str()),
        "coarse_response_shape": final_answer_shape
            .map(|shape| shape.coarse_response_shape().as_str()),
        "allows_model_language": final_answer_shape.map(crate::evidence_policy::FinalAnswerShape::allows_model_language),
        "evidence_coverage": evidence_coverage.as_ref().map(TaskJournalEvidenceCoverage::to_trace_json),
        "evidence_coverage_complete": evidence_coverage.as_ref().map(TaskJournalEvidenceCoverage::is_complete),
        "missing_evidence": evidence_coverage
            .as_ref()
            .map(|coverage| coverage.missing_evidence.clone())
            .unwrap_or_default(),
        "parsed": summary.parsed,
        "contract_ok": summary.contract_ok,
        "completion_ok": summary.completion_ok,
        "grounded_ok": summary.grounded_ok,
        "format_ok": summary.format_ok,
        "needs_clarify": summary.needs_clarify,
        "confidence": summary.confidence,
        "used_evidence_ids_count": summary.used_evidence_ids_count,
        "evidence_quotes_count": summary.evidence_quotes_count,
    })
}

pub(super) fn answer_verifier_summary_json(summary: &TaskJournalAnswerVerifierSummary) -> Value {
    json!({
        "pass": summary.pass,
        "missing_evidence_fields": summary.missing_evidence_fields,
        "answer_incomplete_reason": crate::truncate_for_log(&summary.answer_incomplete_reason),
        "should_retry": summary.should_retry,
        "retry_instruction": crate::truncate_for_log(&summary.retry_instruction),
        "confidence": summary.confidence,
        "repair_signal": answer_verifier_repair_signal_json(summary),
    })
}

fn answer_verifier_repair_signal_json(summary: &TaskJournalAnswerVerifierSummary) -> Option<Value> {
    summary.high_confidence_retry_gap().then(|| {
        crate::repair_signal::RepairSignal::from_answer_verifier_parts(
            &summary.missing_evidence_fields,
            summary.should_retry,
            summary.confidence,
        )
        .to_json()
    })
}

fn plan_step_action_ref(step: &crate::PlanStep) -> Option<String> {
    let action = crate::evidence_policy::ActionRef::from_skill_args(&step.skill, &step.args)?;
    Some(action.as_key())
}

fn plan_step_raw_action_ref(step: &crate::PlanStep) -> Option<String> {
    crate::evidence_policy::ActionRef::from_skill_args(&step.skill, &step.args)
        .map(|action| action.as_key())
}

fn plan_step_fallback_action_ref(step: &crate::PlanStep) -> Option<String> {
    plan_step_raw_action_ref(step).or_else(|| {
        [step.skill.trim(), step.action_type.trim()]
            .into_iter()
            .find(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn canonical_json_for_fingerprint(value: &Value) -> String {
    canonical_value_for_fingerprint(value).to_string()
}

fn plan_step_args_fingerprint(step: &crate::PlanStep) -> Option<String> {
    let action_ref = plan_step_action_ref(step).or_else(|| plan_step_fallback_action_ref(step))?;
    Some(crate::evidence_policy::fnv1a_hex(&format!(
        "{}\n{}",
        action_ref.trim(),
        canonical_json_for_fingerprint(&step.args)
    )))
}

fn canonical_value_for_fingerprint(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(canonical_value_for_fingerprint)
                .collect::<Vec<_>>(),
        ),
        Value::Object(map) => {
            let mut sorted = serde_json::Map::new();
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(item) = map.get(key) {
                    sorted.insert(key.clone(), canonical_value_for_fingerprint(item));
                }
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

fn verifier_issue_forbidden_repeat_fingerprint(
    issue: &TaskJournalVerifyIssue,
    plan: Option<&crate::PlanResult>,
) -> Option<String> {
    let step = plan?
        .steps
        .iter()
        .find(|step| step.step_id == issue.step_id)?;
    let action_ref = plan_step_action_ref(step).or_else(|| plan_step_fallback_action_ref(step))?;
    let args_fingerprint = plan_step_args_fingerprint(step)?;
    Some(format!("{}:{}", action_ref.trim(), args_fingerprint))
}

fn verifier_issue_repair_signal_json(
    issue: &TaskJournalVerifyIssue,
    plan: Option<&crate::PlanResult>,
) -> Value {
    let rejected_action = plan
        .and_then(|plan| plan.steps.iter().find(|step| step.step_id == issue.step_id))
        .and_then(|step| {
            plan_step_action_ref(step).or_else(|| plan_step_fallback_action_ref(step))
        });
    crate::repair_signal::RepairSignal::from_verifier_issue_parts(
        &issue.step_id,
        issue.kind,
        &issue.detail,
    )
    .with_missing_fields(&issue.missing_fields)
    .with_rejected_action(rejected_action)
    .with_forbidden_repeat_fingerprint(verifier_issue_forbidden_repeat_fingerprint(issue, plan))
    .to_json()
}

pub(super) fn plan_summary_json(plan: &crate::PlanResult) -> Value {
    json!({
        "goal": crate::truncate_for_log(&plan.goal),
        "plan_kind": plan.plan_kind.as_str(),
        "output_contract": plan.output_contract.as_ref().map(output_contract_json),
        "step_count": plan.steps.len(),
        "missing_slots": plan.missing_slots,
        "needs_confirmation": plan.needs_confirmation,
    })
}

pub(super) fn plan_trace_json(plan: &crate::PlanResult) -> Value {
    json!({
        "goal": crate::truncate_for_log(&plan.goal),
        "plan_kind": plan.plan_kind.as_str(),
        "planner_notes": crate::truncate_for_log(&plan.planner_notes),
        "raw_plan_text": crate::truncate_for_log(&plan.raw_plan_text),
        "output_contract": plan.output_contract.as_ref().map(output_contract_json),
        "step_count": plan.steps.len(),
        "steps": plan.steps.iter().map(|step| {
            let raw_action_ref = plan_step_raw_action_ref(step);
            let matrix_action_ref = plan_step_action_ref(step);
            let args_fingerprint = plan_step_args_fingerprint(step);
            json!({
                "step_id": &step.step_id,
                "action_type": &step.action_type,
                "skill": &step.skill,
                "action_ref": matrix_action_ref.clone(),
                "matrix_action_ref": matrix_action_ref,
                "raw_action_ref": raw_action_ref,
                "args_fingerprint": args_fingerprint,
                "depends_on": &step.depends_on,
                "why": crate::truncate_for_log(&step.why),
            })
        }).collect::<Vec<_>>(),
    })
}

pub(super) fn output_contract_json(contract: &crate::IntentOutputContract) -> Value {
    json!({
        "response_shape": contract.response_shape.as_str(),
        "exact_sentence_count": contract.exact_sentence_count,
        "requires_content_evidence": contract.requires_content_evidence,
        "delivery_required": contract.delivery_required,
        "locator_kind": contract.locator_kind.as_str(),
        "delivery_intent": contract.delivery_intent.as_str(),
        "result_kind": contract.semantic_kind.as_str(),
    })
}

pub(super) fn turn_analysis_json(analysis: &crate::turn_context::TurnAnalysis) -> Value {
    json!({
        "turn_type": analysis.turn_type.map(crate::turn_context::TurnType::as_str),
        "target_task_policy": analysis
            .target_task_policy
            .map(crate::turn_context::TargetTaskPolicy::as_str),
        "should_interrupt_active_run": analysis.should_interrupt_active_run,
        "has_state_patch": analysis.state_patch.is_some(),
        "attachment_processing_required": analysis.attachment_processing_required,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RequestedPlanCapability {
    pub(super) action_type: String,
    pub(super) capability: String,
    pub(super) action_ref: Option<String>,
    pub(super) args_fingerprint: Option<String>,
}

pub(super) fn raw_plan_steps(raw_plan_text: &str) -> Vec<Value> {
    let Some(value) =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw_plan_text)
    else {
        return Vec::new();
    };
    if let Some(steps) = value.get("steps").and_then(Value::as_array) {
        return steps.clone();
    }
    value.as_array().cloned().unwrap_or_default()
}

fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn requested_capability_from_raw_step(step: &Value) -> Option<RequestedPlanCapability> {
    let mut action_type = string_field(step, &["type", "action_type", "action"])?;
    let capability = match action_type {
        "call_tool" => string_field(step, &["tool", "skill", "name"]),
        "call_skill" => string_field(step, &["skill", "tool", "name"]),
        "call_capability" => string_field(step, &["capability", "name"]),
        "respond" | "synthesize_answer" | "think" => Some(action_type),
        _ => {
            if let Some(tool) = string_field(step, &["tool"]) {
                action_type = "call_tool";
                Some(tool)
            } else if let Some(skill) = string_field(step, &["skill"]) {
                action_type = "call_skill";
                Some(skill)
            } else if let Some(capability) = string_field(step, &["capability"]) {
                action_type = "call_capability";
                Some(capability)
            } else {
                Some(action_type)
            }
        }
    }?;
    Some(RequestedPlanCapability {
        action_type: action_type.to_string(),
        capability: capability.to_string(),
        action_ref: None,
        args_fingerprint: None,
    })
}

fn requested_capabilities_for_plan(plan: &crate::PlanResult) -> Vec<RequestedPlanCapability> {
    let raw_steps = raw_plan_steps(&plan.raw_plan_text);
    plan.steps
        .iter()
        .enumerate()
        .map(|(idx, normalized_step)| {
            let mut requested = raw_steps
                .get(idx)
                .and_then(requested_capability_from_raw_step)
                .unwrap_or_else(|| RequestedPlanCapability {
                    action_type: normalized_step.action_type.clone(),
                    capability: normalized_step.skill.clone(),
                    action_ref: None,
                    args_fingerprint: None,
                });
            if requested.action_ref.is_none() {
                requested.action_ref = plan_step_action_ref(normalized_step);
            }
            requested.args_fingerprint = plan_step_args_fingerprint(normalized_step);
            requested
        })
        .collect()
}

pub(super) fn requested_capability_sequence(journal: &TaskJournal) -> Vec<RequestedPlanCapability> {
    let mut requested = Vec::new();
    for round in &journal.rounds {
        if let Some(plan) = round.plan_result.as_ref() {
            requested.extend(requested_capabilities_for_plan(plan));
        }
    }
    if requested.is_empty() {
        if let Some(plan) = journal.plan_result.as_ref() {
            requested.extend(requested_capabilities_for_plan(plan));
        }
    }
    requested
}

pub(super) fn boundary_context_summary_json(journal: &TaskJournal) -> Option<Value> {
    journal
        .rollout_attribution
        .iter()
        .find_map(|item| item.boundary_context.clone())
        .or_else(|| {
            journal
                .context_bundle_summary
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|summary| {
                    json!({
                        "schema_version": 1,
                        "source": "context_bundle_summary",
                        "summary": crate::truncate_for_log(summary),
                    })
                })
        })
}

pub(super) fn budget_profile_json(journal: &TaskJournal) -> Option<&str> {
    journal
        .rollout_attribution
        .iter()
        .find_map(|item| item.budget_profile.as_deref())
}

pub(super) fn round_capability_resolution_records_json(
    round: &TaskJournalRoundTrace,
) -> Vec<Value> {
    let Some(plan) = round.plan_result.as_ref() else {
        return Vec::new();
    };
    let requested = requested_capabilities_for_plan(plan);
    plan.steps
        .iter()
        .zip(requested)
        .map(|(step, requested)| {
            let action_type = requested.action_type.clone();
            json!({
                "step_id": &step.step_id,
                "requested_action_type": action_type,
                "requested_capability": requested.capability,
                "requested_action_ref": requested.action_ref,
                "args_fingerprint": requested.args_fingerprint,
                "resolution_source": capability_resolution_source(&action_type),
            })
        })
        .collect()
}

pub(super) fn capability_resolution_source(action_type: &str) -> &'static str {
    match action_type {
        "call_capability" => "capability_resolver",
        "call_tool" | "call_skill" => "direct_tool_or_skill_trace",
        "respond" | "synthesize_answer" | "think" => "planner_terminal_action",
        _ => "planner_action",
    }
}

pub(super) fn verify_repair_signals_json(
    verify: Option<&TaskJournalVerifySummary>,
    plan: Option<&crate::PlanResult>,
) -> Vec<Value> {
    verify
        .into_iter()
        .flat_map(|summary| summary.issues.iter())
        .map(|issue| verifier_issue_repair_signal_json(issue, plan))
        .collect()
}

pub(super) fn next_requested_capability(
    requested: &mut Vec<RequestedPlanCapability>,
    step: &TaskJournalStepTrace,
) -> Option<RequestedPlanCapability> {
    if requested.is_empty() {
        return None;
    }
    let requested_idx = requested
        .iter()
        .position(|candidate| requested_capability_matches_step(candidate, step))
        .unwrap_or(0);
    Some(requested.remove(requested_idx))
}

fn requested_capability_matches_step(
    requested: &RequestedPlanCapability,
    step: &TaskJournalStepTrace,
) -> bool {
    let step_skill = step.skill.trim();
    if step_skill.is_empty() {
        return false;
    }
    if requested.capability.eq_ignore_ascii_case(step_skill) {
        return true;
    }
    if requested
        .action_ref
        .as_deref()
        .and_then(|action_ref| action_ref.split_once('.'))
        .is_some_and(|(skill, _)| skill.eq_ignore_ascii_case(step_skill))
    {
        return true;
    }
    matches!(
        (requested.action_type.as_str(), step_skill),
        ("respond", "respond") | ("synthesize_answer", "synthesize_answer") | ("think", "think")
    )
}

pub(super) fn step_action_kind(
    step: &TaskJournalStepTrace,
    requested: Option<&RequestedPlanCapability>,
) -> String {
    if let Some(requested) = requested {
        return requested.action_type.clone();
    }
    match step.skill.as_str() {
        "respond" | "synthesize_answer" | "think" | "answer_verifier" => step.skill.clone(),
        _ => "call_skill".to_string(),
    }
}

fn output_evidence_ids(step: &TaskJournalStepTrace, evidence: Option<&Value>) -> Vec<String> {
    evidence
        .and_then(|value| value.get("items"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(idx, _)| format!("{}:evidence:{}", step.step_id, idx + 1))
                .collect()
        })
        .unwrap_or_default()
}

fn artifact_refs_from_step_output(output: Option<&str>) -> Vec<Value> {
    let Some(value) = output.and_then(|text| serde_json::from_str::<Value>(text.trim()).ok())
    else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    collect_artifact_refs(&value, &mut refs, 0, false);
    refs
}

fn structured_workspace_mutation_from_step_output(output: Option<&str>) -> Option<Value> {
    let value = output.and_then(|text| serde_json::from_str::<Value>(text.trim()).ok())?;
    let source = value
        .get("extra")
        .filter(|extra| extra.is_object())
        .unwrap_or(&value);
    if !matches!(
        source.get("source").and_then(Value::as_str),
        Some("workspace_patch" | "workspace_mutation")
    ) {
        return None;
    }
    let action = source.get("action").and_then(Value::as_str)?;
    if action == "diff" {
        return None;
    }
    let mut mutation = serde_json::Map::new();
    for field in [
        "schema_version",
        "source",
        "status",
        "action",
        "patch_id",
        "mutation_id",
        "checkpoint_id",
        "compensates_patch_id",
        "compensates_mutation_id",
        "compensates_checkpoint_id",
        "state",
        "target_path",
        "isolation_root",
        "reversible",
        "additions",
        "deletions",
        "hunk_count",
        "changed_hunks",
        "changed_files",
        "restored_files",
        "files",
        "before",
        "after",
        "artifact_refs",
    ] {
        if let Some(value) = source.get(field) {
            mutation.insert(field.to_string(), value.clone());
        }
    }
    Some(Value::Object(mutation))
}

fn collect_artifact_refs(
    value: &Value,
    refs: &mut Vec<Value>,
    depth: usize,
    allow_string_leaf: bool,
) {
    if refs.len() >= 8 || depth > 3 {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(items) = map.get("artifact_refs").and_then(Value::as_array) {
                for item in items {
                    push_explicit_artifact_ref(refs, item);
                    if refs.len() >= 8 {
                        return;
                    }
                }
            }
            for key in [
                "path",
                "resolved_path",
                "file_path",
                "output_path",
                "artifact_path",
                "archive_path",
            ] {
                if let Some(path) = map.get(key).and_then(Value::as_str) {
                    push_artifact_ref(refs, key, path);
                }
            }
            for key in [
                "paths",
                "file_paths",
                "artifact_paths",
                "files",
                "artifacts",
            ] {
                if let Some(items) = map.get(key).and_then(Value::as_array) {
                    for item in items {
                        collect_artifact_refs(item, refs, depth + 1, true);
                        if refs.len() >= 8 {
                            return;
                        }
                    }
                }
            }
            for key in ["extra", "result", "data", "metadata", "payload"] {
                if let Some(item) = map.get(key) {
                    collect_artifact_refs(item, refs, depth + 1, false);
                    if refs.len() >= 8 {
                        return;
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_artifact_refs(item, refs, depth + 1, allow_string_leaf);
                if refs.len() >= 8 {
                    return;
                }
            }
        }
        Value::String(path) if allow_string_leaf && artifact_string_looks_like_path(path) => {
            push_artifact_ref(refs, "value", path)
        }
        Value::String(_) => {}
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn push_explicit_artifact_ref(refs: &mut Vec<Value>, item: &Value) {
    let Some(reference) = item
        .get("ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    if refs.iter().any(|item| {
        item.get("ref")
            .and_then(Value::as_str)
            .is_some_and(|existing| existing == reference)
    }) {
        return;
    }
    refs.push(json!({
        "kind": item.get("kind").and_then(Value::as_str).unwrap_or("artifact"),
        "ref": crate::truncate_for_log(reference),
    }));
}

fn artifact_string_looks_like_path(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if value.contains(['\n', '\r']) {
        return false;
    }
    if value == "." || value == ".." {
        return true;
    }
    if value.starts_with('/') || value.starts_with("./") || value.starts_with("../") {
        return true;
    }
    if value.starts_with("~/") || value.contains('/') || value.contains('\\') {
        return true;
    }
    let file_name = value.rsplit(['/', '\\']).next().unwrap_or(value);
    file_name.starts_with('.') && file_name.len() > 1
        || file_name.contains('.')
            && !file_name.ends_with('.')
            && file_name.chars().all(|ch| !ch.is_whitespace())
}

fn push_artifact_ref(refs: &mut Vec<Value>, field: &str, path: &str) {
    let path = path.trim();
    if path.is_empty() || refs.len() >= 8 {
        return;
    }
    if refs.iter().any(|item| {
        item.get("ref")
            .and_then(Value::as_str)
            .is_some_and(|existing| existing == path)
    }) {
        return;
    }
    refs.push(json!({
        "kind": "path",
        "field": field,
        "ref": crate::truncate_for_log(path),
    }));
}

pub(super) fn step_trace_json(
    step: &TaskJournalStepTrace,
    requested: Option<&RequestedPlanCapability>,
    output_contract: Option<&crate::IntentOutputContract>,
) -> Value {
    let structured_error = step
        .error_excerpt
        .as_deref()
        .and_then(crate::skills::parse_structured_skill_error);
    let failure_attribution = structured_error_failure_attribution(structured_error.as_ref());
    let contract_policy = contract_policy_trace_json(structured_error.as_ref());
    let action_kind = step_action_kind(step, requested);
    let observed_evidence = observed_evidence_for_step_trace(step);
    let output_evidence_ids = output_evidence_ids(step, observed_evidence.as_ref());
    let output_evidence_count = output_evidence_ids.len();
    let artifact_refs = artifact_refs_from_step_output(step.output_excerpt.as_deref());
    let artifact_ref_count = artifact_refs.len();
    let structured_workspace_mutation =
        structured_workspace_mutation_from_step_output(step.output_excerpt.as_deref());
    let mutation_reversibility = (step.skill == "run_cmd").then(|| {
        json!({
            "source": "shell_command",
            "reversible": false,
            "status": "not_rewindable",
            "reason_code": "shell_side_effects_not_tracked",
        })
    });
    json!({
        "step_id": &step.step_id,
        "action_kind": action_kind,
        "skill": &step.skill,
        "requested_action_type": requested.map(|value| value.action_type.as_str()),
        "requested_capability": requested.map(|value| value.capability.as_str()),
        "requested_action_ref": requested.and_then(|value| value.action_ref.as_deref()),
        "executed_skill": &step.skill,
        "resolved_tool_or_skill": &step.skill,
        "resolved_capability": requested
            .filter(|value| value.action_type == "call_capability")
            .map(|value| value.capability.as_str()),
        "resolution_source": requested
            .map(|value| capability_resolution_source(&value.action_type))
            .unwrap_or("step_trace_fallback"),
        "status": step.status.as_str(),
        "error_kind": structured_error.as_ref().map(|value| value.error_kind.as_str()),
        "failure_attribution": failure_attribution.as_deref(),
        "contract_policy": contract_policy,
        "contract": step_contract_trace_json(output_contract, requested),
        "sanitized_args_summary": requested.and_then(|value| value.action_ref.as_deref()),
        "sanitized_args_summary_status": requested
            .and_then(|value| value.action_ref.as_deref())
            .map(|_| "action_ref_only")
            .unwrap_or("not_recorded_in_step_trace"),
        "args_fingerprint": requested.and_then(|value| value.args_fingerprint.as_deref()),
        "args_fingerprint_status": requested
            .and_then(|value| value.args_fingerprint.as_deref())
            .map(|_| "recorded_hash_only")
            .unwrap_or("not_recorded"),
        "output_excerpt": step.output_excerpt.as_deref(),
        "observed_evidence": observed_evidence,
        "output_evidence_ids": output_evidence_ids,
        "output_evidence_count": output_evidence_count,
        "artifact_refs": artifact_refs,
        "artifact_ref_count": artifact_ref_count,
        "structured_workspace_mutation": structured_workspace_mutation,
        "mutation_reversibility": mutation_reversibility,
        "retry_fingerprint": null,
        "retry_fingerprint_status": "not_recorded_in_step_trace",
        "error_excerpt": step.error_excerpt.as_deref(),
        "started_at": step.started_at,
        "finished_at": step.finished_at,
    })
}

fn step_contract_trace_json(
    output_contract: Option<&crate::IntentOutputContract>,
    requested: Option<&RequestedPlanCapability>,
) -> Option<Value> {
    let output_contract = output_contract?;
    let contract = crate::evidence_policy::trace_snapshot_for_output_contract(output_contract)?;
    let requested_action_ref = requested.and_then(|value| value.action_ref.as_deref());
    let action_policy = requested_action_ref.and_then(|action_ref| {
        crate::evidence_policy::action_trace_for_output_contract(output_contract, action_ref)
    });
    Some(json!({
        "contract_match": contract.get("contract_match").and_then(Value::as_str),
        "policy_mode": contract.get("policy_mode").and_then(Value::as_str),
        "required_evidence": contract.get("required_evidence").cloned(),
        "evidence_expression": contract.get("evidence_expression").cloned(),
        "final_answer_shape": contract.get("final_answer_shape").and_then(Value::as_str),
        "final_answer_shape_class": contract.get("final_answer_shape_class").and_then(Value::as_str),
        "coarse_response_shape": contract.get("coarse_response_shape").and_then(Value::as_str),
        "allows_model_language": contract.get("allows_model_language").and_then(Value::as_bool),
        "requested_action_ref": requested_action_ref,
        "action_policy": action_policy,
    }))
}

/// Serializes one ask state-machine transition into machine JSON.
pub(super) fn ask_transition_json(t: &crate::AskTransition) -> Value {
    json!({
        "from": t.from.map(crate::AskState::as_str),
        "to": t.to.as_str(),
        "reason": crate::truncate_for_log(&t.reason),
        "at_ms": t.at_ms,
        "round_no": t.round_no,
    })
}

pub(super) fn task_metrics_json(metrics: &TaskJournalTaskMetrics) -> Value {
    let by_prompt_value = metrics.by_prompt.as_ref().map(|map| {
        let mut entries: Vec<(&String, &crate::LlmPromptBucket)> = map.iter().collect();
        // Sort by cost-driving buckets first for quick prompt inspection.
        entries.sort_by(|a, b| {
            b.1.count
                .cmp(&a.1.count)
                .then_with(|| b.1.elapsed_ms.cmp(&a.1.elapsed_ms))
                .then_with(|| a.0.cmp(b.0))
        });
        let object: serde_json::Map<String, Value> = entries
            .into_iter()
            .map(|(label, bucket)| {
                (
                    label.clone(),
                    json!({
                        "count": bucket.count,
                        "elapsed_ms": bucket.elapsed_ms,
                        "provider_attempt_count": bucket.provider_attempt_count,
                        "provider_retry_count": bucket.provider_retry_count,
                        "provider_selection_count": bucket.provider_selection_count,
                        "provider_fallback_count": bucket.provider_fallback_count,
                        "provider_circuit_skip_count": bucket.provider_circuit_skip_count,
                        "provider_circuit_trial_count": bucket.provider_circuit_trial_count,
                        "provider_retryable_error_count": bucket.provider_retryable_error_count,
                        "provider_final_error_count": bucket.provider_final_error_count,
                        "provider_last_retry_error_kinds": bucket.provider_last_retry_error_kinds,
                        "provider_final_error_kinds": bucket.provider_final_error_kinds,
                        "provider_breaker_snapshots": bucket.provider_breaker_snapshots,
                        "prompt_truncation_count": bucket.prompt_truncation_count,
                        "prompt_bytes_before_max": bucket.prompt_bytes_before_max,
                        "prompt_bytes_budget_min": bucket.prompt_bytes_budget_min,
                        "prompt_bytes_after_max": bucket.prompt_bytes_after_max,
                        "prompt_truncated_bytes_total": bucket.prompt_truncated_bytes_total,
                    }),
                )
            })
            .collect();
        Value::Object(object)
    });
    let provider_selection_count = metrics
        .by_prompt
        .as_ref()
        .map(|map| {
            map.values()
                .map(|bucket| bucket.provider_selection_count)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let provider_fallback_count = metrics
        .by_prompt
        .as_ref()
        .map(|map| {
            map.values()
                .map(|bucket| bucket.provider_fallback_count)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let provider_retry_count = metrics
        .by_prompt
        .as_ref()
        .map(|map| {
            map.values()
                .map(|bucket| bucket.provider_retry_count)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let provider_circuit_skip_count = metrics
        .by_prompt
        .as_ref()
        .map(|map| {
            map.values()
                .map(|bucket| bucket.provider_circuit_skip_count)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let provider_circuit_trial_count = metrics
        .by_prompt
        .as_ref()
        .map(|map| {
            map.values()
                .map(|bucket| bucket.provider_circuit_trial_count)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let logical_calls = metrics.llm_calls_per_task.unwrap_or(0);
    json!({
        "used_evidence_ids_count": metrics.used_evidence_ids_count,
        "delivery_consistent": metrics.delivery_consistent,
        "llm_calls_per_task": metrics.llm_calls_per_task,
        "llm_elapsed_ms_per_task": metrics.llm_elapsed_ms_per_task,
        "prompt_truncation_count": metrics
            .by_prompt
            .as_ref()
            .map(|map| map.values().map(|bucket| bucket.prompt_truncation_count).sum::<u64>())
            .unwrap_or(0),
        "frontdoor_llm": frontdoor_llm_metrics_json(metrics),
        "by_prompt": by_prompt_value,
        "provider_routing": {
            "logical_calls": logical_calls,
            "provider_selections": provider_selection_count,
            "provider_fallbacks": provider_fallback_count,
            "provider_retries": provider_retry_count,
            "circuit_skips": provider_circuit_skip_count,
            "circuit_trials": provider_circuit_trial_count,
            "fallback_amplification_millis": if logical_calls == 0 {
                0
            } else {
                provider_selection_count
                    .saturating_mul(1_000)
                    .saturating_div(logical_calls)
            },
        },
        "llm_cost": metrics.llm_cost_summary,
        "llm_cost_records": metrics.llm_cost_records,
    })
}

fn frontdoor_llm_metrics_json(metrics: &TaskJournalTaskMetrics) -> Value {
    let sequence = metrics.llm_call_sequence.as_deref().unwrap_or_default();
    let first_call = sequence.iter().min_by_key(|entry| entry.call_index);
    let first_planner_call_index = sequence
        .iter()
        .filter(|entry| entry.prompt_label == "plan")
        .map(|entry| entry.call_index)
        .min();
    let pre_planner = sequence
        .iter()
        .filter(|entry| {
            first_planner_call_index
                .map(|planner_index| entry.call_index < planner_index)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let mut pre_planner_labels = pre_planner
        .iter()
        .map(|entry| entry.prompt_label.clone())
        .collect::<Vec<_>>();
    pre_planner_labels.dedup();
    json!({
        "first_call_index": first_call.map(|entry| entry.call_index),
        "first_prompt_label": first_call.map(|entry| entry.prompt_label.as_str()),
        "first_planner_call_index": first_planner_call_index,
        "pre_planner_llm_calls": pre_planner.len(),
        "pre_planner_prompt_bytes": pre_planner
            .iter()
            .map(|entry| entry.prompt_bytes as u64)
            .sum::<u64>(),
        "pre_planner_prompt_labels": pre_planner_labels,
    })
}

pub(super) fn cost_budget_json(journal: &TaskJournal) -> Value {
    const SIMPLE_BOUNDED_LLM_CALL_LIMIT: u64 = 8;
    const SIMPLE_BOUNDED_ELAPSED_MS_LIMIT: u64 = 180_000;
    let by_prompt = journal.task_metrics.by_prompt.as_ref();
    let prompt_truncation_count = by_prompt
        .map(|map| {
            map.values()
                .map(|bucket| bucket.prompt_truncation_count)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let provider_retry_count = by_prompt
        .map(|map| {
            map.values()
                .map(|bucket| bucket.provider_retry_count)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let provider_attempt_count = by_prompt
        .map(|map| {
            map.values()
                .map(|bucket| bucket.provider_attempt_count)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let verifier_attempt_count = by_prompt
        .and_then(|map| map.get("verifier").map(|bucket| bucket.count))
        .unwrap_or(0);
    let repair_attempt_count = by_prompt
        .map(|map| {
            map.iter()
                .filter(|(label, _)| label.split('_').any(|part| part == "repair"))
                .map(|(_, bucket)| bucket.count)
                .sum::<u64>()
        })
        .unwrap_or(0);
    let tool_calls = journal
        .step_results
        .iter()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
        })
        .count();
    let llm_calls = journal.task_metrics.llm_calls_per_task.unwrap_or(0);
    let llm_elapsed_ms = journal.task_metrics.llm_elapsed_ms_per_task.unwrap_or(0);
    let mut signals = Vec::new();
    if llm_calls > SIMPLE_BOUNDED_LLM_CALL_LIMIT {
        signals.push("llm_call_threshold_exceeded");
    }
    if llm_elapsed_ms > SIMPLE_BOUNDED_ELAPSED_MS_LIMIT {
        signals.push("elapsed_threshold_exceeded");
    }
    if prompt_truncation_count > 0 {
        signals.push("prompt_truncation_observed");
    }
    if provider_retry_count > 0 {
        signals.push("provider_retry_observed");
    }
    if verifier_attempt_count > 1 {
        signals.push("verifier_retry_observed");
    }
    json!({
        "schema_version": 1,
        "owner_layer": "agent_loop",
        "policy_kind": "loop_telemetry_rollout_gate",
        "semantic_authority": false,
        "enforcement": "observe",
        "long_tail_escape": "checkpoint_background_resume",
        "thresholds": {
            "simple_bounded_llm_calls": SIMPLE_BOUNDED_LLM_CALL_LIMIT,
            "simple_bounded_elapsed_ms": SIMPLE_BOUNDED_ELAPSED_MS_LIMIT,
            "prompt_truncation_count": 0,
        },
        "observed": {
            "llm_calls": llm_calls,
            "tool_calls": tool_calls,
            "rounds": journal.rounds.len(),
            "steps": journal.step_results.len(),
            "llm_elapsed_ms": llm_elapsed_ms,
            "verifier_attempts": verifier_attempt_count,
            "repair_attempts": repair_attempt_count,
            "provider_attempts": provider_attempt_count,
            "provider_retries": provider_retry_count,
            "prompt_truncations": prompt_truncation_count,
            "llm_cost_status": journal
                .task_metrics
                .llm_cost_summary
                .as_ref()
                .map(|summary| summary.status.as_str()),
            "estimated_cost_usd_nanos": journal
                .task_metrics
                .llm_cost_summary
                .as_ref()
                .map(|summary| summary.estimated_cost_usd_nanos),
        },
        "signals": signals,
    })
}
