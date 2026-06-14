use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

#[path = "task_journal_decision_envelope.rs"]
mod decision_envelope;
use self::decision_envelope::{
    agent_action_capability_delta, agent_loop_decision_envelope_json,
    agent_loop_round_plan_decision_envelope_json, first_layer_agent_decision_delta,
    first_non_think_action_capability_ref, first_non_think_action_decision,
    output_contract_ref_for_route,
};

#[path = "task_journal_evidence_collect.rs"]
mod task_journal_evidence_collect;
#[path = "task_journal_evidence_coverage.rs"]
mod task_journal_evidence_coverage;
#[path = "task_journal_evidence_registry.rs"]
mod task_journal_evidence_registry;
#[path = "task_journal_trace_storage.rs"]
mod task_journal_trace_storage;

use task_journal_evidence_collect::*;
use task_journal_evidence_coverage::*;
pub(crate) use task_journal_evidence_coverage::{
    evidence_coverage_for_route, failure_attribution_for_error_text, step_reads_text_content,
    TaskJournalEvidenceCoverage,
};
use task_journal_evidence_registry::*;
pub(crate) use task_journal_evidence_registry::{
    evidence_extractor_registry_contains, evidence_extractor_registry_trace,
    observed_evidence_for_step_trace, observed_evidence_from_output,
};
use task_journal_trace_storage::*;

const MAX_OBSERVED_EVIDENCE_ITEMS: usize = 24;
const MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS: usize = 240;
const MAX_OBSERVED_EVIDENCE_KEYS: usize = 16;
const MAX_OBSERVED_EVIDENCE_DEPTH: usize = 3;
const MAX_OBSERVED_MULTILINE_EXCERPT_LINES: usize = 12;
const MAX_OBSERVED_ARRAY_SAMPLES: usize = 3;
const MAX_OBSERVED_ARRAY_VALUE_SAMPLES: usize = 48;
const MAX_RESULT_TRACE_BYTES: usize = 128 * 1024;
const MAX_RESULT_TRACE_ARRAY_ITEMS: usize = 24;
const MAX_RESULT_TRACE_STRING_CHARS: usize = 768;
const MAX_RESULT_TRACE_COMPACT_ARRAY_ITEMS: usize = 8;
const MAX_RESULT_TRACE_COMPACT_STRING_CHARS: usize = 240;
const JSON_EVIDENCE_PRIORITY_KEYS: &[&str] = &[
    "title",
    "content_excerpt",
    "excerpt",
    "text",
    "summary",
    "snippet",
    "field_value",
    "path",
    "resolved_path",
    "metadata",
    "sort_by",
    "clawd_process_count",
    "telegramd_process_count",
    "clawd_health_port_open",
    "clawd_log",
    "telegramd_log",
    "listener_count",
    "public_listener_count",
    "localhost_listener_count",
    "public_ports",
    "ports",
    "public_listeners",
    "listeners",
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

    pub(crate) fn required_evidence_failure_payload_text(&self) -> String {
        json!({
            "schema_version": 1,
            "message_key": "answer_verifier_required_evidence_block",
            "reason_code": "answer_verifier_required_evidence_block",
            "missing_evidence_fields": &self.missing_evidence_fields,
            "answer_incomplete_reason": &self.answer_incomplete_reason,
            "confidence": self.confidence,
        })
        .to_string()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TaskJournalRolloutAttribution {
    pub(crate) switch_name: String,
    pub(crate) event: String,
    pub(crate) outcome: String,
    pub(crate) reason_code: Option<String>,
    pub(crate) owner_layer: Option<String>,
    pub(crate) decision: Option<String>,
    pub(crate) skill: Option<String>,
    pub(crate) action: Option<String>,
    pub(crate) capability_ref: Option<String>,
    pub(crate) dedup_scope: Option<String>,
    pub(crate) fingerprint: Option<String>,
    pub(crate) repeat_count: Option<usize>,
    pub(crate) limit: Option<usize>,
    pub(crate) failure_attribution: Option<String>,
    pub(crate) missing_slots: Vec<String>,
    pub(crate) required_evidence: Vec<String>,
    pub(crate) missing_evidence_fields: Vec<String>,
    pub(crate) confidence: Option<f64>,
    pub(crate) risk_level: Option<String>,
    pub(crate) output_contract_ref: Option<String>,
    pub(crate) old_first_layer_decision: Option<String>,
    pub(crate) agent_decision: Option<String>,
    pub(crate) decision_delta: Option<String>,
    pub(crate) route_layer_that_disagreed: Option<String>,
    pub(crate) old_required_evidence: Vec<String>,
    pub(crate) agent_required_evidence: Vec<String>,
    pub(crate) capability_delta: Option<String>,
    pub(crate) risk_delta: Option<String>,
    pub(crate) output_contract_delta: Option<String>,
    pub(crate) final_outcome: Option<String>,
    pub(crate) verifier_pass: Option<bool>,
    pub(crate) llm_call_count: Option<u64>,
    pub(crate) tool_call_count: Option<u64>,
    pub(crate) external_tool_call_count: Option<u64>,
    pub(crate) latency_ms: Option<u64>,
    pub(crate) budget_profile: Option<String>,
    pub(crate) boundary_context: Option<Value>,
    pub(crate) decision_envelope: Option<Value>,
}

impl TaskJournalRolloutAttribution {
    pub(crate) fn answer_verifier_required_evidence_block(
        summary: Option<&TaskJournalAnswerVerifierSummary>,
    ) -> Self {
        Self {
            switch_name: "answer_verifier_enforce_required".to_string(),
            event: "answer_verifier_required_evidence_block".to_string(),
            outcome: "blocked".to_string(),
            reason_code: Some("answer_verifier_required_evidence_block".to_string()),
            owner_layer: Some("answer_verifier".to_string()),
            decision: Some("blocked".to_string()),
            failure_attribution: Some(
                crate::contract_matrix::FailureAttribution::ContractGap
                    .as_str()
                    .to_string(),
            ),
            missing_evidence_fields: summary
                .map(|summary| summary.missing_evidence_fields.clone())
                .unwrap_or_default(),
            confidence: summary.map(|summary| summary.confidence),
            ..Self::default()
        }
    }

    pub(crate) fn registry_idempotency_guard_block(
        reason_code: impl Into<String>,
        skill: impl Into<String>,
        action: Option<String>,
        dedup_scope: impl Into<String>,
        fingerprint: impl Into<String>,
        repeat_count: Option<usize>,
        limit: Option<usize>,
    ) -> Self {
        Self {
            switch_name: "registry_idempotency_guard".to_string(),
            event: "registry_idempotency_guard_block".to_string(),
            outcome: "blocked".to_string(),
            reason_code: Some(reason_code.into()),
            owner_layer: Some("execution_guard".to_string()),
            decision: Some("blocked".to_string()),
            skill: Some(skill.into()),
            action,
            dedup_scope: Some(dedup_scope.into()),
            fingerprint: Some(fingerprint.into()),
            repeat_count,
            limit,
            ..Self::default()
        }
    }

    pub(crate) fn document_heading_answer_verifier_recovery(
        summary: Option<&TaskJournalAnswerVerifierSummary>,
    ) -> Self {
        Self {
            switch_name: "deterministic_answer_recovery".to_string(),
            event: "document_heading_answer_verifier_recovery".to_string(),
            outcome: "recovered".to_string(),
            reason_code: Some(
                "document_heading_recovered_from_observed_markdown_heading".to_string(),
            ),
            owner_layer: Some("answer_verifier_recovery".to_string()),
            decision: Some("recovered".to_string()),
            missing_evidence_fields: summary
                .map(|summary| summary.missing_evidence_fields.clone())
                .unwrap_or_default(),
            confidence: summary.map(|summary| summary.confidence),
            final_outcome: Some("success".to_string()),
            ..Self::default()
        }
    }

    pub(crate) fn agent_decides_shadow_snapshot(
        route: &crate::RouteResult,
        budget_profile: impl Into<String>,
        boundary_context: Option<Value>,
    ) -> Self {
        let contract = crate::TaskContract::from_route_result(route);
        let required_evidence = contract.required_evidence_fields.clone();
        Self {
            switch_name: "agent_decides_semantic_route".to_string(),
            event: "agent_decides_shadow_snapshot".to_string(),
            outcome: "shadow_only".to_string(),
            reason_code: Some("agent_decides_shadow_not_evaluated".to_string()),
            owner_layer: Some("agent_loop_shadow".to_string()),
            decision: Some(route.first_layer_decision().as_str().to_string()),
            old_first_layer_decision: Some(route.first_layer_decision().as_str().to_string()),
            agent_decision: Some("not_evaluated".to_string()),
            decision_delta: Some("not_evaluated".to_string()),
            missing_slots: contract.missing_parameters,
            required_evidence: required_evidence.clone(),
            risk_level: Some(route.risk_ceiling.as_str().to_string()),
            output_contract_ref: Some(output_contract_ref_for_route(route)),
            old_required_evidence: required_evidence,
            agent_required_evidence: Vec::new(),
            capability_delta: Some("not_evaluated".to_string()),
            risk_delta: Some("not_evaluated".to_string()),
            output_contract_delta: Some("not_evaluated".to_string()),
            budget_profile: Some(budget_profile.into()),
            boundary_context,
            ..Self::default()
        }
    }

    pub(crate) fn agent_decides_shadow_first_action(
        route: &crate::RouteResult,
        budget_profile: impl Into<String>,
        actions: &[crate::AgentAction],
        boundary_context: Option<Value>,
    ) -> Self {
        let contract = crate::TaskContract::from_route_result(route);
        let agent_decision = first_non_think_action_decision(actions);
        let decision_delta =
            first_layer_agent_decision_delta(route.first_layer_decision(), agent_decision);
        let route_layer_that_disagreed =
            (decision_delta == "different_gate").then(|| "first_layer_vs_agent_loop".to_string());
        let required_evidence = contract.required_evidence_fields.clone();
        let output_contract_ref = output_contract_ref_for_route(route);
        Self {
            switch_name: "agent_decides_semantic_route".to_string(),
            event: "agent_decides_shadow_first_action".to_string(),
            outcome: "shadow_only".to_string(),
            reason_code: Some("agent_decides_shadow_delta_observed".to_string()),
            owner_layer: Some("agent_loop_shadow".to_string()),
            decision: Some(agent_decision.to_string()),
            capability_ref: first_non_think_action_capability_ref(actions).map(str::to_string),
            old_first_layer_decision: Some(route.first_layer_decision().as_str().to_string()),
            agent_decision: Some(agent_decision.to_string()),
            decision_delta: Some(decision_delta.to_string()),
            route_layer_that_disagreed,
            missing_slots: contract.missing_parameters,
            required_evidence: required_evidence.clone(),
            risk_level: Some(route.risk_ceiling.as_str().to_string()),
            output_contract_ref: Some(output_contract_ref.clone()),
            old_required_evidence: required_evidence.clone(),
            agent_required_evidence: required_evidence,
            capability_delta: Some(agent_action_capability_delta(actions).to_string()),
            risk_delta: Some("not_evaluated".to_string()),
            output_contract_delta: Some("not_evaluated".to_string()),
            budget_profile: Some(budget_profile.into()),
            boundary_context,
            decision_envelope: Some(agent_loop_decision_envelope_json(
                route,
                actions,
                &output_contract_ref,
                "planner_first_action_shadow",
                "planner_loop_shadow",
            )),
            ..Self::default()
        }
    }
}

fn rollout_attribution_json(attribution: &TaskJournalRolloutAttribution) -> Value {
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

fn verify_summary_json(verify: &TaskJournalVerifySummary) -> Value {
    let first_issue_reason_code = verify.issues.first().map(|issue| issue.kind.reason_code());
    json!({
        "approved": verify.approved,
        "mode": verify.mode.as_str(),
        "owner_layer": "plan_verifier",
        "blocked_reason": verify.blocked_reason.as_deref().map(crate::truncate_for_log),
        "blocked_reason_code": first_issue_reason_code,
        "shadow_blocked_reason": verify.shadow_blocked_reason.as_deref().map(crate::truncate_for_log),
        "needs_confirmation": verify.needs_confirmation,
        "issue_count": verify.issues.len(),
    })
}

fn verify_trace_json(verify: &TaskJournalVerifySummary) -> Value {
    let first_issue_reason_code = verify.issues.first().map(|issue| issue.kind.reason_code());
    json!({
        "approved": verify.approved,
        "mode": verify.mode.as_str(),
        "owner_layer": "plan_verifier",
        "blocked_reason": verify.blocked_reason.as_deref().map(crate::truncate_for_log),
        "blocked_reason_code": first_issue_reason_code,
        "shadow_blocked_reason": verify.shadow_blocked_reason.as_deref().map(crate::truncate_for_log),
        "needs_confirmation": verify.needs_confirmation,
        "issues": verify.issues.iter().map(|issue| {
            json!({
                "step_id": &issue.step_id,
                "kind": issue.kind.as_str(),
                "reason_code": issue.kind.reason_code(),
                "owner_layer": "plan_verifier",
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
            "structured_field_selector": route
                .output_contract
                .self_extension
                .structured_field_selector
                .as_deref(),
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
                        "provider_attempt_count": bucket.provider_attempt_count,
                        "provider_retry_count": bucket.provider_retry_count,
                        "provider_retryable_error_count": bucket.provider_retryable_error_count,
                        "provider_final_error_count": bucket.provider_final_error_count,
                        "provider_last_retry_error_kinds": bucket.provider_last_retry_error_kinds,
                        "provider_final_error_kinds": bucket.provider_final_error_kinds,
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
    pub(crate) rollout_switches_enabled: Vec<String>,
    pub(crate) rollout_attribution: Vec<TaskJournalRolloutAttribution>,
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

    pub(crate) fn record_rollout_switches_enabled<I, S>(&mut self, switches: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut switches = switches
            .into_iter()
            .map(Into::into)
            .map(|value: String| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        switches.sort();
        switches.dedup();
        self.rollout_switches_enabled = switches;
    }

    pub(crate) fn record_rollout_attribution(
        &mut self,
        attribution: TaskJournalRolloutAttribution,
    ) {
        if attribution.switch_name.trim().is_empty()
            || attribution.event.trim().is_empty()
            || attribution.outcome.trim().is_empty()
        {
            return;
        }
        if self.rollout_attribution.iter().any(|existing| {
            existing.switch_name == attribution.switch_name
                && existing.event == attribution.event
                && existing.reason_code == attribution.reason_code
                && existing.skill == attribution.skill
                && existing.action == attribution.action
                && existing.fingerprint == attribution.fingerprint
        }) {
            return;
        }
        self.rollout_attribution.push(attribution);
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
        if self.rollout_switches_enabled.is_empty() {
            self.rollout_switches_enabled = other.rollout_switches_enabled.clone();
        }
        for attribution in &other.rollout_attribution {
            self.record_rollout_attribution(attribution.clone());
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
            "rollout_switches_enabled": self.rollout_switches_enabled.clone(),
            "rollout_attribution": self
                .rollout_attribution
                .iter()
                .map(rollout_attribution_json)
                .collect::<Vec<_>>(),
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
            "rollout_switches_enabled": self.rollout_switches_enabled.clone(),
            "rollout_attribution": self
                .rollout_attribution
                .iter()
                .map(rollout_attribution_json)
                .collect::<Vec<_>>(),
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
                    "decision_envelope": self.route_result.as_ref().and_then(|route| {
                        round
                            .plan_result
                            .as_ref()
                            .map(|plan| agent_loop_round_plan_decision_envelope_json(route, plan))
                    }),
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
#[path = "task_journal_decision_envelope_tests.rs"]
mod decision_envelope_tests;

#[cfg(test)]
#[path = "task_journal_recent_artifacts_tests.rs"]
mod recent_artifacts_tests;

#[cfg(test)]
#[path = "task_journal_tests.rs"]
mod tests;
