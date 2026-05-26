use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

const MAX_OBSERVED_EVIDENCE_ITEMS: usize = 24;
const MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS: usize = 240;
const MAX_OBSERVED_EVIDENCE_KEYS: usize = 16;
const MAX_OBSERVED_EVIDENCE_DEPTH: usize = 3;
const MAX_OBSERVED_ARRAY_SAMPLES: usize = 3;
const MAX_RESULT_TRACE_BYTES: usize = 128 * 1024;
const MAX_RESULT_TRACE_ARRAY_ITEMS: usize = 24;
const MAX_RESULT_TRACE_STRING_CHARS: usize = 768;
const MAX_RESULT_TRACE_COMPACT_ARRAY_ITEMS: usize = 8;
const MAX_RESULT_TRACE_COMPACT_STRING_CHARS: usize = 240;
const JSON_EVIDENCE_PRIORITY_KEYS: &[&str] = &[
    "candidates",
    "names",
    "names_by_kind",
    "paths",
    "files",
    "dirs",
    "items",
    "results",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskJournalFinalStatus {
    Success,
    Failure,
    Clarify,
    ResumeFailure,
}

impl TaskJournalFinalStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Clarify => "clarify",
            Self::ResumeFailure => "resume_failure",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskJournalFinalizerStage {
    General,
    ObservedRead,
    ObservedListDir,
    ObservedGeneric,
}

impl TaskJournalFinalizerStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::ObservedRead => "observed_read",
            Self::ObservedListDir => "observed_list_dir",
            Self::ObservedGeneric => "observed_generic",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskJournalFinalizerFallback {
    RawText,
    NoAnswerNonQualified,
    NoAnswerParseFailed,
}

impl TaskJournalFinalizerFallback {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RawText => "raw_text",
            Self::NoAnswerNonQualified => "no_answer_nonqualified",
            Self::NoAnswerParseFailed => "no_answer_parse_failed",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalVerifyIssue {
    pub(crate) step_id: String,
    pub(crate) kind: crate::verifier::VerifyIssueKind,
    pub(crate) detail: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalVerifySummary {
    pub(crate) mode: crate::verifier::VerifyMode,
    pub(crate) approved: bool,
    pub(crate) blocked_reason: Option<String>,
    pub(crate) shadow_blocked_reason: Option<String>,
    pub(crate) needs_confirmation: bool,
    pub(crate) issues: Vec<TaskJournalVerifyIssue>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalRoundTrace {
    pub(crate) round_no: usize,
    pub(crate) goal: String,
    pub(crate) execution_recipe_summary: Option<String>,
    pub(crate) plan_result: Option<crate::PlanResult>,
    pub(crate) verify_result: Option<TaskJournalVerifySummary>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalStepTrace {
    pub(crate) step_id: String,
    pub(crate) skill: String,
    pub(crate) status: crate::executor::StepExecutionStatus,
    pub(crate) output_excerpt: Option<String>,
    pub(crate) error_excerpt: Option<String>,
    pub(crate) started_at: u64,
    pub(crate) finished_at: u64,
}

#[cfg(test)]
impl TaskJournalStepTrace {
    pub(crate) fn ok(
        step_id: impl Into<String>,
        skill: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            skill: skill.into(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(output.into()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalFinalizerSummary {
    pub(crate) stage: Option<TaskJournalFinalizerStage>,
    pub(crate) disposition: Option<crate::finalize::FinalizerDisposition>,
    pub(crate) fallback: Option<TaskJournalFinalizerFallback>,
    pub(crate) parsed: bool,
    pub(crate) contract_ok: bool,
    pub(crate) completion_ok: Option<bool>,
    pub(crate) grounded_ok: Option<bool>,
    pub(crate) format_ok: Option<bool>,
    pub(crate) needs_clarify: Option<bool>,
    pub(crate) confidence: Option<f64>,
    pub(crate) used_evidence_ids_count: usize,
    pub(crate) evidence_quotes_count: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalAnswerVerifierSummary {
    pub(crate) pass: bool,
    pub(crate) missing_evidence_fields: Vec<String>,
    pub(crate) answer_incomplete_reason: String,
    pub(crate) should_retry: bool,
    pub(crate) retry_instruction: String,
    pub(crate) confidence: f64,
}

impl TaskJournalAnswerVerifierSummary {
    pub(crate) fn high_confidence_retry_gap(&self) -> bool {
        self.confidence >= 0.55 && !self.pass
    }
}

fn verify_summary_json(verify: &TaskJournalVerifySummary) -> Value {
    json!({
        "approved": verify.approved,
        "mode": verify.mode.as_str(),
        "blocked_reason": verify.blocked_reason.as_deref().map(crate::truncate_for_log),
        "shadow_blocked_reason": verify.shadow_blocked_reason.as_deref().map(crate::truncate_for_log),
        "needs_confirmation": verify.needs_confirmation,
        "issue_count": verify.issues.len(),
    })
}

fn verify_trace_json(verify: &TaskJournalVerifySummary) -> Value {
    json!({
        "approved": verify.approved,
        "mode": verify.mode.as_str(),
        "blocked_reason": verify.blocked_reason.as_deref().map(crate::truncate_for_log),
        "shadow_blocked_reason": verify.shadow_blocked_reason.as_deref().map(crate::truncate_for_log),
        "needs_confirmation": verify.needs_confirmation,
        "issues": verify.issues.iter().map(|issue| {
            json!({
                "step_id": &issue.step_id,
                "kind": issue.kind.as_str(),
                "failure_attribution": issue.kind.failure_attribution().as_str(),
                "detail": crate::truncate_for_log(&issue.detail),
            })
        }).collect::<Vec<_>>(),
    })
}

fn finalizer_summary_json(
    summary: &TaskJournalFinalizerSummary,
    route: Option<&crate::RouteResult>,
    journal: &TaskJournal,
) -> Value {
    let evidence_coverage = route.map(|route| evidence_coverage_for_route(route, journal));
    let final_answer_shape = route.and_then(|route| {
        crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
    });
    json!({
        "stage": summary.stage.map(TaskJournalFinalizerStage::as_str),
        "disposition": summary.disposition.map(crate::finalize::FinalizerDisposition::as_str),
        "fallback": summary.fallback.map(TaskJournalFinalizerFallback::as_str),
        "final_answer_shape": final_answer_shape.map(crate::contract_matrix::FinalAnswerShape::as_str),
        "final_answer_shape_class": final_answer_shape.map(|shape| shape.class().as_str()),
        "coarse_response_shape": final_answer_shape
            .map(|shape| shape.coarse_response_shape().as_str()),
        "allows_model_language": final_answer_shape.map(crate::contract_matrix::FinalAnswerShape::allows_model_language),
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

fn answer_verifier_summary_json(summary: &TaskJournalAnswerVerifierSummary) -> Value {
    json!({
        "pass": summary.pass,
        "missing_evidence_fields": summary.missing_evidence_fields,
        "answer_incomplete_reason": crate::truncate_for_log(&summary.answer_incomplete_reason),
        "should_retry": summary.should_retry,
        "retry_instruction": crate::truncate_for_log(&summary.retry_instruction),
        "confidence": summary.confidence,
    })
}

fn plan_step_action_ref(
    step: &crate::PlanStep,
    route: Option<&crate::RouteResult>,
) -> Option<String> {
    let action = crate::contract_matrix::ActionRef::from_skill_args(&step.skill, &step.args)?;
    let raw_key = action.as_key();
    if let Some(compact) = route.and_then(|route| {
        crate::contract_matrix::contract_trace_action_key_for_output_contract(
            &route.output_contract,
            &raw_key,
        )
    }) {
        return Some(compact);
    }
    Some(raw_key)
}

fn plan_summary_json(plan: &crate::PlanResult) -> Value {
    json!({
        "goal": crate::truncate_for_log(&plan.goal),
        "plan_kind": plan.plan_kind.as_str(),
        "step_count": plan.steps.len(),
        "missing_slots": plan.missing_slots,
        "needs_confirmation": plan.needs_confirmation,
    })
}

fn plan_trace_json(plan: &crate::PlanResult, route: Option<&crate::RouteResult>) -> Value {
    json!({
        "goal": crate::truncate_for_log(&plan.goal),
        "plan_kind": plan.plan_kind.as_str(),
        "planner_notes": crate::truncate_for_log(&plan.planner_notes),
        "raw_plan_text": crate::truncate_for_log(&plan.raw_plan_text),
        "step_count": plan.steps.len(),
        "steps": plan.steps.iter().map(|step| {
            json!({
                "step_id": &step.step_id,
                "action_type": &step.action_type,
                "skill": &step.skill,
                "action_ref": plan_step_action_ref(step, route),
                "depends_on": &step.depends_on,
            })
        }).collect::<Vec<_>>(),
    })
}

fn route_result_json(route: &crate::RouteResult) -> Value {
    json!({
        "route_gate_kind": route.gate_kind().as_str(),
        "first_layer_decision": route.first_layer_decision().as_str(),
        "route_label": route.derived_route_label(),
        "needs_clarify": route.needs_clarify,
        "should_refresh_long_term_memory": route.should_refresh_long_term_memory,
        "agent_display_name_hint": route.agent_display_name_hint,
        "route_reason": crate::truncate_for_log(&route.route_reason),
        "risk_ceiling": route.risk_ceiling.as_str(),
        "self_extension": {
            "mode": route.output_contract.self_extension.mode.as_str(),
            "trigger": route.output_contract.self_extension.trigger.as_str(),
            "execute_now": route.output_contract.self_extension.execute_now,
        },
    })
}

fn turn_analysis_json(analysis: &crate::intent_router::TurnAnalysis) -> Value {
    json!({
        "turn_type": analysis.turn_type.map(crate::intent_router::TurnType::as_str),
        "target_task_policy": analysis
            .target_task_policy
            .map(crate::intent_router::TargetTaskPolicy::as_str),
        "should_interrupt_active_run": analysis.should_interrupt_active_run,
        "has_state_patch": analysis.state_patch.is_some(),
        "attachment_processing_required": analysis.attachment_processing_required,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestedPlanCapability {
    action_type: String,
    capability: String,
    action_ref: Option<String>,
}

fn raw_plan_steps(raw_plan_text: &str) -> Vec<Value> {
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
    })
}

fn requested_capabilities_for_plan(
    plan: &crate::PlanResult,
    route: Option<&crate::RouteResult>,
) -> Vec<RequestedPlanCapability> {
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
                });
            if requested.action_ref.is_none() {
                requested.action_ref = plan_step_action_ref(normalized_step, route);
            }
            requested
        })
        .collect()
}

fn requested_capability_queues(
    journal: &TaskJournal,
) -> BTreeMap<String, Vec<RequestedPlanCapability>> {
    let mut requested_by_step_id: BTreeMap<String, Vec<RequestedPlanCapability>> = BTreeMap::new();
    for round in &journal.rounds {
        if let Some(plan) = round.plan_result.as_ref() {
            let requested = requested_capabilities_for_plan(plan, journal.route_result.as_ref());
            for (step, requested) in plan.steps.iter().zip(requested.into_iter()) {
                requested_by_step_id
                    .entry(step.step_id.clone())
                    .or_default()
                    .push(requested);
            }
        }
    }
    if requested_by_step_id.is_empty() {
        if let Some(plan) = journal.plan_result.as_ref() {
            let requested = requested_capabilities_for_plan(plan, journal.route_result.as_ref());
            for (step, requested) in plan.steps.iter().zip(requested.into_iter()) {
                requested_by_step_id
                    .entry(step.step_id.clone())
                    .or_default()
                    .push(requested);
            }
        }
    }
    requested_by_step_id
}

fn next_requested_capability(
    requested_by_step_id: &mut BTreeMap<String, Vec<RequestedPlanCapability>>,
    step_id: &str,
) -> Option<RequestedPlanCapability> {
    let queue = requested_by_step_id.get_mut(step_id)?;
    if queue.is_empty() {
        None
    } else {
        Some(queue.remove(0))
    }
}

fn step_trace_json(
    step: &TaskJournalStepTrace,
    requested: Option<&RequestedPlanCapability>,
    route: Option<&crate::RouteResult>,
) -> Value {
    let structured_error = step
        .error_excerpt
        .as_deref()
        .and_then(crate::skills::parse_structured_skill_error);
    let failure_attribution = structured_error_failure_attribution(structured_error.as_ref());
    let contract_policy = contract_policy_trace_json(structured_error.as_ref());
    json!({
        "step_id": &step.step_id,
        "skill": &step.skill,
        "requested_action_type": requested.map(|value| value.action_type.as_str()),
        "requested_capability": requested.map(|value| value.capability.as_str()),
        "requested_action_ref": requested.and_then(|value| value.action_ref.as_deref()),
        "executed_skill": &step.skill,
        "status": step.status.as_str(),
        "error_kind": structured_error.as_ref().map(|value| value.error_kind.as_str()),
        "failure_attribution": failure_attribution.as_deref(),
        "contract_policy": contract_policy,
        "contract": step_contract_trace_json(route, requested),
        "output_excerpt": step.output_excerpt.as_deref(),
        "observed_evidence": observed_evidence_for_step_trace(step),
        "error_excerpt": step.error_excerpt.as_deref(),
        "started_at": step.started_at,
        "finished_at": step.finished_at,
    })
}

fn step_contract_trace_json(
    route: Option<&crate::RouteResult>,
    requested: Option<&RequestedPlanCapability>,
) -> Option<Value> {
    let route = route?;
    let contract = crate::contract_matrix::trace_snapshot_for_route(route)?;
    let requested_action_ref = requested.and_then(|value| value.action_ref.as_deref());
    let action_policy = requested_action_ref.and_then(|action_ref| {
        crate::contract_matrix::action_trace_for_output_contract(&route.output_contract, action_ref)
    });
    Some(json!({
        "contract_match": contract.get("contract_match").and_then(Value::as_str),
        "semantic_kind": contract.get("semantic_kind").and_then(Value::as_str),
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

pub(crate) fn observed_evidence_for_step_trace(step: &TaskJournalStepTrace) -> Option<Value> {
    observed_evidence_from_output(step.output_excerpt.as_deref())
        .or_else(|| observed_evidence_from_error(step.error_excerpt.as_deref()))
}

pub(crate) fn observed_evidence_from_output(output: Option<&str>) -> Option<Value> {
    let output = output.map(str::trim).filter(|value| !value.is_empty())?;
    let mut collector = ObservedEvidenceCollector::default();
    let format = match serde_json::from_str::<Value>(output) {
        Ok(value) => {
            collect_json_observed_evidence(&mut collector, "json_output", "", &value, 0);
            "json"
        }
        Err(_) => {
            collect_text_observed_evidence(&mut collector, output);
            "text"
        }
    };
    if collector.items.is_empty() {
        return None;
    }
    let item_count = collector.total_count;
    Some(json!({
        "schema_version": 1,
        "source": "step_output",
        "format": format,
        "storage": "redacted_excerpt_hash",
        "item_count": item_count,
        "truncated": item_count > collector.items.len(),
        "items": collector.items,
    }))
}

fn observed_evidence_from_error(error: Option<&str>) -> Option<Value> {
    let error = error.map(str::trim).filter(|value| !value.is_empty())?;
    let mut collector = ObservedEvidenceCollector::default();
    let format = if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
        collector.push(json_observed_evidence_item(
            "structured_error",
            "error_text",
            &json!(structured.error_text),
        ));
        if let Some(extra) = structured.extra.as_ref() {
            collect_json_observed_evidence(&mut collector, "structured_error.extra", "", extra, 0);
        }
        "json"
    } else {
        collect_text_observed_evidence(&mut collector, error);
        "text"
    };
    if collector.items.is_empty() {
        return None;
    }
    let item_count = collector.total_count;
    Some(json!({
        "schema_version": 1,
        "source": "step_error",
        "format": format,
        "storage": "redacted_excerpt_hash",
        "item_count": item_count,
        "truncated": item_count > collector.items.len(),
        "items": collector.items,
    }))
}

#[derive(Default)]
struct ObservedEvidenceCollector {
    items: Vec<Value>,
    total_count: usize,
}

impl ObservedEvidenceCollector {
    fn push(&mut self, item: Value) {
        self.total_count += 1;
        if self.items.len() < MAX_OBSERVED_EVIDENCE_ITEMS {
            self.items.push(item);
        }
    }
}

fn collect_json_observed_evidence(
    collector: &mut ObservedEvidenceCollector,
    source: &str,
    prefix: &str,
    value: &Value,
    depth: usize,
) {
    if depth > MAX_OBSERVED_EVIDENCE_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            if depth > 0 {
                collector.push(json_observed_evidence_item(source, prefix, value));
            }
            let mut emitted_priority_keys = BTreeSet::new();
            if depth == 0 && prefix.is_empty() {
                for key in JSON_EVIDENCE_PRIORITY_KEYS {
                    if let Some(child) = map.get(*key) {
                        collect_json_object_child(collector, source, depth, prefix, key, child);
                        emitted_priority_keys.insert((*key).to_string());
                    }
                }
            }
            for (key, child) in map {
                if emitted_priority_keys.contains(key.as_str()) {
                    continue;
                }
                collect_json_object_child(collector, source, depth, prefix, key, child);
            }
        }
        Value::Array(items) => {
            if depth == 0 || prefix.is_empty() {
                collector.push(json_observed_evidence_item(source, "value", value));
            }
            for (idx, child) in items.iter().take(MAX_OBSERVED_ARRAY_SAMPLES).enumerate() {
                let field = if prefix.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{prefix}[{idx}]")
                };
                collector.push(json_observed_evidence_item(source, &field, child));
                if depth < MAX_OBSERVED_EVIDENCE_DEPTH
                    && matches!(child, Value::Object(_) | Value::Array(_))
                {
                    collect_json_observed_evidence(collector, source, &field, child, depth + 1);
                }
            }
        }
        _ => collector.push(json_observed_evidence_item(source, "value", value)),
    }
}

fn collect_json_object_child(
    collector: &mut ObservedEvidenceCollector,
    source: &str,
    depth: usize,
    prefix: &str,
    key: &str,
    child: &Value,
) {
    let field = if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    };
    collector.push(json_observed_evidence_item(source, &field, child));
    if depth < MAX_OBSERVED_EVIDENCE_DEPTH && matches!(child, Value::Object(_) | Value::Array(_)) {
        let child_source = if depth == 0 && key == "extra" {
            "json_output.extra"
        } else {
            source
        };
        collect_json_observed_evidence(collector, child_source, &field, child, depth + 1);
    }
}

fn json_observed_evidence_item(source: &str, field: &str, value: &Value) -> Value {
    let sensitive_field = evidence_field_is_sensitive(field);
    let mut item = serde_json::Map::new();
    item.insert("field".to_string(), json!(field));
    item.insert("source".to_string(), json!(source));
    item.insert("kind".to_string(), json!(json_value_kind(value)));
    match value {
        Value::Object(map) => {
            item.insert(
                "keys".to_string(),
                json!(map
                    .keys()
                    .take(MAX_OBSERVED_EVIDENCE_KEYS)
                    .collect::<Vec<_>>()),
            );
            item.insert("key_count".to_string(), json!(map.len()));
        }
        Value::Array(items) => {
            item.insert("count".to_string(), json!(items.len()));
            item.insert(
                "sample_kinds".to_string(),
                json!(items
                    .iter()
                    .take(MAX_OBSERVED_EVIDENCE_KEYS)
                    .map(json_value_kind)
                    .collect::<Vec<_>>()),
            );
        }
        Value::Null => {
            item.insert("excerpt".to_string(), json!("null"));
            item.insert("hash".to_string(), json!(stable_trace_hash("null")));
        }
        Value::Bool(value) => {
            let text = value.to_string();
            item.insert("excerpt".to_string(), json!(text));
            item.insert("hash".to_string(), json!(stable_trace_hash(&text)));
        }
        Value::Number(value) => {
            let text = value.to_string();
            item.insert("excerpt".to_string(), json!(text));
            item.insert("hash".to_string(), json!(stable_trace_hash(&text)));
        }
        Value::String(value) => {
            if sensitive_field || text_looks_sensitive(value) {
                item.insert("redacted".to_string(), json!(true));
            } else {
                item.insert("excerpt".to_string(), json!(evidence_excerpt(value)));
                item.insert("hash".to_string(), json!(stable_trace_hash(value)));
            }
        }
    }
    Value::Object(item)
}

fn text_observed_evidence_item(output: &str) -> Value {
    let excerpt = redacted_text_excerpt(output);
    json!({
        "field": "text_excerpt",
        "source": "text_output",
        "kind": "text",
        "excerpt": excerpt,
        "hash": stable_trace_hash(output),
    })
}

fn collect_text_observed_evidence(collector: &mut ObservedEvidenceCollector, output: &str) {
    collector.push(text_observed_evidence_item(output));
    if let Some(count) = text_count_evidence(output) {
        collector.push(json_observed_evidence_item(
            "text_output.extractor",
            "count",
            &json!(count),
        ));
    }
    if let Some(subject) = text_git_oneline_subject_evidence(output) {
        collector.push(text_extracted_evidence_item("subject", &subject));
    }
    if let Some(path) = text_path_evidence(output) {
        collector.push(text_extracted_evidence_item("path", &path));
    }
    collect_text_machine_key_value_evidence(collector, output);
    if let Some(state) = text_git_state_evidence(output) {
        collector.push(text_extracted_evidence_item("state", state));
    }
    let lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() > 1
        && lines
            .iter()
            .all(|line| text_line_looks_like_list_item(line))
    {
        collector.push(json_observed_evidence_item(
            "text_output.extractor",
            "count",
            &json!(lines.len()),
        ));
        let hidden_count = lines
            .iter()
            .filter(|line| text_line_looks_like_hidden_entry(line))
            .count();
        if hidden_count > 0 {
            collector.push(json_observed_evidence_item(
                "text_output.extractor",
                "hidden_count",
                &json!(hidden_count),
            ));
        }
        for (idx, line) in lines.iter().take(MAX_OBSERVED_EVIDENCE_ITEMS).enumerate() {
            collector.push(text_extracted_evidence_item(
                &format!("results[{idx}]"),
                line,
            ));
        }
    }
}

fn collect_text_machine_key_value_evidence(
    collector: &mut ObservedEvidenceCollector,
    output: &str,
) {
    let mut seen = BTreeSet::new();
    for token in output.lines().flat_map(str::split_whitespace) {
        let token = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
                )
            })
            .trim();
        let Some((raw_key, raw_value)) = token.split_once('=') else {
            continue;
        };
        let key = normalize_evidence_field(raw_key);
        if !machine_key_value_evidence_key_allowed(&key) || evidence_field_is_sensitive(&key) {
            continue;
        }
        let value = raw_value
            .trim()
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';'));
        if value.is_empty() || text_looks_sensitive(value) {
            continue;
        }
        if seen.insert((key.clone(), value.to_string())) {
            collector.push(text_extracted_evidence_item(&key, value));
        }
    }
}

fn machine_key_value_evidence_key_allowed(key: &str) -> bool {
    matches!(
        key,
        "field_value"
            | "value"
            | "status"
            | "state"
            | "version"
            | "schema_version"
            | "package_manager"
            | "manager"
            | "subject"
            | "branch"
            | "commit"
            | "valid"
            | "available"
            | "size_bytes"
            | "bytes"
            | "exit"
            | "exit_code"
            | "error_kind"
    )
}

fn text_extracted_evidence_item(field: &str, value: &str) -> Value {
    let excerpt = redacted_text_excerpt(value);
    json!({
        "field": field,
        "source": "text_output.extractor",
        "kind": "text",
        "excerpt": excerpt,
        "hash": stable_trace_hash(value),
    })
}

fn text_count_evidence(output: &str) -> Option<i64> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return Some(value);
    }
    let normalized = trimmed
        .replace(',', " ")
        .replace(':', " ")
        .replace(';', " ");
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let mut counts = BTreeSet::new();
    for window in tokens.windows(2) {
        let number = window[0].parse::<i64>().ok();
        let unit = window[1].trim_matches(|ch: char| !ch.is_ascii_alphabetic());
        if let Some(value) = number {
            let unit = unit.to_ascii_lowercase();
            if matches!(
                unit.as_str(),
                "file" | "files" | "item" | "items" | "entry" | "entries" | "row" | "rows"
            ) {
                counts.insert(value);
            }
        }
    }
    (counts.len() == 1).then(|| *counts.iter().next().expect("single count"))
}

fn text_git_state_evidence(output: &str) -> Option<&'static str> {
    let mut saw_git_branch = false;
    let mut saw_change = false;
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with("## ") {
            saw_git_branch = true;
            continue;
        }
        if line == "exit=0" {
            continue;
        }
        if line
            .chars()
            .take(2)
            .any(|ch| matches!(ch, 'M' | 'A' | 'D' | 'R' | 'C' | 'U' | '?' | '!'))
        {
            saw_change = true;
        }
    }
    if saw_git_branch {
        Some(if saw_change { "dirty" } else { "clean" })
    } else {
        None
    }
}

fn text_path_evidence(output: &str) -> Option<String> {
    let lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() == 1 && text_line_looks_like_standalone_path(lines[0]) {
        return Some(lines[0].to_string());
    }
    if let Some(path) = labeled_text_path_evidence(output) {
        return Some(path);
    }
    let mut paths = BTreeSet::new();
    for token in output.split_whitespace() {
        let candidate = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '。' | '，'
                )
            })
            .trim();
        if text_line_looks_like_path(candidate) {
            paths.insert(candidate.to_string());
            continue;
        }
        if let Some((_, rhs)) = candidate.split_once('=') {
            let rhs = rhs.trim();
            if text_line_looks_like_path(rhs) {
                paths.insert(rhs.to_string());
            }
        }
    }
    (paths.len() == 1).then(|| paths.into_iter().next().expect("single path"))
}

fn labeled_text_path_evidence(output: &str) -> Option<String> {
    let mut paths = BTreeSet::new();
    for token in output.split_whitespace() {
        let candidate = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '。' | '，'
                )
            })
            .trim();
        let Some((key, rhs)) = candidate.split_once('=') else {
            continue;
        };
        let key = normalize_evidence_field(key);
        if !matches!(
            key.as_str(),
            "path"
                | "archive"
                | "archive_path"
                | "output"
                | "output_path"
                | "dest"
                | "dest_path"
                | "destination"
        ) {
            continue;
        }
        let rhs = rhs.trim();
        if text_line_looks_like_path(rhs) {
            paths.insert(rhs.to_string());
        }
    }
    (paths.len() == 1).then(|| paths.into_iter().next().expect("single labeled path"))
}

fn text_git_oneline_subject_evidence(output: &str) -> Option<String> {
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line == "exit=0" {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let Some(hash) = parts.next() else {
            continue;
        };
        let Some(subject) = parts.next() else {
            continue;
        };
        let subject = subject.trim();
        if text_looks_like_git_hash(hash) && !subject.is_empty() {
            return Some(subject.to_string());
        }
    }
    None
}

fn text_looks_like_git_hash(value: &str) -> bool {
    (7..=40).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn text_line_looks_like_path(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty()
        && line.len() <= MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS
        && !line.contains(|ch| matches!(ch, '\n' | '\r' | '\0'))
        && !line.contains("://")
        && !line.ends_with(['.', '。'])
        && (line.starts_with('/')
            || line.starts_with("./")
            || line.starts_with("../")
            || line.contains('/'))
}

fn text_line_looks_like_standalone_path(line: &str) -> bool {
    text_line_looks_like_path(line) && line.split_whitespace().count() == 1
}

fn text_line_looks_like_list_item(line: &str) -> bool {
    let line = line.trim();
    if line == "." {
        return true;
    }
    !line.is_empty()
        && line.len() <= MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS
        && !line.contains(|ch| matches!(ch, '\n' | '\r' | '\0'))
        && !line.contains("://")
        && !line.ends_with(['.', '。', ':', '：'])
        && line.split_whitespace().count() <= 4
}

fn text_line_looks_like_hidden_entry(line: &str) -> bool {
    let leaf = line
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';'))
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim();
    leaf.starts_with('.') && leaf != "." && leaf != ".."
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn evidence_field_is_sensitive(field: &str) -> bool {
    let normalized = field.to_ascii_lowercase().replace(['-', '.'], "_");
    [
        "secret",
        "token",
        "password",
        "passwd",
        "credential",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
        "cookie",
        "authorization",
        "auth_header",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn evidence_excerpt(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS {
        return collapsed;
    }
    let mut out =
        crate::utf8_safe_prefix(&collapsed, MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS).to_string();
    out.push_str("...(truncated)");
    out
}

fn redacted_text_excerpt(text: &str) -> String {
    let redacted = text
        .split_whitespace()
        .map(|token| {
            if text_looks_sensitive(token) {
                "[redacted]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    evidence_excerpt(&redacted)
}

fn text_looks_sensitive(text: &str) -> bool {
    let trimmed =
        text.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-');
    if known_non_secret_config_risk_label(trimmed) {
        return false;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return false;
    }
    if trimmed.len() < 24 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("sk-") || lower.starts_with("sk_") {
        return true;
    }
    let dense_chars = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '+'))
        .count();
    dense_chars * 100 / trimmed.len().max(1) >= 85
}

fn known_non_secret_config_risk_label(text: &str) -> bool {
    let Some((field, value)) = text.split_once('=') else {
        return false;
    };
    let field = field.trim().to_ascii_lowercase();
    let value = value
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'));
    if !matches!(
        field.as_str(),
        "tools.allow"
            | "tools.allow_sudo"
            | "tools.allow_path_outside_workspace"
            | "telegram.sendfile.full_access"
            | "server.listen"
            | "self_extension.enabled"
            | "worker.task_timeout_seconds"
    ) {
        return false;
    }
    if value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("false")
        || value.parse::<i64>().is_ok()
        || value.parse::<f64>().is_ok()
    {
        return true;
    }
    field == "tools.allow" && value == "[\"*\"]"
        || field == "server.listen" && (value == "0.0.0.0" || value.starts_with("0.0.0.0:"))
}

fn stable_trace_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64:{hash:016x}")
}

#[derive(Debug, Default)]
struct TraceStorageStats {
    truncated_arrays: usize,
    omitted_array_items: usize,
    truncated_strings: usize,
}

fn trace_json_bytes(value: &Value) -> usize {
    serde_json::to_vec(value).map(|bytes| bytes.len()).unwrap_or(0)
}

fn trace_json_hash(value: &Value) -> String {
    serde_json::to_string(value)
        .map(|text| stable_trace_hash(&text))
        .unwrap_or_else(|_| stable_trace_hash("<unserializable-trace>"))
}

fn compact_result_trace_value(
    value: &mut Value,
    stats: &mut TraceStorageStats,
    max_array_items: usize,
    max_string_chars: usize,
) {
    match value {
        Value::String(text) => {
            if text.chars().count() > max_string_chars {
                let mut truncated = crate::utf8_safe_prefix(text, max_string_chars).to_string();
                truncated.push_str("...(truncated)");
                *text = truncated;
                stats.truncated_strings += 1;
            }
        }
        Value::Array(items) => {
            if items.len() > max_array_items {
                stats.truncated_arrays += 1;
                stats.omitted_array_items += items.len() - max_array_items;
                items.truncate(max_array_items);
            }
            for item in items {
                compact_result_trace_value(item, stats, max_array_items, max_string_chars);
            }
        }
        Value::Object(map) => {
            for child in map.values_mut() {
                compact_result_trace_value(child, stats, max_array_items, max_string_chars);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn result_trace_storage_meta(
    original_bytes: usize,
    stored_bytes: usize,
    original_hash: String,
    stats: &TraceStorageStats,
    truncated: bool,
) -> Value {
    json!({
        "schema_version": 1,
        "max_bytes": MAX_RESULT_TRACE_BYTES,
        "truncated": truncated,
        "original_bytes": original_bytes,
        "stored_bytes": stored_bytes,
        "original_hash": original_hash,
        "truncated_arrays": stats.truncated_arrays,
        "omitted_array_items": stats.omitted_array_items,
        "truncated_strings": stats.truncated_strings,
    })
}

fn insert_result_trace_storage_meta(trace: &mut Value, meta: Value) {
    if let Some(obj) = trace.as_object_mut() {
        obj.insert("trace_storage".to_string(), meta);
    }
}

fn result_trace_json_with_storage_limit(mut trace: Value) -> Value {
    let original_bytes = trace_json_bytes(&trace);
    let original_hash = trace_json_hash(&trace);
    if original_bytes <= MAX_RESULT_TRACE_BYTES {
        let stats = TraceStorageStats::default();
        let meta = result_trace_storage_meta(
            original_bytes,
            original_bytes,
            original_hash,
            &stats,
            false,
        );
        insert_result_trace_storage_meta(&mut trace, meta);
        return trace;
    }

    let mut stats = TraceStorageStats::default();
    compact_result_trace_value(
        &mut trace,
        &mut stats,
        MAX_RESULT_TRACE_ARRAY_ITEMS,
        MAX_RESULT_TRACE_STRING_CHARS,
    );
    if trace_json_bytes(&trace) > MAX_RESULT_TRACE_BYTES {
        compact_result_trace_value(
            &mut trace,
            &mut stats,
            MAX_RESULT_TRACE_COMPACT_ARRAY_ITEMS,
            MAX_RESULT_TRACE_COMPACT_STRING_CHARS,
        );
    }
    let stored_bytes = trace_json_bytes(&trace);
    let meta = result_trace_storage_meta(
        original_bytes,
        stored_bytes,
        original_hash,
        &stats,
        true,
    );
    insert_result_trace_storage_meta(&mut trace, meta);
    trace
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TaskJournalEvidenceCoverage {
    pub(crate) required_evidence: Vec<String>,
    pub(crate) evidence_expression: Option<Value>,
    pub(crate) observed_fields: BTreeSet<String>,
    pub(crate) observed_canonical: BTreeSet<String>,
    pub(crate) missing_evidence: Vec<String>,
}

impl TaskJournalEvidenceCoverage {
    pub(crate) fn is_complete(&self) -> bool {
        self.missing_evidence.is_empty()
    }

    fn to_trace_json(&self) -> Value {
        json!({
            "schema_version": 1,
            "required_evidence": self.required_evidence.clone(),
            "evidence_expression": self.evidence_expression.clone(),
            "observed_fields": self.observed_fields.iter().take(64).cloned().collect::<Vec<_>>(),
            "observed_canonical": self.observed_canonical.iter().take(64).cloned().collect::<Vec<_>>(),
            "missing_evidence": self.missing_evidence.clone(),
        })
    }
}

pub(crate) fn evidence_coverage_for_route(
    route: &crate::RouteResult,
    journal: &TaskJournal,
) -> TaskJournalEvidenceCoverage {
    let required_evidence =
        crate::task_contract::required_evidence_fields_for_output_contract(&route.output_contract);
    let (observed_fields, observed_field_leaf_counts, mut observed_canonical) =
        observed_evidence_field_sets(journal);
    augment_route_canonical_evidence(
        route,
        &observed_fields,
        &observed_field_leaf_counts,
        &mut observed_canonical,
    );
    let evidence_expression = crate::contract_matrix::bundled_contract_matrix()
        .and_then(|matrix| matrix.match_output_contract(&route.output_contract))
        .map(|matched| matched.evidence_expression());
    let missing_evidence = evidence_expression
        .as_ref()
        .map(|expression| missing_evidence_for_expression(expression, &observed_canonical))
        .unwrap_or_else(|| missing_required_evidence(&required_evidence, &observed_canonical));
    TaskJournalEvidenceCoverage {
        required_evidence,
        evidence_expression: evidence_expression
            .as_ref()
            .map(|expression| expression.to_trace_json(&[])),
        observed_fields,
        observed_canonical,
        missing_evidence,
    }
}

fn missing_required_evidence(
    required_evidence: &[String],
    observed_canonical: &BTreeSet<String>,
) -> Vec<String> {
    required_evidence
        .iter()
        .filter(|field| !observed_canonical.contains(field.as_str()))
        .cloned()
        .collect()
}

fn missing_evidence_for_expression(
    expression: &crate::contract_matrix::EvidenceExpression,
    observed_canonical: &BTreeSet<String>,
) -> Vec<String> {
    let mut missing = Vec::new();
    missing.extend(
        expression
            .all_of
            .iter()
            .filter(|field| !observed_canonical.contains(field.as_str()))
            .cloned(),
    );
    if !expression.one_of.is_empty()
        && !expression
            .one_of
            .iter()
            .any(|field| observed_canonical.contains(field.as_str()))
    {
        missing.push(format!("one_of({})", expression.one_of.join("|")));
    }
    if !expression.any_of.is_empty()
        && !expression
            .any_of
            .iter()
            .any(|field| observed_canonical.contains(field.as_str()))
    {
        missing.push(format!("any_of({})", expression.any_of.join("|")));
    }
    missing.dedup();
    missing
}

fn evidence_coverage_trace_json(route: &crate::RouteResult, journal: &TaskJournal) -> Value {
    evidence_coverage_for_route(route, journal).to_trace_json()
}

fn task_outcome_summary_json(journal: &TaskJournal) -> Value {
    let final_shape = journal
        .route_result
        .as_ref()
        .and_then(crate::contract_matrix::trace_snapshot_for_route)
        .and_then(|snapshot| {
            snapshot
                .get("final_answer_shape")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });
    let missing_evidence = journal
        .route_result
        .as_ref()
        .map(|route| evidence_coverage_for_route(route, journal).missing_evidence)
        .unwrap_or_default();
    let missing_count = missing_evidence.len();
    let state = match journal.final_status {
        Some(TaskJournalFinalStatus::Success) if missing_count == 0 => "completed",
        Some(TaskJournalFinalStatus::Success) => "needs_attention",
        Some(TaskJournalFinalStatus::Clarify) => "needs_input",
        Some(TaskJournalFinalStatus::Failure | TaskJournalFinalStatus::ResumeFailure) => "failed",
        None => "in_progress",
    };
    let (message_zh, message_en, next_step_zh, next_step_en) = match state {
        "completed" => (
            "任务已完成。",
            "The task completed.",
            "可以直接查看结果。",
            "You can review the result.",
        ),
        "needs_attention" => (
            "任务已返回结果，但证据没有完全匹配。",
            "The task returned a result, but evidence did not fully match.",
            "请展开技术详情确认缺少的证据，必要时补充目标后重试。",
            "Open technical details to check missing evidence, then add the target and retry if needed.",
        ),
        "needs_input" => (
            "任务需要你补充信息。",
            "The task needs more information.",
            "请按提示补充目标、路径或确认信息。",
            "Provide the requested target, path, or confirmation.",
        ),
        "failed" if missing_count > 0 => (
            "任务没有完成，缺少必要证据。",
            "The task did not complete because required evidence is missing.",
            "请补充明确目标后重试。",
            "Add a clearer target and retry.",
        ),
        "failed" => (
            "任务没有完成。",
            "The task did not complete.",
            "请根据错误信息处理后重试；技术详情已保留在下方。",
            "Use the error message to decide the next step, then retry. Technical details are available below.",
        ),
        _ => (
            "任务正在处理。",
            "The task is in progress.",
            "稍后重新查询任务状态。",
            "Query the task again shortly.",
        ),
    };
    json!({
        "schema_version": 1,
        "state": state,
        "message_zh": message_zh,
        "message_en": message_en,
        "next_step_zh": next_step_zh,
        "next_step_en": next_step_en,
        "final_answer_shape": final_shape,
        "missing_evidence_count": missing_count,
        "has_technical_details": true,
    })
}

fn observed_evidence_field_sets(
    journal: &TaskJournal,
) -> (BTreeSet<String>, BTreeMap<String, usize>, BTreeSet<String>) {
    let mut observed_fields = BTreeSet::new();
    let mut observed_field_leaf_counts = BTreeMap::new();
    let mut observed_canonical = BTreeSet::new();
    for step in &journal.step_results {
        if !step_can_supply_contract_evidence(step, journal.route_result.as_ref()) {
            continue;
        }
        let Some(evidence) = observed_evidence_for_step_trace(step) else {
            continue;
        };
        let Some(items) = evidence.get("items").and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(field) = item.get("field").and_then(Value::as_str) else {
                continue;
            };
            let normalized = normalize_evidence_field(field);
            if normalized.is_empty() {
                continue;
            }
            *observed_field_leaf_counts
                .entry(normalized_field_leaf(&normalized).to_string())
                .or_insert(0) += 1;
            observed_fields.insert(normalized.clone());
            for canonical in canonical_evidence_fields_for_observed_item(&normalized, item) {
                observed_canonical.insert(canonical);
            }
        }
    }
    (
        observed_fields,
        observed_field_leaf_counts,
        observed_canonical,
    )
}

fn augment_route_canonical_evidence(
    route: &crate::RouteResult,
    observed_fields: &BTreeSet<String>,
    observed_field_leaf_counts: &BTreeMap<String, usize>,
    observed_canonical: &mut BTreeSet<String>,
) {
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && observed_canonical.contains("size_bytes")
        && observed_field_leaf_counts
            .get("size_bytes")
            .copied()
            .unwrap_or(0)
            >= 2
    {
        observed_canonical.insert("field_value".to_string());
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::GitCommitSubject | crate::OutputSemanticKind::GitRepositoryState
    ) && (observed_canonical.contains("command_output")
        || observed_canonical.contains("content_excerpt")
        || observed_fields.contains("text_excerpt"))
    {
        observed_canonical.insert("field_value".to_string());
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarPathOnly
        && (observed_canonical.contains("path")
            || observed_canonical.contains("content_match")
            || observed_canonical.contains("candidates")
            || observed_field_with_prefix(observed_fields, "results["))
    {
        observed_canonical.insert("field_value".to_string());
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount
        && (observed_canonical.contains("value") || observed_canonical.contains("field_value"))
    {
        observed_canonical.insert("count".to_string());
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::StructuredKeys
        && (observed_canonical.contains("keys")
            || observed_field_with_prefix(observed_fields, "keys["))
    {
        observed_canonical.insert("field_value".to_string());
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::DockerContainerLifecycle
            | crate::OutputSemanticKind::DockerPs
            | crate::OutputSemanticKind::DockerImages
            | crate::OutputSemanticKind::DockerLogs
    ) && (observed_canonical.contains("command_output")
        || observed_canonical.contains("content_excerpt")
        || observed_fields.contains("text_excerpt"))
    {
        match route.output_contract.semantic_kind {
            crate::OutputSemanticKind::DockerContainerLifecycle => {
                observed_canonical.insert("field_value".to_string());
            }
            crate::OutputSemanticKind::DockerPs
            | crate::OutputSemanticKind::DockerImages
            | crate::OutputSemanticKind::DockerLogs => {
                observed_canonical.insert("candidates".to_string());
            }
            _ => {}
        }
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::SqliteDatabaseKindJudgment
        && (observed_canonical.contains("candidates")
            || observed_fields.contains("rows")
            || observed_fields.contains("columns"))
    {
        observed_canonical.insert("field_value".to_string());
    }
}

fn observed_field_with_prefix(observed_fields: &BTreeSet<String>, prefix: &str) -> bool {
    observed_fields
        .iter()
        .any(|field| field.starts_with(prefix))
}

fn normalized_field_leaf(field: &str) -> &str {
    let leaf = field.rsplit('.').next().unwrap_or(field);
    leaf.split_once('[').map_or(leaf, |(prefix, _)| prefix)
}

fn step_can_supply_contract_evidence(
    step: &TaskJournalStepTrace,
    route: Option<&crate::RouteResult>,
) -> bool {
    if matches!(
        step.skill.as_str(),
        "respond" | "synthesize_answer" | "think" | "answer_verifier"
    ) {
        return false;
    }
    match step.status {
        crate::executor::StepExecutionStatus::Ok => true,
        crate::executor::StepExecutionStatus::Error => {
            step.skill == "run_cmd"
                && route.is_some_and(|route| {
                    route.output_contract.semantic_kind
                        == crate::OutputSemanticKind::ExecutionFailedStep
                        || crate::task_contract::required_evidence_fields_for_output_contract(
                            &route.output_contract,
                        )
                        .iter()
                        .any(|field| field == "command_output")
                })
        }
    }
}

fn normalize_evidence_field(field: &str) -> String {
    field
        .trim()
        .trim_matches('.')
        .to_ascii_lowercase()
        .replace('-', "_")
}

fn canonical_evidence_fields_for_observed_field(field: &str) -> Vec<String> {
    let leaf = normalized_field_leaf(field);
    let mut values = BTreeSet::new();
    values.insert(field.to_string());
    values.insert(leaf.to_string());

    for (canonical, aliases) in [
        (
            "candidates",
            &[
                "candidates",
                "items",
                "names",
                "paths",
                "files",
                "entries",
                "results",
                "facts",
                "rows",
                "tables",
                "containers",
                "images",
                "members",
                "risks",
            ][..],
        ),
        (
            "content_excerpt",
            &[
                "content_excerpt",
                "excerpt",
                "body",
                "content",
                "text",
                "lines",
                "text_excerpt",
            ][..],
        ),
        (
            "content_match",
            &[
                "content_match",
                "match",
                "matches",
                "grep_matches",
                "lines",
                "results",
            ][..],
        ),
        (
            "path",
            &[
                "path",
                "resolved_path",
                "file_path",
                "target_path",
                "output_path",
                "archive",
                "archive_path",
                "dest",
                "dest_path",
                "destination",
            ][..],
        ),
        (
            "field_value",
            &[
                "field_value",
                "value",
                "status",
                "state",
                "version",
                "schema_version",
                "package_manager",
                "manager",
                "subject",
                "branch",
                "commit",
                "valid",
                "available",
                "healthy",
                "running",
                "is_running",
                "port_open",
                "process_count",
                "clawd_health_port_open",
                "clawd_process_count",
                "modified_ts",
                "modified",
                "mtime",
                "mtime_ts",
                "exit",
                "exit_code",
                "error_kind",
            ][..],
        ),
        (
            "count",
            &[
                "count",
                "total",
                "length",
                "item_count",
                "row_count",
                "risk_count",
            ][..],
        ),
        (
            "size_bytes",
            &["size_bytes", "bytes", "file_size", "size"][..],
        ),
        ("exists", &["exists", "found", "present"][..]),
        ("kind", &["kind", "file_type", "type"][..]),
        (
            "command_output",
            &[
                "command_output",
                "stdout",
                "stderr",
                "output",
                "text_excerpt",
            ][..],
        ),
    ] {
        if aliases
            .iter()
            .any(|alias| *alias == leaf || *alias == field)
        {
            values.insert(canonical.to_string());
        }
    }
    values.into_iter().collect()
}

fn canonical_evidence_fields_for_observed_item(field: &str, item: &Value) -> Vec<String> {
    let mut values = canonical_evidence_fields_for_observed_field(field)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if values.contains("exists")
        && item.get("kind").and_then(Value::as_str) == Some("bool")
        && item.get("excerpt").and_then(Value::as_str).is_some()
    {
        match item
            .get("excerpt")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "true" => {
                values.insert("exists_true".to_string());
            }
            "false" => {
                values.insert("exists_false".to_string());
            }
            _ => {}
        }
    }
    values.into_iter().collect()
}

fn structured_error_extra_string(
    structured_error: Option<&crate::skills::StructuredSkillError>,
    key: &str,
) -> Option<String> {
    structured_error
        .and_then(|value| value.extra.as_ref())
        .and_then(|extra| extra.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn structured_error_failure_attribution(
    structured_error: Option<&crate::skills::StructuredSkillError>,
) -> Option<String> {
    if let Some(raw) = structured_error_extra_string(structured_error, "failure_attribution") {
        return crate::contract_matrix::FailureAttribution::parse(&raw)
            .map(|kind| kind.as_str().to_string())
            .or(Some(raw));
    }
    structured_error
        .and_then(|value| failure_attribution_for_structured_error_kind(&value.error_kind))
        .map(|kind| kind.as_str().to_string())
}

pub(crate) fn failure_attribution_for_error_text(
    error_text: &str,
) -> Option<crate::contract_matrix::FailureAttribution> {
    let trimmed = error_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(structured) = crate::skills::parse_structured_skill_error(trimmed) {
        if let Some(raw) = structured_error_extra_string(Some(&structured), "failure_attribution") {
            if let Some(kind) = crate::contract_matrix::FailureAttribution::parse(&raw) {
                return Some(kind);
            }
        }
        if let Some(kind) = failure_attribution_for_structured_error_kind(&structured.error_kind) {
            return Some(kind);
        }
    }

    let normalized = trimmed.to_ascii_lowercase().replace('-', "_");
    if normalized.contains("schema_validation_failed")
        || normalized.contains("schema validation")
        || normalized.contains("json schema")
        || normalized.contains("invalid schema")
    {
        return Some(crate::contract_matrix::FailureAttribution::SchemaError);
    }
    if normalized.contains("all llm providers in circuit_breaker cooldown")
        || normalized.contains("unknown llm error")
        || (normalized.contains("provider=") && normalized.contains(" failed"))
        || normalized.contains("provider_error")
        || normalized.contains("provider_retryable")
        || normalized.contains("provider_non_retryable")
        || normalized.contains("rate_limited")
        || normalized.contains("quota_exhausted")
    {
        return Some(crate::contract_matrix::FailureAttribution::ProviderError);
    }
    if normalized.contains("channel_send_failed")
        || normalized.contains("delivery_error")
        || normalized.contains("delivery failed")
        || normalized.contains("send status=")
    {
        return Some(crate::contract_matrix::FailureAttribution::DeliveryError);
    }
    None
}

fn failure_attribution_for_structured_error_kind(
    error_kind: &str,
) -> Option<crate::contract_matrix::FailureAttribution> {
    let normalized = error_kind.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "schema_error"
        | "schema_validation_failed"
        | "schema_recovery_failed"
        | "json_schema_error"
        | "invalid_json_schema"
        | "missing_required_field" => Some(crate::contract_matrix::FailureAttribution::SchemaError),
        "provider_error"
        | "provider_retryable_response"
        | "provider_retryable_business"
        | "provider_non_retryable_business"
        | "provider_response_invalid"
        | "provider_schema_error"
        | "transport_retryable"
        | "rate_limited"
        | "quota_exhausted"
        | "llm_provider_error"
        | "llm_provider_unavailable" => {
            Some(crate::contract_matrix::FailureAttribution::ProviderError)
        }
        "delivery_error"
        | "delivery_failed"
        | "channel_send_failed"
        | "file_delivery_failed"
        | "media_delivery_failed"
        | "missing_delivery_artifact"
        | "delivery_token_invalid" => {
            Some(crate::contract_matrix::FailureAttribution::DeliveryError)
        }
        "permission_denied" | "policy_denied" | "skill_disabled" | "requires_confirmation" => {
            Some(crate::contract_matrix::FailureAttribution::PermissionDenied)
        }
        "contract_action_rejected" | "contract_policy_violation" | "contract_missing" => {
            Some(crate::contract_matrix::FailureAttribution::ContractGap)
        }
        "budget_exhausted" | "round_budget_exhausted" | "tool_budget_exhausted" => {
            Some(crate::contract_matrix::FailureAttribution::BudgetExhausted)
        }
        "prompt_budget_error" => {
            Some(crate::contract_matrix::FailureAttribution::PromptBudgetError)
        }
        _ => None,
    }
}

fn contract_policy_trace_json(
    structured_error: Option<&crate::skills::StructuredSkillError>,
) -> Option<Value> {
    let structured_error = structured_error?;
    if structured_error.error_kind != "contract_action_rejected" {
        return None;
    }
    let extra = structured_error.extra.as_ref()?;
    Some(json!({
        "decision": extra.get("decision").and_then(Value::as_str),
        "action": extra.get("action").and_then(Value::as_str),
        "contract_match": extra.get("contract_match").and_then(Value::as_str),
        "required_evidence": extra.get("required_evidence").cloned(),
        "preferred_actions": extra.get("preferred_actions").cloned(),
        "evidence_expression": extra.get("evidence_expression").cloned(),
        "final_answer_shape": extra.get("final_answer_shape").and_then(Value::as_str),
        "policy_mode": extra.get("policy_mode").and_then(Value::as_str),
        "evidence_scope": extra.get("evidence_scope").and_then(Value::as_str),
        "freshness": extra.get("freshness").and_then(Value::as_str),
        "artifact_kind": extra.get("artifact_kind").and_then(Value::as_str),
        "channel_visibility": extra.get("channel_visibility").and_then(Value::as_str),
    }))
}

/// §3.1: 单条 ask 状态机 transition 的 JSON 序列化。
fn ask_transition_json(t: &crate::AskTransition) -> Value {
    json!({
        "from": t.from.map(crate::AskState::as_str),
        "to": t.to.as_str(),
        "reason": crate::truncate_for_log(&t.reason),
        "at_ms": t.at_ms,
        "round_no": t.round_no,
    })
}

fn task_metrics_json(metrics: &TaskJournalTaskMetrics) -> Value {
    let by_prompt_value = metrics.by_prompt.as_ref().map(|map| {
        let mut entries: Vec<(&String, &crate::LlmPromptBucket)> = map.iter().collect();
        // 按 count 降序输出，方便人眼一眼看到"哪个 prompt 把额度烧光了"。
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
        "by_prompt": by_prompt_value,
    })
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalTaskMetrics {
    pub(crate) used_evidence_ids_count: Option<usize>,
    pub(crate) delivery_consistent: Option<bool>,
    pub(crate) llm_calls_per_task: Option<u64>,
    /// Phase 1.3/1.4: 本次任务期间累计的 LLM 调用耗时（ms），与
    /// `llm_calls_per_task` 一起暴露，方便快速识别"某条任务把
    /// 预算耗在了 LLM 上 vs. 耗在了 skill/runner 上"。
    pub(crate) llm_elapsed_ms_per_task: Option<u64>,
    /// Phase 1.5: per-task 按 prompt label 分桶的 (count, elapsed_ms)。
    /// 取自 [`crate::AppState::task_llm_by_prompt`]。
    /// 用于在 `task_journal_summary.task_metrics.by_prompt` 暴露细分维度。
    pub(crate) by_prompt: Option<std::collections::HashMap<String, crate::LlmPromptBucket>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournal {
    pub(crate) task_id: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) input_text: String,
    pub(crate) context_bundle_summary: Option<String>,
    pub(crate) memory_trace: Option<Value>,
    pub(crate) turn_analysis: Option<crate::intent_router::TurnAnalysis>,
    pub(crate) route_result: Option<crate::RouteResult>,
    pub(crate) plan_result: Option<crate::PlanResult>,
    pub(crate) verify_result: Option<TaskJournalVerifySummary>,
    pub(crate) rounds: Vec<TaskJournalRoundTrace>,
    pub(crate) step_results: Vec<TaskJournalStepTrace>,
    pub(crate) task_observations: Vec<Value>,
    pub(crate) finalizer_summary: Option<TaskJournalFinalizerSummary>,
    pub(crate) answer_verifier_summary: Option<TaskJournalAnswerVerifierSummary>,
    pub(crate) task_metrics: TaskJournalTaskMetrics,
    pub(crate) final_answer: Option<String>,
    pub(crate) final_status: Option<TaskJournalFinalStatus>,
    pub(crate) final_stop_signal: Option<String>,
    pub(crate) final_failure_attribution: Option<String>,
    /// §3.1: ask 状态机 transition 序列。由 `log_ask_transition` 在每次状态切换时
    /// 追加。Stage A 仅占位，Stage B 起 logger 接入，Stage D 进 journal JSON 输出。
    pub(crate) transitions: Vec<crate::AskTransition>,
}

pub(crate) fn stop_signal_failure_attribution(
    stop_signal: &str,
) -> Option<crate::contract_matrix::FailureAttribution> {
    match stop_signal.trim() {
        "recipe_repair_budget_exhausted" | "answer_verifier_retry_exhausted" => {
            Some(crate::contract_matrix::FailureAttribution::BudgetExhausted)
        }
        "prompt_budget_error" => {
            Some(crate::contract_matrix::FailureAttribution::PromptBudgetError)
        }
        _ => None,
    }
}

fn summarize_verify_result(
    verify_result: &crate::verifier::VerifyResult,
) -> TaskJournalVerifySummary {
    TaskJournalVerifySummary {
        mode: verify_result.mode,
        approved: verify_result.approved,
        blocked_reason: verify_result.blocked_reason.clone(),
        shadow_blocked_reason: verify_result.shadow_blocked_reason.clone(),
        needs_confirmation: verify_result.needs_confirmation,
        issues: verify_result
            .issues
            .iter()
            .map(|issue| TaskJournalVerifyIssue {
                step_id: issue.step_id.clone(),
                kind: issue.kind,
                detail: issue.detail.clone(),
            })
            .collect(),
    }
}

#[allow(dead_code)]
impl TaskJournal {
    pub(crate) fn new(input_text: impl Into<String>) -> Self {
        Self {
            input_text: input_text.into(),
            ..Self::default()
        }
    }

    pub(crate) fn for_task(
        task_id: impl Into<String>,
        kind: impl Into<String>,
        input_text: impl Into<String>,
    ) -> Self {
        let mut journal = Self::new(input_text);
        journal.task_id = Some(task_id.into());
        journal.kind = Some(kind.into());
        journal
    }

    pub(crate) fn set_task_identity(
        &mut self,
        task_id: impl Into<String>,
        kind: impl Into<String>,
    ) {
        self.task_id = Some(task_id.into());
        self.kind = Some(kind.into());
    }

    pub(crate) fn record_context_bundle_summary(&mut self, summary: impl Into<String>) {
        self.context_bundle_summary = Some(summary.into());
    }

    pub(crate) fn record_memory_trace(&mut self, trace: Value) {
        self.memory_trace = Some(trace);
    }

    pub(crate) fn record_turn_analysis(
        &mut self,
        turn_analysis: &crate::intent_router::TurnAnalysis,
    ) {
        self.turn_analysis = Some(turn_analysis.clone());
    }

    pub(crate) fn record_route_result(&mut self, route_result: &crate::RouteResult) {
        self.route_result = Some(route_result.clone());
    }

    pub(crate) fn record_plan_result(&mut self, plan_result: &crate::PlanResult) {
        self.plan_result = Some(plan_result.clone());
    }

    pub(crate) fn record_verify_result(&mut self, verify_result: &crate::verifier::VerifyResult) {
        self.verify_result = Some(summarize_verify_result(verify_result));
    }

    pub(crate) fn push_round_trace(
        &mut self,
        round_no: usize,
        goal: impl Into<String>,
        execution_recipe_summary: Option<String>,
        plan_result: &crate::PlanResult,
        verify_result: &crate::verifier::VerifyResult,
    ) {
        self.plan_result = Some(plan_result.clone());
        self.verify_result = Some(summarize_verify_result(verify_result));
        self.rounds.push(TaskJournalRoundTrace {
            round_no,
            goal: goal.into(),
            execution_recipe_summary,
            plan_result: Some(plan_result.clone()),
            verify_result: Some(summarize_verify_result(verify_result)),
        });
    }

    pub(crate) fn push_step_result(&mut self, step_result: &crate::executor::StepExecutionResult) {
        self.step_results.push(TaskJournalStepTrace {
            step_id: step_result.step_id.clone(),
            skill: step_result.skill.clone(),
            status: step_result.status,
            output_excerpt: step_result.output.as_deref().map(crate::truncate_for_log),
            error_excerpt: step_result.error.as_deref().map(crate::truncate_for_log),
            started_at: step_result.started_at,
            finished_at: step_result.finished_at,
        });
    }

    pub(crate) fn push_task_observation(&mut self, observation: Value) {
        self.task_observations.push(observation);
    }

    pub(crate) fn record_finalizer_summary(
        &mut self,
        finalizer_summary: TaskJournalFinalizerSummary,
    ) {
        self.task_metrics.used_evidence_ids_count = Some(finalizer_summary.used_evidence_ids_count);
        self.finalizer_summary = Some(finalizer_summary);
    }

    pub(crate) fn record_answer_verifier_summary(
        &mut self,
        verifier_out: crate::answer_verifier::AnswerVerifierOut,
    ) {
        self.answer_verifier_summary = Some(TaskJournalAnswerVerifierSummary {
            pass: verifier_out.pass,
            missing_evidence_fields: verifier_out.missing_evidence_fields,
            answer_incomplete_reason: verifier_out.answer_incomplete_reason,
            should_retry: verifier_out.should_retry,
            retry_instruction: verifier_out.retry_instruction,
            confidence: verifier_out.confidence,
        });
    }

    pub(crate) fn record_used_evidence_ids_count(&mut self, used_evidence_ids_count: usize) {
        self.task_metrics.used_evidence_ids_count = Some(used_evidence_ids_count);
    }

    pub(crate) fn record_delivery_consistent(&mut self, delivery_consistent: bool) {
        self.task_metrics.delivery_consistent = Some(delivery_consistent);
    }

    pub(crate) fn record_llm_calls_per_task(&mut self, llm_calls_per_task: u64) {
        self.task_metrics.llm_calls_per_task = Some(llm_calls_per_task);
    }

    pub(crate) fn record_llm_elapsed_ms_per_task(&mut self, llm_elapsed_ms_per_task: u64) {
        self.task_metrics.llm_elapsed_ms_per_task = Some(llm_elapsed_ms_per_task);
    }

    /// Phase 1.5: 写入 per-task LLM 调用的 by-prompt 分桶。
    /// 来源是 [`crate::AppState::task_llm_by_prompt`] 在收口时取的快照。
    /// 空 map 也接受（表示这次没产生任何 LLM 调用）。
    pub(crate) fn record_llm_by_prompt(
        &mut self,
        by_prompt: std::collections::HashMap<String, crate::LlmPromptBucket>,
    ) {
        self.task_metrics.by_prompt = Some(by_prompt);
    }

    pub(crate) fn record_final_answer(&mut self, final_answer: impl Into<String>) {
        self.final_answer = Some(final_answer.into());
    }

    pub(crate) fn record_final_status(&mut self, final_status: TaskJournalFinalStatus) {
        self.final_status = Some(final_status);
    }

    pub(crate) fn record_final_stop_signal(&mut self, stop_signal: impl Into<String>) {
        let stop_signal = stop_signal.into();
        self.final_failure_attribution =
            stop_signal_failure_attribution(&stop_signal).map(|kind| kind.as_str().to_string());
        self.final_stop_signal = Some(stop_signal);
    }

    pub(crate) fn record_final_failure_attribution_from_error(&mut self, error_text: &str) {
        if self.final_failure_attribution.is_none() {
            self.final_failure_attribution = failure_attribution_for_error_text(error_text)
                .map(|kind| kind.as_str().to_string());
        }
    }

    pub(crate) fn merge_from(&mut self, other: &TaskJournal) {
        if self.task_id.is_none() {
            self.task_id = other.task_id.clone();
        }
        if self.kind.is_none() {
            self.kind = other.kind.clone();
        }
        if self.input_text.trim().is_empty() {
            self.input_text = other.input_text.clone();
        }
        if self.context_bundle_summary.is_none() {
            self.context_bundle_summary = other.context_bundle_summary.clone();
        }
        if self.memory_trace.is_none() {
            self.memory_trace = other.memory_trace.clone();
        }
        if self.turn_analysis.is_none() {
            self.turn_analysis = other.turn_analysis.clone();
        }
        if self.route_result.is_none() {
            self.route_result = other.route_result.clone();
        }
        if self.plan_result.is_none() {
            self.plan_result = other.plan_result.clone();
        }
        if self.verify_result.is_none() {
            self.verify_result = other.verify_result.clone();
        }
        if self.rounds.is_empty() {
            self.rounds = other.rounds.clone();
        }
        if self.step_results.is_empty() {
            self.step_results = other.step_results.clone();
        }
        if self.task_observations.is_empty() {
            self.task_observations = other.task_observations.clone();
        }
        if self.finalizer_summary.is_none() {
            self.finalizer_summary = other.finalizer_summary.clone();
        }
        if self.answer_verifier_summary.is_none() {
            self.answer_verifier_summary = other.answer_verifier_summary.clone();
        }
        if self.task_metrics.used_evidence_ids_count.is_none() {
            self.task_metrics.used_evidence_ids_count = other.task_metrics.used_evidence_ids_count;
        }
        if self.task_metrics.delivery_consistent.is_none() {
            self.task_metrics.delivery_consistent = other.task_metrics.delivery_consistent;
        }
        if self.task_metrics.llm_calls_per_task.is_none() {
            self.task_metrics.llm_calls_per_task = other.task_metrics.llm_calls_per_task;
        }
        if self.task_metrics.llm_elapsed_ms_per_task.is_none() {
            self.task_metrics.llm_elapsed_ms_per_task = other.task_metrics.llm_elapsed_ms_per_task;
        }
        if self.task_metrics.by_prompt.is_none() {
            self.task_metrics.by_prompt = other.task_metrics.by_prompt.clone();
        }
        if self.final_answer.is_none() {
            self.final_answer = other.final_answer.clone();
        }
        if self.final_status.is_none() {
            self.final_status = other.final_status.clone();
        }
        if self.final_stop_signal.is_none() {
            self.final_stop_signal = other.final_stop_signal.clone();
        }
        if self.final_failure_attribution.is_none() {
            self.final_failure_attribution = other.final_failure_attribution.clone();
        }
    }

    pub(crate) fn attach_to_result(&self, mut result: Value) -> Value {
        let summary = self.to_summary_json();
        let trace = result_trace_json_with_storage_limit(self.to_trace_json());
        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "task_journal".to_string(),
                json!({
                    "summary": summary,
                    "trace": trace,
                }),
            );
            result
        } else {
            json!({
                "result": result,
                "task_journal": {
                    "summary": summary,
                    "trace": trace,
                }
            })
        }
    }

    pub(crate) fn to_summary_json(&self) -> Value {
        json!({
            "task_id": self.task_id.as_deref(),
            "kind": self.kind.as_deref(),
            "round_count": self.rounds.len(),
            "step_count": self.step_results.len(),
            "task_observation_count": self.task_observations.len(),
            "final_status": self.final_status.map(TaskJournalFinalStatus::as_str),
            "final_stop_signal": self.final_stop_signal.as_deref().map(crate::truncate_for_log),
            "final_failure_attribution": self.final_failure_attribution.as_deref(),
            "input_text": crate::truncate_for_log(&self.input_text),
            "context_bundle_summary": self.context_bundle_summary.as_deref().map(crate::truncate_for_log),
            "memory_trace": self.memory_trace.clone(),
            "turn_analysis": self.turn_analysis.as_ref().map(turn_analysis_json),
            "route_result": self.route_result.as_ref().map(route_result_json),
            "latest_execution_recipe_summary": self
                .rounds
                .last()
                .and_then(|round| round.execution_recipe_summary.as_deref())
                .map(crate::truncate_for_log),
            "plan_result": self.plan_result.as_ref().map(plan_summary_json),
            "verify_result": self.verify_result.as_ref().map(verify_summary_json),
            "finalizer_summary": self
                .finalizer_summary
                .as_ref()
                .map(|summary| finalizer_summary_json(summary, self.route_result.as_ref(), self)),
            "answer_verifier_summary": self.answer_verifier_summary.as_ref().map(answer_verifier_summary_json),
            "task_outcome": task_outcome_summary_json(self),
            "task_metrics": task_metrics_json(&self.task_metrics),
            "final_answer": self.final_answer.as_deref().map(crate::truncate_for_log),
        })
    }

    pub(crate) fn to_trace_json(&self) -> Value {
        let mut requested_by_step_id = requested_capability_queues(self);
        json!({
            "task_id": self.task_id.as_deref(),
            "kind": self.kind.as_deref(),
            "final_stop_signal": self.final_stop_signal.as_deref().map(crate::truncate_for_log),
            "final_failure_attribution": self.final_failure_attribution.as_deref(),
            "memory_trace": self.memory_trace.clone(),
            "turn_analysis": self.turn_analysis.as_ref().map(turn_analysis_json),
            "route_result": self.route_result.as_ref().map(route_result_json),
            "contract_matrix": self
                .route_result
                .as_ref()
                .and_then(crate::contract_matrix::trace_snapshot_for_route),
            "runtime_contract_snapshot": self
                .route_result
                .as_ref()
                .and_then(crate::contract_matrix::runtime_contract_snapshot_for_route),
            "evidence_coverage": self
                .route_result
                .as_ref()
                .map(|route| evidence_coverage_trace_json(route, self)),
            "rounds": self.rounds.iter().map(|round| {
                json!({
                    "round_no": round.round_no,
                    "goal": crate::truncate_for_log(&round.goal),
                    "execution_recipe_summary": round
                        .execution_recipe_summary
                        .as_deref()
                        .map(crate::truncate_for_log),
                    "plan_result": round
                        .plan_result
                        .as_ref()
                        .map(|plan| plan_trace_json(plan, self.route_result.as_ref())),
                    "verify_result": round.verify_result.as_ref().map(verify_trace_json),
                })
            }).collect::<Vec<_>>(),
            "step_results": self.step_results.iter().map(|step| {
                let requested = next_requested_capability(&mut requested_by_step_id, &step.step_id);
                step_trace_json(step, requested.as_ref(), self.route_result.as_ref())
            }).collect::<Vec<_>>(),
            "task_observations": self.task_observations.clone(),
            "finalizer_summary": self
                .finalizer_summary
                .as_ref()
                .map(|summary| finalizer_summary_json(summary, self.route_result.as_ref(), self)),
            "answer_verifier_summary": self.answer_verifier_summary.as_ref().map(answer_verifier_summary_json),
            "task_metrics": task_metrics_json(&self.task_metrics),
            "ask_state_transitions": self.transitions.iter().map(ask_transition_json).collect::<Vec<_>>(),
        })
    }

    pub(crate) fn to_log_json(&self) -> Value {
        json!({
            "task_id": self.task_id.as_deref(),
            "kind": self.kind.as_deref(),
            "summary": self.to_summary_json(),
            "trace": self.to_trace_json(),
        })
    }
}

pub(crate) fn delivery_payload_consistent(text: &str, messages: &[String]) -> bool {
    let text = text.trim();
    let last_message = messages.iter().rev().find_map(|message| {
        let trimmed = message.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    if matches!(last_message, Some(message) if message == text) {
        return true;
    }
    let publishable_joined = messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .collect::<Vec<_>>()
        .join("\n\n");
    if !publishable_joined.is_empty() {
        return publishable_joined == text;
    }
    messages.is_empty()
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::{
        delivery_payload_consistent, evidence_coverage_for_route, observed_evidence_from_output,
        TaskJournal, TaskJournalFinalStatus, TaskJournalFinalizerFallback,
        TaskJournalFinalizerStage, TaskJournalFinalizerSummary, TaskJournalRoundTrace,
        TaskJournalVerifyIssue, TaskJournalVerifySummary,
    };

    fn route_for_semantic(semantic_kind: crate::OutputSemanticKind) -> crate::RouteResult {
        crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                semantic_kind,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                ..Default::default()
            },
        }
    }

    #[test]
    fn summary_json_includes_finalizer_and_task_metrics() {
        let mut journal = TaskJournal::for_task("task-1", "ask", "总结 README");
        journal.record_route_result(&crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "不要用现有技能，先规划一个新能力".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "structured self_extension contract".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                self_extension: crate::SelfExtensionContract {
                    mode: crate::SelfExtensionMode::PermanentExtension,
                    trigger: crate::SelfExtensionTrigger::ExplicitUserRequest,
                    execute_now: true,
                },
                ..Default::default()
            },
        });
        journal.record_finalizer_summary(TaskJournalFinalizerSummary {
            stage: Some(TaskJournalFinalizerStage::General),
            disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
            fallback: Some(TaskJournalFinalizerFallback::RawText),
            parsed: false,
            contract_ok: false,
            completion_ok: None,
            grounded_ok: None,
            format_ok: None,
            needs_clarify: None,
            confidence: None,
            used_evidence_ids_count: 2,
            evidence_quotes_count: 0,
        });
        journal.record_delivery_consistent(true);
        journal.record_llm_calls_per_task(3);
        let mut by_prompt = std::collections::HashMap::new();
        by_prompt.insert(
            "normalizer".to_string(),
            crate::LlmPromptBucket {
                count: 1,
                elapsed_ms: 42,
                prompt_truncation_count: 1,
                prompt_bytes_before_max: Some(157_037),
                prompt_bytes_budget_min: Some(125_200),
                prompt_bytes_after_max: Some(125_180),
                prompt_truncated_bytes_total: 31_857,
            },
        );
        journal.record_llm_by_prompt(by_prompt);
        let summary = journal.to_summary_json();

        assert_eq!(
            summary.get("task_id").and_then(Value::as_str),
            Some("task-1")
        );
        assert_eq!(
            summary
                .get("finalizer_summary")
                .and_then(|v| v.get("stage"))
                .and_then(Value::as_str),
            Some("general")
        );
        assert_eq!(
            summary
                .get("finalizer_summary")
                .and_then(|v| v.get("final_answer_shape"))
                .and_then(Value::as_str),
            Some("free")
        );
        assert_eq!(
            summary
                .get("finalizer_summary")
                .and_then(|v| v.get("final_answer_shape_class"))
                .and_then(Value::as_str),
            Some("freeform")
        );
        assert_eq!(
            summary
                .get("finalizer_summary")
                .and_then(|v| v.get("coarse_response_shape"))
                .and_then(Value::as_str),
            Some("free")
        );
        assert_eq!(
            summary
                .get("finalizer_summary")
                .and_then(|v| v.get("allows_model_language"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            summary
                .get("finalizer_summary")
                .and_then(|v| v.get("evidence_coverage_complete"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            summary
                .get("task_metrics")
                .and_then(|v| v.get("used_evidence_ids_count"))
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            summary
                .get("task_metrics")
                .and_then(|v| v.get("delivery_consistent"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            summary
                .get("task_metrics")
                .and_then(|v| v.get("llm_calls_per_task"))
                .and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            summary
                .get("task_metrics")
                .and_then(|v| v.get("prompt_truncation_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            summary
                .get("task_metrics")
                .and_then(|v| v.get("by_prompt"))
                .and_then(|v| v.get("normalizer"))
                .and_then(|v| v.get("prompt_bytes_before_max"))
                .and_then(Value::as_u64),
            Some(157_037)
        );
        assert_eq!(
            summary
                .get("route_result")
                .and_then(|v| v.get("self_extension"))
                .and_then(|v| v.get("mode"))
                .and_then(Value::as_str),
            Some("permanent_extension")
        );
        assert_eq!(
            summary
                .get("route_result")
                .and_then(|v| v.get("self_extension"))
                .and_then(|v| v.get("trigger"))
                .and_then(Value::as_str),
            Some("explicit_user_request")
        );
        assert_eq!(
            summary
                .get("route_result")
                .and_then(|v| v.get("self_extension"))
                .and_then(|v| v.get("execute_now"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn delivery_payload_consistency_uses_last_non_empty_message() {
        assert!(delivery_payload_consistent(
            "最终结果",
            &["".to_string(), "最终结果".to_string()]
        ));
        assert!(!delivery_payload_consistent(
            "最终结果",
            &["中间消息".to_string(), "别的结果".to_string()]
        ));
        assert!(delivery_payload_consistent(
            "第一段\n\n第二段",
            &[
                "**执行过程**\n1. 调用技能 `read_file`".to_string(),
                "第一段".to_string(),
                "第二段".to_string()
            ]
        ));
        assert!(delivery_payload_consistent("任意文本", &[]));
    }

    #[test]
    fn trace_json_includes_execution_recipe_summary() {
        let mut journal = TaskJournal::for_task("task-2", "ask", "修复并验证");
        journal.rounds.push(super::TaskJournalRoundTrace {
            round_no: 1,
            goal: "repair service".to_string(),
            execution_recipe_summary: Some(
                "kind=ops_closed_loop profile=code_change target_scope=external_workspace phase=validate inspect_first=true validation_required=true repair_count=1 max_repairs=3 saw_inspect=true saw_mutation=true saw_validation=false saw_external_target=true saw_greenfield_creation=false".to_string(),
            ),
            ..Default::default()
        });

        let summary = journal.to_summary_json();
        let trace = journal.to_trace_json();

        assert_eq!(
            summary
                .get("latest_execution_recipe_summary")
                .and_then(Value::as_str),
            Some(
                "kind=ops_closed_loop profile=code_change target_scope=external_workspace phase=validate inspect_first=true validation_required=true repair_count=1 max_repairs=3 saw_inspect=true saw_mutation=true saw_validation=false saw_external_target=true saw_greenfield_creation=false"
            )
        );
        assert_eq!(
            trace.get("rounds")
                .and_then(Value::as_array)
                .and_then(|rounds| rounds.first())
                .and_then(|round| round.get("execution_recipe_summary"))
                .and_then(Value::as_str),
            Some(
                "kind=ops_closed_loop profile=code_change target_scope=external_workspace phase=validate inspect_first=true validation_required=true repair_count=1 max_repairs=3 saw_inspect=true saw_mutation=true saw_validation=false saw_external_target=true saw_greenfield_creation=false"
            )
        );
        assert_eq!(trace.get("task_id").and_then(Value::as_str), Some("task-2"));
        assert_eq!(trace.get("kind").and_then(Value::as_str), Some("ask"));
        let log = journal.to_log_json();
        assert_eq!(log.get("task_id").and_then(Value::as_str), Some("task-2"));
        assert_eq!(log.get("kind").and_then(Value::as_str), Some("ask"));
    }

    #[test]
    fn trace_json_includes_memory_trace() {
        let mut journal = TaskJournal::for_task("task-memory", "ask", "根据记忆回复");
        journal.record_memory_trace(json!({
            "stage": "execution",
            "use_policy": "task_relevant",
            "recalled": [
                {
                    "source_kind": "memory_fact",
                    "source_ref": "memory_fact:1",
                    "score": 0.91,
                    "included": true,
                    "reason": "task_relevant"
                }
            ],
            "budget": {
                "max_chars": 4000,
                "used_chars": 128
            }
        }));

        let summary = journal.to_summary_json();
        let trace = journal.to_trace_json();

        assert_eq!(
            summary
                .get("memory_trace")
                .and_then(|v| v.get("use_policy"))
                .and_then(Value::as_str),
            Some("task_relevant")
        );
        assert_eq!(
            trace
                .get("memory_trace")
                .and_then(|v| v.get("recalled"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn attach_to_result_caps_large_trace_and_preserves_contract_summary_fields() {
        let mut journal = TaskJournal::for_task("task-large-trace", "ask", "列出文件名");
        journal.record_route_result(&route_for_semantic(crate::OutputSemanticKind::FileNames));
        for idx in 0..300 {
            journal.push_task_observation(json!({
                "idx": idx,
                "payload": "x".repeat(1200),
                "items": (0..40).map(|value| json!({
                    "value": value,
                    "note": "y".repeat(1200),
                })).collect::<Vec<_>>(),
            }));
        }

        let result = journal.attach_to_result(json!({"text": "ok"}));
        let trace = result
            .pointer("/task_journal/trace")
            .expect("trace should be attached");
        let serialized = serde_json::to_vec(trace).expect("trace should serialize");
        assert!(
            serialized.len() <= super::MAX_RESULT_TRACE_BYTES,
            "stored trace should be bounded, got {} bytes",
            serialized.len()
        );
        assert_eq!(
            trace
                .pointer("/trace_storage/truncated")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(
            trace
                .pointer("/trace_storage/original_bytes")
                .and_then(Value::as_u64)
                .unwrap_or_default()
                > trace
                    .pointer("/trace_storage/stored_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
        );
        assert!(
            trace
                .pointer("/trace_storage/original_hash")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .starts_with("fnv64:")
        );
        assert!(
            trace
                .get("contract_matrix")
                .and_then(|value| value.get("contract_match"))
                .and_then(Value::as_str)
                .is_some(),
            "contract snapshot should survive trace compaction"
        );
        assert!(
            trace.get("evidence_coverage").is_some(),
            "evidence coverage should survive trace compaction"
        );
        assert!(
            trace
                .get("task_observations")
                .and_then(Value::as_array)
                .map(|items| items.len() <= super::MAX_RESULT_TRACE_ARRAY_ITEMS)
                .unwrap_or(false),
            "task observations should be truncated by count"
        );
    }

    #[test]
    fn trace_json_includes_verifier_issue_failure_attribution() {
        let mut journal = TaskJournal::for_task("task-verifier-attribution", "ask", "列文件");
        journal.rounds.push(TaskJournalRoundTrace {
            round_no: 1,
            goal: "list files".to_string(),
            verify_result: Some(TaskJournalVerifySummary {
                mode: crate::verifier::VerifyMode::ObserveOnly,
                approved: true,
                blocked_reason: None,
                shadow_blocked_reason: Some("contract action rejected".to_string()),
                needs_confirmation: false,
                issues: vec![TaskJournalVerifyIssue {
                    step_id: "step_1".to_string(),
                    kind: crate::verifier::VerifyIssueKind::ContractActionRejected,
                    detail: "action rejected".to_string(),
                }],
            }),
            ..Default::default()
        });

        let trace = journal.to_trace_json();
        let issue = trace
            .get("rounds")
            .and_then(Value::as_array)
            .and_then(|rounds| rounds.first())
            .and_then(|round| round.get("verify_result"))
            .and_then(|verify| verify.get("issues"))
            .and_then(Value::as_array)
            .and_then(|issues| issues.first())
            .expect("verify issue should be present");
        assert_eq!(
            issue.get("kind").and_then(Value::as_str),
            Some("ContractActionRejected")
        );
        assert_eq!(
            issue.get("failure_attribution").and_then(Value::as_str),
            Some("contract_gap")
        );
    }

    #[test]
    fn final_stop_signal_records_budget_failure_attribution() {
        let mut journal = TaskJournal::for_task("task-budget", "ask", "继续修复直到通过");
        journal.record_final_status(TaskJournalFinalStatus::Failure);
        journal.record_final_stop_signal("recipe_repair_budget_exhausted");

        let summary = journal.to_summary_json();
        let trace = journal.to_trace_json();

        assert_eq!(
            summary.get("final_stop_signal").and_then(Value::as_str),
            Some("recipe_repair_budget_exhausted")
        );
        assert_eq!(
            summary
                .get("final_failure_attribution")
                .and_then(Value::as_str),
            Some("budget_exhausted")
        );
        assert_eq!(
            trace
                .get("final_failure_attribution")
                .and_then(Value::as_str),
            Some("budget_exhausted")
        );
    }

    #[test]
    fn trace_json_distinguishes_requested_tool_from_executed_skill() {
        let mut journal = TaskJournal::for_task("task-3", "ask", "列出当前目录前三项");
        let plan = crate::PlanResult {
            goal: "list workspace".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            steps: vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "system_basic".to_string(),
                args: json!({"action": "inventory_dir", "path": "."}),
                depends_on: Vec::new(),
                why: "list directory".to_string(),
            }],
            planner_notes: String::new(),
            plan_kind: crate::PlanKind::Single,
            raw_plan_text:
                r#"{"steps":[{"type":"call_tool","tool":"list_dir","args":{"path":".","limit":3}}]}"#
                    .to_string(),
        };
        journal.rounds.push(super::TaskJournalRoundTrace {
            round_no: 1,
            goal: "list workspace".to_string(),
            plan_result: Some(plan),
            ..Default::default()
        });
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("README.md\nCargo.toml".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let trace = journal.to_trace_json();
        let step = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .expect("step result should be present");
        assert_eq!(
            step.get("requested_action_type").and_then(Value::as_str),
            Some("call_tool")
        );
        assert_eq!(
            step.get("requested_capability").and_then(Value::as_str),
            Some("list_dir")
        );
        let plan_action_ref = trace
            .pointer("/rounds/0/plan_result/steps/0/action_ref")
            .and_then(Value::as_str);
        assert_eq!(plan_action_ref, Some("system_basic.inventory_dir"));
        assert_eq!(
            step.get("requested_action_ref").and_then(Value::as_str),
            Some("system_basic.inventory_dir")
        );
        assert_eq!(
            step.get("executed_skill").and_then(Value::as_str),
            Some("system_basic")
        );
        assert_eq!(
            step.get("skill").and_then(Value::as_str),
            Some("system_basic")
        );
    }

    #[test]
    fn trace_json_compacts_plan_action_ref_to_contract_action() {
        let mut journal = TaskJournal::for_task("task-service", "ask", "check service");
        journal.record_route_result(&crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "check clawd service".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Strict,
                requires_content_evidence: true,
                semantic_kind: crate::OutputSemanticKind::ServiceStatus,
                ..Default::default()
            },
        });
        let plan = crate::PlanResult {
            goal: "check service".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            steps: vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "process_basic".to_string(),
                args: json!({"action": "ps", "filter": "clawd"}),
                depends_on: Vec::new(),
                why: "inspect process".to_string(),
            }],
            planner_notes: String::new(),
            plan_kind: crate::PlanKind::Single,
            raw_plan_text: String::new(),
        };
        journal.rounds.push(super::TaskJournalRoundTrace {
            round_no: 1,
            goal: "check service".to_string(),
            plan_result: Some(plan),
            ..Default::default()
        });

        let trace = journal.to_trace_json();
        let plan_action_ref = trace
            .pointer("/rounds/0/plan_result/steps/0/action_ref")
            .and_then(Value::as_str);
        assert_eq!(plan_action_ref, Some("process_basic"));
    }

    #[test]
    fn trace_json_includes_contract_policy_for_contract_rejection() {
        let mut journal = TaskJournal::for_task("task-contract", "ask", "列出文件名");
        let err = crate::skills::structured_skill_error_from_parts(
            "run_cmd",
            "contract_action_rejected",
            "action `run_cmd` is rejected by contract `file_names`",
            None,
            Some(json!({
                "failure_attribution": "contract_gap",
                "decision": "rejected_not_allowed",
                "action": "run_cmd",
                "contract_match": "file_names",
                "required_evidence": ["candidates"],
                "preferred_actions": ["fs_basic.list_dir"],
                "evidence_expression": {"all_of": ["candidates"], "one_of": [], "any_of": [], "negative_evidence": []},
                "final_answer_shape": "name_list",
            })),
        );
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some(err),
            started_at: 1,
            finished_at: 1,
        });

        let trace = journal.to_trace_json();
        let step = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .expect("step result should be present");

        assert_eq!(
            step.get("error_kind").and_then(Value::as_str),
            Some("contract_action_rejected")
        );
        assert_eq!(
            step.get("failure_attribution").and_then(Value::as_str),
            Some("contract_gap")
        );
        assert_eq!(
            step.get("contract_policy")
                .and_then(|value| value.get("decision"))
                .and_then(Value::as_str),
            Some("rejected_not_allowed")
        );
        assert_eq!(
            step.get("contract_policy")
                .and_then(|value| value.get("contract_match"))
                .and_then(Value::as_str),
            Some("file_names")
        );
        assert_eq!(
            step.get("contract_policy")
                .and_then(|value| value.get("evidence_expression"))
                .and_then(|value| value.get("all_of"))
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str),
            Some("candidates")
        );
    }

    #[test]
    fn trace_json_infers_failure_attribution_from_standard_error_kind() {
        for (error_kind, expected) in [
            ("schema_validation_failed", "schema_error"),
            ("provider_retryable_response", "provider_error"),
            ("channel_send_failed", "delivery_error"),
        ] {
            let mut journal = TaskJournal::for_task(
                format!("task-{error_kind}"),
                "ask",
                "trigger structured error",
            );
            let err = crate::skills::structured_skill_error_from_parts(
                "runtime",
                error_kind,
                "structured failure",
                None,
                None,
            );
            journal.push_step_result(&crate::executor::StepExecutionResult {
                step_id: "step_1".to_string(),
                skill: "runtime".to_string(),
                status: crate::executor::StepExecutionStatus::Error,
                output: None,
                error: Some(err),
                started_at: 1,
                finished_at: 1,
            });

            let trace = journal.to_trace_json();
            let step = trace
                .get("step_results")
                .and_then(Value::as_array)
                .and_then(|steps| steps.first())
                .expect("step result should be present");

            assert_eq!(
                step.get("error_kind").and_then(Value::as_str),
                Some(error_kind)
            );
            assert_eq!(
                step.get("failure_attribution").and_then(Value::as_str),
                Some(expected)
            );
        }
    }

    #[test]
    fn final_error_text_records_failure_attribution() {
        for (error_text, expected) in [
            (
                "provider=minimax failed: timeout while reading response",
                "provider_error",
            ),
            (
                "direct_answer_gate schema_validation_failed task_id=t1 err=missing field",
                "schema_error",
            ),
            (
                "wechat send status=500 body={\"err\":\"bad gateway\"}",
                "delivery_error",
            ),
        ] {
            let mut journal =
                TaskJournal::for_task(format!("task-{expected}"), "ask", "trigger final error");
            journal.record_final_failure_attribution_from_error(error_text);
            journal.record_final_status(TaskJournalFinalStatus::Failure);

            assert_eq!(
                journal
                    .to_trace_json()
                    .get("final_failure_attribution")
                    .and_then(Value::as_str),
                Some(expected)
            );
        }
    }

    #[test]
    fn trace_json_includes_redacted_observed_evidence_for_json_output() {
        let mut journal = TaskJournal::for_task("task-observed-evidence", "ask", "读取配置");
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "config_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "action": "read_fields",
                    "count": 2,
                    "extra": {
                        "field_value": "enabled",
                        "api_key": "sk-test-super-secret-token-value-1234567890"
                    }
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let trace = journal.to_trace_json();
        let observed = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .and_then(|step| step.get("observed_evidence"))
            .expect("observed evidence should be present");
        assert_eq!(observed.get("format").and_then(Value::as_str), Some("json"));
        assert_eq!(
            observed.get("storage").and_then(Value::as_str),
            Some("redacted_excerpt_hash")
        );

        let items = observed
            .get("items")
            .and_then(Value::as_array)
            .expect("observed evidence items should be present");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("extra.field_value")
                && item.get("excerpt").and_then(Value::as_str) == Some("enabled")
                && item.get("hash").and_then(Value::as_str).is_some()
        }));
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("extra.api_key")
                && item.get("redacted").and_then(Value::as_bool) == Some(true)
                && item.get("excerpt").is_none()
        }));
        assert!(!serde_json::to_string(observed)
            .expect("serialize observed evidence")
            .contains("sk-test-super-secret-token-value"));
    }

    #[test]
    fn trace_json_includes_observed_evidence_for_text_output() {
        let mut journal = TaskJournal::for_task("task-observed-text", "ask", "运行命令");
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("first line\nsecond line".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let trace = journal.to_trace_json();
        let observed = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .and_then(|step| step.get("observed_evidence"))
            .expect("observed evidence should be present");
        assert_eq!(observed.get("format").and_then(Value::as_str), Some("text"));
        assert!(observed
            .get("items")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("field").and_then(Value::as_str) == Some("text_excerpt")
                        && item.get("excerpt").and_then(Value::as_str)
                            == Some("first line second line")
                        && item.get("hash").and_then(Value::as_str).is_some()
                })
            }));
    }

    #[test]
    fn text_observed_evidence_extracts_count_path_and_candidates() {
        let archive_listing = "exit=0\nArchive: /tmp/test.zip\n  Length Name\n  22 notes.txt\n  20 nested/config.ini\n  42 2 files";
        let observed = observed_evidence_from_output(Some(archive_listing))
            .expect("text evidence should be present");
        let items = observed
            .get("items")
            .and_then(Value::as_array)
            .expect("evidence items");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("count")
                && item.get("excerpt").and_then(Value::as_str) == Some("2")
        }));

        let observed = observed_evidence_from_output(Some("/home/guagua/rustclaw/Cargo.toml"))
            .expect("path evidence should be present");
        let items = observed
            .get("items")
            .and_then(Value::as_array)
            .expect("path evidence items");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("path")
                && item.get("source").and_then(Value::as_str) == Some("text_output.extractor")
        }));
        let observed = observed_evidence_from_output(Some(
            "written 40 bytes to /home/guagua/rustclaw/document/pwd_line.txt",
        ))
        .expect("path token evidence should be present");
        let items = observed
            .get("items")
            .and_then(Value::as_array)
            .expect("path token evidence items");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("path")
                && item.get("excerpt").and_then(Value::as_str)
                    == Some("/home/guagua/rustclaw/document/pwd_line.txt")
        }));
        let observed = observed_evidence_from_output(Some(
            "archive_path=/home/guagua/rustclaw/tmp/bundle.zip\nexit=0\n  adding: /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md (deflated 32%)",
        ))
        .expect("labeled archive path evidence should be present");
        let items = observed
            .get("items")
            .and_then(Value::as_array)
            .expect("labeled path evidence items");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("path")
                && item.get("excerpt").and_then(Value::as_str)
                    == Some("/home/guagua/rustclaw/tmp/bundle.zip")
        }));

        let observed = observed_evidence_from_output(Some(
            "exit=0\n09342a6a fix: expose nl execution and locator flows\n",
        ))
        .expect("git subject evidence should be present");
        let items = observed
            .get("items")
            .and_then(Value::as_array)
            .expect("git subject evidence items");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("subject")
                && item.get("excerpt").and_then(Value::as_str)
                    == Some("fix: expose nl execution and locator flows")
        }));

        let mut journal = TaskJournal::for_task("task-text-candidates", "ask", "列出文件名");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::FileNames,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("Cargo.toml\nREADME.md".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete());
        assert!(coverage.observed_canonical.contains("candidates"));
        assert!(coverage.observed_canonical.contains("count"));

        let observed = observed_evidence_from_output(Some(".git\nREADME.md\n.env\nsrc\n"))
            .expect("hidden list evidence should be present");
        let items = observed
            .get("items")
            .and_then(Value::as_array)
            .expect("hidden list evidence items");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("hidden_count")
                && item.get("excerpt").and_then(Value::as_str) == Some("2")
        }));
    }

    #[test]
    fn docker_unavailable_text_counts_as_field_value_evidence() {
        let mut journal =
            TaskJournal::for_task("task-docker-unavailable", "ask", "检查 Docker 是否可用");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::DockerContainerLifecycle,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "docker_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("docker unavailable: No such file or directory (os error 2)".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));

        assert!(coverage.observed_canonical.contains("command_output"));
    }

    #[test]
    fn docker_success_exit_text_counts_as_field_value_evidence() {
        let mut journal =
            TaskJournal::for_task("task-docker-version", "ask", "检查 Docker 是否可用");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::DockerContainerLifecycle,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "docker_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("exit=0\nClient: Docker Engine".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));

        let trace = journal.to_trace_json();
        let items = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .and_then(|step| step.get("observed_evidence"))
            .and_then(|observed| observed.get("items"))
            .and_then(Value::as_array)
            .expect("observed evidence items should be present");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("exit")
                && item.get("excerpt").and_then(Value::as_str) == Some("0")
        }));
    }

    #[test]
    fn package_manager_key_value_text_counts_as_field_value_evidence() {
        let mut journal = TaskJournal::for_task("task-package-manager", "ask", "检测包管理器");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::PackageManagerDetection,
            locator_kind: crate::OutputLocatorKind::None,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "package_manager".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("package_manager=apt-get".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));

        let observed = observed_evidence_from_output(Some("package_manager=apt-get"))
            .expect("machine key/value evidence should be present");
        let items = observed
            .get("items")
            .and_then(Value::as_array)
            .expect("observed evidence items");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("package_manager")
                && item.get("excerpt").and_then(Value::as_str) == Some("apt-get")
        }));
    }

    #[test]
    fn docker_unavailable_text_counts_as_list_candidate_evidence() {
        let mut journal =
            TaskJournal::for_task("task-docker-images-unavailable", "ask", "列出 Docker 镜像");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::DockerImages,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "docker_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("docker unavailable: No such file or directory (os error 2)".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("candidates"));

        assert!(coverage.observed_canonical.contains("command_output"));
    }

    #[test]
    fn structured_keys_array_counts_as_field_value_evidence() {
        let mut journal = TaskJournal::for_task("task-structured-keys", "ask", "列出配置键");
        let route = route_for_semantic(crate::OutputSemanticKind::StructuredKeys);
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "config_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "action": "structured_keys",
                    "exists": true,
                    "keys": ["app", "features", "paths"],
                    "count": 3
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));
    }

    #[test]
    fn docker_command_not_found_text_counts_as_docker_contract_evidence() {
        for (semantic_kind, expected_canonical) in [
            (
                crate::OutputSemanticKind::DockerContainerLifecycle,
                "field_value",
            ),
            (crate::OutputSemanticKind::DockerLogs, "candidates"),
        ] {
            let mut journal =
                TaskJournal::for_task("task-docker-command-not-found", "ask", "检查 Docker");
            let route = route_for_semantic(semantic_kind);
            journal.record_route_result(&route);
            journal.push_step_result(&crate::executor::StepExecutionResult {
                step_id: "step_1".to_string(),
                skill: "run_cmd".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output: Some("bash: line 1: docker: command not found\n".to_string()),
                error: None,
                started_at: 1,
                finished_at: 2,
            });

            let coverage = evidence_coverage_for_route(&route, &journal);
            assert!(coverage.is_complete(), "coverage: {coverage:?}");
            assert!(coverage.observed_canonical.contains(expected_canonical));
        }
    }

    #[test]
    fn scalar_count_json_value_counts_as_count_evidence() {
        let mut journal = TaskJournal::for_task("task-scalar-count", "ask", "输出数量");
        let route = route_for_semantic(crate::OutputSemanticKind::ScalarCount);
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("3\n".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("count"));
    }

    #[test]
    fn scalar_path_only_results_array_counts_as_field_value_evidence() {
        let mut journal = TaskJournal::for_task("task-scalar-path", "ask", "找到路径");
        let route = route_for_semantic(crate::OutputSemanticKind::ScalarPathOnly);
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "action": "find_name",
                    "count": 1,
                    "results": ["scripts/nl_tests/fixtures/device_local/package.json"]
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));
    }

    #[test]
    fn json_observed_evidence_prioritizes_complete_candidate_names_before_entry_details() {
        let output = r#"{
            "action": "inventory_dir",
            "counts": {"dirs": 1, "files": 2, "hidden": 0, "total": 3},
            "dirs_only": false,
            "entries": [
                {"hidden": false, "kind": "dir", "modified_ts": 1, "name": "archive", "path": "docs/archive", "size_bytes": 0},
                {"hidden": false, "kind": "file", "modified_ts": 1, "name": "release_checklist.md", "path": "docs/release_checklist.md", "size_bytes": 153},
                {"hidden": false, "kind": "file", "modified_ts": 1, "name": "service_notes.md", "path": "docs/service_notes.md", "size_bytes": 272}
            ],
            "names": ["archive", "release_checklist.md", "service_notes.md"],
            "names_by_kind": {
                "dirs": ["archive"],
                "files": ["release_checklist.md", "service_notes.md"],
                "other": []
            }
        }"#;

        let observed = observed_evidence_from_output(Some(output))
            .expect("json output should produce observed evidence");
        assert_eq!(
            observed.get("truncated").and_then(Value::as_bool),
            Some(true)
        );
        let items = observed
            .get("items")
            .and_then(Value::as_array)
            .expect("observed evidence items");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("names[2]")
                && item.get("excerpt").and_then(Value::as_str) == Some("service_notes.md")
        }));
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("names_by_kind.files[1]")
                && item.get("excerpt").and_then(Value::as_str) == Some("service_notes.md")
        }));
    }

    #[test]
    fn service_status_health_check_fields_count_as_field_value_evidence() {
        let mut journal =
            TaskJournal::for_task("task-service-status", "ask", "检查 clawd 服务状态");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::ServiceStatus,
            locator_kind: crate::OutputLocatorKind::None,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "health_check".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(r#"{"clawd_health_port_open":true,"clawd_process_count":1}"#.to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));
    }

    #[test]
    fn git_status_text_counts_as_field_value_evidence() {
        let mut journal = TaskJournal::for_task("task-git-state", "ask", "检查仓库状态");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "git_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                "exit=0\n## main...origin/main\n M crates/clawd/src/task_journal.rs\n".to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));

        let trace = journal.to_trace_json();
        let items = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .and_then(|step| step.get("observed_evidence"))
            .and_then(|observed| observed.get("items"))
            .and_then(Value::as_array)
            .expect("observed evidence items should be present");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("state")
                && item.get("excerpt").and_then(Value::as_str) == Some("dirty")
        }));
    }

    #[test]
    fn git_subject_plain_text_counts_as_field_value_evidence() {
        let mut journal = TaskJournal::for_task("task-git-subject", "ask", "最近一次 git 提交标题");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::GitCommitSubject,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("Harden contract matrix execution coverage\n".to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));
    }

    #[test]
    fn git_status_text_ignores_non_ascii_summary_without_panic() {
        assert_eq!(
            super::text_git_state_evidence(
                "exit=0\n## main...origin/main\n执行 cat /definitely_missing_rustclaw_contract_case 失败\n"
            ),
            Some("clean")
        );
    }

    #[test]
    fn config_validation_evidence_coverage_accepts_valid_flag() {
        let mut journal = TaskJournal::for_task("task-config-validation", "ask", "验证配置");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::ConfigValidation,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::Path,
            locator_hint: "configs/config.toml".to_string(),
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "config_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "action": "validate_structured",
                    "path": "configs/config.toml",
                    "format": "toml",
                    "valid": true,
                    "root_type": "object",
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));
    }

    #[test]
    fn sqlite_database_kind_uses_db_structure_as_field_value_evidence() {
        let mut journal =
            TaskJournal::for_task("task-sqlite-kind", "ask", "判断 sqlite 数据库类型");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::Path,
            locator_hint: "data/test_contract.sqlite".to_string(),
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "db_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "columns": ["name"],
                    "rows": [
                        {"name": "orders"},
                        {"name": "service_logs"},
                        {"name": "users"}
                    ]
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("candidates"));
        assert!(coverage.observed_canonical.contains("field_value"));
    }

    #[test]
    fn quantity_comparison_size_bytes_counts_as_field_value_evidence() {
        let mut journal =
            TaskJournal::for_task("task-quantity-comparison", "ask", "比较两个文件大小");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_kind: crate::OutputLocatorKind::Path,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "action": "path_batch_facts",
                    "facts": [
                        {
                            "path": "release_checklist.md",
                            "exists": true,
                            "fact": {
                                "kind": "file",
                                "size_bytes": 153
                            }
                        },
                        {
                            "path": "package.json",
                            "exists": true,
                            "fact": {
                                "kind": "file",
                                "size_bytes": 246
                            }
                        }
                    ]
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));
        assert!(coverage.observed_canonical.contains("size_bytes"));
    }

    #[test]
    fn quantity_comparison_text_size_bytes_counts_as_field_value_evidence() {
        let mut journal = TaskJournal::for_task(
            "task-quantity-comparison-text",
            "ask",
            "compare two file sizes",
        );
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_kind: crate::OutputLocatorKind::Path,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                "path=release_checklist.md size_bytes=153\npath=package.json size_bytes=246"
                    .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));
        assert!(coverage.observed_canonical.contains("size_bytes"));
    }

    #[test]
    fn trace_json_reports_required_vs_observed_evidence_coverage() {
        let mut journal = TaskJournal::for_task("task-evidence-coverage", "ask", "列出文件名");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::FileNames,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(json!({"names": ["Cargo.toml", "README.md"]}).to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let trace = journal.to_trace_json();
        let coverage = trace
            .get("evidence_coverage")
            .expect("evidence coverage should be present");
        assert_eq!(
            coverage
                .get("required_evidence")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(vec!["candidates"])
        );
        assert_eq!(
            coverage
                .get("missing_evidence")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(Vec::<&str>::new())
        );
        assert!(coverage
            .get("observed_canonical")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("candidates"))));
        let summary = journal.to_summary_json();
        assert_eq!(
            summary
                .get("task_outcome")
                .and_then(|value| value.get("state"))
                .and_then(Value::as_str),
            Some("in_progress")
        );
    }

    #[test]
    fn config_risk_evidence_coverage_accepts_guard_findings() {
        let mut journal = TaskJournal::for_task("task-config-risk-evidence", "ask", "检查配置风险");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::ConfigRiskAssessment,
            locator_kind: crate::OutputLocatorKind::Path,
            locator_hint: "configs/config.toml".to_string(),
            requires_content_evidence: true,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "config_edit".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "action": "guard_config",
                    "format": "toml",
                    "path": "configs/config.toml",
                    "resolved_path": "/home/guagua/rustclaw/configs/config.toml",
                    "risk_count": 2,
                    "risks": [
                        "tools.allow_sudo=true",
                        "tools.allow_path_outside_workspace=true"
                    ]
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        let trace = journal.to_trace_json();
        let items = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .and_then(|step| step.get("observed_evidence"))
            .and_then(|observed| observed.get("items"))
            .and_then(Value::as_array)
            .expect("observed evidence items should be present");

        assert!(coverage.is_complete());
        assert_eq!(coverage.required_evidence, vec!["candidates", "count"]);
        assert!(coverage.observed_canonical.contains("candidates"));
        assert!(coverage.observed_canonical.contains("count"));
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("risks[1]")
                && item.get("excerpt").and_then(Value::as_str)
                    == Some("tools.allow_path_outside_workspace=true")
                && item.get("redacted").is_none()
        }));
    }

    #[test]
    fn evidence_coverage_ignores_failed_and_synthesis_outputs() {
        let mut journal = TaskJournal::for_task(
            "task-evidence-coverage-filter",
            "ask",
            "summarize file content",
        );
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            requires_content_evidence: true,
            semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
            locator_kind: crate::OutputLocatorKind::Path,
            locator_hint: "README.md".to_string(),
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_failed".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: Some(json!({"content": "failed read should not count"}).to_string()),
            error: Some("read failed".to_string()),
            started_at: 1,
            finished_at: 2,
        });
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_synthesis".to_string(),
            skill: "synthesize_answer".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({"content": "model synthesis should not count as observed content"})
                    .to_string(),
            ),
            error: None,
            started_at: 3,
            finished_at: 4,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);

        assert!(!coverage.is_complete());
        assert_eq!(coverage.missing_evidence, vec!["content_excerpt"]);
        assert!(!coverage.observed_canonical.contains("content_excerpt"));
    }

    #[test]
    fn raw_command_output_error_step_supplies_command_output_evidence() {
        let mut journal = TaskJournal::for_task(
            "task-run-cmd-failure-evidence",
            "ask",
            "cat /definitely_missing_rustclaw_contract_case",
        );
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::ExecutionFailedStep,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some(crate::skills::structured_skill_error_from_parts(
                "run_cmd",
                "nonzero_exit",
                "Command failed with exit code 1",
                Some("linux"),
                Some(json!({
                    "command": "cat /definitely_missing_rustclaw_contract_case",
                    "exit_code": 1,
                    "stderr": "cat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)\n",
                    "stdout": Value::Null,
                })),
            )),
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete(), "coverage: {coverage:?}");
        assert!(coverage.observed_canonical.contains("field_value"));

        let trace = journal.to_trace_json();
        let items = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .and_then(|step| step.get("observed_evidence"))
            .and_then(|observed| observed.get("items"))
            .and_then(Value::as_array)
            .expect("observed evidence items should be present");
        assert!(items.iter().any(|item| {
            item.get("field").and_then(Value::as_str) == Some("stderr")
                && item
                    .get("excerpt")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.contains("No such file or directory"))
        }));
    }

    #[test]
    fn summary_json_includes_user_readable_task_outcome() {
        let mut journal = TaskJournal::for_task("task-outcome", "ask", "列出文件名");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::FileNames,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.record_final_status(TaskJournalFinalStatus::Success);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(json!({"names": ["Cargo.toml", "README.md"]}).to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let outcome = journal
            .to_summary_json()
            .get("task_outcome")
            .cloned()
            .expect("task outcome");

        assert_eq!(
            outcome.get("state").and_then(Value::as_str),
            Some("completed")
        );
        assert_eq!(
            outcome.get("final_answer_shape").and_then(Value::as_str),
            Some("name_list")
        );
        assert_eq!(
            outcome
                .get("missing_evidence_count")
                .and_then(Value::as_u64),
            Some(0)
        );
        assert!(outcome.get("message_zh").and_then(Value::as_str).is_some());
        assert!(outcome
            .get("next_step_en")
            .and_then(Value::as_str)
            .is_some());
    }

    #[test]
    fn trace_json_reports_missing_required_evidence() {
        let mut journal = TaskJournal::for_task("task-evidence-missing", "ask", "这个路径是否存在");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
            locator_kind: crate::OutputLocatorKind::Path,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(json!({"path": "/tmp/missing.txt", "exists": false}).to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let trace = journal.to_trace_json();
        let coverage = trace
            .get("evidence_coverage")
            .expect("evidence coverage should be present");
        assert_eq!(
            coverage
                .get("missing_evidence")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(vec!["kind"])
        );
    }

    #[test]
    fn trace_json_uses_evidence_expression_for_confirmed_absence() {
        let mut journal = TaskJournal::for_task("task-evidence-absence", "ask", "这个路径是否存在");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
            locator_kind: crate::OutputLocatorKind::Path,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "path": "/tmp/missing.txt",
                    "exists": false,
                    "kind": "missing"
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete());
        assert!(coverage.observed_canonical.contains("exists_false"));

        let trace = journal.to_trace_json();
        let coverage = trace
            .get("evidence_coverage")
            .expect("evidence coverage should be present");
        assert_eq!(
            coverage
                .get("evidence_expression")
                .and_then(|value| value.get("negative_evidence"))
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(vec!["exists_false"])
        );
        assert_eq!(
            coverage
                .get("missing_evidence")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(Vec::<&str>::new())
        );
    }

    #[test]
    fn trace_json_reports_missing_evidence_expression_alternative() {
        let mut journal =
            TaskJournal::for_task("task-evidence-missing-alt", "ask", "这个路径是否存在");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
            locator_kind: crate::OutputLocatorKind::Path,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(json!({"path": "/tmp/maybe.txt", "kind": "file"}).to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert_eq!(
            coverage.missing_evidence,
            vec!["one_of(exists_false|exists_true)"]
        );
    }

    #[test]
    fn content_presence_accepts_excerpt_evidence_alternative() {
        let mut journal = TaskJournal::for_task(
            "task-content-presence-excerpt",
            "ask",
            "check whether the file mentions release",
        );
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::ContentPresenceCheck,
            locator_kind: crate::OutputLocatorKind::Path,
            requires_content_evidence: true,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "action": "read_range",
                    "path": "/tmp/release_checklist.md",
                    "excerpt": "1|# Release Checklist"
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let coverage = evidence_coverage_for_route(&route, &journal);
        assert!(coverage.is_complete());
        assert!(coverage.observed_canonical.contains("content_excerpt"));
        assert_eq!(
            coverage
                .evidence_expression
                .as_ref()
                .and_then(|value| value.get("any_of"))
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(vec!["content_excerpt", "content_match"])
        );
    }

    #[test]
    fn trace_json_counts_nested_builtin_tool_evidence() {
        let mut journal = TaskJournal::for_task("task-nested-evidence", "ask", "这个路径是否存在");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
            locator_kind: crate::OutputLocatorKind::Path,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "action": "path_batch_facts",
                    "facts": [{
                        "path": "/tmp/present.txt",
                        "exists": true,
                        "kind": "file"
                    }]
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let trace = journal.to_trace_json();
        let coverage = trace
            .get("evidence_coverage")
            .expect("evidence coverage should be present");
        assert_eq!(
            coverage
                .get("missing_evidence")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(Vec::<&str>::new())
        );
        assert!(coverage
            .get("observed_fields")
            .and_then(Value::as_array)
            .is_some_and(|items| items
                .iter()
                .any(|item| item.as_str() == Some("facts[0].path"))));
    }

    #[test]
    fn trace_json_includes_task_level_contract_matrix_snapshot() {
        let mut journal = TaskJournal::for_task("task-contract-snapshot", "ask", "列出文件名");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::FileNames,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);

        let trace = journal.to_trace_json();
        let snapshot = trace
            .get("contract_matrix")
            .expect("contract matrix snapshot should be present");

        assert_eq!(
            snapshot.get("contract_match").and_then(Value::as_str),
            Some("file_names")
        );
        assert_eq!(
            snapshot
                .get("required_evidence")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(vec!["candidates"])
        );
        assert_eq!(
            snapshot.get("final_answer_shape").and_then(Value::as_str),
            Some("name_list")
        );
        assert!(snapshot
            .get("contract_matrix_hash")
            .and_then(Value::as_str)
            .is_some_and(|hash| !hash.is_empty()));
        let runtime_snapshot = trace
            .get("runtime_contract_snapshot")
            .expect("runtime contract snapshot should be present");
        assert_eq!(
            runtime_snapshot
                .get("contract")
                .and_then(|value| value.get("contract_match"))
                .and_then(Value::as_str),
            Some("file_names")
        );
        assert!(runtime_snapshot
            .get("compact_contract_block")
            .and_then(|value| value.get("hash"))
            .and_then(Value::as_str)
            .is_some_and(|hash| !hash.is_empty()));
    }

    #[test]
    fn step_trace_includes_contract_and_action_policy_for_success() {
        let mut journal = TaskJournal::for_task("task-step-contract", "ask", "列出文件名");
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract = crate::IntentOutputContract {
            semantic_kind: crate::OutputSemanticKind::FileNames,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        journal.record_route_result(&route);
        journal.record_plan_result(&crate::PlanResult {
            goal: "list file names".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            steps: vec![crate::PlanStep {
                step_id: "step_1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({"action": "list_dir", "path": "."}),
                depends_on: Vec::new(),
                why: String::new(),
            }],
            planner_notes: String::new(),
            plan_kind: crate::PlanKind::Single,
            raw_plan_text: String::new(),
        });
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(json!({"items": [{"path": "README.md"}]}).to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let trace = journal.to_trace_json();
        let step_contract = trace
            .pointer("/step_results/0/contract")
            .expect("step contract trace should be present");

        assert_eq!(
            step_contract.get("contract_match").and_then(Value::as_str),
            Some("file_names")
        );
        assert_eq!(
            step_contract
                .get("final_answer_shape")
                .and_then(Value::as_str),
            Some("name_list")
        );
        assert_eq!(
            step_contract
                .get("action_policy")
                .and_then(|value| value.get("decision"))
                .and_then(Value::as_str),
            Some("allowed")
        );
        assert_eq!(
            step_contract
                .get("action_policy")
                .and_then(|value| value.get("action_ref"))
                .and_then(Value::as_str),
            Some("fs_basic.list_dir")
        );
        assert!(trace
            .pointer("/step_results/0/observed_evidence/items")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()));
    }
}
