use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

#[path = "task_journal_decision_envelope.rs"]
mod decision_envelope;
use self::decision_envelope::agent_loop_round_plan_contract_envelope_json;

#[path = "task_journal_coding_state.rs"]
mod task_journal_coding_state;
#[path = "task_journal_coding_workflow.rs"]
mod task_journal_coding_workflow;
#[path = "task_journal_context_budget.rs"]
mod task_journal_context_budget;
#[path = "task_journal_context_compaction.rs"]
mod task_journal_context_compaction;
#[path = "task_journal_context_summary_parse.rs"]
mod task_journal_context_summary_parse;
#[path = "task_journal_event_stream.rs"]
mod task_journal_event_stream;
#[path = "task_journal_evidence_collect.rs"]
mod task_journal_evidence_collect;
#[path = "task_journal_evidence_coverage.rs"]
mod task_journal_evidence_coverage;
#[path = "task_journal_evidence_registry.rs"]
mod task_journal_evidence_registry;
#[path = "task_journal_goal.rs"]
mod task_journal_goal;
#[path = "task_journal_rollout_attribution.rs"]
mod task_journal_rollout_attribution;
#[path = "task_journal/summary_trace.rs"]
mod task_journal_summary_trace;
#[path = "task_journal_trace_storage.rs"]
mod task_journal_trace_storage;
#[path = "task_journal_validation_result.rs"]
mod task_journal_validation_result;

use task_journal_coding_state::{
    coding_milestone_checkpoint_observation, coding_state_transition_observation,
};
use task_journal_coding_workflow::coding_workflow_summary_json;
use task_journal_event_stream::task_event_stream_json;
use task_journal_evidence_collect::*;
use task_journal_evidence_coverage::*;
pub(crate) use task_journal_evidence_coverage::{
    evidence_coverage_for_output_contract, failure_attribution_for_error_text,
    step_reads_text_content, TaskJournalEvidenceCoverage,
};
use task_journal_evidence_registry::*;
pub(crate) use task_journal_evidence_registry::{
    evidence_extractor_registry_contains, evidence_extractor_registry_trace,
    observed_evidence_for_step_trace, observed_evidence_from_output,
};
use task_journal_goal::task_goal_summary_json;
use task_journal_summary_trace::{
    answer_verifier_summary_json, ask_transition_json, boundary_context_summary_json,
    budget_profile_json, capability_resolution_source, cost_budget_json, finalizer_summary_json,
    next_requested_capability, output_contract_json, plan_summary_json, plan_trace_json,
    raw_plan_steps, requested_capability_sequence, rollout_attribution_json,
    round_capability_resolution_records_json, step_action_kind, step_trace_json, task_metrics_json,
    turn_analysis_json, verify_repair_signals_json, verify_summary_json, verify_trace_json,
    RequestedPlanCapability,
};
use task_journal_trace_storage::*;
use task_journal_validation_result::validation_result_json;

const MAX_OBSERVED_EVIDENCE_ITEMS: usize = 40;
const MAX_OBSERVED_EVIDENCE_EXCERPT_CHARS: usize = 240;
const MAX_OBSERVED_EVIDENCE_KEYS: usize = 16;
const MAX_OBSERVED_EVIDENCE_DEPTH: usize = 3;
const MAX_OBSERVED_MULTILINE_EXCERPT_LINES: usize = 12;
const MAX_OBSERVED_ARRAY_SAMPLES: usize = 3;
const MAX_OBSERVED_ARRAY_VALUE_SAMPLES: usize = 48;

pub(crate) fn context_compaction_record_observation(record: Value) -> Value {
    task_journal_context_compaction::record_observation(record)
}

pub(crate) fn agent_loop_round_plan_contract_envelope(plan: &crate::PlanResult) -> Value {
    agent_loop_round_plan_contract_envelope_json(plan)
}
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
    "extra",
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
    "nni_log",
    "nni_server_log",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskJournalFinalizerStage {
    ObservedGeneric,
}

impl TaskJournalFinalizerStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ObservedGeneric => "observed_generic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskJournalFinalizerFallback {
    FreeformText,
}

impl TaskJournalFinalizerFallback {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FreeformText => "freeform_text",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalVerifyIssue {
    pub(crate) step_id: String,
    pub(crate) kind: crate::verifier::VerifyIssueKind,
    pub(crate) detail: String,
    pub(crate) missing_fields: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalVerifySummary {
    pub(crate) mode: crate::verifier::VerifyMode,
    pub(crate) approved: bool,
    pub(crate) blocked_reason: Option<String>,
    pub(crate) shadow_blocked_reason: Option<String>,
    pub(crate) permission_decision: Value,
    pub(crate) needs_confirmation: bool,
    pub(crate) issues: Vec<TaskJournalVerifyIssue>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalRoundTrace {
    pub(crate) round_no: usize,
    pub(crate) goal: String,
    pub(crate) execution_recipe_summary: Option<String>,
    pub(crate) plan_result: Option<crate::PlanResult>,
    pub(crate) verify_result: Option<TaskJournalVerifySummary>,
}

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

fn step_output_excerpt_for_journal(output: &str) -> String {
    compact_workspace_patch_output_for_journal(output)
        .or_else(|| compact_structured_action_output_for_journal(output))
        .or_else(|| compact_structured_listing_output_for_journal(output))
        .unwrap_or_else(|| crate::truncate_for_log(output))
}

fn compact_workspace_patch_output_for_journal(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
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
    let mut compact = serde_json::Map::new();
    for field in [
        "schema_version",
        "source",
        "status",
        "action",
        "message_key",
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
        "artifact_refs",
        "diff_available",
    ] {
        copy_listing_field(source, &mut compact, field);
    }
    if let Some(files) = source.get("files").and_then(Value::as_array) {
        compact.insert(
            "files".to_string(),
            Value::Array(
                files
                    .iter()
                    .take(128)
                    .filter_map(compact_workspace_patch_file)
                    .collect(),
            ),
        );
    }
    for field in ["before", "after"] {
        if let Some(entries) = source.get(field).and_then(Value::as_array) {
            compact.insert(
                field.to_string(),
                Value::Array(
                    entries
                        .iter()
                        .take(128)
                        .filter_map(compact_workspace_snapshot_entry)
                        .collect(),
                ),
            );
        }
    }
    serde_json::to_string(&json!({ "extra": Value::Object(compact) })).ok()
}

fn compact_workspace_patch_file(file: &Value) -> Option<Value> {
    let path = file.get("path").and_then(Value::as_str)?.trim();
    if path.is_empty() {
        return None;
    }
    let mut compact = serde_json::Map::new();
    compact.insert("path".to_string(), json!(path));
    for field in [
        "existed",
        "before_sha256",
        "after_sha256",
        "additions",
        "deletions",
    ] {
        copy_listing_field(file, &mut compact, field);
    }
    Some(Value::Object(compact))
}

fn compact_workspace_snapshot_entry(entry: &Value) -> Option<Value> {
    let path = entry.get("path").and_then(Value::as_str)?.trim();
    if path.is_empty() {
        return None;
    }
    let mut compact = serde_json::Map::new();
    compact.insert("path".to_string(), json!(path));
    for field in ["kind", "sha256", "size_bytes"] {
        copy_listing_field(entry, &mut compact, field);
    }
    Some(Value::Object(compact))
}

fn compact_structured_action_output_for_journal(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    let source = value
        .get("extra")
        .filter(|extra| extra.is_object())
        .unwrap_or(&value);
    let action = source.get("action").and_then(Value::as_str)?;
    if !matches!(
        action,
        "make_dir" | "write_text" | "append_text" | "read_range" | "read_text_range" | "grep_text"
    ) {
        return None;
    }
    let mut compact = serde_json::Map::new();
    copy_listing_field(source, &mut compact, "action");
    copy_listing_field(source, &mut compact, "path");
    copy_listing_field(source, &mut compact, "resolved_path");
    copy_listing_field(source, &mut compact, "effective_path");
    copy_listing_field(source, &mut compact, "append");
    copy_listing_field(source, &mut compact, "content_bytes");
    copy_listing_field(source, &mut compact, "start_line");
    copy_listing_field(source, &mut compact, "end_line");
    copy_listing_field(source, &mut compact, "total_lines");
    copy_listing_field(source, &mut compact, "mode");
    copy_listing_field(source, &mut compact, "requested_n");
    copy_listing_field(source, &mut compact, "root");
    copy_listing_field(source, &mut compact, "query");
    copy_listing_field(source, &mut compact, "count");
    copy_listing_field(source, &mut compact, "matches");
    copy_listing_field(source, &mut compact, "match_count");
    copy_listing_field(source, &mut compact, "name_count");
    copy_listing_field(source, &mut compact, "name_patterns");
    copy_listing_field(source, &mut compact, "name_results");
    if let Some(excerpt) = source.get("excerpt").and_then(Value::as_str) {
        compact.insert(
            "excerpt".to_string(),
            Value::String(crate::truncate_for_log(excerpt)),
        );
    }
    if compact.len() <= 1 {
        return None;
    }
    serde_json::to_string(&json!({ "extra": Value::Object(compact) })).ok()
}

fn compact_structured_listing_output_for_journal(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    let source = value
        .get("extra")
        .filter(|extra| extra.is_object())
        .unwrap_or(&value);
    if !value_is_structured_listing_output(source) {
        return None;
    }
    let mut compact = serde_json::Map::new();
    copy_listing_field(source, &mut compact, "action");
    copy_listing_field(source, &mut compact, "counts");
    copy_listing_field(source, &mut compact, "path");
    copy_listing_field(source, &mut compact, "resolved_path");
    copy_listing_field(source, &mut compact, "sort_by");
    copy_listing_field(source, &mut compact, "include_hidden");
    copy_listing_field(source, &mut compact, "dirs_only");
    copy_listing_field(source, &mut compact, "files_only");
    copy_listing_field(source, &mut compact, "names_by_kind");
    let has_names_by_kind = compact.contains_key("names_by_kind");
    if !has_names_by_kind {
        copy_listing_field(source, &mut compact, "names");
    }
    if let Some(entries) = compact_listing_entries(source.get("entries")) {
        if !has_names_by_kind || entries.iter().any(compact_listing_entry_has_metadata) {
            compact.insert("entries".to_string(), Value::Array(entries));
        }
    }
    if compact.len() <= 1 {
        return None;
    }
    serde_json::to_string(&json!({ "extra": Value::Object(compact) })).ok()
}

fn value_is_structured_listing_output(value: &Value) -> bool {
    matches!(
        value.get("action").and_then(Value::as_str),
        Some("inventory_dir" | "list_dir")
    ) || value
        .get("names_by_kind")
        .and_then(Value::as_object)
        .is_some()
        || value.get("names").and_then(Value::as_array).is_some()
        || value.get("entries").and_then(Value::as_array).is_some()
}

fn copy_listing_field(source: &Value, compact: &mut serde_json::Map<String, Value>, field: &str) {
    if let Some(value) = source.get(field) {
        compact.insert(field.to_string(), value.clone());
    }
}

fn compact_listing_entries(value: Option<&Value>) -> Option<Vec<Value>> {
    let entries = value.and_then(Value::as_array)?;
    let compact = entries
        .iter()
        .filter_map(compact_listing_entry)
        .collect::<Vec<_>>();
    (!compact.is_empty()).then_some(compact)
}

fn compact_listing_entry(entry: &Value) -> Option<Value> {
    let name = entry.get("name").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }
    let mut compact = serde_json::Map::new();
    compact.insert("name".to_string(), Value::String(name.to_string()));
    copy_listing_field(entry, &mut compact, "kind");
    copy_listing_field(entry, &mut compact, "path");
    copy_listing_field(entry, &mut compact, "hidden");
    copy_listing_field(entry, &mut compact, "size_bytes");
    copy_listing_field(entry, &mut compact, "modified_ts");
    Some(Value::Object(compact))
}

fn compact_listing_entry_has_metadata(entry: &Value) -> bool {
    entry
        .as_object()
        .is_some_and(|entry| entry.contains_key("size_bytes") || entry.contains_key("modified_ts"))
}

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
            "status_code": "answer_verifier_required_evidence_block",
            "failure_attribution": "answer_verifier_gap",
            "retryable": false,
            "missing_evidence_fields": &self.missing_evidence_fields,
            "answer_incomplete_reason": &self.answer_incomplete_reason,
            "confidence": self.confidence,
        })
        .to_string()
    }
}

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
    /// Ordered machine metadata only; prompt and response text are excluded.
    pub(crate) llm_call_sequence: Option<Vec<crate::LlmCallSequenceEntry>>,
    /// Provider-call usage and cost records contain machine metadata only.
    pub(crate) llm_cost_records: Option<Vec<crate::providers::LlmCallCostRecord>>,
    pub(crate) llm_cost_summary: Option<crate::providers::LlmTaskCostSummary>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournal {
    pub(crate) task_id: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) input_text: String,
    pub(crate) task_goal_spec: Option<Value>,
    pub(crate) context_bundle_summary: Option<String>,
    pub(crate) memory_trace: Option<Value>,
    pub(crate) turn_analysis: Option<crate::turn_context::TurnAnalysis>,
    pub(crate) output_contract: Option<crate::IntentOutputContract>,
    pub(crate) plan_result: Option<crate::PlanResult>,
    pub(crate) verify_result: Option<TaskJournalVerifySummary>,
    pub(crate) rounds: Vec<TaskJournalRoundTrace>,
    pub(crate) step_results: Vec<TaskJournalStepTrace>,
    pub(crate) task_observations: Vec<Value>,
    pub(crate) finalizer_summary: Option<TaskJournalFinalizerSummary>,
    pub(crate) answer_verifier_summary: Option<TaskJournalAnswerVerifierSummary>,
    pub(crate) task_metrics: TaskJournalTaskMetrics,
    pub(crate) task_lifecycle: Option<Value>,
    pub(crate) task_checkpoint: Option<Value>,
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
) -> Option<crate::evidence_policy::FailureAttribution> {
    match stop_signal.trim() {
        "recipe_repair_budget_exhausted" | "answer_verifier_retry_exhausted" => {
            Some(crate::evidence_policy::FailureAttribution::BudgetExhausted)
        }
        "prompt_budget_error" => {
            Some(crate::evidence_policy::FailureAttribution::PromptBudgetError)
        }
        _ => None,
    }
}

pub(crate) const ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL: &str =
    "answer_verifier_recovered_terminal_answer";

pub(crate) fn is_answer_verifier_recovered_terminal_stop_signal(stop_signal: &str) -> bool {
    stop_signal == ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL
}

fn summarize_verify_result(
    verify_result: &crate::verifier::VerifyResult,
) -> TaskJournalVerifySummary {
    TaskJournalVerifySummary {
        mode: verify_result.mode,
        approved: verify_result.approved,
        blocked_reason: verify_result.blocked_reason.clone(),
        shadow_blocked_reason: verify_result.shadow_blocked_reason.clone(),
        permission_decision: verify_result.permission_decision.clone(),
        needs_confirmation: verify_result.needs_confirmation,
        issues: verify_result
            .issues
            .iter()
            .map(|issue| TaskJournalVerifyIssue {
                step_id: issue.step_id.clone(),
                kind: issue.kind,
                detail: issue.detail.clone(),
                missing_fields: issue.missing_fields.clone(),
            })
            .collect(),
    }
}

impl TaskJournal {
    pub(crate) fn event_stream_snapshot(&self) -> Vec<Value> {
        task_event_stream_json(self)
    }

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

    pub(crate) fn record_task_goal_spec(&mut self, goal: Value) {
        if goal.is_object() {
            self.task_goal_spec = Some(goal);
        }
    }

    pub(crate) fn record_task_goal_spec_from_payload_json(&mut self, payload_json: &str) {
        if let Some(goal) = task_journal_goal::task_goal_spec_from_payload_json(payload_json) {
            self.record_task_goal_spec(goal);
        }
    }

    pub(crate) fn record_context_bundle_summary(&mut self, summary: impl Into<String>) {
        self.context_bundle_summary = Some(summary.into());
    }

    pub(crate) fn record_memory_trace(&mut self, trace: Value) {
        self.memory_trace = Some(trace);
    }

    pub(crate) fn record_turn_analysis(
        &mut self,
        turn_analysis: &crate::turn_context::TurnAnalysis,
    ) {
        self.turn_analysis = Some(turn_analysis.clone());
    }

    pub(crate) fn record_output_contract(&mut self, output_contract: &crate::IntentOutputContract) {
        self.output_contract = Some(output_contract.clone());
    }

    pub(crate) fn record_plan_result(&mut self, plan_result: &crate::PlanResult) {
        if let Some(output_contract) = plan_result.output_contract.as_ref() {
            self.record_output_contract(output_contract);
        }
        self.plan_result = Some(plan_result.clone());
    }

    pub(crate) fn record_verify_result(&mut self, verify_result: &crate::verifier::VerifyResult) {
        self.verify_result = Some(summarize_verify_result(verify_result));
    }

    pub(crate) fn push_step_result(&mut self, step_result: &crate::executor::StepExecutionResult) {
        self.step_results.push(TaskJournalStepTrace {
            step_id: step_result.step_id.clone(),
            skill: step_result.skill.clone(),
            status: step_result.status,
            output_excerpt: step_result
                .output
                .as_deref()
                .map(step_output_excerpt_for_journal),
            error_excerpt: step_result.error.as_deref().map(crate::truncate_for_log),
            started_at: step_result.started_at,
            finished_at: step_result.finished_at,
        });
        if let Some(observation) = coding_state_transition_observation(step_result) {
            let checkpoint =
                coding_milestone_checkpoint_observation(&observation, &self.task_observations);
            self.task_observations.push(observation);
            if let Some(checkpoint) = checkpoint {
                self.task_observations.push(checkpoint);
            }
        }
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

    pub(crate) fn record_llm_call_sequence(&mut self, sequence: Vec<crate::LlmCallSequenceEntry>) {
        self.task_metrics.llm_call_sequence = Some(sequence);
    }

    pub(crate) fn record_runtime_llm_metrics(&mut self, state: &crate::AppState, task_id: &str) {
        self.record_llm_calls_per_task(state.task_llm_call_count(task_id));
        self.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(task_id));
        self.record_llm_by_prompt(state.task_llm_by_prompt(task_id));
        self.record_llm_call_sequence(state.task_llm_call_sequence(task_id));
        self.task_metrics.llm_cost_records = Some(state.task_llm_cost_records(task_id));
        self.task_metrics.llm_cost_summary = Some(state.task_llm_cost_summary(task_id));
    }

    pub(crate) fn record_task_lifecycle(&mut self, lifecycle: Value) {
        if lifecycle.is_object() {
            self.task_lifecycle = Some(lifecycle);
        }
    }

    pub(crate) fn record_task_checkpoint(&mut self, checkpoint: Value) {
        if checkpoint.is_object() {
            self.task_checkpoint = Some(checkpoint);
        }
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
        if self.output_contract.is_none() {
            self.output_contract = other.output_contract.clone();
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
        if self.task_metrics.llm_call_sequence.is_none() {
            self.task_metrics.llm_call_sequence = other.task_metrics.llm_call_sequence.clone();
        }
        if self.task_metrics.llm_cost_records.is_none() {
            self.task_metrics.llm_cost_records = other.task_metrics.llm_cost_records.clone();
        }
        if self.task_metrics.llm_cost_summary.is_none() {
            self.task_metrics.llm_cost_summary = other.task_metrics.llm_cost_summary.clone();
        }
        if self.task_lifecycle.is_none() {
            self.task_lifecycle = other.task_lifecycle.clone();
        }
        if self.task_checkpoint.is_none() {
            self.task_checkpoint = other.task_checkpoint.clone();
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
            "context_budget_report": task_journal_context_budget::context_budget_report_json(self.context_bundle_summary.as_deref()),
            "transcript_compaction_records": task_journal_context_compaction::transcript_compaction_records_json(&self.task_observations),
            "memory_trace": self.memory_trace.clone(),
            "turn_analysis": self.turn_analysis.as_ref().map(turn_analysis_json),
            "output_contract": self.output_contract.as_ref().map(output_contract_json),
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
                .map(|summary| finalizer_summary_json(summary, self.output_contract.as_ref(), self)),
            "answer_verifier_summary": self.answer_verifier_summary.as_ref().map(answer_verifier_summary_json),
            "task_lifecycle": self.task_lifecycle.clone(),
            "task_checkpoint": self.task_checkpoint.clone(),
            "task_goal": task_goal_summary_json(self),
            "coding_workflow": coding_workflow_summary_json(self),
            "task_outcome": task_outcome_summary_json(self),
            "task_metrics": task_metrics_json(&self.task_metrics),
            "cost_budget": cost_budget_json(self),
            "validation_result": validation_result_json(self),
            "final_answer": self.final_answer.as_deref().map(crate::truncate_for_log),
        })
    }

    pub(crate) fn to_trace_json(&self) -> Value {
        let mut requested = requested_capability_sequence(self);
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
            "transcript_compaction_records": task_journal_context_compaction::transcript_compaction_records_json(&self.task_observations),
            "turn_analysis": self.turn_analysis.as_ref().map(turn_analysis_json),
            "output_contract": self.output_contract.as_ref().map(output_contract_json),
            "evidence_policy": self
                .output_contract
                .as_ref()
                .and_then(crate::evidence_policy::trace_snapshot_for_output_contract),
            "runtime_contract_snapshot": self
                .output_contract
                .as_ref()
                .and_then(crate::evidence_policy::runtime_contract_snapshot_for_output_contract),
            "evidence_coverage": self
                .output_contract
                .as_ref()
                .map(|contract| evidence_coverage_trace_json(contract, self)),
            "rounds": self.rounds.iter().map(|round| {
                let decision_envelope = round
                    .plan_result
                    .as_ref()
                    .map(agent_loop_round_plan_contract_envelope_json);
                let first_action_decision = decision_envelope
                    .as_ref()
                    .and_then(|value| value.get("decision"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let first_action_capability_ref = decision_envelope
                    .as_ref()
                    .and_then(|value| value.get("capability_ref"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                json!({
                    "round_no": round.round_no,
                    "owner_layer": "agent_loop_round",
                    "goal": crate::truncate_for_log(&round.goal),
                    "boundary_context_summary": boundary_context_summary_json(self),
                    "budget_profile": budget_profile_json(self),
                    "stop_signal": self.final_stop_signal.as_deref().map(crate::truncate_for_log),
                    "first_action_decision": first_action_decision,
                    "first_action_capability_ref": first_action_capability_ref,
                    "capability_resolution_records": round_capability_resolution_records_json(round),
                    "repair_signals": verify_repair_signals_json(
                        round.verify_result.as_ref(),
                        round.plan_result.as_ref(),
                    ),
                    "execution_recipe_summary": round
                        .execution_recipe_summary
                        .as_deref()
                        .map(crate::truncate_for_log),
                    "plan_result": round
                        .plan_result
                        .as_ref()
                        .map(plan_trace_json),
                    "verify_result": round.verify_result.as_ref().map(|verify| {
                        verify_trace_json(verify, round.plan_result.as_ref())
                    }),
                    "decision_envelope": decision_envelope,
                })
            }).collect::<Vec<_>>(),
            "step_results": self.step_results.iter().map(|step| {
                let requested = next_requested_capability(&mut requested, step);
                step_trace_json(step, requested.as_ref(), self.output_contract.as_ref())
            }).collect::<Vec<_>>(),
            "task_observations": self.task_observations.clone(),
            "event_stream": task_event_stream_json(self),
            "finalizer_summary": self
                .finalizer_summary
                .as_ref()
                .map(|summary| finalizer_summary_json(summary, self.output_contract.as_ref(), self)),
            "answer_verifier_summary": self.answer_verifier_summary.as_ref().map(answer_verifier_summary_json),
            "task_lifecycle": self.task_lifecycle.clone(),
            "task_checkpoint": self.task_checkpoint.clone(),
            "task_goal": task_goal_summary_json(self),
            "coding_workflow": coding_workflow_summary_json(self),
            "task_metrics": task_metrics_json(&self.task_metrics),
            "cost_budget": cost_budget_json(self),
            "validation_result": validation_result_json(self),
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
#[path = "task_journal_coding_state_tests.rs"]
mod coding_state_tests;

#[cfg(test)]
#[path = "task_journal_coding_workflow_tests.rs"]
mod coding_workflow_tests;

#[cfg(test)]
#[path = "task_journal_context_compaction_tests.rs"]
mod context_compaction_tests;

#[cfg(test)]
#[path = "task_journal_cost_tests.rs"]
mod cost_tests;

#[cfg(test)]
#[path = "task_journal_provider_routing_tests.rs"]
mod provider_routing_tests;

#[cfg(test)]
#[path = "task_journal_goal_tests.rs"]
mod goal_tests;

#[cfg(test)]
#[path = "task_journal_service_capability_evidence_tests.rs"]
mod service_capability_evidence_tests;

#[cfg(test)]
#[path = "task_journal_workspace_patch_tests.rs"]
mod workspace_patch_tests;

#[cfg(test)]
#[path = "task_journal_tests.rs"]
mod tests;
