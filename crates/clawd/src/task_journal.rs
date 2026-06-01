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
    "sort_by",
    "candidates",
    "names",
    "entries",
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
    pub(crate) fn new(
        step_id: impl Into<String>,
        skill: impl Into<String>,
        status: crate::executor::StepExecutionStatus,
        output_excerpt: Option<String>,
        error_excerpt: Option<String>,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            skill: skill.into(),
            status,
            output_excerpt,
            error_excerpt,
            started_at: 0,
            finished_at: 0,
        }
    }

    pub(crate) fn ok(
        step_id: impl Into<String>,
        skill: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self::new(
            step_id,
            skill,
            crate::executor::StepExecutionStatus::Ok,
            Some(output.into()),
            None,
        )
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
        !self.pass
            && (self.confidence >= 0.55
                || (self.should_retry
                    && (!self.answer_incomplete_reason.trim().is_empty()
                        || !self.missing_evidence_fields.is_empty())))
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

fn plan_step_raw_action_ref(step: &crate::PlanStep) -> Option<String> {
    crate::contract_matrix::ActionRef::from_skill_args(&step.skill, &step.args)
        .map(|action| action.as_key())
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
            let raw_action_ref = plan_step_raw_action_ref(step);
            let matrix_action_ref = plan_step_action_ref(step, route);
            json!({
                "step_id": &step.step_id,
                "action_type": &step.action_type,
                "skill": &step.skill,
                "action_ref": matrix_action_ref.clone(),
                "matrix_action_ref": matrix_action_ref,
                "raw_action_ref": raw_action_ref,
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
    observed_evidence_from_step_output(step)
        .or_else(|| observed_evidence_from_error(step.error_excerpt.as_deref()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvidenceObservationSource {
    StepOutput,
    StepError,
}

impl EvidenceObservationSource {
    fn as_str(self) -> &'static str {
        match self {
            EvidenceObservationSource::StepOutput => "step_output",
            EvidenceObservationSource::StepError => "step_error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvidenceExtractorKind {
    StructuredJson,
    TextLegacy,
}

impl EvidenceExtractorKind {
    fn as_str(self) -> &'static str {
        match self {
            EvidenceExtractorKind::StructuredJson => "structured_json",
            EvidenceExtractorKind::TextLegacy => "text_legacy",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct EvidenceExtractorSpec {
    observation_source: EvidenceObservationSource,
    extractor_ref: &'static str,
    kind: EvidenceExtractorKind,
    format: &'static str,
    schema_version: u64,
    source_action_ref: Option<&'static str>,
    provided_evidence: &'static [&'static str],
    strict_shape_eligible: bool,
    fallback: bool,
}

impl EvidenceExtractorSpec {
    fn to_trace_json(self) -> Value {
        json!({
            "schema_version": self.schema_version,
            "extractor_ref": self.extractor_ref,
            "kind": self.kind.as_str(),
            "observation_source": self.observation_source.as_str(),
            "format": self.format,
            "source_action_ref": self.source_action_ref,
            "provided_evidence": self.provided_evidence,
            "strict_shape_eligible": self.strict_shape_eligible,
            "fallback": self.fallback,
            "provider_safety": extractor_provider_safety_trace_json(),
        })
    }
}

fn extractor_provider_safety_trace_json() -> Value {
    json!({
        "provider_evidence_view": "provider_safe_redacted",
        "raw_excerpt_policy": "no_full_raw_excerpt",
        "storage": "redacted_excerpt_hash",
        "sensitive_field_policy": "redact_sensitive_keys_and_secret_like_values",
    })
}

const EVIDENCE_EXTRACTOR_REGISTRY: &[EvidenceExtractorSpec] = &[
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "step_output.structured_json_v1",
        kind: EvidenceExtractorKind::StructuredJson,
        format: "json",
        schema_version: 1,
        source_action_ref: None,
        provided_evidence: &["generic_json_fields"],
        strict_shape_eligible: false,
        fallback: true,
    },
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "step_output.text_legacy_v1",
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: None,
        provided_evidence: &["legacy_text_excerpt"],
        strict_shape_eligible: false,
        fallback: true,
    },
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepError,
        extractor_ref: "step_error.structured_json_v1",
        kind: EvidenceExtractorKind::StructuredJson,
        format: "json",
        schema_version: 1,
        source_action_ref: None,
        provided_evidence: &["error_text", "error_kind", "generic_json_fields"],
        strict_shape_eligible: false,
        fallback: true,
    },
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepError,
        extractor_ref: "step_error.text_legacy_v1",
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: None,
        provided_evidence: &["legacy_error_excerpt"],
        strict_shape_eligible: false,
        fallback: true,
    },
];

const EXPLICIT_EVIDENCE_EXTRACTOR_REGISTRY: &[EvidenceExtractorSpec] = &[
    step_json_extractor(
        "fs_basic",
        "fs_basic.structured_json_v1",
        &[
            "candidates",
            "count",
            "exists",
            "field_value",
            "kind",
            "modified_ts",
            "path",
            "size_bytes",
            "sort_by",
        ],
    ),
    step_json_extractor(
        "fs_basic.stat_paths",
        "fs_basic.stat_paths.structured_json_v1",
        &[
            "exists",
            "field_value",
            "kind",
            "modified_ts",
            "path",
            "size_bytes",
        ],
    ),
    step_json_extractor(
        "fs_basic.compare_paths",
        "fs_basic.compare_paths.structured_json_v1",
        &["field_value", "modified_ts", "path", "size_bytes"],
    ),
    step_json_extractor(
        "fs_basic.list_dir",
        "fs_basic.list_dir.structured_json_v1",
        &[
            "candidates",
            "count",
            "field_value",
            "kind",
            "modified_ts",
            "path",
            "size_bytes",
            "sort_by",
        ],
    ),
    step_json_extractor(
        "fs_basic.find_entries",
        "fs_basic.find_entries.structured_json_v1",
        &["candidates", "count", "path"],
    ),
    step_json_extractor(
        "fs_basic.count_entries",
        "fs_basic.count_entries.structured_json_v1",
        &["count", "field_value", "size_bytes"],
    ),
    step_json_extractor(
        "system_basic.tree_summary",
        "system_basic.tree_summary.structured_json_v1",
        &["candidates", "count", "kind", "path", "size_bytes"],
    ),
    step_json_extractor(
        "system_basic.inventory_dir",
        "system_basic.inventory_dir.structured_json_v1",
        &[
            "candidates",
            "count",
            "field_value",
            "kind",
            "modified_ts",
            "path",
            "size_bytes",
            "sort_by",
        ],
    ),
    step_json_extractor(
        "system_basic.read_range",
        "system_basic.read_range.structured_json_v1",
        &["content_excerpt", "path"],
    ),
    step_json_extractor(
        "system_basic.extract_field",
        "system_basic.extract_field.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "system_basic.extract_fields",
        "system_basic.extract_fields.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "system_basic.path_batch_facts",
        "system_basic.path_batch_facts.structured_json_v1",
        &[
            "candidates",
            "count",
            "exists",
            "kind",
            "path",
            "size_bytes",
        ],
    ),
    step_json_extractor(
        "system_basic.info",
        "system_basic.info.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "system_basic.runtime_status",
        "system_basic.runtime_status.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "fs_basic.grep_text",
        "fs_basic.grep_text.structured_json_v1",
        &["content_excerpt", "content_match", "path"],
    ),
    step_json_extractor(
        "fs_basic.read_text_range",
        "fs_basic.read_text_range.structured_json_v1",
        &["content_excerpt", "path"],
    ),
    step_json_extractor(
        "fs_basic.write_text",
        "fs_basic.write_text.structured_json_v1",
        &["path"],
    ),
    step_json_extractor(
        "fs_basic.make_dir",
        "fs_basic.make_dir.structured_json_v1",
        &["path"],
    ),
    step_json_extractor(
        "config_basic.read_field",
        "config_basic.read_field.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "config_basic.read_fields",
        "config_basic.read_fields.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "config_basic.list_keys",
        "config_basic.list_keys.structured_json_v1",
        &["count", "field_value"],
    ),
    step_json_extractor(
        "config_basic.validate",
        "config_basic.validate.structured_json_v1",
        &["field_value", "valid"],
    ),
    step_json_extractor(
        "config_basic.guard_rustclaw_config",
        "config_basic.guard_rustclaw_config.structured_json_v1",
        &["candidates", "count", "valid"],
    ),
    step_json_extractor(
        "config_basic",
        "config_basic.structured_json_v1",
        &["count", "field_value", "valid"],
    ),
    step_json_extractor(
        "config_edit.guard_config",
        "config_edit.guard_config.structured_json_v1",
        &["candidates", "count", "valid"],
    ),
    step_json_extractor(
        "config_edit.plan_config_change",
        "config_edit.plan_config_change.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "config_edit.apply_config_change",
        "config_edit.apply_config_change.structured_json_v1",
        &["field_value", "path", "valid"],
    ),
    step_json_extractor(
        "config_edit.validate_config",
        "config_edit.validate_config.structured_json_v1",
        &["field_value", "valid"],
    ),
    step_json_extractor(
        "config_edit.read_back",
        "config_edit.read_back.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "config_edit.restart_if_requested",
        "config_edit.restart_if_requested.structured_json_v1",
        &["field_value"],
    ),
    step_json_extractor(
        "config_guard",
        "config_guard.structured_json_v1",
        &["candidates", "count", "valid"],
    ),
    step_json_extractor(
        "db_basic",
        "db_basic.structured_json_v1",
        &["candidates", "count", "field_value"],
    ),
    step_json_extractor(
        "doc_parse",
        "doc_parse.structured_json_v1",
        &["content_excerpt", "path"],
    ),
    step_json_extractor(
        "git_basic",
        "git_basic.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "health_check",
        "health_check.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "log_analyze",
        "log_analyze.structured_json_v1",
        &["field_value", "content_excerpt"],
    ),
    step_json_extractor(
        "package_manager.detect",
        "package_manager.detect.structured_json_v1",
        &["field_value"],
    ),
    step_json_extractor(
        "process_basic",
        "process_basic.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "docker_basic",
        "docker_basic.structured_json_v1",
        &["candidates", "field_value", "status"],
    ),
    step_json_extractor(
        "service_control",
        "service_control.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor("transform", "transform.structured_json_v1", &["path"]),
    step_json_extractor(
        "audio_synthesize",
        "audio_synthesize.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "rss_fetch",
        "rss_fetch.structured_json_v1",
        &["candidates", "content_excerpt", "field_value"],
    ),
    step_json_extractor("x", "x.structured_json_v1", &["field_value"]),
    step_json_extractor(
        "image_generate",
        "image_generate.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "image_edit",
        "image_edit.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "archive_basic",
        "archive_basic.structured_json_v1",
        &["candidates", "content_excerpt", "count", "path"],
    ),
    step_json_extractor(
        "archive_basic.list",
        "archive_basic.list.structured_json_v1",
        &["candidates", "count", "path"],
    ),
    step_json_extractor(
        "archive_basic.read",
        "archive_basic.read.structured_json_v1",
        &["content_excerpt", "path"],
    ),
    step_json_extractor(
        "browser_web",
        "browser_web.structured_json_v1",
        &["content_excerpt", "field_value", "path"],
    ),
    step_json_extractor(
        "browser_web.open_extract",
        "browser_web.open_extract.structured_json_v1",
        &["content_excerpt", "field_value", "path"],
    ),
    step_json_extractor(
        "web_search_extract",
        "web_search_extract.structured_json_v1",
        &["candidates", "field_value"],
    ),
    step_json_extractor(
        "web_search_extract.search",
        "web_search_extract.search.structured_json_v1",
        &["candidates", "field_value"],
    ),
    step_json_extractor(
        "web_search_extract.search_extract",
        "web_search_extract.search_extract.structured_json_v1",
        &["candidates", "field_value"],
    ),
    step_json_extractor(
        "weather",
        "weather.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "weather.query",
        "weather.query.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "stock",
        "stock.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "stock.quote",
        "stock.quote.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "crypto",
        "crypto.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "crypto.quote",
        "crypto.quote.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "crypto.multi_quote",
        "crypto.multi_quote.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision",
        "image_vision.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.describe",
        "image_vision.describe.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.analyze",
        "image_vision.analyze.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.extract",
        "image_vision.extract.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.compare",
        "image_vision.compare.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.screenshot_summary",
        "image_vision.screenshot_summary.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "archive_basic.pack",
        "archive_basic.pack.structured_json_v1",
        &["path"],
    ),
    step_json_extractor(
        "archive_basic.unpack",
        "archive_basic.unpack.structured_json_v1",
        &["path"],
    ),
    step_text_extractor(
        "archive_basic",
        "archive_basic.text_legacy_v1",
        &["candidates", "count", "legacy_machine_tokens", "path"],
    ),
    step_text_extractor(
        "git_basic",
        "git_basic.text_legacy_v1",
        &["field_value", "legacy_machine_tokens", "subject"],
    ),
    step_text_extractor(
        "http_basic",
        "http_basic.text_legacy_v1",
        &["command_output", "content_excerpt", "field_value", "status"],
    ),
    step_text_extractor(
        "write_file",
        "write_file.text_legacy_v1",
        &["legacy_machine_tokens", "path"],
    ),
    step_text_extractor(
        "x",
        "x.text_legacy_v1",
        &["field_value", "legacy_machine_tokens"],
    ),
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "run_cmd.text_legacy_v1",
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: Some("run_cmd"),
        provided_evidence: &["command_output", "legacy_machine_tokens"],
        strict_shape_eligible: true,
        fallback: false,
    },
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "list_dir.text_legacy_v1",
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: Some("list_dir"),
        provided_evidence: &["candidates", "count", "legacy_machine_tokens"],
        strict_shape_eligible: true,
        fallback: false,
    },
];

const MATRIX_ADMITTED_EXTERNAL_STRUCTURED_JSON_EXTRACTOR: EvidenceExtractorSpec =
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "matrix_admitted_external.structured_json_v1",
        kind: EvidenceExtractorKind::StructuredJson,
        format: "json",
        schema_version: 1,
        source_action_ref: Some("matrix_admitted_external"),
        provided_evidence: &["admitted_extra_fields"],
        strict_shape_eligible: true,
        fallback: false,
    };

const fn step_json_extractor(
    source_action_ref: &'static str,
    extractor_ref: &'static str,
    provided_evidence: &'static [&'static str],
) -> EvidenceExtractorSpec {
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref,
        kind: EvidenceExtractorKind::StructuredJson,
        format: "json",
        schema_version: 1,
        source_action_ref: Some(source_action_ref),
        provided_evidence,
        strict_shape_eligible: true,
        fallback: false,
    }
}

const fn step_text_extractor(
    source_action_ref: &'static str,
    extractor_ref: &'static str,
    provided_evidence: &'static [&'static str],
) -> EvidenceExtractorSpec {
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref,
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: Some(source_action_ref),
        provided_evidence,
        strict_shape_eligible: true,
        fallback: false,
    }
}

fn evidence_extractor_spec(
    observation_source: EvidenceObservationSource,
    kind: EvidenceExtractorKind,
) -> EvidenceExtractorSpec {
    EVIDENCE_EXTRACTOR_REGISTRY
        .iter()
        .copied()
        .find(|spec| spec.observation_source == observation_source && spec.kind == kind)
        .expect("evidence extractor registry contains all built-in extractor specs")
}

pub(crate) fn evidence_extractor_registry_trace(
    source_action_ref: &str,
    extractor_kind: &str,
) -> Option<Value> {
    explicit_evidence_extractor_spec(source_action_ref, extractor_kind).map(|spec| {
        json!({
            "extractor_ref": spec.extractor_ref,
            "source_action_ref": spec.source_action_ref,
            "provided_evidence": spec.provided_evidence,
            "strict_shape_eligible": spec.strict_shape_eligible,
            "fallback": spec.fallback,
            "provider_safety": extractor_provider_safety_trace_json(),
        })
    })
}

pub(crate) fn evidence_extractor_registry_contains(
    source_action_ref: &str,
    extractor_kind: &str,
) -> bool {
    explicit_evidence_extractor_spec(source_action_ref, extractor_kind).is_some()
}

fn explicit_evidence_extractor_spec(
    source_action_ref: &str,
    extractor_kind: &str,
) -> Option<EvidenceExtractorSpec> {
    let source_action_ref = normalize_source_action_ref(source_action_ref)?;
    let kind = parse_evidence_extractor_kind(extractor_kind)?;
    EXPLICIT_EVIDENCE_EXTRACTOR_REGISTRY
        .iter()
        .copied()
        .find(|spec| {
            spec.kind == kind
                && spec
                    .source_action_ref
                    .is_some_and(|value| value == source_action_ref)
        })
}

fn parse_evidence_extractor_kind(extractor_kind: &str) -> Option<EvidenceExtractorKind> {
    match normalize_machine_token(extractor_kind).as_str() {
        "structured_json" => Some(EvidenceExtractorKind::StructuredJson),
        "text_legacy" => Some(EvidenceExtractorKind::TextLegacy),
        _ => None,
    }
}

pub(crate) fn observed_evidence_from_output(output: Option<&str>) -> Option<Value> {
    let output = output.map(str::trim).filter(|value| !value.is_empty())?;
    let (collector, extractor) = collect_observed_evidence_from_output(output);
    observed_evidence_from_collector(collector, extractor)
}

fn observed_evidence_from_step_output(step: &TaskJournalStepTrace) -> Option<Value> {
    let output = step
        .output_excerpt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let fallback_extractor = match serde_json::from_str::<Value>(output) {
        Ok(value) => {
            let mut collector = ObservedEvidenceCollector::default();
            collect_json_observed_evidence(&mut collector, "json_output", "", &value, 0);
            let fallback_extractor = evidence_extractor_spec(
                EvidenceObservationSource::StepOutput,
                EvidenceExtractorKind::StructuredJson,
            );
            let extractor =
                explicit_step_output_extractor_spec(step, output, fallback_extractor.kind)
                    .unwrap_or(fallback_extractor);
            return observed_evidence_from_collector(collector, extractor);
        }
        Err(_) => evidence_extractor_spec(
            EvidenceObservationSource::StepOutput,
            EvidenceExtractorKind::TextLegacy,
        ),
    };
    let extractor = explicit_step_output_extractor_spec(step, output, fallback_extractor.kind)
        .unwrap_or(fallback_extractor);
    let mut collector = ObservedEvidenceCollector::default();
    collect_text_observed_evidence_for_extractor(&mut collector, output, extractor);
    observed_evidence_from_collector(collector, extractor)
}

fn collect_observed_evidence_from_output(
    output: &str,
) -> (ObservedEvidenceCollector, EvidenceExtractorSpec) {
    let mut collector = ObservedEvidenceCollector::default();
    let extractor = match serde_json::from_str::<Value>(output) {
        Ok(value) => {
            collect_json_observed_evidence(&mut collector, "json_output", "", &value, 0);
            evidence_extractor_spec(
                EvidenceObservationSource::StepOutput,
                EvidenceExtractorKind::StructuredJson,
            )
        }
        Err(_) => {
            collect_text_observed_evidence(&mut collector, output);
            evidence_extractor_spec(
                EvidenceObservationSource::StepOutput,
                EvidenceExtractorKind::TextLegacy,
            )
        }
    };
    (collector, extractor)
}

fn observed_evidence_from_collector(
    collector: ObservedEvidenceCollector,
    extractor: EvidenceExtractorSpec,
) -> Option<Value> {
    if collector.items.is_empty() {
        return None;
    }
    let item_count = collector.total_count;
    Some(json!({
        "schema_version": 1,
        "source": "step_output",
        "format": extractor.format,
        "extractor": extractor.to_trace_json(),
        "storage": "redacted_excerpt_hash",
        "item_count": item_count,
        "truncated": item_count > collector.items.len(),
        "items": collector.items,
    }))
}

fn explicit_step_output_extractor_spec(
    step: &TaskJournalStepTrace,
    output: &str,
    kind: EvidenceExtractorKind,
) -> Option<EvidenceExtractorSpec> {
    step_output_source_action_refs(step, output)
        .into_iter()
        .find_map(|source_action_ref| {
            EXPLICIT_EVIDENCE_EXTRACTOR_REGISTRY
                .iter()
                .copied()
                .find(|spec| {
                    spec.kind == kind
                        && spec
                            .source_action_ref
                            .is_some_and(|value| value == source_action_ref)
                })
        })
        .or_else(|| matrix_admitted_external_extractor_spec(output, kind))
}

fn matrix_admitted_external_extractor_spec(
    output: &str,
    kind: EvidenceExtractorKind,
) -> Option<EvidenceExtractorSpec> {
    if kind != EvidenceExtractorKind::StructuredJson {
        return None;
    }
    let value = serde_json::from_str::<Value>(output).ok()?;
    let admission = value.get("_matrix_admission")?;
    if !admission
        .get("eligible")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let extractor_kind = admission
        .get("extractor_kind")
        .and_then(Value::as_str)
        .map(normalize_machine_token)
        .unwrap_or_else(|| "structured_json".to_string());
    if extractor_kind != kind.as_str() {
        return None;
    }
    Some(MATRIX_ADMITTED_EXTERNAL_STRUCTURED_JSON_EXTRACTOR)
}

fn step_output_source_action_refs(step: &TaskJournalStepTrace, output: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let skill = normalize_machine_token(&step.skill).replace('-', "_");
    if skill.is_empty() {
        return refs;
    }
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        push_source_action_ref(&mut refs, &skill, Some(&value));
        if skill == "system_basic"
            && value.get("action").and_then(Value::as_str).is_none()
            && value_looks_like_system_basic_info(&value)
        {
            push_unique_source_action_ref(&mut refs, "system_basic.info".to_string());
        }
        push_canonical_source_action_ref(&mut refs, &skill, value.clone());
        if skill == "fs_basic" {
            push_canonical_source_action_ref(&mut refs, "fs_search", value.clone());
        }
        if matches!(skill.as_str(), "fs_basic" | "config_basic" | "system_basic") {
            push_canonical_source_action_ref(&mut refs, "system_basic", value);
        }
    }
    push_source_action_ref(&mut refs, &skill, None);
    refs
}

fn value_looks_like_system_basic_info(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.contains_key("cwd")
        || object.contains_key("workspace_root")
        || (object.contains_key("hostname")
            && (object.contains_key("os") || object.contains_key("arch")))
}

fn push_source_action_ref(refs: &mut Vec<String>, skill: &str, value: Option<&Value>) {
    let source = match value
        .and_then(|value| value.get("action"))
        .and_then(Value::as_str)
    {
        Some(action) => format!(
            "{skill}.{}",
            normalize_machine_token(action).replace('-', "_")
        ),
        None => skill.to_string(),
    };
    if let Some(source) = normalize_source_action_ref(&source) {
        push_unique_source_action_ref(refs, source);
    }
}

fn push_canonical_source_action_ref(refs: &mut Vec<String>, skill: &str, value: Value) {
    let Some(canonical) = crate::virtual_tools::canonicalize_legacy_tool_call(skill, value) else {
        return;
    };
    let Some(source) = canonical_source_action_ref(&canonical.tool, &canonical.args) else {
        return;
    };
    push_unique_source_action_ref(refs, source);
}

fn canonical_source_action_ref(skill: &str, args: &Value) -> Option<String> {
    let skill = normalize_machine_token(skill).replace('-', "_");
    if skill.is_empty() {
        return None;
    }
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_machine_token)
        .map(|value| value.replace('-', "_"))
        .filter(|value| !value.is_empty());
    normalize_source_action_ref(&match action {
        Some(action) => format!("{skill}.{action}"),
        None => skill,
    })
}

fn normalize_source_action_ref(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (skill, action) = raw
        .split_once('.')
        .map_or((raw, None), |(skill, action)| (skill, Some(action)));
    let skill = normalize_machine_token(skill).replace('-', "_");
    if skill.is_empty() {
        return None;
    }
    let action = action
        .map(normalize_machine_token)
        .map(|value| value.replace('-', "_"))
        .filter(|value| !value.is_empty());
    Some(match action {
        Some(action) => format!("{skill}.{action}"),
        None => skill,
    })
}

fn push_unique_source_action_ref(refs: &mut Vec<String>, source: String) {
    if !refs.iter().any(|value| value == &source) {
        refs.push(source);
    }
}

fn normalize_machine_token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn observed_evidence_from_error(error: Option<&str>) -> Option<Value> {
    let error = error.map(str::trim).filter(|value| !value.is_empty())?;
    let mut collector = ObservedEvidenceCollector::default();
    let extractor = if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
        collector.push(json_observed_evidence_item(
            "structured_error",
            "error_text",
            &json!(structured.error_text),
        ));
        if let Some(extra) = structured.extra.as_ref() {
            collect_json_observed_evidence(&mut collector, "structured_error.extra", "", extra, 0);
        }
        evidence_extractor_spec(
            EvidenceObservationSource::StepError,
            EvidenceExtractorKind::StructuredJson,
        )
    } else {
        collect_text_observed_evidence(&mut collector, error);
        evidence_extractor_spec(
            EvidenceObservationSource::StepError,
            EvidenceExtractorKind::TextLegacy,
        )
    };
    if collector.items.is_empty() {
        return None;
    }
    let item_count = collector.total_count;
    Some(json!({
        "schema_version": 1,
        "source": "step_error",
        "format": extractor.format,
        "extractor": extractor.to_trace_json(),
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
                        if *key == "entries" {
                            collector.push(json_observed_evidence_item(source, key, child));
                        } else {
                            collect_json_object_child(collector, source, depth, prefix, key, child);
                            emitted_priority_keys.insert((*key).to_string());
                        }
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
    if key == "_matrix_admission" {
        return;
    }
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
            let sample_keys = items
                .iter()
                .filter_map(Value::as_object)
                .flat_map(|map| map.keys())
                .take(MAX_OBSERVED_EVIDENCE_KEYS)
                .collect::<BTreeSet<_>>();
            if !sample_keys.is_empty() {
                item.insert(
                    "sample_keys".to_string(),
                    json!(sample_keys.into_iter().collect::<Vec<_>>()),
                );
            }
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
    collect_text_observed_evidence_fields(collector, output);
}

fn collect_text_observed_evidence_for_extractor(
    collector: &mut ObservedEvidenceCollector,
    output: &str,
    extractor: EvidenceExtractorSpec,
) {
    collector.push(text_observed_evidence_item(output));
    collect_text_observed_evidence_fields(collector, output);
    if extractor.extractor_ref == "git_basic.text_legacy_v1" {
        collect_git_text_observed_evidence_fields(collector, output);
    }
}

fn collect_text_observed_evidence_fields(collector: &mut ObservedEvidenceCollector, output: &str) {
    if let Some(count) = text_count_evidence(output) {
        collector.push(json_observed_evidence_item(
            "text_output.extractor",
            "count",
            &json!(count),
        ));
    }
    if let Some(path) = text_path_evidence(output) {
        collector.push(text_extracted_evidence_item("path", &path));
    }
    collect_text_machine_key_value_evidence(collector, output);
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

fn collect_git_text_observed_evidence_fields(
    collector: &mut ObservedEvidenceCollector,
    output: &str,
) {
    if let Some(subject) = text_git_oneline_subject_evidence(output) {
        collector.push(text_extracted_evidence_item("subject", &subject));
    }
    if let Some(state) = text_git_state_evidence(output) {
        collector.push(text_extracted_evidence_item("state", state));
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
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
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
        let meta =
            result_trace_storage_meta(original_bytes, original_bytes, original_hash, &stats, false);
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
    let meta = result_trace_storage_meta(original_bytes, stored_bytes, original_hash, &stats, true);
    insert_result_trace_storage_meta(&mut trace, meta);
    trace
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TaskJournalEvidenceCoverage {
    pub(crate) required_evidence: Vec<String>,
    pub(crate) evidence_expression: Option<Value>,
    pub(crate) observed_fields: BTreeSet<String>,
    pub(crate) observed_canonical: BTreeSet<String>,
    pub(crate) observed_extractors: BTreeSet<String>,
    pub(crate) observed_evidence_sources: BTreeMap<String, BTreeSet<String>>,
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
            "observed_extractors": self.observed_extractors.iter().take(64).cloned().collect::<Vec<_>>(),
            "observed_evidence_sources": observed_evidence_sources_trace_json(&self.observed_evidence_sources),
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
    let (observed_fields, mut observed_canonical, observed_extractors, observed_evidence_sources) =
        observed_evidence_field_sets(journal);
    augment_route_canonical_evidence(route, &observed_fields, &mut observed_canonical);
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
        observed_extractors,
        observed_evidence_sources,
        missing_evidence,
    }
}

fn observed_evidence_sources_trace_json(
    observed_evidence_sources: &BTreeMap<String, BTreeSet<String>>,
) -> Value {
    Value::Object(
        observed_evidence_sources
            .iter()
            .take(64)
            .map(|(field, extractors)| {
                (
                    field.clone(),
                    json!(extractors.iter().take(16).cloned().collect::<Vec<_>>()),
                )
            })
            .collect(),
    )
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
) -> (
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeMap<String, BTreeSet<String>>,
) {
    let mut observed_fields = BTreeSet::new();
    let mut observed_canonical = BTreeSet::new();
    let mut observed_extractors = BTreeSet::new();
    let mut observed_evidence_sources = BTreeMap::<String, BTreeSet<String>>::new();
    for step in &journal.step_results {
        if !step_can_supply_contract_evidence(step, journal.route_result.as_ref()) {
            continue;
        }
        let Some(evidence) = observed_evidence_for_step_trace(step) else {
            continue;
        };
        let extractor_ref = evidence
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(extractor_ref) = extractor_ref.as_ref() {
            observed_extractors.insert(extractor_ref.clone());
        }
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
            observed_fields.insert(normalized.clone());
            let canonical_fields = canonical_evidence_fields_for_observed_item(&normalized, item);
            if let Some(extractor_ref) = extractor_ref.as_ref() {
                observed_evidence_sources
                    .entry(normalized.clone())
                    .or_default()
                    .insert(extractor_ref.clone());
            }
            for canonical in canonical_fields {
                if let Some(extractor_ref) = extractor_ref.as_ref() {
                    observed_evidence_sources
                        .entry(canonical.clone())
                        .or_default()
                        .insert(extractor_ref.clone());
                }
                observed_canonical.insert(canonical);
            }
        }
    }
    (
        observed_fields,
        observed_canonical,
        observed_extractors,
        observed_evidence_sources,
    )
}

fn augment_route_canonical_evidence(
    route: &crate::RouteResult,
    observed_fields: &BTreeSet<String>,
    observed_canonical: &mut BTreeSet<String>,
) {
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && observed_canonical.contains("size_bytes")
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
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
        && (observed_canonical.contains("status")
            || observed_canonical.contains("command_output")
            || observed_canonical.contains("content_excerpt")
            || observed_fields.contains("text_excerpt"))
    {
        observed_canonical.insert("field_value".to_string());
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::PublishingPreview
        && (observed_canonical.contains("command_output")
            || observed_canonical.contains("content_excerpt")
            || observed_fields.contains("text_excerpt"))
    {
        observed_canonical.insert("field_value".to_string());
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
    if route.is_some_and(|route| {
        !route.output_contract.requires_content_evidence
            && !route.output_contract.delivery_required
            && !route.wants_file_delivery
    }) && step_reads_text_content(step)
    {
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

pub(crate) fn step_reads_text_content(step: &TaskJournalStepTrace) -> bool {
    match step.skill.as_str() {
        "read_file" | "doc_parse" => return true,
        _ => {}
    }
    let Some(output) = step.output_excerpt.as_deref().map(str::trim) else {
        return false;
    };
    if output.is_empty() {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return false;
    };
    let action = value
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_evidence_field);
    match step.skill.as_str() {
        "fs_basic" | "system_basic" => matches!(
            action.as_deref(),
            Some("read_range" | "read_text_range" | "read_file" | "read")
        ),
        "archive_basic" => matches!(action.as_deref(), Some("read")),
        _ => false,
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
                "children",
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
                "recent_matches",
                "recent_notable_lines",
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
                "cwd",
                "workspace_root",
            ][..],
        ),
        (
            "field_value",
            &[
                "field_value",
                "new_value",
                "old_value",
                "value_text",
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
                "keyword_counts",
                "level_counts",
                "hostname",
                "os",
                "arch",
                "cwd",
                "workspace_root",
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
            &[
                "size_bytes",
                "total_size_bytes",
                "bytes",
                "file_size",
                "size",
            ][..],
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
    if let Some(keys) = item.get("keys").and_then(Value::as_array) {
        for key in keys.iter().filter_map(Value::as_str) {
            values.extend(canonical_evidence_fields_for_observed_field(
                &normalize_evidence_field(key),
            ));
        }
    }
    if let Some(sample_keys) = item.get("sample_keys").and_then(Value::as_array) {
        for key in sample_keys.iter().filter_map(Value::as_str) {
            values.extend(canonical_evidence_fields_for_observed_field(
                &normalize_evidence_field(key),
            ));
        }
    }
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
#[path = "task_journal_tests.rs"]
mod tests;
