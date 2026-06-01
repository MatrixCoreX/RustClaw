//! Intent routing and unified normalizer for ask tasks.
//!
//! **Ask main path:** Only `run_intent_normalizer` is used (resolved intent, resume_behavior,
//! schedule_kind, first-layer decision, needs_clarify, and output contract in one LLM call).
//!
//! **Fallback when normalizer LLM fails / parse fails:** stay on AskClarify unless the current
//! request contains an explicit structured tool/capability domain with no competing locator.
//! Those narrow fallbacks keep semantic routing owned by explicit contracts instead of
//! natural-language hard-match recovery code.

use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use tracing::{info, warn};

use crate::{
    llm_gateway, schedule_service, ActFinalizeStyle, AppState, ClaimedTask, FirstLayerDecision,
    RiskCeiling, RouteResult,
};

pub(crate) use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, ScheduleKind, SelfExtensionContract, SelfExtensionMode,
    SelfExtensionTrigger,
};

const CLARIFY_QUESTION_PROMPT_LOGICAL_PATH: &str = "prompts/clarify_question_prompt.md";
const INTENT_NORMALIZER_PROMPT_LOGICAL_PATH: &str = "prompts/intent_normalizer_prompt.md";
const CONTRACT_REPAIR_JUDGE_PROMPT_LOGICAL_PATH: &str = "prompts/contract_repair_judge_prompt.md";
const ROUTING_POLICY_PERSONA_PROMPT: &str = "Neutral routing policy classifier. Ignore style/persona preferences and optimize for correct intent resolution, clarification, and guard decisions.";

#[derive(Clone, Debug, Default)]
struct ContractRepairReport {
    sources: BTreeSet<&'static str>,
    details: BTreeSet<&'static str>,
}

impl ContractRepairReport {
    fn add(&mut self, source: &'static str, detail: &'static str) {
        self.sources.insert(source);
        self.details.insert(detail);
    }

    fn source_csv(&self) -> String {
        if self.sources.is_empty() {
            "none".to_string()
        } else {
            self.sources.iter().copied().collect::<Vec<_>>().join(",")
        }
    }

    fn detail_csv(&self) -> String {
        if self.details.is_empty() {
            "none".to_string()
        } else {
            self.details.iter().copied().collect::<Vec<_>>().join(",")
        }
    }

    fn has_detail(&self, detail: &'static str) -> bool {
        self.details.contains(detail)
    }

    fn needs_llm_semantic_repair(&self) -> bool {
        if self.sources.contains("tool_payload") || self.sources.contains("semantic_suspect") {
            return true;
        }
        if !self.sources.contains("conservative_none") {
            return false;
        }
        self.details
            .iter()
            .copied()
            .any(|detail| detail != "execution_recipe_untrusted_text_ignored")
    }

    fn merge(&mut self, other: &Self) {
        self.sources.extend(other.sources.iter().copied());
        self.details.extend(other.details.iter().copied());
    }
}

fn render_auth_policy_context(state: &AppState, task: &ClaimedTask) -> String {
    let auth_role = task
        .user_key
        .as_deref()
        .and_then(|user_key| {
            crate::resolve_auth_identity_by_key(state, user_key)
                .ok()
                .flatten()
        })
        .map(|identity| identity.role)
        .unwrap_or_else(|| "unknown".to_string());
    let current_process_cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    format!(
        "current_auth_role: {auth_role}\nallow_path_outside_workspace_for_task: {}\nallow_sudo_for_task: {}\nworkspace_root: {}\ncurrent_process_cwd: {}",
        crate::skills::task_allows_path_outside_workspace(state, Some(task)),
        crate::skills::task_allows_sudo(state, Some(task)),
        state.skill_rt.workspace_root.display(),
        current_process_cwd
    )
}

#[derive(Debug)]
struct RouteDecision {
    resolved_user_intent: String,
    needs_clarify: bool,
    clarify_question: String,
    reason: String,
    confidence: Option<f64>,
    schedule_kind: ScheduleKind,
    schedule_intent: Option<crate::ScheduleIntentOutput>,
    wants_file_delivery: bool,
    should_refresh_long_term_memory: bool,
    agent_display_name_hint: String,
    output_contract: IntentOutputContract,
}

impl TurnType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TaskRequest => "task_request",
            Self::TaskAppend => "task_append",
            Self::TaskReplace => "task_replace",
            Self::TaskCorrect => "task_correct",
            Self::TaskScopeUpdate => "task_scope_update",
            Self::RunControl => "run_control",
            Self::ApprovalDecision => "approval_decision",
            Self::StatusQuery => "status_query",
            Self::FeedbackOrError => "feedback_or_error",
            Self::PreferenceOrMemory => "preference_or_memory",
        }
    }
}

impl TargetTaskPolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReuseActive => "reuse_active",
            Self::ReplaceActive => "replace_active",
            Self::PauseAndQueue => "pause_and_queue",
            Self::Standalone => "standalone",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ContextResolution {
    pub(crate) resolved_user_intent: String,
    pub(crate) needs_clarify: bool,
    pub(crate) confidence: Option<f64>,
    pub(crate) reason: String,
}

/// Output of the unified intent normalizer (replaces resume_followup_intent + context_resolver + schedule_intent + intent_router in one LLM call).
#[derive(Debug, Clone)]
pub(crate) struct IntentNormalizerOutput {
    pub(crate) resolved_user_intent: String,
    pub(crate) resume_behavior: ResumeBehavior,
    pub(crate) schedule_kind: ScheduleKind,
    pub(crate) schedule_intent: Option<crate::ScheduleIntentOutput>,
    pub(crate) wants_file_delivery: bool,
    pub(crate) should_refresh_long_term_memory: bool,
    pub(crate) agent_display_name_hint: String,
    pub(crate) needs_clarify: bool,
    pub(crate) clarify_question: String,
    pub(crate) reason: String,
    pub(crate) confidence: f64,
    pub(crate) output_contract: IntentOutputContract,
    pub(crate) execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    /// First-layer semantic decision from the normalizer.
    pub(crate) first_layer_decision: FirstLayerDecision,
    /// Execution finalization style. This is not a semantic gate.
    pub(crate) execution_finalize_style: ActFinalizeStyle,
    pub(crate) turn_analysis: Option<TurnAnalysis>,
    pub(crate) fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnType {
    TaskRequest,
    TaskAppend,
    TaskReplace,
    TaskCorrect,
    TaskScopeUpdate,
    RunControl,
    ApprovalDecision,
    StatusQuery,
    FeedbackOrError,
    PreferenceOrMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetTaskPolicy {
    ReuseActive,
    ReplaceActive,
    PauseAndQueue,
    Standalone,
}

#[derive(Debug, Clone)]
pub(crate) struct TurnAnalysis {
    pub(crate) turn_type: Option<TurnType>,
    pub(crate) target_task_policy: Option<TargetTaskPolicy>,
    pub(crate) should_interrupt_active_run: bool,
    pub(crate) state_patch: Option<Value>,
    pub(crate) attachment_processing_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClarifyQuestionPolicy {
    #[default]
    AllowModel,
    SafeFallback,
}

pub(crate) fn route_result_from_normalizer(
    state: &AppState,
    task: &ClaimedTask,
    normalizer_out: &IntentNormalizerOutput,
) -> RouteResult {
    let _turn_analysis_present = normalizer_out.turn_analysis.is_some();
    RouteResult {
        ask_mode: crate::AskMode::from_first_layer_decision_with_finalize(
            normalizer_out.first_layer_decision,
            normalizer_out.execution_finalize_style,
        ),
        resolved_intent: normalizer_out.resolved_user_intent.clone(),
        needs_clarify: normalizer_out.needs_clarify,
        clarify_question: normalizer_out.clarify_question.clone(),
        route_reason: normalizer_out.reason.clone(),
        route_confidence: Some(normalizer_out.confidence),
        visible_skill_candidates: state.planner_available_skills_for_task(task),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: normalizer_out.resume_behavior,
        schedule_kind: normalizer_out.schedule_kind,
        schedule_intent: normalizer_out.schedule_intent.clone(),
        wants_file_delivery: normalizer_out.wants_file_delivery,
        should_refresh_long_term_memory: normalizer_out.should_refresh_long_term_memory,
        agent_display_name_hint: normalizer_out.agent_display_name_hint.clone(),
        output_contract: normalizer_out.output_contract.clone(),
    }
}

#[derive(Debug, Deserialize)]
struct IntentNormalizerOut {
    #[serde(default)]
    resolved_user_intent: String,
    #[serde(default)]
    answer_candidate: String,
    #[serde(default)]
    resume_behavior: String,
    #[serde(default)]
    schedule_kind: String,
    #[serde(default)]
    wants_file_delivery: bool,
    #[serde(default)]
    should_refresh_long_term_memory: bool,
    #[serde(default)]
    agent_display_name_hint: String,
    #[serde(default)]
    needs_clarify: bool,
    #[serde(default)]
    clarify_question: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    decision: String,
    #[serde(default)]
    schedule_intent: Option<crate::ScheduleIntentOutput>,
    #[serde(default)]
    output_contract: Option<IntentOutputContractOut>,
    #[serde(default)]
    execution_recipe: Option<IntentExecutionRecipeOut>,
    #[serde(default)]
    turn_type: String,
    #[serde(default)]
    target_task_policy: String,
    #[serde(default)]
    should_interrupt_active_run: bool,
    #[serde(default)]
    state_patch: Option<Value>,
    #[serde(default)]
    attachment_processing_required: bool,
}

#[derive(Debug, Deserialize)]
struct ContractRepairJudgeOut {
    #[serde(default)]
    apply: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    decision: String,
    #[serde(default)]
    needs_clarify: bool,
    #[serde(default)]
    clarify_question: String,
    #[serde(default)]
    resolved_user_intent: String,
    #[serde(default)]
    output_contract: Option<IntentOutputContractOut>,
    #[serde(default)]
    execution_recipe: Option<IntentExecutionRecipeOut>,
    #[serde(default)]
    turn_type: String,
    #[serde(default)]
    target_task_policy: String,
    #[serde(default)]
    state_patch: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct IntentOutputContractOut {
    #[serde(default)]
    response_shape: String,
    #[serde(default)]
    exact_sentence_count: Option<Value>,
    #[serde(default)]
    requires_content_evidence: bool,
    #[serde(default)]
    delivery_required: bool,
    #[serde(default)]
    locator_kind: String,
    #[serde(default)]
    delivery_intent: String,
    #[serde(default)]
    semantic_kind: String,
    #[serde(default)]
    locator_hint: String,
    #[serde(default)]
    self_extension: Option<SelfExtensionContractOut>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SelfExtensionContractOut {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    trigger: String,
    #[serde(default)]
    execute_now: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct IntentExecutionRecipeOut {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    profile: String,
    #[serde(default)]
    target_scope: String,
}

fn parse_resume_behavior(s: &str) -> ResumeBehavior {
    match s.trim().to_ascii_lowercase().as_str() {
        "resume_execute" | "resume" => ResumeBehavior::ResumeExecute,
        "resume_discuss" | "defer" => ResumeBehavior::ResumeDiscuss,
        _ => ResumeBehavior::None,
    }
}

fn parse_turn_type(s: &str) -> Option<TurnType> {
    match s.trim().to_ascii_lowercase().as_str() {
        "task_request" => Some(TurnType::TaskRequest),
        "task_append" => Some(TurnType::TaskAppend),
        "task_replace" => Some(TurnType::TaskReplace),
        "task_correct" => Some(TurnType::TaskCorrect),
        "task_scope_update" => Some(TurnType::TaskScopeUpdate),
        "run_control" => Some(TurnType::RunControl),
        "approval_decision" => Some(TurnType::ApprovalDecision),
        "status_query" => Some(TurnType::StatusQuery),
        "feedback_or_error" => Some(TurnType::FeedbackOrError),
        "preference_or_memory" => Some(TurnType::PreferenceOrMemory),
        _ => None,
    }
}

fn parse_target_task_policy(s: &str) -> Option<TargetTaskPolicy> {
    match s.trim().to_ascii_lowercase().as_str() {
        "reuse_active" => Some(TargetTaskPolicy::ReuseActive),
        "replace_active" => Some(TargetTaskPolicy::ReplaceActive),
        "pause_and_queue" => Some(TargetTaskPolicy::PauseAndQueue),
        "standalone" => Some(TargetTaskPolicy::Standalone),
        _ => None,
    }
}

fn infer_missing_turn_type_from_policy(
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    first_layer_decision: FirstLayerDecision,
    needs_clarify: bool,
    schedule_kind: ScheduleKind,
    should_refresh_long_term_memory: bool,
) -> Option<TurnType> {
    if turn_type.is_some()
        || needs_clarify
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || matches!(first_layer_decision, FirstLayerDecision::Clarify)
    {
        return turn_type;
    }
    match target_task_policy {
        Some(TargetTaskPolicy::Standalone) => Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::ReuseActive) => Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReplaceActive) => Some(TurnType::TaskReplace),
        Some(TargetTaskPolicy::PauseAndQueue) | None => None,
    }
}

fn should_detach_bare_acknowledgement_from_active_task(
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
    should_refresh_long_term_memory: bool,
) -> bool {
    matches!(turn_type, Some(TurnType::PreferenceOrMemory))
        && matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        && matches!(first_layer_decision, FirstLayerDecision::DirectAnswer)
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && !should_refresh_long_term_memory
        && state_patch.is_none()
}

fn should_downgrade_orphan_output_shape_clarify_to_direct_answer(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
    should_refresh_long_term_memory: bool,
    attachment_processing_required: bool,
) -> bool {
    matches!(first_layer_decision, FirstLayerDecision::Clarify)
        && active_primary_task_prompt(session_snapshot).is_none()
        && matches!(
            turn_type,
            Some(TurnType::TaskAppend | TurnType::TaskCorrect)
        )
        && matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        && !should_refresh_long_term_memory
        && state_patch.is_none()
        && !attachment_processing_required
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
}

fn infer_missing_target_policy_from_contract(
    target_task_policy: Option<TargetTaskPolicy>,
    turn_type: Option<TurnType>,
    first_layer_decision: FirstLayerDecision,
    needs_clarify: bool,
    schedule_kind: ScheduleKind,
    should_refresh_long_term_memory: bool,
    output_contract: &IntentOutputContract,
) -> Option<TargetTaskPolicy> {
    if target_task_policy.is_some()
        || turn_type.is_some()
        || needs_clarify
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || !matches!(first_layer_decision, FirstLayerDecision::DirectAnswer)
    {
        return target_task_policy;
    }

    let strict_chat_deliverable =
        matches!(output_contract.response_shape, OutputResponseShape::Strict)
            && !output_contract.requires_content_evidence
            && !output_contract.delivery_required
            && matches!(output_contract.locator_kind, OutputLocatorKind::None)
            && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
            && matches!(output_contract.semantic_kind, OutputSemanticKind::None);

    if strict_chat_deliverable {
        Some(TargetTaskPolicy::Standalone)
    } else {
        target_task_policy
    }
}

fn is_meaningful_state_patch(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Object(map) => map.values().any(is_meaningful_state_patch),
        Value::Array(items) => items.iter().any(is_meaningful_state_patch),
        Value::String(text) => !text.trim().is_empty(),
        _ => true,
    }
}

fn parse_schedule_kind(s: &str) -> ScheduleKind {
    match s.trim().to_ascii_lowercase().as_str() {
        "create" => ScheduleKind::Create,
        "update" | "pause" | "resume" => ScheduleKind::Update,
        "delete" => ScheduleKind::Delete,
        "query" | "list" => ScheduleKind::Query,
        _ => ScheduleKind::None,
    }
}

fn parse_output_response_shape(s: &str) -> OutputResponseShape {
    match s.trim().to_ascii_lowercase().as_str() {
        "one_sentence" => OutputResponseShape::OneSentence,
        "strict"
        | "exact"
        | "exact_text"
        | "strict_text"
        | "list"
        | "array"
        | "list_only"
        | "names_list"
        | "exact_format"
        | "one_line"
        | "single_line"
        | "line_only"
        | "one_line_string"
        | "single_line_string"
        | "one_line_text"
        | "single_line_text"
        | "one_line_result"
        | "single_line_result"
        | "one_line_comparison"
        | "single_line_comparison" => OutputResponseShape::Strict,
        "scalar" => OutputResponseShape::Scalar,
        "file_token" => OutputResponseShape::FileToken,
        _ => OutputResponseShape::Free,
    }
}

fn semantic_kind_token_requests_scalar_response_shape(s: &str) -> bool {
    matches!(
        normalize_schema_token(s).as_str(),
        "scalar"
            | "scalar_value"
            | "scalar_only"
            | "value"
            | "value_only"
            | "single_value"
            | "field_value"
            | "file_field_value"
    )
}

fn parse_output_locator_kind(s: &str) -> OutputLocatorKind {
    match s.trim().to_ascii_lowercase().as_str() {
        "path" => OutputLocatorKind::Path,
        "current_workspace" => OutputLocatorKind::CurrentWorkspace,
        "url" => OutputLocatorKind::Url,
        "filename" => OutputLocatorKind::Filename,
        _ => OutputLocatorKind::None,
    }
}

fn parse_output_delivery_intent(s: &str) -> OutputDeliveryIntent {
    match s.trim().to_ascii_lowercase().as_str() {
        "file_single" | "single_file" | "file" => OutputDeliveryIntent::FileSingle,
        "directory_lookup" | "dir_lookup" => OutputDeliveryIntent::DirectoryLookup,
        "directory_batch_files" | "batch_directory_delivery" | "dir_batch" => {
            OutputDeliveryIntent::DirectoryBatchFiles
        }
        _ => OutputDeliveryIntent::None,
    }
}

fn parse_output_semantic_kind_token(s: &str) -> OutputSemanticKind {
    match s.trim().to_ascii_lowercase().as_str() {
        "raw_command_output"
        | "raw_output"
        | "command_output"
        | "command_result"
        | "command_execution_result" => OutputSemanticKind::RawCommandOutput,
        "service_status"
        | "service_state"
        | "service_running_status"
        | "process_status"
        | "process_state"
        | "process_running_status"
        | "daemon_status"
        | "daemon_state" => OutputSemanticKind::ServiceStatus,
        "hidden_entries_check"
        | "hidden_entry_check"
        | "hidden_files_check"
        | "hidden_file_check"
        | "hidden_files_example"
        | "hidden_entries_example"
        | "hidden_entries"
        | "hidden_files" => OutputSemanticKind::HiddenEntriesCheck,
        "file_names"
        | "file_names_only"
        | "file_name_only"
        | "filename_only"
        | "filenames_only"
        | "names_only"
        | "entry_names"
        | "directory_entry_names" => OutputSemanticKind::FileNames,
        "directory_names"
        | "directory_names_only"
        | "directory_name_only"
        | "dir_names"
        | "dir_names_only"
        | "folder_names"
        | "folder_names_only"
        | "folders_only" => OutputSemanticKind::DirectoryNames,
        "directory_entry_groups"
        | "directory_file_groups"
        | "file_directory_groups"
        | "entry_kind_groups"
        | "entries_by_kind"
        | "grouped_entries"
        | "grouped_entry_names" => OutputSemanticKind::DirectoryEntryGroups,
        "file_paths"
        | "file_paths_only"
        | "path_list"
        | "paths_list"
        | "file_path_list"
        | "repository_file_paths"
        | "workspace_file_paths" => OutputSemanticKind::FilePaths,
        "directory_purpose_summary" | "listing_purpose_summary" | "directory_listing_summary" => {
            OutputSemanticKind::DirectoryPurposeSummary
        }
        "content_excerpt_summary" | "document_excerpt_summary" | "file_excerpt_summary" => {
            OutputSemanticKind::ContentExcerptSummary
        }
        "content_excerpt_with_summary"
        | "excerpt_with_summary"
        | "raw_excerpt_with_summary"
        | "bounded_excerpt_with_summary" => OutputSemanticKind::ContentExcerptWithSummary,
        "content_presence_check"
        | "content_contains_check"
        | "content_match_check"
        | "identifier_presence_check"
        | "field_presence_check"
        | "text_presence_check" => OutputSemanticKind::ContentPresenceCheck,
        "excerpt_kind_judgment" | "content_excerpt_judgment" | "log_vs_checklist" => {
            OutputSemanticKind::ExcerptKindJudgment
        }
        "recent_artifacts_judgment" | "artifact_style_classification" => {
            OutputSemanticKind::RecentArtifactsJudgment
        }
        "workspace_project_summary" | "project_overview" | "workspace_overview_summary" => {
            OutputSemanticKind::WorkspaceProjectSummary
        }
        "scalar" => OutputSemanticKind::None,
        "scalar_count" | "count" => OutputSemanticKind::ScalarCount,
        "quantity_comparison" | "comparison" => OutputSemanticKind::QuantityComparison,
        "execution_failed_step"
        | "failed_step"
        | "failed_command_step"
        | "execution_failure_step" => OutputSemanticKind::ExecutionFailedStep,
        "generated_file_delivery"
        | "new_file_delivery"
        | "created_file_delivery"
        | "write_then_send_file" => OutputSemanticKind::GeneratedFileDelivery,
        "scalar_path_only" | "path_only" => OutputSemanticKind::ScalarPathOnly,
        "existence_with_path" | "exists_with_path" => OutputSemanticKind::ExistenceWithPath,
        "existence_with_path_summary"
        | "exists_with_path_summary"
        | "existence_with_path_purpose"
        | "exists_with_path_purpose" => OutputSemanticKind::ExistenceWithPathSummary,
        "recent_scalar_equality_check"
        | "same_or_different"
        | "equality_check"
        | "scalar_equality"
        | "value_equality"
        | "value_comparison"
        | "field_equality"
        | "field_value_equality"
        | "key_value_comparison" => OutputSemanticKind::RecentScalarEqualityCheck,
        "git_commit_subject"
        | "git_commit_title"
        | "commit_subject"
        | "commit_title"
        | "latest_commit_subject"
        | "latest_commit_title" => OutputSemanticKind::GitCommitSubject,
        "git_repository_state"
        | "git_workspace_state"
        | "git_state"
        | "git_status"
        | "git_branch"
        | "git_current_branch"
        | "git_remote"
        | "git_changed_files"
        | "git_rev_parse" => OutputSemanticKind::GitRepositoryState,
        "structured_keys"
        | "structured_key_names"
        | "structured_top_level_keys"
        | "top_level_keys"
        | "object_keys"
        | "config_keys" => OutputSemanticKind::StructuredKeys,
        "config_validation" | "structured_config_validation" | "structured_file_validation" => {
            OutputSemanticKind::ConfigValidation
        }
        "config_mutation" | "config_write" | "config_set" | "structured_config_mutation" => {
            OutputSemanticKind::ConfigMutation
        }
        "config_risk_assessment" | "config_risk" | "structured_config_risk" | "config_guard" => {
            OutputSemanticKind::ConfigRiskAssessment
        }
        "sqlite_table_listing" | "sqlite_tables_listing" | "sqlite_tables_summary" => {
            OutputSemanticKind::SqliteTableListing
        }
        "sqlite_table_names_only" | "sqlite_table_names" | "sqlite_names_only" => {
            OutputSemanticKind::SqliteTableNamesOnly
        }
        "sqlite_database_kind_judgment" | "sqlite_db_kind" | "database_kind_judgment" => {
            OutputSemanticKind::SqliteDatabaseKindJudgment
        }
        "sqlite_schema_version" | "sqlite_db_schema_version" => {
            OutputSemanticKind::SqliteSchemaVersion
        }
        "rss_news_fetch" | "rss_latest_news" | "rss_feed_fetch" | "external_news_fetch" => {
            OutputSemanticKind::RssNewsFetch
        }
        "web_page_summary"
        | "webpage_summary"
        | "web_content_summary"
        | "url_content_summary"
        | "browser_page_summary" => OutputSemanticKind::WebPageSummary,
        "web_search_summary" | "web_search_results" | "search_results_summary" => {
            OutputSemanticKind::WebSearchSummary
        }
        "weather_query" | "weather_current" | "weather_forecast" | "weather_report" => {
            OutputSemanticKind::WeatherQuery
        }
        "market_quote" | "stock_quote" | "crypto_quote" | "asset_quote" | "market_price" => {
            OutputSemanticKind::MarketQuote
        }
        "image_understanding"
        | "image_description"
        | "image_describe"
        | "image_vision"
        | "image_extract"
        | "image_compare"
        | "screenshot_summary" => OutputSemanticKind::ImageUnderstanding,
        "publishing_preview" | "social_post_preview" | "channel_draft_preview" => {
            OutputSemanticKind::PublishingPreview
        }
        "package_manager_detection" | "package_manager_detect" | "package_detect_manager" => {
            OutputSemanticKind::PackageManagerDetection
        }
        "archive_list" | "archive_listing" | "archive_contents" => OutputSemanticKind::ArchiveList,
        "archive_read" | "archive_member_read" | "archive_file_read" => {
            OutputSemanticKind::ArchiveRead
        }
        "archive_pack" | "archive_create" | "archive_compress" => OutputSemanticKind::ArchivePack,
        "archive_unpack" | "archive_extract" | "archive_decompress" => {
            OutputSemanticKind::ArchiveUnpack
        }
        "docker_ps" | "docker_containers" | "docker_container_list" => OutputSemanticKind::DockerPs,
        "docker_images" | "docker_image_list" => OutputSemanticKind::DockerImages,
        "docker_logs" => OutputSemanticKind::DockerLogs,
        "docker_container_lifecycle" | "docker_lifecycle" => {
            OutputSemanticKind::DockerContainerLifecycle
        }
        _ => OutputSemanticKind::None,
    }
}

fn parse_output_semantic_kind(s: &str) -> OutputSemanticKind {
    let mut parsed = OutputSemanticKind::None;
    let mut saw_separator = false;
    for token in s.split(['|', ',', ';']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        saw_separator = true;
        let candidate = parse_output_semantic_kind_token(token);
        if candidate != OutputSemanticKind::None {
            parsed = candidate;
        }
    }
    if saw_separator && parsed != OutputSemanticKind::None {
        parsed
    } else {
        parse_output_semantic_kind_token(s)
    }
}

fn parse_self_extension_mode(s: &str) -> SelfExtensionMode {
    match s.trim().to_ascii_lowercase().as_str() {
        "temporary_fix" => SelfExtensionMode::TemporaryFix,
        "permanent_extension" => SelfExtensionMode::PermanentExtension,
        _ => SelfExtensionMode::None,
    }
}

fn parse_self_extension_trigger(s: &str) -> SelfExtensionTrigger {
    match s.trim().to_ascii_lowercase().as_str() {
        "explicit_user_request" => SelfExtensionTrigger::ExplicitUserRequest,
        "capability_gap" => SelfExtensionTrigger::CapabilityGap,
        _ => SelfExtensionTrigger::None,
    }
}

fn parse_output_contract(
    out: Option<IntentOutputContractOut>,
    wants_file_delivery: bool,
) -> IntentOutputContract {
    let mut contract = IntentOutputContract::default();
    if let Some(raw) = out {
        let semantic_token_requests_scalar_shape =
            semantic_kind_token_requests_scalar_response_shape(&raw.semantic_kind);
        contract.response_shape = parse_output_response_shape(&raw.response_shape);
        contract.exact_sentence_count = raw
            .exact_sentence_count
            .as_ref()
            .and_then(parse_positive_usize_value);
        contract.requires_content_evidence = raw.requires_content_evidence;
        contract.delivery_required = raw.delivery_required;
        contract.locator_kind = parse_output_locator_kind(&raw.locator_kind);
        contract.delivery_intent = parse_output_delivery_intent(&raw.delivery_intent);
        contract.semantic_kind = parse_output_semantic_kind(&raw.semantic_kind);
        contract.locator_hint = raw.locator_hint.trim().to_string();
        if semantic_token_requests_scalar_shape
            && !matches!(contract.response_shape, OutputResponseShape::FileToken)
        {
            contract.response_shape = OutputResponseShape::Scalar;
            contract.semantic_kind = OutputSemanticKind::None;
        }
        if let Some(self_extension) = raw.self_extension {
            contract.self_extension = SelfExtensionContract {
                mode: parse_self_extension_mode(&self_extension.mode),
                trigger: parse_self_extension_trigger(&self_extension.trigger),
                execute_now: self_extension.execute_now,
            };
        }
    }
    if contract.exact_sentence_count.is_some_and(|count| count > 1)
        && matches!(contract.response_shape, OutputResponseShape::OneSentence)
    {
        contract.response_shape = OutputResponseShape::Strict;
    }
    if wants_file_delivery {
        contract.delivery_required = true;
        if matches!(contract.response_shape, OutputResponseShape::Free) {
            contract.response_shape = OutputResponseShape::FileToken;
        }
        if matches!(contract.locator_kind, OutputLocatorKind::None) {
            contract.locator_kind = OutputLocatorKind::Path;
        }
        if matches!(contract.delivery_intent, OutputDeliveryIntent::None) {
            contract.delivery_intent = OutputDeliveryIntent::FileSingle;
        }
    } else if contract.delivery_required
        && !matches!(contract.response_shape, OutputResponseShape::FileToken)
    {
        if matches!(contract.delivery_intent, OutputDeliveryIntent::None)
            || (matches!(contract.delivery_intent, OutputDeliveryIntent::FileSingle)
                && !matches!(contract.semantic_kind, OutputSemanticKind::None))
        {
            contract.delivery_required = false;
            contract.delivery_intent = OutputDeliveryIntent::None;
        }
    }
    contract
}

fn parse_positive_usize_value(value: &Value) -> Option<usize> {
    match value {
        Value::Number(number) => number.as_u64().and_then(|n| usize::try_from(n).ok()),
        Value::String(text) => text.trim().parse::<usize>().ok(),
        _ => None,
    }
    .filter(|n| *n > 0)
}

fn normalized_scope_patch_hint_text(raw: &str) -> Option<String> {
    let mut value = raw.trim().trim_matches(['"', '\'']).trim();
    if value.is_empty() {
        return None;
    }
    loop {
        let lower = value.to_ascii_lowercase();
        let stripped = if lower.ends_with("_only") {
            value[..value.len().saturating_sub("_only".len())].trim()
        } else if lower.ends_with("-only") {
            value[..value.len().saturating_sub("-only".len())].trim()
        } else if lower.ends_with(" only") {
            value[..value.len().saturating_sub(" only".len())].trim()
        } else {
            value
        };
        if stripped == value {
            break;
        }
        value = stripped.trim_matches(['_', '-', ' ']).trim();
        if value.is_empty() {
            return None;
        }
    }
    let simple_scope_token = value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '\\' | '.'));
    if !simple_scope_token || matches!(value, "." | "./" | "/" | "\\") {
        return None;
    }
    Some(value.to_string())
}

fn scope_patch_hint_value(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => normalized_scope_patch_hint_text(raw),
        Value::Array(values) => values.iter().find_map(scope_patch_hint_value),
        Value::Object(map) => [
            "scope",
            "module",
            "area",
            "section",
            "topic",
            "focus",
            "target_scope",
        ]
        .iter()
        .filter_map(|key| map.get(*key))
        .find_map(scope_patch_hint_value),
        _ => None,
    }
}

fn locator_hint_is_unset_or_broad(hint: &str) -> bool {
    let hint = hint.trim();
    hint.is_empty() || matches!(hint, "." | "./" | "/" | "\\") || Path::new(hint).is_absolute()
}

fn locator_hint_names_workspace_root(hint: &str, workspace_root: &Path) -> bool {
    let Some(root_name) = workspace_root.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let normalized_root = normalize_locator_identity_token(root_name);
    let normalized_hint = normalize_locator_identity_token(hint);
    !normalized_root.is_empty() && normalized_hint == normalized_root
}

fn locator_hint_points_to_workspace_root(hint: &str, workspace_root: &Path) -> bool {
    if locator_hint_names_workspace_root(hint, workspace_root) {
        return true;
    }
    let hint = hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return false;
    }
    let candidate = Path::new(hint);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    normalize_compare_path(candidate) == normalize_compare_path(workspace_root.to_path_buf())
}

fn normalize_locator_identity_token(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | ','
                    | '.'
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | ')'
                    | '('
                    | ']'
                    | '['
                    | '）'
                    | '（'
                    | '】'
                    | '【'
                    | '>'
                    | '<'
                    | '》'
                    | '《'
            )
        })
        .to_ascii_lowercase()
}

fn apply_workspace_scope_patch_to_contract(
    output_contract: &mut IntentOutputContract,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    state_patch: Option<&Value>,
) -> Option<String> {
    if !matches!(turn_type, Some(TurnType::TaskScopeUpdate))
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        || output_contract.semantic_kind != OutputSemanticKind::WorkspaceProjectSummary
    {
        return None;
    }
    let scope_hint = scope_patch_hint_value(state_patch?)?;
    if !locator_hint_is_unset_or_broad(&output_contract.locator_hint) {
        return None;
    }
    output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    output_contract.locator_hint = scope_hint.clone();
    Some(scope_hint)
}

fn apply_current_turn_structural_contract_repair(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    first_layer_decision: FirstLayerDecision,
    answer_candidate: &str,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
) -> Option<&'static str> {
    let mut reason = None;
    if should_preserve_existing_observed_context_synthesis_contract(
        output_contract,
        req_surface,
        turn_type,
        target_task_policy,
    ) {
        output_contract.requires_content_evidence = false;
        reason = Some("existing_observed_context_synthesis");
    } else if output_semantic_kind_requires_fresh_evidence(output_contract.semantic_kind) {
        output_contract.requires_content_evidence = true;
        reason = Some("semantic_contract_requires_evidence");
    }

    if let Some((semantic_kind, locator_hint)) =
        archive_pair_contract_from_surface(output_contract, req_surface)
    {
        output_contract.semantic_kind = semantic_kind;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = match semantic_kind {
            OutputSemanticKind::ArchivePack => OutputResponseShape::Scalar,
            OutputSemanticKind::ArchiveUnpack => OutputResponseShape::OneSentence,
            _ => output_contract.response_shape,
        };
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some(match semantic_kind {
            OutputSemanticKind::ArchivePack => "archive_pack_pair_contract_repair",
            OutputSemanticKind::ArchiveUnpack => "archive_unpack_pair_contract_repair",
            _ => "archive_pair_contract_repair",
        });
    }

    if planner_execute_inline_structured_payload_context(
        req,
        req_surface,
        first_layer_decision,
        output_contract,
    ) {
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.locator_kind = OutputLocatorKind::None;
        output_contract.locator_hint.clear();
        output_contract.semantic_kind = OutputSemanticKind::None;
        if matches!(
            output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        ) {
            output_contract.response_shape = OutputResponseShape::Strict;
        }
        reason = Some("inline_structured_payload_context_execute");
    }

    if planner_execute_inline_structured_transform_contract_context(
        req_surface,
        first_layer_decision,
        output_contract,
        answer_candidate,
    ) {
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.locator_kind = OutputLocatorKind::None;
        output_contract.locator_hint.clear();
        output_contract.semantic_kind = OutputSemanticKind::None;
        if matches!(
            output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        ) {
            output_contract.response_shape = OutputResponseShape::Strict;
        }
        reason = Some("inline_structured_transform_contract_repair");
    }

    if matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
        && output_contract.delivery_required
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        && matches!(
            output_contract.delivery_intent,
            OutputDeliveryIntent::FileSingle
        )
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && answer_candidate.trim().is_empty()
        && !req_surface.has_delivery_token_reference()
    {
        output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
        output_contract.requires_content_evidence = true;
        reason = Some("file_token_delivery_contract_repair");
    }

    if let Some(filename) =
        generated_file_delivery_filename_only_existing_target_repair(output_contract, req_surface)
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = true;
        output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
        output_contract.response_shape = OutputResponseShape::FileToken;
        output_contract.locator_kind = OutputLocatorKind::Filename;
        output_contract.locator_hint = filename;
        reason = Some("generated_file_delivery_filename_only_existing_target_repair");
    }

    if let Some(locator_hint) =
        generated_file_delivery_existing_content_summary_repair(output_contract, workspace_root)
    {
        output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = true;
        output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
        output_contract.response_shape = OutputResponseShape::Strict;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("generated_file_delivery_existing_content_summary_repair");
    }

    if let Some(locator_hint) = archive_read_contract_from_surface(output_contract, req_surface) {
        output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = OutputResponseShape::Free;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("archive_read_member_contract_repair");
    }

    if let Some(locator_hint) = config_mutation_contract_from_surface(
        output_contract,
        req,
        req_surface,
        first_layer_decision,
    ) {
        output_contract.semantic_kind = OutputSemanticKind::ConfigMutation;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = OutputResponseShape::Free;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("config_mutation_structural_contract_repair");
    }

    if let Some(locator_hint) = structured_config_keys_contract_from_surface(output_contract, req) {
        output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = OutputResponseShape::Strict;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("structured_config_keys_overrides_file_names");
    }

    if output_contract.semantic_kind == OutputSemanticKind::ScalarPathOnly
        && req_surface.has_structured_target_refinement()
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.requires_content_evidence = true;
        reason = Some("structured_file_scalar_repair");
    }

    if output_contract.semantic_kind == OutputSemanticKind::StructuredKeys
        && req_surface.dotted_field_selector.is_some()
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.response_shape = OutputResponseShape::Scalar;
        output_contract.requires_content_evidence = true;
        reason = Some("structured_field_selector_requires_scalar_value");
    }

    if output_contract.semantic_kind == OutputSemanticKind::ConfigValidation
        && req_surface
            .dotted_field_selector
            .as_deref()
            .is_some_and(|field_path| !structural_config_value_after_field(req, field_path))
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.response_shape = OutputResponseShape::Scalar;
        output_contract.requires_content_evidence = true;
        reason = Some("config_validation_field_selector_requires_scalar_value");
    }

    if let Some(locator_hint) =
        structured_identifier_presence_contract_from_surface(output_contract, req, workspace_root)
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.delivery_intent = OutputDeliveryIntent::None;
        output_contract.response_shape = OutputResponseShape::Scalar;
        output_contract.locator_kind = OutputLocatorKind::Path;
        output_contract.locator_hint = locator_hint;
        reason = Some("structured_identifier_presence_requires_content_evidence");
    }

    if output_contract.semantic_kind == OutputSemanticKind::StructuredKeys
        && matches!(output_contract.response_shape, OutputResponseShape::Scalar)
        && !output_contract.delivery_required
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        )
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.requires_content_evidence = true;
        reason = Some("structured_keys_scalar_response_requires_field_value");
    }

    if output_contract.semantic_kind == OutputSemanticKind::WorkspaceProjectSummary
        && !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        && locator_hint_points_to_workspace_root(&output_contract.locator_hint, workspace_root)
    {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        reason = reason.or(Some("workspace_summary_root_locator_repair"));
    }

    let scalar_direct_answer = matches!(first_layer_decision, FirstLayerDecision::DirectAnswer)
        && !answer_candidate.trim().is_empty()
        && !req_surface.has_structured_target_refinement();

    if matches!(output_contract.response_shape, OutputResponseShape::Scalar)
        && !output_contract.delivery_required
        && !scalar_direct_answer
        && (req_surface.has_explicit_path_or_url() || req_surface.has_filename_candidates())
    {
        output_contract.requires_content_evidence = true;
        reason = reason.or(Some("scalar_locator_requires_evidence"));
    }

    if matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && req_surface.inline_json_shape.is_none()
        && (req_surface.has_explicit_path_or_url() || req_surface.has_filename_candidates())
        && !req_surface.is_structural_locator_only_reply()
    {
        output_contract.requires_content_evidence = true;
        reason = reason.or(Some("planner_locator_requires_evidence"));
    }

    if output_contract.requires_content_evidence
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && !semantic_kind_uses_locatorless_system_observation(output_contract.semantic_kind)
        && !planner_execute_inline_structured_payload_context(
            req,
            req_surface,
            first_layer_decision,
            output_contract,
        )
    {
        let filename_candidates = req_surface.filename_candidates_excluding_field_selectors();
        if let Some(locator) =
            crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req)
        {
            output_contract.locator_kind = locator.locator_kind;
            output_contract.locator_hint = locator.locator_hint;
            reason = reason.or(Some("structured_locator_contract_repair"));
        } else if filename_candidates.len() == 1 {
            output_contract.locator_kind = OutputLocatorKind::Filename;
            output_contract.locator_hint = filename_candidates[0].clone();
            reason = reason.or(Some("filename_target_contract_repair"));
        } else if !filename_candidates.is_empty() {
            output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
            output_contract.locator_hint = workspace_root.display().to_string();
            reason = reason.or(Some("workspace_filename_targets_contract_repair"));
        }
    }

    if output_contract.requires_content_evidence
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
        && matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::ExistenceWithPath | OutputSemanticKind::ExistenceWithPathSummary
        )
        && explicit_surface_path_facts_fallback_decision(req, req_surface, workspace_root).is_some()
    {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
        reason = reason.or(Some("explicit_multi_path_facts_workspace_contract_repair"));
    }

    reason
}

fn structured_config_keys_contract_from_surface(
    output_contract: &IntentOutputContract,
    req: &str,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.semantic_kind, OutputSemanticKind::FileNames)
    {
        return None;
    }
    output_contract_structured_config_path(output_contract).or_else(|| {
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req).and_then(
            |locator| {
                path_has_structured_config_extension(&locator.locator_hint)
                    .then_some(locator.locator_hint)
            },
        )
    })
}

fn config_mutation_contract_from_surface(
    output_contract: &IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    first_layer_decision: FirstLayerDecision,
) -> Option<String> {
    if !matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::None
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::ScalarPathOnly
                | OutputSemanticKind::StructuredKeys
                | OutputSemanticKind::ConfigValidation
        )
    {
        return None;
    }
    let field_path = req_surface.dotted_field_selector.as_deref()?;
    if !structural_config_value_after_field(req, field_path) {
        return None;
    }
    output_contract_structured_config_path(output_contract).or_else(|| {
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req).and_then(
            |locator| {
                path_has_structured_config_extension(&locator.locator_hint)
                    .then_some(locator.locator_hint)
            },
        )
    })
}

fn structured_identifier_presence_contract_from_surface(
    output_contract: &IntentOutputContract,
    req: &str,
    workspace_root: &Path,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::ExistenceWithPath | OutputSemanticKind::ConfigValidation
        )
    {
        return None;
    }
    let locator_hint = output_contract_structured_config_path(output_contract).or_else(|| {
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req).and_then(
            |locator| {
                path_has_structured_config_extension(&locator.locator_hint)
                    .then_some(locator.locator_hint)
            },
        )
    })?;
    if !request_has_code_identifier_outside_locator(req, &locator_hint) {
        return None;
    }
    let path = Path::new(&locator_hint);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    if !resolved.is_file() {
        return None;
    }
    Some(locator_hint)
}

fn request_has_code_identifier_outside_locator(req: &str, locator_hint: &str) -> bool {
    let locator_parts = identifier_parts_from_locator(locator_hint);
    req.split(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '_' | '-' | '$'))
        .map(str::trim)
        .filter(|token| {
            token.chars().any(|ch| matches!(ch, '_' | '-' | '$'))
                && token
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$'))
        })
        .any(|token| !locator_parts.contains(&token.to_ascii_lowercase()))
}

fn identifier_parts_from_locator(locator_hint: &str) -> BTreeSet<String> {
    locator_hint
        .split(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '_' | '-' | '$'))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn output_contract_structured_config_path(
    output_contract: &IntentOutputContract,
) -> Option<String> {
    let hint = output_contract.locator_hint.trim();
    if hint.is_empty() || !path_has_structured_config_extension(hint) {
        return None;
    }
    matches!(
        output_contract.locator_kind,
        OutputLocatorKind::Path | OutputLocatorKind::Filename
    )
    .then(|| hint.to_string())
}

fn path_has_structured_config_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| matches!(ext.as_str(), "json" | "toml" | "yaml" | "yml"))
}

fn structural_config_value_after_field(req: &str, field_path: &str) -> bool {
    let req_lower = req.to_ascii_lowercase();
    let field_lower = field_path.to_ascii_lowercase();
    let Some(field_idx) = req_lower.find(&field_lower) else {
        return false;
    };
    let Some(suffix) = req.get(field_idx + field_path.len()..) else {
        return false;
    };
    structural_config_value_candidate_tokens(suffix).any(|token| {
        token.eq_ignore_ascii_case("true")
            || token.eq_ignore_ascii_case("false")
            || token.eq_ignore_ascii_case("null")
            || token.parse::<i64>().is_ok()
            || token.parse::<f64>().is_ok()
    })
}

fn structural_config_value_candidate_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，'
                    | '。'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
    })
    .map(|token| {
        token
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '=' | '>' | '-' | '→'))
            .trim()
            .to_string()
    })
    .filter(|token| !token.is_empty())
}

fn planner_execute_inline_structured_payload_context(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
) -> bool {
    matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
        && req_surface.inline_json_shape.is_some()
        && !crate::intent::surface_signals::inline_json_transform_request(req)
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && !req_surface.has_explicit_path_or_url()
        && !req_surface.has_delivery_token_reference()
        && req_surface.locator_target_pair.is_none()
}

fn planner_execute_inline_structured_transform_contract_context(
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    answer_candidate: &str,
) -> bool {
    matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
        && req_surface.inline_json_shape.is_some()
        && answer_candidate.trim().is_empty()
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Strict | OutputResponseShape::Scalar
        )
        && matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::ContentExcerptWithSummary
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::StructuredKeys
        )
        && !req_surface.has_explicit_path_or_url()
        && !req_surface.has_delivery_token_reference()
        && req_surface.locator_target_pair.is_none()
}

fn generated_file_delivery_filename_only_existing_target_repair(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<String> {
    if output_contract.semantic_kind != OutputSemanticKind::GeneratedFileDelivery
        || !output_contract.delivery_required
        || output_contract.delivery_intent != OutputDeliveryIntent::FileSingle
        || output_contract.response_shape != OutputResponseShape::FileToken
        || req_surface.has_explicit_path_or_url()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_delivery_token_reference()
    {
        return None;
    }
    req_surface
        .single_filename_candidate()
        .map(str::trim)
        .filter(|filename| !filename.is_empty())
        .map(ToString::to_string)
}

fn generated_file_delivery_existing_content_summary_repair(
    output_contract: &IntentOutputContract,
    workspace_root: &Path,
) -> Option<String> {
    if output_contract.semantic_kind != OutputSemanticKind::GeneratedFileDelivery
        || !output_contract.delivery_required
        || output_contract.delivery_intent != OutputDeliveryIntent::FileSingle
        || output_contract.response_shape != OutputResponseShape::FileToken
        || output_contract.exact_sentence_count.is_none()
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        )
    {
        return None;
    }
    let raw_hint = output_contract.locator_hint.trim();
    if raw_hint.is_empty() || raw_hint.contains('|') {
        return None;
    }
    let candidate = Path::new(raw_hint);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    if !resolved.is_file() {
        return None;
    }
    Some(
        resolved
            .canonicalize()
            .unwrap_or(resolved)
            .display()
            .to_string(),
    )
}

fn semantic_kind_uses_locatorless_system_observation(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::RawCommandOutput
            | OutputSemanticKind::ServiceStatus
            | OutputSemanticKind::PackageManagerDetection
            | OutputSemanticKind::DockerPs
            | OutputSemanticKind::DockerImages
            | OutputSemanticKind::DockerLogs
            | OutputSemanticKind::DockerContainerLifecycle
            | OutputSemanticKind::WeatherQuery
            | OutputSemanticKind::MarketQuote
            | OutputSemanticKind::ImageUnderstanding
            | OutputSemanticKind::PublishingPreview
    )
}

fn archive_pair_contract_from_surface(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<(OutputSemanticKind, String)> {
    let generated_delivery_contract = output_contract.semantic_kind
        == OutputSemanticKind::GeneratedFileDelivery
        || (output_contract.semantic_kind == OutputSemanticKind::None
            && (output_contract.delivery_required
                || matches!(
                    output_contract.response_shape,
                    OutputResponseShape::FileToken
                )
                || matches!(
                    output_contract.delivery_intent,
                    OutputDeliveryIntent::FileSingle
                )));
    let (left, right) = req_surface.locator_target_pair.as_ref()?;
    let left_is_archive = contract_repair_supported_archive_path(left);
    let right_is_archive = contract_repair_supported_archive_path(right);
    let inferred_kind = match (left_is_archive, right_is_archive) {
        (false, true) => Some((
            OutputSemanticKind::ArchivePack,
            format!("{} | {}", left.trim(), right.trim()),
        )),
        (true, false) => Some((
            OutputSemanticKind::ArchiveUnpack,
            format!("{} | {}", left.trim(), right.trim()),
        )),
        _ => None,
    }?;
    let structural_operation_pair =
        archive_pair_has_structural_operation_shape(inferred_kind.0, left, right);
    let already_archive_contract = output_contract.semantic_kind == inferred_kind.0;
    let scalar_or_drift_contract = structural_operation_pair
        && !matches!(output_contract.response_shape, OutputResponseShape::Strict)
        && matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::None
                | OutputSemanticKind::ScalarPathOnly
                | OutputSemanticKind::ContentExcerptSummary
        );
    if structural_operation_pair
        && (already_archive_contract || generated_delivery_contract || scalar_or_drift_contract)
    {
        return Some(inferred_kind);
    }
    None
}

fn archive_pair_has_structural_operation_shape(
    semantic_kind: OutputSemanticKind,
    left: &str,
    right: &str,
) -> bool {
    match semantic_kind {
        OutputSemanticKind::ArchivePack => {
            !contract_repair_supported_archive_path(left)
                && contract_repair_path_operand_is_structural(left)
                && contract_repair_supported_archive_path(right)
        }
        OutputSemanticKind::ArchiveUnpack => {
            contract_repair_supported_archive_path(left)
                && !contract_repair_supported_archive_path(right)
                && contract_repair_archive_unpack_dest_is_structural(right)
        }
        _ => false,
    }
}

fn contract_repair_archive_unpack_dest_is_structural(path: &str) -> bool {
    let path = path.trim();
    let structurally_path_like = path.starts_with("./")
        || path.starts_with("../")
        || path.starts_with('/')
        || path.starts_with("~/")
        || path.contains('/')
        || path.contains('\\');
    structurally_path_like && !path_basename_looks_like_file(path)
}

fn path_basename_looks_like_file(path: &str) -> bool {
    let basename = path.trim().rsplit(['/', '\\']).next().unwrap_or("").trim();
    let Some((stem, ext)) = basename.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=16).contains(&ext.len())
        && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn contract_repair_path_operand_is_structural(path: &str) -> bool {
    let path = path.trim();
    path.starts_with("./")
        || path.starts_with("../")
        || path.starts_with('/')
        || path.starts_with("~/")
        || path.contains('/')
        || path.contains('\\')
}

fn contract_repair_supported_archive_path(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    path.ends_with(".zip") || path.ends_with(".tar.gz") || path.ends_with(".tgz")
}

fn archive_read_contract_from_surface(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<String> {
    if output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::ArchiveRead
                | OutputSemanticKind::ArchiveUnpack
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::None
        )
    {
        return None;
    }

    if req_surface
        .locator_target_pair
        .as_ref()
        .is_some_and(|(left, right)| {
            archive_pair_has_structural_operation_shape(
                OutputSemanticKind::ArchiveUnpack,
                left,
                right,
            )
        })
    {
        return None;
    }

    let candidates = req_surface.filename_candidates.clone();
    let archive = if contract_repair_supported_archive_path(&output_contract.locator_hint) {
        output_contract.locator_hint.trim().to_string()
    } else {
        candidates
            .iter()
            .find(|candidate| contract_repair_supported_archive_path(candidate))
            .cloned()?
    };
    let archive_key = archive.trim().to_ascii_lowercase();
    let member = candidates
        .iter()
        .find(|candidate| {
            let candidate = candidate.trim();
            !candidate.is_empty()
                && candidate.to_ascii_lowercase() != archive_key
                && archive_member_candidate_is_structural(candidate)
        })?
        .trim()
        .to_string();

    Some(format!("{} | {}", archive.trim(), member))
}

fn archive_member_candidate_is_structural(candidate: &str) -> bool {
    let candidate = candidate.trim();
    !candidate.is_empty()
        && !contract_repair_supported_archive_path(candidate)
        && !candidate.ends_with('/')
        && (candidate.contains('.') || candidate.contains('/') || candidate.contains('\\'))
}

fn archive_unpack_has_supported_archive_locator(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    output_contract
        .locator_hint
        .split('|')
        .any(contract_repair_supported_archive_path)
        || req_surface
            .locator_target_pair
            .as_ref()
            .is_some_and(|(left, right)| {
                contract_repair_supported_archive_path(left)
                    || contract_repair_supported_archive_path(right)
            })
        || req_surface
            .filename_candidates_excluding_field_selectors()
            .iter()
            .any(|candidate| contract_repair_supported_archive_path(candidate))
        || active_session_has_supported_archive_locator(session_snapshot)
}

fn active_session_has_supported_archive_locator(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let Some(snapshot) = session_snapshot else {
        return false;
    };
    snapshot
        .active_followup_frame
        .as_ref()
        .and_then(|frame| frame.bound_target.as_deref())
        .is_some_and(contract_repair_supported_archive_path)
        || snapshot
            .active_observed_facts
            .as_ref()
            .and_then(|facts| facts.bound_target.as_deref())
            .is_some_and(contract_repair_supported_archive_path)
        || snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(|facts| {
                facts
                    .delivery_targets
                    .iter()
                    .any(|target| contract_repair_supported_archive_path(target))
            })
}

fn apply_archive_unpack_missing_archive_locator_clarify(
    output_contract: &mut IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::ArchiveUnpack
    ) || !output_contract.requires_content_evidence
        || archive_unpack_has_supported_archive_locator(
            output_contract,
            req_surface,
            session_snapshot,
        )
    {
        return None;
    }
    *needs_clarify = true;
    clarify_question.clear();
    *first_layer_decision = FirstLayerDecision::Clarify;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    output_contract.delivery_required = false;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    Some("archive_unpack_missing_archive_locator_clarify")
}

fn apply_self_contained_payload_direct_answer_contract_repair(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    needs_clarify: bool,
    answer_candidate: &str,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if needs_clarify
        || answer_candidate.trim().is_empty()
        || req_surface.inline_json_shape.is_none()
        || crate::intent::surface_signals::inline_json_transform_request(req)
        || wants_file_delivery
        || !matches!(schedule_kind, ScheduleKind::None)
        || execution_recipe_hint.is_some_and(|spec| {
            !matches!(
                spec.kind,
                crate::execution_recipe::ExecutionRecipeKind::None
            )
        })
        || req_surface.has_explicit_path_or_url()
        || req_surface.has_filename_candidates()
        || req_surface.has_delivery_token_reference()
        || output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.self_extension.mode, SelfExtensionMode::None)
        || !matches!(
            output_contract.self_extension.trigger,
            SelfExtensionTrigger::None
        )
        || output_contract.self_extension.execute_now
    {
        return None;
    }

    output_contract.requires_content_evidence = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    output_contract.semantic_kind = OutputSemanticKind::None;
    *first_layer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("self_contained_payload_direct_answer_contract")
}

fn apply_inline_structured_transform_direct_answer_repair(
    output_contract: &mut IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    needs_clarify: bool,
    answer_candidate: &str,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if needs_clarify
        || !matches!(*first_layer_decision, FirstLayerDecision::DirectAnswer)
        || req_surface.inline_json_shape.is_none()
        || !answer_candidate_has_structured_transform_result(answer_candidate)
        || wants_file_delivery
        || !matches!(schedule_kind, ScheduleKind::None)
        || execution_recipe_hint.is_some_and(|spec| {
            !matches!(
                spec.kind,
                crate::execution_recipe::ExecutionRecipeKind::None
            )
        })
        || req_surface.has_explicit_path_or_url()
        || req_surface.has_filename_candidates()
        || req_surface.has_delivery_token_reference()
        || output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.self_extension.mode, SelfExtensionMode::None)
        || !matches!(
            output_contract.self_extension.trigger,
            SelfExtensionTrigger::None
        )
        || output_contract.self_extension.execute_now
    {
        return None;
    }

    output_contract.requires_content_evidence = true;
    output_contract.delivery_required = false;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.response_shape = OutputResponseShape::Strict;
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = ActFinalizeStyle::ChatWrapped;
    Some("inline_structured_transform_contract_repair")
}

fn answer_candidate_has_structured_transform_result(answer_candidate: &str) -> bool {
    let trimmed = answer_candidate.trim();
    if trimmed.is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .is_some_and(|value| {
            matches!(
                value,
                serde_json::Value::Array(_) | serde_json::Value::Object(_)
            )
        })
        || answer_candidate_is_markdown_table(trimmed)
}

fn answer_candidate_is_markdown_table(candidate: &str) -> bool {
    let lines = candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines.len() >= 2
        && lines
            .first()
            .is_some_and(|line| line.starts_with('|') && line.ends_with('|'))
        && lines
            .get(1)
            .is_some_and(|line| line.chars().all(|ch| matches!(ch, '|' | '-' | ':' | ' ')))
}

fn clean_answer_candidate_path_token(answer_candidate: &str) -> Option<String> {
    let token = answer_candidate
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .trim();
    if token.is_empty() || token.contains('\n') {
        None
    } else {
        Some(token.to_string())
    }
}

fn existing_answer_candidate_path(answer_candidate: &str, workspace_root: &Path) -> Option<String> {
    let token = clean_answer_candidate_path_token(answer_candidate)?;
    let path = Path::new(&token);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    candidate.exists().then_some(token)
}

fn answer_candidate_has_path_context(path: &str) -> bool {
    let path = Path::new(path);
    if path.is_absolute() {
        return true;
    }
    let mut normal_components = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => normal_components += 1,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return true;
            }
        }
    }
    normal_components > 1
}

fn normalize_state_patch_text_token(text: &str) -> String {
    text.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn request_uses_filename_only_schema_token(req: &str) -> bool {
    let normalized = normalize_schema_token(req);
    [
        "filename_only",
        "file_name_only",
        "basename_only",
        "output_filename_only",
    ]
    .iter()
    .any(|token| normalized.contains(token))
}

fn state_patch_requests_filename_only_output(state_patch: Option<&Value>) -> bool {
    fn value_requests_filename_only(value: &Value) -> bool {
        match value {
            Value::String(text) => matches!(
                normalize_state_patch_text_token(text).as_str(),
                "filename_only" | "basename_only" | "file_name_only"
            ),
            Value::Array(items) => items.iter().any(value_requests_filename_only),
            Value::Object(map) => map.iter().any(|(key, value)| {
                matches!(
                    normalize_state_patch_text_token(key).as_str(),
                    "output_format"
                        | "output_shape"
                        | "format"
                        | "answer_format"
                        | "delivery_format"
                ) && value_requests_filename_only(value)
            }),
            _ => false,
        }
    }
    state_patch.is_some_and(value_requests_filename_only)
}

fn state_patch_deictic_reference_requires_clarify(state_patch: Option<&Value>) -> bool {
    state_patch_deictic_reference_target(state_patch).is_some_and(|target| {
        matches!(
            target,
            "unresolved_prior_object" | "missing_locator" | "ambiguous_locator"
        )
    })
}

fn state_patch_deictic_reference_target(state_patch: Option<&Value>) -> Option<&str> {
    state_patch
        .and_then(|patch| patch.get("deictic_reference"))
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("target"))
        .and_then(Value::as_str)
}

fn state_patch_deictic_reference_is_resolved(state_patch: Option<&Value>) -> bool {
    state_patch_deictic_reference_target(state_patch).is_some_and(|target| {
        matches!(
            target,
            "current_action_result" | "current_turn_locator" | "comparison_result"
        )
    })
}

fn token_is_unbound_scope_identifier(candidate: &str) -> bool {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    });
    if trimmed.len() < 2
        || trimmed.len() > 128
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('.')
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        return false;
    }
    let mut has_ascii_alnum = false;
    let mut has_scope_separator = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            has_ascii_alnum = true;
            continue;
        }
        if matches!(ch, '_' | '-') {
            has_scope_separator = true;
            continue;
        }
        return false;
    }
    has_ascii_alnum && has_scope_separator
}

fn single_unbound_scope_identifier_outside_filename(
    prompt: &str,
    filename: &str,
) -> Option<String> {
    let mut matches = Vec::new();
    for token in prompt.split_whitespace().flat_map(|token| {
        token.split(|ch: char| matches!(ch, ',' | '，' | '。' | ';' | '；' | '、' | ':' | '：'))
    }) {
        let trimmed = token.trim();
        if trimmed.eq_ignore_ascii_case(filename) || !token_is_unbound_scope_identifier(trimmed) {
            continue;
        }
        if !matches
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
        {
            matches.push(trimmed.to_string());
        }
    }
    (matches.len() == 1).then(|| matches.remove(0))
}

fn surface_has_unbound_scope_plus_single_filename_target(
    output_contract: &IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    if output_contract.semantic_kind != OutputSemanticKind::ExistenceWithPath
        || output_contract.delivery_required
        || req_surface.has_explicit_path_or_url()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_structured_target_refinement()
    {
        return false;
    }
    let filenames = req_surface.filename_candidates_excluding_field_selectors();
    if filenames.len() != 1 {
        return false;
    }
    single_unbound_scope_identifier_outside_filename(req, &filenames[0]).is_some()
}

fn apply_answer_candidate_path_evidence_repair(
    output_contract: &mut IntentOutputContract,
    answer_candidate: &str,
    state_patch: Option<&Value>,
    workspace_root: &Path,
    needs_clarify: bool,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if needs_clarify
        || !matches!(first_layer_decision, FirstLayerDecision::DirectAnswer)
        || output_contract.requires_content_evidence
        || output_contract.delivery_required
        || output_contract.response_shape != OutputResponseShape::Scalar
        || output_contract.locator_kind != OutputLocatorKind::None
        || output_contract.delivery_intent != OutputDeliveryIntent::None
    {
        return None;
    }
    if state_patch_requests_filename_only_output(state_patch)
        || state_patch_deictic_reference_requires_clarify(state_patch)
    {
        return None;
    }
    let path = existing_answer_candidate_path(answer_candidate, workspace_root)?;
    if !answer_candidate_has_path_context(&path) {
        return None;
    }
    output_contract.requires_content_evidence = true;
    output_contract.locator_kind = OutputLocatorKind::Path;
    output_contract.locator_hint = path;
    output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("answer_candidate_path_requires_evidence")
}

fn semantic_kind_can_use_existing_observed_context(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::ContentExcerptSummary
            | OutputSemanticKind::ContentPresenceCheck
            | OutputSemanticKind::ExcerptKindJudgment
            | OutputSemanticKind::RecentArtifactsJudgment
    )
}

fn should_preserve_existing_observed_context_synthesis_contract(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
) -> bool {
    matches!(turn_type, Some(TurnType::TaskAppend))
        && matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
        && semantic_kind_can_use_existing_observed_context(output_contract.semantic_kind)
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
        && !req_surface.has_concrete_locator_hint()
        && !req_surface.has_structured_target_refinement()
        && !req_surface.has_delivery_token_reference()
}

fn apply_spurious_structured_observation_clarify_repair(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify || is_bare_path_only_input_for_clarify(req, req_surface) {
        return None;
    }
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    if surface_has_unbound_scope_plus_single_filename_target(output_contract, req, req_surface) {
        return None;
    }
    let has_current_turn_locator = req_surface.has_explicit_path_or_url()
        || req_surface.has_single_filename_candidate()
        || req_surface.has_filename_candidates()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_concrete_locator_hint();
    let has_observable_answer_shape = matches!(
        output_contract.response_shape,
        OutputResponseShape::Scalar | OutputResponseShape::Strict | OutputResponseShape::FileToken
    ) || output_semantic_kind_requires_fresh_evidence(
        output_contract.semantic_kind,
    ) || req_surface.has_structured_target_refinement()
        || req_surface.locator_target_pair.is_some();
    if surface_locator_is_insufficient_for_clarify_repair(
        output_contract,
        req_surface,
        has_observable_answer_shape,
    ) {
        return None;
    }
    if !has_current_turn_locator
        || (!has_observable_answer_shape && !req_surface.has_concrete_locator_hint())
    {
        return None;
    }
    let fallback_locator = if matches!(output_contract.locator_kind, OutputLocatorKind::None) {
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req)
    } else {
        None
    };
    if matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && fallback_locator.is_none()
        && !req_surface.has_filename_candidates()
        && req_surface.locator_target_pair.is_none()
    {
        return None;
    }

    output_contract.requires_content_evidence = true;
    if output_contract.locator_hint.trim().is_empty() && req_surface.locator_target_pair.is_some() {
        if let Some((left, right)) = req_surface.locator_target_pair.as_ref() {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = format!("{left}, {right}");
        }
    }
    if output_contract.locator_hint.trim().is_empty() {
        if let Some(filename) = req_surface.single_filename_candidate() {
            output_contract.locator_kind = OutputLocatorKind::Filename;
            output_contract.locator_hint = filename.to_string();
        }
    }
    if let Some(locator) =
        fallback_locator.filter(|_| output_contract.locator_hint.trim().is_empty())
    {
        output_contract.locator_kind = locator.locator_kind;
        output_contract.locator_hint = locator.locator_hint;
    } else if matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && req_surface.has_filename_candidates()
    {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
    }
    *needs_clarify = false;
    clarify_question.clear();
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("structured_observation_clarify_repair")
}

fn surface_locator_is_insufficient_for_clarify_repair(
    output_contract: &IntentOutputContract,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    has_observable_answer_shape: bool,
) -> bool {
    if !req_surface.has_concrete_locator_hint()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_filename_candidates()
    {
        return false;
    }
    if matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::ArchivePack | OutputSemanticKind::ArchiveUnpack
    ) {
        return true;
    }
    !output_contract.requires_content_evidence && !has_observable_answer_shape
}

fn semantic_kind_can_use_workspace_default_for_observation(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::HiddenEntriesCheck
            | OutputSemanticKind::FileNames
            | OutputSemanticKind::DirectoryNames
            | OutputSemanticKind::DirectoryEntryGroups
            | OutputSemanticKind::FilePaths
            | OutputSemanticKind::DirectoryPurposeSummary
            | OutputSemanticKind::WorkspaceProjectSummary
            | OutputSemanticKind::ScalarCount
            | OutputSemanticKind::ExistenceWithPath
            | OutputSemanticKind::ExistenceWithPathSummary
            | OutputSemanticKind::GitCommitSubject
            | OutputSemanticKind::GitRepositoryState
            | OutputSemanticKind::DockerPs
            | OutputSemanticKind::DockerImages
            | OutputSemanticKind::DockerLogs
            | OutputSemanticKind::DockerContainerLifecycle
    )
}

pub(crate) fn contract_test_hint_value(req: &str, wanted_key: &str) -> Option<String> {
    let hint_block = req
        .split_once("[CONTRACT_TEST_HINT]")?
        .1
        .split_once("[/CONTRACT_TEST_HINT]")?
        .0;
    for line in hint_block.lines().map(str::trim) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != wanted_key {
            continue;
        }
        let value = value.trim();
        return (!value.is_empty()).then(|| value.to_string());
    }
    None
}

pub(crate) fn contract_test_hint_semantic_kind(req: &str) -> Option<OutputSemanticKind> {
    let semantic_kind =
        parse_output_semantic_kind(&contract_test_hint_value(req, "semantic_kind")?);
    (semantic_kind != OutputSemanticKind::None).then_some(semantic_kind)
}

pub(crate) fn request_without_contract_test_hint(req: &str) -> String {
    let mut remaining = req;
    let mut out = String::with_capacity(req.len());
    while let Some((before, after_start)) = remaining.split_once("[CONTRACT_TEST_HINT]") {
        out.push_str(before);
        let Some((_, after_end)) = after_start.split_once("[/CONTRACT_TEST_HINT]") else {
            remaining = "";
            break;
        };
        remaining = after_end;
    }
    out.push_str(remaining);
    out
}

fn apply_structured_contract_hint_repair(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    wants_file_delivery: &mut bool,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    let semantic_kind = contract_test_hint_semantic_kind(req)?;
    let surface_req = request_without_contract_test_hint(req);
    output_contract.semantic_kind = semantic_kind;
    output_contract.requires_content_evidence =
        output_semantic_kind_requires_fresh_evidence(semantic_kind);
    output_contract.delivery_required = false;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.response_shape = response_shape_for_contract_hint_fallback(semantic_kind);
    apply_contract_hint_delivery_defaults(output_contract, wants_file_delivery);
    match semantic_kind {
        OutputSemanticKind::GitCommitSubject | OutputSemanticKind::GitRepositoryState => {
            if matches!(
                output_contract.locator_kind,
                OutputLocatorKind::None | OutputLocatorKind::Path
            ) && output_contract.locator_hint.trim().is_empty()
            {
                output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
                output_contract.locator_hint = workspace_root.display().to_string();
            }
        }
        OutputSemanticKind::PackageManagerDetection => {
            output_contract.locator_kind = OutputLocatorKind::None;
            output_contract.locator_hint.clear();
        }
        OutputSemanticKind::DockerPs
        | OutputSemanticKind::DockerImages
        | OutputSemanticKind::DockerLogs
        | OutputSemanticKind::DockerContainerLifecycle => {
            if output_contract.locator_hint.trim().is_empty() {
                output_contract.locator_kind = OutputLocatorKind::None;
            }
        }
        _ => {}
    }
    apply_contract_hint_locator_defaults(
        output_contract,
        &surface_req,
        req_surface,
        workspace_root,
    );
    if output_contract.requires_content_evidence {
        *needs_clarify = false;
        clarify_question.clear();
        *first_layer_decision = FirstLayerDecision::PlannerExecute;
        *execution_finalize_style =
            crate::post_route_policy::content_evidence_execution_finalize_style(
                output_contract,
                false,
            )
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    }
    Some("structured_contract_hint_repair")
}

fn apply_workspace_default_observation_clarify_repair(
    output_contract: &mut IntentOutputContract,
    workspace_root: &Path,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify
        || !output_contract.requires_content_evidence
        || !semantic_kind_can_use_workspace_default_for_observation(output_contract.semantic_kind)
    {
        return None;
    }
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    if matches!(output_contract.locator_kind, OutputLocatorKind::None) {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
    }
    *needs_clarify = false;
    clarify_question.clear();
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("workspace_default_observation_clarify_repair")
}

fn resolved_existing_directory_from_current_request(state: &AppState, req: &str) -> Option<String> {
    match crate::worker::try_resolve_implicit_locator_path(
        state,
        req,
        "",
        OutputLocatorKind::Path,
        None,
    ) {
        Some(crate::worker::LocatorAutoResolution::Direct(path)) if Path::new(&path).is_dir() => {
            return Some(path);
        }
        Some(crate::worker::LocatorAutoResolution::Direct(_))
        | Some(crate::worker::LocatorAutoResolution::Fuzzy(_))
        | None => {}
    }
    resolve_unique_direct_child_directory_token(state, req)
}

fn resolved_directory_pair_from_current_request(
    state: &AppState,
    req: &str,
) -> Option<(String, String)> {
    let mut out = Vec::new();
    for token in current_request_locator_tokens(req) {
        if !strong_structural_locator_token(&token) {
            continue;
        }
        let Some(path) = resolve_unique_directory_token_under_workspace(state, &token) else {
            continue;
        };
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&path))
        {
            out.push(path);
        }
        if out.len() >= 2 {
            break;
        }
    }
    (out.len() == 2).then(|| (out.remove(0), out.remove(0)))
}

fn strong_structural_locator_token(token: &str) -> bool {
    token.contains(['_', '-', '.']) || token.chars().any(|ch| ch.is_ascii_digit())
}

fn resolve_unique_directory_token_under_workspace(state: &AppState, token: &str) -> Option<String> {
    let workspace_root = state.skill_rt.workspace_root.as_path();
    if !workspace_root.is_dir() || token.trim().is_empty() {
        return None;
    }
    let mut stack = vec![workspace_root.to_path_buf()];
    let mut matches = Vec::new();
    let mut visits = 0usize;
    let max_visits = state.skill_rt.locator_scan_max_files.max(50_000);
    while let Some(dir) = stack.pop() {
        visits = visits.saturating_add(1);
        if visits > max_visits {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                file_type.is_dir().then(|| entry.path())
            })
            .collect::<Vec<_>>();
        children.sort();
        for child in children.into_iter().rev() {
            if child
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(token))
            {
                let canonical = child.canonicalize().unwrap_or(child.clone());
                matches.push(canonical.display().to_string());
                if matches.len() > 1 {
                    return None;
                }
            }
            stack.push(child);
        }
    }
    matches.pop()
}

fn resolve_unique_direct_child_directory_token(state: &AppState, req: &str) -> Option<String> {
    let mut matches = Vec::new();
    for token in current_request_locator_tokens(req) {
        for root in [
            state.skill_rt.workspace_root.as_path(),
            state.skill_rt.default_locator_search_dir.as_path(),
        ] {
            collect_direct_child_directory_token_matches(root, &token, &mut matches);
        }
    }
    matches.sort();
    matches.dedup();
    (matches.len() == 1).then(|| matches.remove(0))
}

fn current_request_locator_tokens(req: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in req.split_whitespace() {
        for token in raw.split(|ch: char| {
            matches!(
                ch,
                ',' | '，'
                    | '。'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
        }) {
            let token = token
                .trim_matches(|ch: char| {
                    !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
                })
                .trim();
            if token.chars().count() < 2
                || token.contains('/')
                || token.contains('\\')
                || token.starts_with('.')
                || token.chars().all(|ch| ch.is_ascii_digit())
                || !token
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
            {
                continue;
            }
            if !out.iter().any(|existing: &String| existing == token) {
                out.push(token.to_string());
            }
        }
    }
    out
}

fn collect_direct_child_directory_token_matches(
    root: &Path,
    token: &str,
    matches: &mut Vec<String>,
) {
    if !root.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.eq_ignore_ascii_case(token) {
            continue;
        }
        let canonical = path.canonicalize().unwrap_or(path);
        matches.push(canonical.display().to_string());
    }
}

fn apply_resolved_directory_observation_clarify_repair(
    state: &AppState,
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    state_patch: Option<&Value>,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !*needs_clarify
        || req_surface.is_structural_locator_only_reply()
        || req_surface.token_count <= 2
        || output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
    {
        return None;
    }
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return None;
    }
    if !crate::worker::semantic_kind_can_bind_workspace_child_locator(output_contract.semantic_kind)
    {
        return None;
    }
    let directory = resolved_existing_directory_from_current_request(state, req)?;
    output_contract.requires_content_evidence = true;
    output_contract.delivery_required = false;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.locator_kind = OutputLocatorKind::Path;
    output_contract.locator_hint = directory;
    *needs_clarify = false;
    clarify_question.clear();
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style =
        crate::post_route_policy::content_evidence_execution_finalize_style(output_contract, false)
            .unwrap_or_else(|| execution_finalize_style_for_contract(output_contract));
    Some("resolved_directory_observation_clarify_repair")
}

fn apply_unbound_workspace_generic_content_clarify_repair(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if *needs_clarify
        || !matches!(*first_layer_decision, FirstLayerDecision::PlannerExecute)
        || !output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        || !short_unbound_topic_surface(req, req_surface)
    {
        return None;
    }

    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    *needs_clarify = true;
    clarify_question.clear();
    *first_layer_decision = FirstLayerDecision::Clarify;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("unbound_workspace_generic_content_requires_clarify")
}

fn short_unbound_topic_surface(
    req: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    let trimmed = req.trim();
    if trimmed.is_empty()
        || surface.token_count != 1
        || surface.inline_json_shape.is_some()
        || surface.has_concrete_locator_hint()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_structured_target_refinement()
        || surface.has_delivery_token_reference()
        || surface.has_deictic_reference()
        || surface.is_structural_locator_only_reply()
        || trimmed.contains(['/', '\\', '.', ':'])
    {
        return false;
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return false;
    }
    let signal_chars = trimmed.chars().filter(|ch| ch.is_alphanumeric()).count();
    if signal_chars == 0 {
        return false;
    }
    if trimmed.is_ascii() {
        signal_chars <= 32
    } else {
        signal_chars <= 8
    }
}

fn bare_path_only_input_can_fill_active_observable_task(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
) -> bool {
    let active_delivery_frame = session_snapshot
        .and_then(|snapshot| snapshot.active_followup_frame.as_ref())
        .is_some_and(|frame| {
            matches!(
                frame.op_kind,
                crate::followup_frame::FollowupOpKind::Delivery
            )
        });
    let file_delivery_contract = output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || matches!(
            output_contract.delivery_intent,
            OutputDeliveryIntent::FileSingle
        );
    if active_delivery_frame
        && file_delivery_contract
        && matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
    {
        return true;
    }

    let active_followup_policy = matches!(
        turn_type,
        Some(TurnType::TaskAppend | TurnType::TaskCorrect | TurnType::TaskReplace)
    ) && matches!(
        target_task_policy,
        Some(TargetTaskPolicy::ReuseActive | TargetTaskPolicy::ReplaceActive)
    );
    let executable_observation_contract = output_contract.requires_content_evidence
        && (output_semantic_kind_requires_fresh_evidence(output_contract.semantic_kind)
            || matches!(
                output_contract.response_shape,
                OutputResponseShape::Scalar
                    | OutputResponseShape::Strict
                    | OutputResponseShape::FileToken
            )
            || matches!(
                output_contract.locator_kind,
                OutputLocatorKind::Path
                    | OutputLocatorKind::Filename
                    | OutputLocatorKind::Url
                    | OutputLocatorKind::CurrentWorkspace
            )
            || !output_contract.locator_hint.trim().is_empty());
    let active_replacement_locator_policy = matches!(turn_type, Some(TurnType::TaskRequest))
        && matches!(target_task_policy, Some(TargetTaskPolicy::ReplaceActive))
        && executable_observation_contract;
    let active_clarify_locator_policy = active_clarify_locator_task_prompt(session_snapshot)
        .is_some()
        && executable_observation_contract;
    let active_implicit_locator_policy =
        turn_type.is_none() && target_task_policy.is_none() && executable_observation_contract;

    let decision_can_fill_active_task =
        matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
            || (matches!(first_layer_decision, FirstLayerDecision::Clarify)
                && executable_observation_contract);

    if active_observable_task_prompt(session_snapshot).is_none()
        || !decision_can_fill_active_task
        || !(active_followup_policy
            || active_replacement_locator_policy
            || active_clarify_locator_policy
            || active_implicit_locator_policy)
    {
        return false;
    }

    output_contract.requires_content_evidence
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::Scalar
                | OutputResponseShape::Strict
                | OutputResponseShape::FileToken
        )
        || output_semantic_kind_requires_fresh_evidence(output_contract.semantic_kind)
}

fn sanitize_resolved_intent_for_current_turn_locator(
    resolved_user_intent: &str,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Option<String> {
    if !req_surface.has_concrete_locator_hint()
        || req_surface.has_explicit_path_or_url()
        || crate::worker::has_explicit_path_or_url_locator_hint(req)
    {
        return None;
    }
    let req_lower = req.to_ascii_lowercase();
    let resolved_introduced_path =
        crate::worker::has_explicit_path_or_url_locator_hint(resolved_user_intent)
            || crate::delivery_utils::extract_filename_candidates(resolved_user_intent)
                .into_iter()
                .any(|candidate| !req_lower.contains(&candidate.to_ascii_lowercase()));
    if !resolved_introduced_path {
        return None;
    }
    let trimmed_req = req.trim();
    if trimmed_req.is_empty() {
        return None;
    }
    Some(trimmed_req.to_string())
}

fn normalize_compare_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn locator_hint_compare_path(locator_hint: &str, workspace_root: &Path) -> Option<PathBuf> {
    let hint = locator_hint.trim();
    if hint.is_empty()
        || hint.contains('\n')
        || hint.contains('|')
        || locator_hint_looks_like_multi_target_list(hint, workspace_root)
        || hint.contains("->")
        || hint.starts_with("http://")
        || hint.starts_with("https://")
    {
        return None;
    }
    let path = Path::new(hint);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    Some(normalize_compare_path(path))
}

fn locator_hint_looks_like_multi_target_list(hint: &str, workspace_root: &Path) -> bool {
    if serde_json::from_str::<serde_json::Value>(hint)
        .ok()
        .and_then(|value| value.as_array().map(|items| items.len() > 1))
        .unwrap_or(false)
    {
        return true;
    }

    if !hint.contains(',') && !hint.contains(';') && !hint.contains('、') {
        return false;
    }

    let path = Path::new(hint);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    if path.exists() {
        return false;
    }

    hint.split([',', ';', '、'])
        .filter(|part| !part.trim().is_empty())
        .take(2)
        .count()
        > 1
}

fn first_compare_path_from_text(text: &str, workspace_root: &Path) -> Option<PathBuf> {
    let locator = crate::intent::locator_extractor::extract_explicit_locator_for_fallback(text)?;
    locator_hint_compare_path(&locator.locator_hint, workspace_root)
}

fn compare_path_targets_current_anchor(candidate: &Path, current_anchor: &Path) -> bool {
    candidate == current_anchor || candidate.starts_with(current_anchor)
}

fn normalizer_target_drifted_from_current_anchor(
    output_contract: &IntentOutputContract,
    resolved_user_intent: &str,
    current_anchor_path: &str,
    workspace_root: &Path,
) -> bool {
    let Some(current_anchor) = locator_hint_compare_path(current_anchor_path, workspace_root)
    else {
        return false;
    };

    let mut saw_model_target = false;
    if let Some(contract_target) =
        locator_hint_compare_path(&output_contract.locator_hint, workspace_root)
    {
        saw_model_target = true;
        if compare_path_targets_current_anchor(&contract_target, &current_anchor) {
            return false;
        }
    }
    if let Some(resolved_target) =
        first_compare_path_from_text(resolved_user_intent, workspace_root)
    {
        saw_model_target = true;
        if compare_path_targets_current_anchor(&resolved_target, &current_anchor) {
            return false;
        }
    }

    saw_model_target
}

fn apply_current_turn_anchor_drift_repair(
    output_contract: &mut IntentOutputContract,
    resolved_user_intent: &str,
    current_anchor_path: &str,
    workspace_root: &Path,
) -> Option<&'static str> {
    if matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::ArchivePack | OutputSemanticKind::ArchiveUnpack
    ) && output_contract.locator_hint.contains('|')
    {
        return None;
    }
    if !normalizer_target_drifted_from_current_anchor(
        output_contract,
        resolved_user_intent,
        current_anchor_path,
        workspace_root,
    ) {
        return None;
    }

    let preserve_file_delivery = output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || matches!(
            output_contract.delivery_intent,
            OutputDeliveryIntent::FileSingle
        );
    let preserve_raw_command =
        output_contract.semantic_kind == OutputSemanticKind::RawCommandOutput;
    let preserve_quantity_comparison =
        output_contract.semantic_kind == OutputSemanticKind::QuantityComparison;

    output_contract.response_shape = if preserve_file_delivery {
        OutputResponseShape::FileToken
    } else if preserve_raw_command {
        output_contract.response_shape
    } else if preserve_quantity_comparison {
        OutputResponseShape::Strict
    } else {
        OutputResponseShape::Free
    };
    output_contract.exact_sentence_count = None;
    output_contract.requires_content_evidence = !preserve_file_delivery;
    output_contract.delivery_required = preserve_file_delivery;
    output_contract.locator_kind = if preserve_raw_command {
        OutputLocatorKind::None
    } else if preserve_quantity_comparison {
        OutputLocatorKind::CurrentWorkspace
    } else {
        OutputLocatorKind::Path
    };
    output_contract.delivery_intent = if preserve_file_delivery {
        OutputDeliveryIntent::FileSingle
    } else {
        OutputDeliveryIntent::None
    };
    output_contract.semantic_kind = if preserve_raw_command {
        OutputSemanticKind::RawCommandOutput
    } else if preserve_quantity_comparison {
        OutputSemanticKind::QuantityComparison
    } else {
        OutputSemanticKind::None
    };
    output_contract.locator_hint = if preserve_raw_command {
        String::new()
    } else if preserve_quantity_comparison {
        workspace_root.display().to_string()
    } else {
        current_anchor_path.trim().to_string()
    };
    output_contract.self_extension = Default::default();
    Some("current_turn_anchor_overrides_contextual_target")
}

fn resolve_current_turn_anchor_path(state: &AppState, req: &str) -> Option<String> {
    match crate::worker::try_resolve_implicit_locator_path(
        state,
        req,
        "",
        OutputLocatorKind::Path,
        None,
    ) {
        Some(crate::worker::LocatorAutoResolution::Direct(path)) => Some(path),
        Some(crate::worker::LocatorAutoResolution::Fuzzy(_)) | None => None,
    }
}

fn current_turn_anchor_drift_repair_allowed(
    first_layer_decision: FirstLayerDecision,
    needs_clarify: bool,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    workspace_root: &Path,
) -> bool {
    if needs_clarify {
        return false;
    }
    if matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::ConfigRiskAssessment
            | OutputSemanticKind::ConfigValidation
            | OutputSemanticKind::ConfigMutation
    ) && !output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    if output_contract.locator_kind == OutputLocatorKind::CurrentWorkspace {
        let hint = output_contract.locator_hint.trim();
        if hint.is_empty() || locator_hint_points_to_workspace_root(hint, workspace_root) {
            return false;
        }
    }
    matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
        || route_has_structured_execution_signal(
            output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        )
}

fn downgrade_executionless_route_to_direct_answer(
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    needs_clarify: bool,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    _execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> Option<&'static str> {
    if needs_clarify || !matches!(first_layer_decision, FirstLayerDecision::PlannerExecute) {
        return None;
    }
    if !matches!(execution_finalize_style, ActFinalizeStyle::ChatWrapped) {
        return None;
    }
    if route_has_structured_execution_signal(
        output_contract,
        wants_file_delivery,
        schedule_kind,
        None,
    ) {
        return None;
    }
    *first_layer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("executionless_route_downgraded_to_direct_answer")
}

fn apply_explicit_command_execution_contract_repair(
    command_runtime: &crate::CommandIntentRuntime,
    current_user_request: &str,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    output_contract: &mut IntentOutputContract,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if crate::agent_engine::explicit_command_segment_for_policy(
        command_runtime,
        current_user_request,
    )
    .is_none()
    {
        return None;
    }
    if matches!(*first_layer_decision, FirstLayerDecision::DirectAnswer)
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
    {
        return None;
    }
    *needs_clarify = false;
    clarify_question.clear();
    output_contract.requires_content_evidence = true;
    output_contract.semantic_kind =
        if output_contract.semantic_kind == OutputSemanticKind::ExecutionFailedStep {
            output_contract.response_shape = OutputResponseShape::Strict;
            OutputSemanticKind::ExecutionFailedStep
        } else {
            OutputSemanticKind::RawCommandOutput
        };
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
    Some("explicit_command_requires_fresh_execution")
}

fn apply_command_payload_contract_repair(
    command_payload_declared: bool,
    output_contract: &mut IntentOutputContract,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !command_payload_declared || output_contract.delivery_required {
        return None;
    }
    if matches!(output_contract.semantic_kind, OutputSemanticKind::None) {
        output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    }
    if !matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::RawCommandOutput
    ) {
        return None;
    }
    output_contract.requires_content_evidence = true;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    *needs_clarify = false;
    clarify_question.clear();
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
    Some("command_payload_requires_raw_output_execution")
}

fn route_has_structured_execution_signal(
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> bool {
    wants_file_delivery
        || !matches!(schedule_kind, ScheduleKind::None)
        || output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !matches!(output_contract.locator_kind, OutputLocatorKind::None)
        || !output_contract.locator_hint.trim().is_empty()
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || !matches!(output_contract.self_extension.mode, SelfExtensionMode::None)
        || !matches!(
            output_contract.self_extension.trigger,
            SelfExtensionTrigger::None
        )
        || output_contract.self_extension.execute_now
        || execution_recipe_hint.is_some_and(|spec| {
            !matches!(
                spec.kind,
                crate::execution_recipe::ExecutionRecipeKind::None
            )
        })
}

fn output_semantic_kind_requires_fresh_evidence(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::RawCommandOutput
            | OutputSemanticKind::ServiceStatus
            | OutputSemanticKind::HiddenEntriesCheck
            | OutputSemanticKind::FileNames
            | OutputSemanticKind::DirectoryNames
            | OutputSemanticKind::DirectoryEntryGroups
            | OutputSemanticKind::FilePaths
            | OutputSemanticKind::DirectoryPurposeSummary
            | OutputSemanticKind::ContentExcerptSummary
            | OutputSemanticKind::ContentPresenceCheck
            | OutputSemanticKind::ExcerptKindJudgment
            | OutputSemanticKind::RecentArtifactsJudgment
            | OutputSemanticKind::WorkspaceProjectSummary
            | OutputSemanticKind::ScalarCount
            | OutputSemanticKind::RecentScalarEqualityCheck
            | OutputSemanticKind::ExecutionFailedStep
            | OutputSemanticKind::GeneratedFileDelivery
            | OutputSemanticKind::ExistenceWithPath
            | OutputSemanticKind::ExistenceWithPathSummary
            | OutputSemanticKind::GitCommitSubject
            | OutputSemanticKind::GitRepositoryState
            | OutputSemanticKind::StructuredKeys
            | OutputSemanticKind::ConfigValidation
            | OutputSemanticKind::ConfigMutation
            | OutputSemanticKind::ConfigRiskAssessment
            | OutputSemanticKind::SqliteTableListing
            | OutputSemanticKind::SqliteTableNamesOnly
            | OutputSemanticKind::SqliteDatabaseKindJudgment
            | OutputSemanticKind::SqliteSchemaVersion
            | OutputSemanticKind::WeatherQuery
            | OutputSemanticKind::MarketQuote
            | OutputSemanticKind::ImageUnderstanding
            | OutputSemanticKind::PublishingPreview
            | OutputSemanticKind::PackageManagerDetection
            | OutputSemanticKind::ArchiveList
            | OutputSemanticKind::ArchiveRead
            | OutputSemanticKind::ArchivePack
            | OutputSemanticKind::ArchiveUnpack
            | OutputSemanticKind::DockerPs
            | OutputSemanticKind::DockerImages
            | OutputSemanticKind::DockerLogs
            | OutputSemanticKind::DockerContainerLifecycle
    )
}

fn parse_execution_recipe_hint(
    out: Option<IntentExecutionRecipeOut>,
) -> Option<crate::execution_recipe::ExecutionRecipeSpec> {
    // 关键语义（B1 修复）：
    //   - `out == None`           → normalizer 没在响应里给出 execution_recipe 字段，
    //                               说明 LLM 没决断；planner-first 主链不再用本地
    //                               keyword detect 代替 LLM 决策。
    //   - `out == Some` 且 kind != none → normalizer 显式给出 ops loop spec，照用。
    //   - `out == Some` 且 kind == none → normalizer 显式说"这不是 ops loop"，
    //                               同样应被信任。返回 Some(default spec)（kind=None,
    //                               runtime.is_active()=false），让下游知道 normalizer
    //                               已分类过，不要再被 legacy local detector 误升级。
    //
    // 这块逻辑是为了修复 act 类只读任务（如 `pwd`）被长期记忆里残留的
    // "configs/" "verify" 等关键字误升级为 OpsClosedLoop config_change，
    // 导致 plan 校验拒绝纯只读 plan、走完 max_repairs 后失败的问题。
    let raw = out?;
    let kind = crate::execution_recipe::parse_execution_recipe_kind_text(&raw.kind);
    let profile = crate::execution_recipe::parse_execution_recipe_profile_text(&raw.profile);
    let target_scope =
        crate::execution_recipe::parse_execution_recipe_target_scope_text(&raw.target_scope);
    if matches!(kind, crate::execution_recipe::ExecutionRecipeKind::None) {
        return Some(crate::execution_recipe::ExecutionRecipeSpec::default());
    }
    crate::execution_recipe::explicit_execution_recipe_spec(kind, profile, target_scope)
        .or_else(|| Some(crate::execution_recipe::ExecutionRecipeSpec::default()))
}

fn active_primary_task_prompt<'a>(
    session_snapshot: Option<&'a crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<&'a str> {
    session_snapshot
        .and_then(|snapshot| snapshot.conversation_state.as_ref())
        .and_then(|state| state.last_primary_task_prompt.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn active_clarify_locator_task_prompt<'a>(
    session_snapshot: Option<&'a crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<&'a str> {
    session_snapshot
        .and_then(|snapshot| snapshot.active_clarify_state.as_ref())
        .filter(|state| {
            matches!(
                state.missing_slot,
                crate::clarify_state::ClarifyMissingSlot::Locator
            )
        })
        .filter(|state| {
            state.delivery_required
                || state.output_shape.is_some()
                || state.semantic_kind.is_some()
                || !state.candidate_targets.is_empty()
        })
        .map(|state| state.source_request.trim())
        .filter(|value| !value.is_empty())
}

fn active_observable_task_prompt<'a>(
    session_snapshot: Option<&'a crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<&'a str> {
    active_primary_task_prompt(session_snapshot)
        .or_else(|| active_clarify_locator_task_prompt(session_snapshot))
}

fn prompt_has_concrete_fileish_cue(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_explicit_path_or_url()
        || surface.field_selector_count > 0
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference()
}

fn active_task_turn_can_reuse_semantic_patch(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    state_patch: Option<&Value>,
) -> bool {
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return false;
    }
    !prompt_has_concrete_fileish_cue(surface)
        && !surface.is_structural_locator_only_reply()
        && surface.inline_json_shape.is_none()
}

fn active_prompt_surface_has_structured_execution_target(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_explicit_path_or_url()
        || surface.locator_target_pair.is_some()
        || surface.has_structured_target_refinement()
        || surface.inline_json_shape.is_some()
        || surface.has_delivery_token_reference()
}

fn active_followup_frame_has_structured_target(
    frame: &crate::followup_frame::FollowupFrame,
) -> bool {
    let has_bound_target = frame
        .bound_target
        .as_deref()
        .map(str::trim)
        .is_some_and(|target| !target.is_empty());
    if has_bound_target
        && matches!(
            frame.op_kind,
            crate::followup_frame::FollowupOpKind::Read
                | crate::followup_frame::FollowupOpKind::List
                | crate::followup_frame::FollowupOpKind::Delivery
                | crate::followup_frame::FollowupOpKind::ClarifyPending
        )
    {
        return true;
    }
    frame.selected_entry_index.is_some() || frame.slice_spec.is_some()
}

fn active_observed_facts_have_structured_target(
    facts: &crate::observed_facts::ObservedFacts,
) -> bool {
    facts
        .bound_target
        .as_deref()
        .map(str::trim)
        .is_some_and(|target| !target.is_empty())
        || !facts.delivery_targets.is_empty()
        || facts.selected_entry_index.is_some()
        || facts.observed_entry_count.is_some()
        || facts.slice_spec.is_some()
}

fn active_session_has_structured_execution_target(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let Some(snapshot) = session_snapshot else {
        return false;
    };
    if let Some(active_prompt) = active_primary_task_prompt(Some(snapshot)) {
        let active_surface = crate::intent::surface_signals::analyze_prompt_surface(active_prompt);
        if active_prompt_surface_has_structured_execution_target(&active_surface) {
            return true;
        }
    }
    snapshot
        .active_followup_frame
        .as_ref()
        .is_some_and(active_followup_frame_has_structured_target)
        || snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(active_observed_facts_have_structured_target)
}

fn active_session_has_ordered_entries(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let Some(snapshot) = session_snapshot else {
        return false;
    };
    snapshot
        .active_followup_frame
        .as_ref()
        .is_some_and(|frame| !frame.ordered_entries.is_empty())
        || snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(|facts| !facts.ordered_entries.is_empty())
}

fn active_session_has_recent_primary_output(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    session_snapshot
        .and_then(|snapshot| snapshot.conversation_state.as_ref())
        .and_then(|state| state.last_primary_task_output.as_deref())
        .map(str::trim)
        .is_some_and(|output| !output.is_empty())
}

fn contract_locator_matches_active_observation(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    output_contract: &IntentOutputContract,
) -> bool {
    let locator_hint = output_contract.locator_hint.trim();
    if locator_hint.is_empty() {
        return true;
    }
    let Some(snapshot) = session_snapshot else {
        return false;
    };
    let mut values = Vec::new();
    if let Some(frame) = snapshot.active_followup_frame.as_ref() {
        if let Some(target) = frame.bound_target.as_deref() {
            values.push(target.trim());
        }
        values.extend(frame.ordered_entries.iter().map(|value| value.trim()));
    }
    if let Some(facts) = snapshot.active_observed_facts.as_ref() {
        if let Some(target) = facts.bound_target.as_deref() {
            values.push(target.trim());
        }
        values.extend(facts.ordered_entries.iter().map(|value| value.trim()));
        values.extend(facts.delivery_targets.iter().map(|value| value.trim()));
    }
    values
        .into_iter()
        .filter(|value| !value.is_empty())
        .any(|value| {
            value == locator_hint
                || std::path::Path::new(value)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == locator_hint)
                || std::path::Path::new(locator_hint)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == value)
        })
}

fn state_patch_has_ordered_entry_ref(state_patch: Option<&Value>) -> bool {
    state_patch.is_some_and(|patch| {
        patch.get("ordered_entry_ref").is_some() || patch.get("ordered_entry_reference").is_some()
    })
}

fn apply_active_ordered_scalar_path_chat_repair(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    state_patch: Option<&Value>,
    answer_candidate: &str,
    needs_clarify: bool,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    output_contract: &mut IntentOutputContract,
) -> Option<&'static str> {
    if needs_clarify
        || !matches!(first_layer_decision, FirstLayerDecision::DirectAnswer)
        || !answer_candidate.trim().is_empty()
        || !active_session_has_ordered_entries(session_snapshot)
        || state_patch_has_ordered_entry_ref(state_patch)
        || output_contract.response_shape != OutputResponseShape::Scalar
        || output_contract.semantic_kind != OutputSemanticKind::ScalarPathOnly
        || output_contract.locator_kind != OutputLocatorKind::None
        || !output_contract.locator_hint.trim().is_empty()
        || output_contract.delivery_required
        || output_contract.delivery_intent != OutputDeliveryIntent::None
    {
        return None;
    }
    output_contract.response_shape = OutputResponseShape::Strict;
    output_contract.requires_content_evidence = false;
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    *first_layer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("active_ordered_scalar_path_chat_repair_without_structured_ref")
}

fn apply_active_observed_output_chat_repair(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    should_refresh_long_term_memory: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    wants_file_delivery: bool,
    answer_candidate: &str,
    needs_clarify: bool,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    output_contract: &mut IntentOutputContract,
) -> Option<&'static str> {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let current_turn_has_concrete_target = prompt_has_concrete_fileish_cue(&surface)
        || surface.inline_json_shape.is_some()
        || !surface
            .filename_candidates_excluding_field_selectors()
            .is_empty();
    if attachment_processing_required
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || execution_recipe_hint.is_some()
        || wants_file_delivery
        || needs_clarify
        || !matches!(
            first_layer_decision,
            FirstLayerDecision::DirectAnswer | FirstLayerDecision::PlannerExecute
        )
        || !matches!(
            turn_type,
            None | Some(TurnType::TaskRequest | TurnType::TaskScopeUpdate)
        )
        || !matches!(
            target_task_policy,
            None | Some(TargetTaskPolicy::ReuseActive)
        )
        || !answer_candidate.trim().is_empty()
        || !active_session_has_structured_execution_target(session_snapshot)
        || !active_session_has_recent_primary_output(session_snapshot)
        || current_turn_has_concrete_target
        || !output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None
                | OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
        || !contract_locator_matches_active_observation(session_snapshot, output_contract)
        || output_contract.delivery_intent != OutputDeliveryIntent::None
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::None
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::ContentExcerptWithSummary
                | OutputSemanticKind::ExcerptKindJudgment
        )
    {
        return None;
    }

    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    *first_layer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("active_observed_output_chat_repair")
}

fn unresolved_deictic_observable_target_should_clarify(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if state_patch_deictic_reference_requires_clarify(state_patch) {
        return true;
    }
    if state_patch_deictic_reference_is_resolved(state_patch) {
        return false;
    }
    if surface.has_deictic_reference()
        && output_contract.requires_content_evidence
        && !surface.has_explicit_path_or_url()
    {
        return true;
    }
    false
}

fn should_resolve_task_scope_update_clarify_with_active_task(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if attachment_processing_required
        || !matches!(first_layer_decision, FirstLayerDecision::Clarify)
        || active_primary_task_prompt(session_snapshot).is_none()
        || !matches!(turn_type, Some(TurnType::TaskScopeUpdate))
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch) {
        return false;
    }
    active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
}

fn should_resolve_task_append_clarify_with_active_task(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if attachment_processing_required
        || !matches!(first_layer_decision, FirstLayerDecision::Clarify)
        || active_primary_task_prompt(session_snapshot).is_none()
        || !matches!(
            turn_type,
            Some(TurnType::TaskAppend | TurnType::TaskCorrect | TurnType::TaskScopeUpdate)
        )
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch) {
        return false;
    }
    active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
}

fn should_resolve_task_replace_clarify_with_active_task(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if attachment_processing_required
        || !matches!(first_layer_decision, FirstLayerDecision::Clarify)
        || active_primary_task_prompt(session_snapshot).is_none()
        || !matches!(turn_type, Some(TurnType::TaskReplace))
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReplaceActive))
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch) {
        return false;
    }
    active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
}

fn should_route_active_task_mutation_to_direct_answer(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    state_patch: Option<&Value>,
) -> bool {
    if attachment_processing_required
        || !matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
        || active_primary_task_prompt(session_snapshot).is_none()
        || !output_contract_allows_chat_only_task_mutation(output_contract)
    {
        return false;
    }
    let turn_type = match turn_type {
        Some(value) => value,
        None => return false,
    };
    let target_task_policy = match target_task_policy {
        Some(value) => value,
        None => return false,
    };
    if !matches!(
        turn_type,
        TurnType::TaskAppend
            | TurnType::TaskCorrect
            | TurnType::TaskReplace
            | TurnType::TaskScopeUpdate
    ) {
        return false;
    }
    if !matches!(
        target_task_policy,
        TargetTaskPolicy::ReuseActive | TargetTaskPolicy::ReplaceActive
    ) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
}

fn state_patch_has_semantic_update(state_patch: Option<&Value>) -> bool {
    let Some(Value::Object(map)) = state_patch else {
        return false;
    };
    !map.is_empty() && map.values().any(is_meaningful_state_patch)
}

fn prompt_has_concrete_locator_for_patch_repair(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_explicit_path_or_url()
        || surface.locator_target_pair.is_some()
        || surface.field_selector_count > 0
        || surface.dotted_field_selector.is_some()
        || surface.has_delivery_token_reference()
        || !surface
            .filename_candidates_excluding_field_selectors()
            .is_empty()
}

fn active_primary_text_context(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> Option<(&str, Option<&str>)> {
    let state = session_snapshot.and_then(|snapshot| snapshot.conversation_state.as_ref())?;
    let prompt = state
        .last_primary_task_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let output = state
        .last_primary_task_output
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !crate::finalize::is_execution_summary_message(value));
    Some((prompt, output))
}

fn active_text_patch_locator_context_is_safe(
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    output_contract: &IntentOutputContract,
) -> bool {
    if output_contract.locator_hint.trim().is_empty() {
        return true;
    }
    matches!(
        (turn_type, target_task_policy),
        (
            Some(
                TurnType::TaskAppend
                    | TurnType::TaskCorrect
                    | TurnType::TaskReplace
                    | TurnType::TaskScopeUpdate
            ),
            Some(TargetTaskPolicy::ReuseActive | TargetTaskPolicy::ReplaceActive)
        )
    )
}

fn apply_active_task_structured_patch_repair(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: &mut Option<TurnType>,
    target_task_policy: &mut Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    needs_clarify: &mut bool,
    schedule_kind: ScheduleKind,
    should_refresh_long_term_memory: bool,
    output_contract: &mut IntentOutputContract,
    state_patch: Option<&Value>,
) -> Option<&'static str> {
    active_primary_text_context(session_snapshot)?;
    if attachment_processing_required
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || crate::conversation_state::state_patch_is_alias_bindings_only(state_patch?)
        || !state_patch_has_semantic_update(state_patch)
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        || !active_text_patch_locator_context_is_safe(
            *turn_type,
            *target_task_policy,
            output_contract,
        )
    {
        return None;
    }

    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if prompt_has_concrete_locator_for_patch_repair(&surface)
        || unresolved_deictic_observable_target_should_clarify(
            &surface,
            output_contract,
            state_patch,
        )
        || !active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
    {
        return None;
    }

    if !matches!(
        turn_type,
        None | Some(TurnType::TaskRequest | TurnType::TaskCorrect | TurnType::TaskAppend)
    ) || !matches!(
        target_task_policy,
        None | Some(TargetTaskPolicy::Standalone | TargetTaskPolicy::ReuseActive)
    ) {
        return None;
    }

    *turn_type = Some(TurnType::TaskCorrect);
    *target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    *needs_clarify = false;
    *first_layer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_hint.clear();
    Some("active_task_structured_patch_repair")
}

fn apply_active_task_scope_refinement_repair(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: &mut Option<TurnType>,
    target_task_policy: &mut Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    needs_clarify: &mut bool,
    schedule_kind: ScheduleKind,
    should_refresh_long_term_memory: bool,
    output_contract: &mut IntentOutputContract,
    state_patch: Option<&Value>,
) -> Option<&'static str> {
    if attachment_processing_required
        || active_primary_task_prompt(session_snapshot).is_none()
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || !matches!(turn_type, None | Some(TurnType::TaskRequest))
        || !matches!(
            target_task_policy,
            None | Some(TargetTaskPolicy::Standalone)
        )
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
    {
        return None;
    }

    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch) {
        return None;
    }
    if !active_task_turn_can_reuse_semantic_patch(&surface, state_patch) {
        return None;
    }

    let unresolved_observation_missing_locator = *needs_clarify
        && output_contract.requires_content_evidence
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None);
    if unresolved_observation_missing_locator {
        return None;
    }

    let standalone_observation_without_missing_slot = !*needs_clarify
        && output_contract.requires_content_evidence
        && !output_contract.delivery_required;
    if standalone_observation_without_missing_slot {
        return None;
    }

    let model_lifted_prompt_into_execution_target = matches!(
        first_layer_decision,
        FirstLayerDecision::Clarify | FirstLayerDecision::PlannerExecute
    ) && (output_contract
        .requires_content_evidence
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        || !matches!(output_contract.semantic_kind, OutputSemanticKind::None));

    if !*needs_clarify && !model_lifted_prompt_into_execution_target {
        return None;
    }

    let repair_reason = if active_session_has_structured_execution_target(session_snapshot) {
        *turn_type = None;
        *target_task_policy = None;
        "active_task_scope_refinement_detached_from_structured_anchor"
    } else {
        *turn_type = Some(TurnType::TaskScopeUpdate);
        *target_task_policy = Some(TargetTaskPolicy::ReuseActive);
        "active_task_scope_refinement_repair"
    };
    *needs_clarify = false;
    *first_layer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_hint.clear();
    Some(repair_reason)
}

fn output_contract_allows_chat_only_task_mutation(output_contract: &IntentOutputContract) -> bool {
    !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
}

fn clear_output_contract_for_active_text_followup(output_contract: &mut IntentOutputContract) {
    output_contract.requires_content_evidence = false;
    output_contract.delivery_required = false;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_hint.clear();
}

fn output_contract_looks_like_contextual_text_followup(
    output_contract: &IntentOutputContract,
) -> bool {
    let contextual_semantic = matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        || semantic_kind_can_use_existing_observed_context(output_contract.semantic_kind);
    !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        && contextual_semantic
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
}

fn active_context_has_structured_observation_anchor(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let Some(snapshot) = session_snapshot else {
        return false;
    };

    let followup_anchor = snapshot
        .active_followup_frame
        .as_ref()
        .is_some_and(|frame| {
            matches!(
                frame.op_kind,
                crate::followup_frame::FollowupOpKind::Read
                    | crate::followup_frame::FollowupOpKind::List
            ) || frame
                .bound_target
                .as_deref()
                .is_some_and(|target| !target.trim().is_empty())
                || !frame.ordered_entries.is_empty()
                || frame.selected_entry_index.is_some()
                || frame.slice_spec.is_some()
        });
    if followup_anchor {
        return true;
    }

    snapshot
        .active_observed_facts
        .as_ref()
        .is_some_and(|facts| {
            facts
                .bound_target
                .as_deref()
                .is_some_and(|target| !target.trim().is_empty())
                || !facts.ordered_entries.is_empty()
                || facts.selected_entry_index.is_some()
                || facts.observed_entry_count.is_some()
                || facts.slice_spec.is_some()
        })
}

fn answer_candidate_can_conflict_with_active_text_followup(
    binding: Option<&AnswerCandidateBindingReport>,
) -> bool {
    binding.is_some_and(|binding| {
        binding.is_distinctive()
            && !binding.in_current_request
            && (binding.in_recent_assistant_replies
                || binding.in_recent_turns_full
                || binding.in_last_turn_full
                || binding.in_memory_context)
    })
}
fn apply_active_text_followup_route_repair(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: &mut Option<TurnType>,
    target_task_policy: &mut Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    needs_clarify: &mut bool,
    schedule_kind: ScheduleKind,
    should_refresh_long_term_memory: bool,
    wants_file_delivery: &mut bool,
    output_contract: &mut IntentOutputContract,
    state_patch: Option<&Value>,
    current_request_has_runtime_locator_anchor: bool,
    semantic_active_text_candidate_repair: bool,
    answer_candidate: &mut String,
) -> Option<&'static str> {
    let (_, active_primary_output) = active_primary_text_context(session_snapshot)?;
    if attachment_processing_required
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || *wants_file_delivery
        || !output_contract_looks_like_contextual_text_followup(output_contract)
    {
        return None;
    }

    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let existing_output_can_satisfy_contextual_evidence = active_primary_output.is_some()
        && semantic_kind_can_use_existing_observed_context(output_contract.semantic_kind);
    let unresolved_deictic_needs_clarify =
        unresolved_deictic_observable_target_should_clarify(&surface, output_contract, state_patch)
            && !existing_output_can_satisfy_contextual_evidence;
    if prompt_has_concrete_locator_for_patch_repair(&surface)
        || current_request_has_runtime_locator_anchor
        || unresolved_deictic_needs_clarify
        || !active_task_turn_can_reuse_semantic_patch(&surface, state_patch)
    {
        return None;
    }

    let model_already_bound_active_task = matches!(
        (*turn_type, *target_task_policy),
        (
            Some(
                TurnType::TaskAppend
                    | TurnType::TaskCorrect
                    | TurnType::TaskReplace
                    | TurnType::TaskScopeUpdate
            ),
            Some(TargetTaskPolicy::ReuseActive | TargetTaskPolicy::ReplaceActive)
        )
    );
    let stale_contextual_evidence = output_contract.requires_content_evidence
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        );
    if stale_contextual_evidence
        && active_context_has_structured_observation_anchor(session_snapshot)
        && !existing_output_can_satisfy_contextual_evidence
    {
        return None;
    }
    let stale_scalar_candidate = semantic_active_text_candidate_repair;

    if !(model_already_bound_active_task || stale_contextual_evidence || stale_scalar_candidate) {
        return None;
    }

    if !matches!(
        turn_type,
        None | Some(
            TurnType::TaskRequest
                | TurnType::TaskAppend
                | TurnType::TaskCorrect
                | TurnType::TaskReplace
                | TurnType::TaskScopeUpdate
                | TurnType::PreferenceOrMemory
        )
    ) || !matches!(
        target_task_policy,
        None | Some(
            TargetTaskPolicy::Standalone
                | TargetTaskPolicy::ReuseActive
                | TargetTaskPolicy::ReplaceActive
        )
    ) {
        return None;
    }

    if !matches!(*target_task_policy, Some(TargetTaskPolicy::ReplaceActive))
        && !matches!(*turn_type, Some(TurnType::TaskReplace))
    {
        *target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    }
    if turn_type.is_none()
        || matches!(
            *turn_type,
            Some(TurnType::TaskRequest | TurnType::PreferenceOrMemory)
        )
    {
        *turn_type = Some(if stale_scalar_candidate {
            TurnType::TaskScopeUpdate
        } else {
            TurnType::TaskCorrect
        });
    }
    *needs_clarify = false;
    *first_layer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    *wants_file_delivery = false;
    answer_candidate.clear();
    clear_output_contract_for_active_text_followup(output_contract);
    Some("active_text_followup_route_repair")
}

fn render_self_extension_runtime(state: &AppState) -> String {
    serde_json::to_string_pretty(&json!({
        "enabled": state.policy.self_extension.enabled,
        "auto_on_capability_gap": state.policy.self_extension.auto_on_capability_gap,
        "allow_execute": state.policy.self_extension.allow_execute,
        "allow_package_install": state.policy.self_extension.allow_package_install,
        "allow_permanent_extension": state.policy.self_extension.allow_permanent_extension,
        "allow_runtime_enable": state.policy.self_extension.allow_runtime_enable,
        "supported_modes": ["temporary_fix", "permanent_extension"],
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn normalizer_output_from_fallback(
    user_request: &str,
    fallback_reason_prefix: &str,
    decision: RouteDecision,
    fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
) -> IntentNormalizerOutput {
    let first_layer_decision = if decision.needs_clarify {
        FirstLayerDecision::Clarify
    } else if route_has_structured_execution_signal(
        &decision.output_contract,
        decision.wants_file_delivery,
        decision.schedule_kind,
        None,
    ) {
        FirstLayerDecision::PlannerExecute
    } else {
        FirstLayerDecision::DirectAnswer
    };
    let mut first_layer_decision = first_layer_decision;
    let mut execution_finalize_style =
        execution_finalize_style_for_contract(&decision.output_contract);
    if let Some(finalize_style) =
        crate::post_route_policy::content_evidence_execution_finalize_style(
            &decision.output_contract,
            decision.needs_clarify,
        )
    {
        first_layer_decision = FirstLayerDecision::PlannerExecute;
        execution_finalize_style = finalize_style;
    }
    let reason = if decision.reason.trim().is_empty() {
        fallback_reason_prefix.to_string()
    } else {
        format!("{fallback_reason_prefix}; {}", decision.reason.trim())
    };
    let resolved_user_intent = if decision.resolved_user_intent.trim().is_empty() {
        user_request.trim().to_string()
    } else {
        decision.resolved_user_intent
    };
    IntentNormalizerOutput {
        resolved_user_intent,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: decision.schedule_kind,
        schedule_intent: decision.schedule_intent,
        wants_file_delivery: decision.wants_file_delivery,
        should_refresh_long_term_memory: decision.should_refresh_long_term_memory,
        agent_display_name_hint: decision.agent_display_name_hint,
        needs_clarify: decision.needs_clarify,
        clarify_question: decision.clarify_question,
        reason,
        confidence: decision.confidence.unwrap_or(0.0),
        output_contract: decision.output_contract,
        execution_recipe_hint: None,
        first_layer_decision,
        execution_finalize_style,
        turn_analysis: None,
        fallback_source,
    }
}

fn normalize_schedule_intent_from_normalizer(
    schedule_kind: ScheduleKind,
    schedule_intent: Option<crate::ScheduleIntentOutput>,
    resolved_user_intent: &str,
    reason: &str,
    needs_clarify: bool,
    clarify_question: &str,
    confidence: f64,
) -> Option<crate::ScheduleIntentOutput> {
    if matches!(schedule_kind, ScheduleKind::None) {
        return None;
    }
    let mut intent = schedule_intent.unwrap_or_default();
    let cleaned_kind = crate::schedule_service::clean_schedule_kind(&intent.kind);
    if !cleaned_kind.is_empty() && cleaned_kind != schedule_kind.as_str() {
        return None;
    }
    if cleaned_kind.is_empty() {
        intent.kind = schedule_kind.as_str().to_string();
    }
    if intent.raw.trim().is_empty() {
        intent.raw = resolved_user_intent.trim().to_string();
    }
    if intent.reason.trim().is_empty() {
        intent.reason = reason.trim().to_string();
    }
    if needs_clarify {
        intent.needs_clarify = true;
        if intent.clarify_question.trim().is_empty() && !clarify_question.trim().is_empty() {
            intent.clarify_question = clarify_question.trim().to_string();
        }
    }
    if intent.confidence <= 0.0 {
        intent.confidence = confidence;
    }
    Some(intent)
}

fn merge_answer_candidate_into_resolved_intent(resolved: String, answer_candidate: &str) -> String {
    let answer = answer_candidate.trim();
    if answer.is_empty() || resolved.contains(answer) {
        return resolved;
    }
    if resolved.trim().is_empty() {
        answer.to_string()
    } else {
        format!("{}\nanswer_candidate: {}", resolved.trim(), answer)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AnswerCandidateBindingReport {
    candidate: String,
    in_current_request: bool,
    in_recent_assistant_replies: bool,
    in_recent_turns_full: bool,
    in_last_turn_full: bool,
    in_recent_execution_context: bool,
    in_memory_context: bool,
}

impl AnswerCandidateBindingReport {
    fn has_current_or_recent_binding(&self) -> bool {
        self.in_current_request
            || self.in_recent_assistant_replies
            || self.in_recent_turns_full
            || self.in_last_turn_full
            || self.in_recent_execution_context
    }

    fn is_memory_only_binding(&self) -> bool {
        self.in_memory_context && !self.has_current_or_recent_binding()
    }

    fn is_distinctive(&self) -> bool {
        answer_candidate_is_distinctive_for_binding(&self.candidate)
    }
}

fn analyze_answer_candidate_binding(
    request: &str,
    answer_candidate: &str,
    route_view: &crate::task_context_builder::RouteContextView,
) -> Option<AnswerCandidateBindingReport> {
    let answer = answer_candidate.trim();
    if answer.is_empty() {
        return None;
    }
    Some(AnswerCandidateBindingReport {
        candidate: answer.to_string(),
        in_current_request: request.contains(answer),
        in_recent_assistant_replies: route_view.recent_assistant_replies.contains(answer),
        in_recent_turns_full: route_view.recent_turns_full.contains(answer),
        in_last_turn_full: route_view.last_turn_full.contains(answer),
        in_recent_execution_context: route_view.recent_execution_context.contains(answer),
        in_memory_context: route_view.memory_context.contains(answer),
    })
}

fn answer_candidate_is_distinctive_for_binding(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    let signal_chars = trimmed
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    let has_identifier_separator = trimmed.contains(['-', '_', '/', ':', '.']);
    signal_chars >= 8 || (signal_chars >= 4 && has_identifier_separator)
}

fn answer_candidate_binding_repair_context(
    report: &AnswerCandidateBindingReport,
    should_refresh_long_term_memory: bool,
) -> String {
    format!(
        "answer_candidate_binding:\n\
         candidate: {}\n\
         should_refresh_long_term_memory: {}\n\
         in_current_request: {}\n\
         in_recent_assistant_replies: {}\n\
         in_recent_turns_full: {}\n\
         in_last_turn_full: {}\n\
         in_recent_execution_context: {}\n\
         in_memory_context: {}\n\
         memory_only_binding: {}\n\
         distinctive_candidate: {}",
        crate::truncate_for_log(&report.candidate),
        should_refresh_long_term_memory,
        report.in_current_request,
        report.in_recent_assistant_replies,
        report.in_recent_turns_full,
        report.in_last_turn_full,
        report.in_recent_execution_context,
        report.in_memory_context,
        report.is_memory_only_binding(),
        report.is_distinctive()
    )
}

fn append_contract_repair_context(context: &mut String, block: String) {
    if block.trim().is_empty() {
        return;
    }
    if context.trim().is_empty() || context == "none" {
        *context = block;
    } else {
        context.push_str("\n\n");
        context.push_str(&block);
    }
}

fn active_text_answer_candidate_conflict_context(
    binding: Option<&AnswerCandidateBindingReport>,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    should_refresh_long_term_memory: bool,
) -> Option<String> {
    if should_refresh_long_term_memory
        || req_surface.has_explicit_path_or_url()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_structured_target_refinement()
        || req_surface.has_delivery_token_reference()
        || req_surface.inline_json_shape.is_some()
    {
        return None;
    }
    let binding = binding?;
    if !answer_candidate_can_conflict_with_active_text_followup(Some(binding)) {
        return None;
    }
    let (prior_prompt, prior_output) = active_primary_text_context(session_snapshot)?;
    if prior_output.is_none() {
        return None;
    }
    Some(format!(
        "active_task_answer_candidate_conflict:\n\
         candidate: {}\n\
         in_recent_assistant_replies: {}\n\
         in_recent_turns_full: {}\n\
         in_last_turn_full: {}\n\
         in_memory_context: {}\n\
         active_task_prompt: {}\n\
         active_task_has_output: {}",
        crate::truncate_for_log(&binding.candidate),
        binding.in_recent_assistant_replies,
        binding.in_recent_turns_full,
        binding.in_last_turn_full,
        binding.in_memory_context,
        crate::truncate_for_log(prior_prompt),
        prior_output.is_some()
    ))
}

fn active_task_invalid_turn_binding_context(
    raw_normalizer_output: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    should_refresh_long_term_memory: bool,
) -> Option<String> {
    if should_refresh_long_term_memory
        || req_surface.has_explicit_path_or_url()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_structured_target_refinement()
        || req_surface.has_delivery_token_reference()
        || req_surface.inline_json_shape.is_some()
    {
        return None;
    }
    let (prior_prompt, prior_output) = active_primary_text_context(session_snapshot)?;
    prior_output?;
    let raw_value =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw_normalizer_output)?;
    let obj = raw_value.as_object()?;
    let raw_turn_type = obj
        .get("turn_type")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let raw_target_task_policy = obj
        .get("target_task_policy")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let turn_type_invalid =
        !raw_turn_type.trim().is_empty() && parse_turn_type(&raw_turn_type).is_none();
    let target_policy_invalid = !raw_target_task_policy.trim().is_empty()
        && parse_target_task_policy(&raw_target_task_policy).is_none();
    if !(turn_type_invalid || target_policy_invalid) {
        return None;
    }
    Some(format!(
        "active_task_invalid_turn_binding:\n\
         raw_turn_type: {}\n\
         raw_target_task_policy: {}\n\
         turn_type_invalid: {}\n\
         target_task_policy_invalid: {}\n\
         active_task_prompt: {}\n\
         active_task_has_output: true",
        crate::truncate_for_log(raw_turn_type.trim()),
        crate::truncate_for_log(raw_target_task_policy.trim()),
        turn_type_invalid,
        target_policy_invalid,
        crate::truncate_for_log(prior_prompt)
    ))
}

fn clear_memory_update_answer_candidate_if_memory_only(
    out: &mut IntentNormalizerOut,
    binding: Option<&AnswerCandidateBindingReport>,
) -> Option<&'static str> {
    if !out.should_refresh_long_term_memory {
        return None;
    }
    let Some(binding) = binding else {
        return None;
    };
    if !binding.is_memory_only_binding() || !binding.is_distinctive() {
        return None;
    }
    out.answer_candidate.clear();
    append_route_reason(
        &mut out.reason,
        "memory_update_unbound_answer_candidate_cleared",
    );
    Some("memory_update_unbound_answer_candidate_cleared")
}

fn recent_distinctive_scalar_conflict_tokens(
    binding: &AnswerCandidateBindingReport,
    route_view: &crate::task_context_builder::RouteContextView,
) -> Vec<String> {
    if !binding.is_memory_only_binding() || !binding.is_distinctive() {
        return Vec::new();
    }
    let mut tokens = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for source in [
        route_view.recent_assistant_replies.as_str(),
        route_view.recent_turns_full.as_str(),
        route_view.last_turn_full.as_str(),
        route_view.recent_execution_context.as_str(),
    ] {
        for token in source.split(|ch: char| {
            !(ch.is_ascii_alphanumeric()
                || ('\u{4e00}'..='\u{9fff}').contains(&ch)
                || matches!(ch, '_' | '-' | '/' | '.' | ':'))
        }) {
            let token = token.trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '.' | ':'));
            if !recent_distinctive_scalar_conflict_token(&binding.candidate, token) {
                continue;
            }
            let normalized = token.to_ascii_lowercase();
            if seen.insert(normalized) {
                tokens.push(token.to_string());
            }
        }
    }
    tokens
}

fn recent_distinctive_scalar_conflict_token(candidate: &str, token: &str) -> bool {
    let token = token.trim();
    if token.is_empty()
        || token.eq_ignore_ascii_case(candidate.trim())
        || token.contains(['/', '\\'])
    {
        return false;
    }
    let signal_chars = token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
    let has_identifier_separator = token.contains(['_', '-', '.', ':']);
    has_digit && ((signal_chars >= 4 && has_identifier_separator) || signal_chars >= 8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DistinctiveMemoryScalarClass {
    LocatorLike,
    DottedVersion,
    StructuredId,
}

fn dotted_version_like_scalar(value: &str) -> bool {
    let trimmed = value.trim().trim_start_matches(['v', 'V']);
    let mut saw_dot = false;
    let mut saw_part = false;
    for part in trimmed.split('.') {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return false;
        }
        saw_part = true;
        saw_dot = true;
    }
    saw_part && saw_dot && trimmed.contains('.')
}

fn locator_like_scalar(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.contains(['/', '\\']) {
        return true;
    }
    if trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    trimmed.contains('.') && !dotted_version_like_scalar(trimmed)
}

fn distinctive_memory_scalar_class(value: &str) -> Option<DistinctiveMemoryScalarClass> {
    let trimmed = value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '\\' | '.' | ':'));
    if trimmed.is_empty() {
        return None;
    }
    if locator_like_scalar(trimmed) {
        return Some(DistinctiveMemoryScalarClass::LocatorLike);
    }
    if dotted_version_like_scalar(trimmed) {
        return Some(DistinctiveMemoryScalarClass::DottedVersion);
    }
    if recent_distinctive_scalar_conflict_token("", trimmed) {
        return Some(DistinctiveMemoryScalarClass::StructuredId);
    }
    None
}

fn pathish_basename(value: &str) -> &str {
    value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '\\' | '.' | ':'))
}

fn memory_recall_rebind_token(
    candidate: &str,
    token: &str,
    candidate_class: DistinctiveMemoryScalarClass,
) -> Option<String> {
    let token = token.trim();
    if token.is_empty() || token.eq_ignore_ascii_case(candidate.trim()) {
        return None;
    }
    let token_class = distinctive_memory_scalar_class(token)?;
    if token_class != candidate_class {
        return None;
    }
    match candidate_class {
        DistinctiveMemoryScalarClass::LocatorLike => {
            let candidate_base = pathish_basename(candidate);
            let token_base = pathish_basename(token);
            if token_base.is_empty() || token_base.eq_ignore_ascii_case(candidate_base) {
                return None;
            }
            if candidate.contains(['/', '\\']) {
                Some(token.to_string())
            } else {
                Some(token_base.to_string())
            }
        }
        DistinctiveMemoryScalarClass::DottedVersion
        | DistinctiveMemoryScalarClass::StructuredId => {
            if recent_distinctive_scalar_conflict_token(candidate, token) {
                Some(token.to_string())
            } else {
                None
            }
        }
    }
}

fn clear_memory_only_answer_candidate_if_recent_context_conflicts(
    out: &mut IntentNormalizerOut,
    binding: Option<&AnswerCandidateBindingReport>,
    route_view: &crate::task_context_builder::RouteContextView,
) -> Option<&'static str> {
    let binding = binding?;
    if recent_distinctive_scalar_conflict_tokens(binding, route_view).is_empty() {
        return None;
    }
    out.answer_candidate.clear();
    append_route_reason(
        &mut out.reason,
        "memory_only_answer_candidate_recent_scalar_conflict_cleared",
    );
    Some("memory_only_answer_candidate_recent_scalar_conflict_cleared")
}

fn latest_user_memory_distinctive_scalar_candidate(
    state: &AppState,
    task: &ClaimedTask,
    binding: &AnswerCandidateBindingReport,
) -> Option<String> {
    if !binding.is_memory_only_binding() || !binding.is_distinctive() {
        return None;
    }
    let candidate_class = distinctive_memory_scalar_class(&binding.candidate)?;
    let user_key = task.user_key.as_deref().unwrap_or_default().trim();
    let db = state.core.db.get().ok()?;
    let mut stmt = db
        .prepare(
            "SELECT search_text
             FROM memory_retrieval_index
             WHERE source_kind = ?1
               AND user_id = ?2
               AND (?3 = '' OR COALESCE(user_key, '') = ?3)
               AND memory_kind IN (?4, ?5, ?6)
             ORDER BY COALESCE(updated_at_ts, created_at_ts, 0) DESC, id DESC
             LIMIT 64",
        )
        .ok()?;
    let rows = stmt
        .query_map(
            params![
                crate::memory::RETRIEVAL_SOURCE_MEMORY,
                task.user_id,
                user_key,
                crate::memory::RETRIEVAL_KIND_ASSISTANT_RESULT,
                crate::memory::RETRIEVAL_KIND_TRIGGER_ANCHOR,
                crate::memory::RETRIEVAL_KIND_EPISODIC_EVENT,
            ],
            |row| row.get::<_, String>(0),
        )
        .ok()?;
    for row in rows.flatten() {
        for token in
            distinctive_scalar_tokens_for_memory_recall(&binding.candidate, candidate_class, &row)
        {
            return Some(token);
        }
    }
    None
}

fn distinctive_scalar_tokens_for_memory_recall(
    candidate: &str,
    candidate_class: DistinctiveMemoryScalarClass,
    text: &str,
) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for token in text.split(|ch: char| {
        !(ch.is_ascii_alphanumeric()
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || matches!(ch, '_' | '-' | '/' | '.' | ':'))
    }) {
        let token = token.trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '.' | ':'));
        let Some(rebind_token) = memory_recall_rebind_token(candidate, token, candidate_class)
        else {
            continue;
        };
        let normalized = rebind_token.to_ascii_lowercase();
        if seen.insert(normalized) {
            tokens.push(rebind_token);
        }
    }
    tokens
}

fn rebind_memory_only_answer_candidate_to_recent_user_memory(
    state: &AppState,
    task: &ClaimedTask,
    out: &mut IntentNormalizerOut,
    binding: Option<&AnswerCandidateBindingReport>,
) -> Option<&'static str> {
    let binding = binding?;
    let candidate = latest_user_memory_distinctive_scalar_candidate(state, task, binding)?;
    out.answer_candidate = candidate;
    out.decision = "direct_answer".to_string();
    out.needs_clarify = false;
    out.clarify_question.clear();
    append_route_reason(
        &mut out.reason,
        "memory_only_answer_candidate_rebound_to_recent_user_memory",
    );
    Some("memory_only_answer_candidate_rebound_to_recent_user_memory")
}

fn intent_normalizer_max_prompt_bytes(state: &AppState, task: &ClaimedTask) -> usize {
    let providers = state.task_llm_providers(task);
    if providers.is_empty() {
        return 192 * 1024;
    }
    let min_tokens = providers
        .iter()
        .map(|provider| crate::memory::service::estimate_context_window_tokens(provider.as_ref()))
        .min()
        .unwrap_or(32_000);
    if min_tokens <= 4_096 {
        return min_tokens.saturating_mul(2).clamp(2_048, 3_300);
    }
    min_tokens
        .saturating_sub(1_400)
        .max(512)
        .saturating_mul(2)
        .min(512 * 1024)
        .max(2_048)
}

fn intent_normalizer_uses_compact_prompt(state: &AppState, task: &ClaimedTask) -> bool {
    state
        .task_llm_providers(task)
        .iter()
        .map(|provider| crate::memory::service::estimate_context_window_tokens(provider.as_ref()))
        .min()
        .is_some_and(|tokens| tokens <= 4_096)
}

fn compact_prompt_slot(label: &str, value: &str, max_bytes: usize) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "<none>" {
        return format!("{label}: <none>");
    }
    if trimmed.len() <= max_bytes {
        let visible = crate::providers::utf8_safe_prefix(trimmed, max_bytes);
        format!("{label}: {visible}")
    } else {
        let marker = "\n...<snip>...\n";
        if max_bytes <= marker.len().saturating_add(16) {
            let visible = crate::providers::utf8_safe_prefix(trimmed, max_bytes);
            return format!("{label}: {visible}...(truncated)");
        }
        let content_budget = max_bytes.saturating_sub(marker.len());
        let head_budget = content_budget / 2;
        let tail_budget = content_budget.saturating_sub(head_budget);
        let head = crate::providers::utf8_safe_prefix(trimmed, head_budget);
        let tail = crate::providers::utf8_safe_suffix(trimmed, tail_budget);
        format!("{label}: {head}{marker}{tail}")
    }
}

fn compact_runtime_context_from_auth(auth_policy_context: &str) -> String {
    let mut lines = Vec::new();
    for line in auth_policy_context.lines().map(str::trim) {
        if line.starts_with("current_process_cwd:") || line.starts_with("workspace_root:") {
            lines.push(line.to_string());
        }
    }
    if lines.is_empty() {
        "<none>".to_string()
    } else {
        format!("### RUNTIME_CONTEXT\n{}", lines.join("\n"))
    }
}

fn render_compact_intent_normalizer_prompt(
    route_view: &crate::task_context_builder::RouteContextView,
    context_bundle: &crate::task_context_builder::TaskContextBundle,
    auth_policy_context: &str,
    request_language_hint: &str,
    req: &str,
) -> String {
    let mut parts = Vec::new();
    parts.push(
        "Compact intent normalizer. Output exactly one raw JSON object and then stop. No markdown, no answer text after JSON.".to_string(),
    );
    parts.push("Normalizer protocol is internal only: the raw-JSON/no-markdown requirement applies only to this classifier response. Never treat it as a user-visible output-format limit; preserve requested final formats such as markdown tables or CSV in the route contract/resolved intent.".to_string());
    parts.push("Always include all top-level schema keys: resolved_user_intent, answer_candidate, resume_behavior, schedule_kind, schedule_intent, wants_file_delivery, should_refresh_long_term_memory, agent_display_name_hint, needs_clarify, clarify_question, reason, confidence, decision, output_contract, execution_recipe, turn_type, target_task_policy, should_interrupt_active_run, state_patch, attachment_processing_required.".to_string());
    parts.push("Use decision as the only first-layer semantic gate: clarify, direct_answer, planner_execute.".to_string());
    parts.push("Prefer decision=direct_answer for greetings, confirmations, memory-only requests, and pure discussion. Use decision=clarify only when a required target/action is truly missing.".to_string());
    parts.push("High-priority: if REQUEST asks to summarize, explain, conclude, judge, or state what a current topic/test/conversation mainly verifies or means, do not treat a prior exact ID/value as the answer. Keep answer_candidate empty unless REQUEST explicitly asks for that scalar, and copy any relevant recent background/goals/purpose into resolved_user_intent. In MEMORY lists, leading decimal numbers are retrieval scores, never user facts or answer candidates.".to_string());
    parts.push("If ACTIVE_TASK is <none>, do not use task_append, task_correct, or task_scope_update. Classify a fresh user goal as task_request or leave turn_type empty for pure chat/memory/status turns.".to_string());
    parts.push("A complete current REQUEST with its own deliverable, topic, object, audience, scope, or factual constraints is standalone unless it semantically modifies the active deliverable. Shared chat identity, similar task type, same product name, or nearby memory is not enough to merge independent tasks.".to_string());
    parts.push("If ACTIVE_TASK or LAST shows an active writing/drafting/planning task and REQUEST only adds audience, tone, length, body-only, wording, count, format, scope, or presentation constraints, keep it attached: turn_type=\"task_append\", target_task_policy=\"reuse_active\", decision=\"direct_answer\", execution_recipe.kind=\"none\", requires_content_evidence=false, locator_kind=\"none\". Do not route such presentation-only follow-ups to planner_execute unless the REQUEST explicitly requires fresh local/system/file/web evidence.".to_string());
    parts.push("If ACTIVE_TASK/LAST shows a low-risk writing/drafting/planning clarification was already asked and REQUEST adds more constraints without answering every optional detail, prefer a best-effort generic draft over repeating clarification: decision=\"direct_answer\", turn_type=\"task_append\" or \"task_scope_update\", target_task_policy=\"reuse_active\", no evidence/delivery.".to_string());
    parts.push("ACTIVE_TASK_PATCH: for active-task corrections/refinements, put exact current-turn content values that must remain visible in state_patch.required_content_literals. For concrete visible replacements, set state_patch.replacement_pairs=[{\"from\":\"old literal\",\"to\":\"new literal\"}] and state_patch.forbidden_visible_literals for old/rejected literals that must disappear. Use exact content literals from the request; do not include generic output-control wording, length limits, body-only/output-only constraints, tone, count, or format instructions.".to_string());
    parts.push("Do not treat a bare acknowledgement request as active-task output refinement. If REQUEST only asks for a short acknowledgement/confirmation and does not explicitly reference ACTIVE_TASK/LAST/the prior answer/result/rewrite target, use standalone chat with no state_patch; answer the acknowledgement itself, not the active task output.".to_string());
    parts.push("If REQUEST's apparent missing topic is only the generic acknowledgement/short-reply target itself, do not ask what topic to answer. Use standalone chat, needs_clarify=false, empty turn_type/target_task_policy, no evidence/delivery, and put the minimal acknowledgement/short reply in answer_candidate when inferable from REQUEST. This is semantic, not phrase-list based.".to_string());
    parts.push("If the same active writing/drafting task is still missing its topic or core subject, keep the new constraint in resolved_user_intent and ask one concise clarification with decision=\"clarify\"; never force planner_execute for a presentation-only constraint.".to_string());
    parts.push("Do not ask optional preference clarifications for harmless creative/chat requests; answer generically when the deliverable is clear. For a negative constraint plus positive deliverable, preserve the constraint and route the positive deliverable.".to_string());
    parts.push("If CAPABILITIES or a visible skill contract says a missing target/parameter can be handled by safe discovery, default behavior, bounded lookup, or a candidate-returning prepare step, keep the request executable instead of asking a front-door clarification; execution can return observed candidates when it cannot choose uniquely.".to_string());
    parts.push("Example pattern: if a photo-organization capability declares external-drive discovery, route the request to execution without a source_dir so the skill can inspect mounts and either preview the unique candidate or return observed candidates. This is a contract example, not a phrase trigger.".to_string());
    parts.push("Inline-data transform invariant: if REQUEST embeds complete structured data and asks to sort, filter, project, aggregate, convert, or render it, do not clarify because of the requested output format. Use an enabled structured transform capability when visible; otherwise direct-answer from the inline data when no local/external evidence is needed.".to_string());
    parts.push("Current REQUEST overrides RECENT/MEMORY. Prior assistant refusals, tool failures, exact IDs, scalar values, or capability claims in history are background only unless the current request explicitly asks for them.".to_string());
    parts.push("Do not import a prior directory/path scope from RECENT/MEMORY into the current REQUEST when the current REQUEST names its own file/dir target. Reuse prior scope only for explicit follow-ups like same directory, that file, or previous result.".to_string());
    parts.push("Fresh unresolved deictic filesystem targets are missing locators: for a fresh directory/listing/count/read request whose target is only a pronoun/deictic role and no unique immediate binding exists, set decision=\"clarify\", turn_type=\"task_request\", state_patch.deictic_reference={\"target\":\"missing_locator\"} or {\"target\":\"ambiguous_locator\"}, locator_kind=\"none\", locator_hint=\"\". Do not convert it to current_workspace or status_query unless the current REQUEST itself semantically names the present workspace/current directory. Do not resolve a fresh deictic filesystem/log/document target from MEMORY alone; MEMORY may explain ambiguity, but execution needs an immediate binding or current-turn locator.".to_string());
    parts.push("When decision=\"clarify\" only because a concrete locator/target is missing, still preserve the intended final-answer contract in output_contract: keep requires_content_evidence=true for tasks that must read/list/inspect local evidence after the user supplies the locator, keep the requested response_shape, and keep the synthesis semantic_kind when the final answer needs explanation, summary, judgment, conclusion, or another model-language answer grounded in that future evidence. Only locator_kind/locator_hint should express the missing target.".to_string());
    parts.push("If REQUEST asks for observable local/system/workspace state, filesystem inspection, command output, file content, directory listing, counts, or extracting a value, choose decision=\"planner_execute\". Do not claim the assistant cannot execute; the runtime has tools and the AUTH block describes permission.".to_string());
    parts.push("For generic baseline diagnostics, local runtime health, service/process status, or system health requests with no narrower unknown target, use decision=\"planner_execute\", turn_type=\"status_query\", output_contract.semantic_kind=\"service_status\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\". Do not leave these as semantic_kind=\"none\" locatorless content-evidence clarifications.".to_string());
    parts.push("For RSS/feed/latest-news requests covered by rss_fetch, use decision=\"planner_execute\", output_contract.semantic_kind=\"rss_news_fetch\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\". Do not ask for a file path merely because no feed URL was supplied; configured RSS categories provide the default source set.".to_string());
    parts.push("For URL/web-page observation requests that need opening, extracting, title reading, or summarizing page content through browser_web, use decision=\"planner_execute\", output_contract.semantic_kind=\"web_page_summary\", requires_content_evidence=true, delivery_required=false, locator_kind=\"url\", locator_hint=<the concrete URL>, and execution_recipe.kind=\"none\".".to_string());
    parts.push("For web-search result requests covered by web_search_extract, use decision=\"planner_execute\", output_contract.semantic_kind=\"web_search_summary\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\"; keep the search query and requested result limit in resolved_user_intent.".to_string());
    parts.push("For current-weather or forecast requests covered by weather, use decision=\"planner_execute\", output_contract.semantic_kind=\"weather_query\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\"; keep the place, date/day target, forecast window, and output language constraints in resolved_user_intent.".to_string());
    parts.push("For stock, crypto, or other market quote/price requests covered by stock or crypto, use decision=\"planner_execute\", output_contract.semantic_kind=\"market_quote\", requires_content_evidence=true, delivery_required=false, locator_kind=\"none\", locator_hint=\"\", and execution_recipe.kind=\"none\"; keep the concrete symbol/code/name, market type if known, and requested brevity/language in resolved_user_intent.".to_string());
    parts.push("For image/photo/screenshot understanding requests covered by image_vision, use decision=\"planner_execute\", output_contract.semantic_kind=\"image_understanding\", requires_content_evidence=true, delivery_required=false, locator_kind=\"url\" with locator_hint set to the concrete image URL when one is supplied, otherwise locator_kind=\"none\", and execution_recipe.kind=\"none\"; keep the requested visual task and response language in resolved_user_intent.".to_string());
    parts.push("If REQUEST asks about the assistant/runtime's current unfinished task queue, running tasks, queued tasks, or canceling those tasks, use the existing task_control capability with its default current-user/current-chat scope; do not ask the user to choose a queue type just because no separate system name was supplied.".to_string());
    parts.push("If REQUEST only asks whether this assistant is currently waiting for user approval, answer from runtime invariants with decision=\"direct_answer\", turn_type=\"status_query\", execution_recipe.kind=\"none\", no evidence/delivery, and state_patch.runtime_status_query={\"kind\":\"approval_wait\",\"scope\":\"current_task\"}; leave answer_candidate empty unless that runtime fact is provided as structured context.".to_string());
    parts.push("Never ask the user to paste local file contents when REQUEST names a local file/dir/workspace target; route the request for tool execution. Capability refusals are only valid after an actual tool failure, not inside this normalizer.".to_string());
    parts.push("Always include output_contract as a JSON object, never as a string token. It is the final answer contract, not a place to invent a task-specific schema. Put exact scalar recall/direct-answer values in answer_candidate as a string only when the current request itself asks for that exact value; never put answer_candidate as an object or inside output_contract. If unsure, still emit the full default output_contract object with response_shape=\"free\", requires_content_evidence=false, delivery_required=false, locator_kind=\"none\", delivery_intent=\"none\", semantic_kind=\"none\", locator_hint=\"\", and self_extension set to none.".to_string());
    parts.push("Allowed output_contract keys only: response_shape, exact_sentence_count, requires_content_evidence, delivery_required, locator_kind, delivery_intent, semantic_kind, locator_hint, self_extension. Do not emit exact_format, required_evidence, fields, examples, post_processing, or custom keys.".to_string());
    parts.push("locator_hint must be a clean concrete locator value or concrete target pair, not a full instruction sentence and not explanatory prose. If no clean locator is known, leave it empty and let needs_clarify/decision express the missing target.".to_string());
    parts.push("Allowed response_shape: free, one_sentence, strict, scalar, file_token. Allowed locator_kind: none, path, current_workspace, url, filename. Allowed delivery_intent: none, file_single, directory_lookup, directory_batch_files.".to_string());
    parts.push("Allowed semantic_kind: none, raw_command_output, service_status, hidden_entries_check, file_names, directory_names, directory_entry_groups, file_paths, directory_purpose_summary, content_excerpt_summary, content_excerpt_with_summary, content_presence_check, excerpt_kind_judgment, recent_artifacts_judgment, workspace_project_summary, scalar_count, quantity_comparison, execution_failed_step, generated_file_delivery, scalar_path_only, existence_with_path, existence_with_path_summary, recent_scalar_equality_check, git_commit_subject, git_repository_state, structured_keys, config_validation, config_mutation, config_risk_assessment, rss_news_fetch, web_page_summary, web_search_summary, weather_query, market_quote, image_understanding, publishing_preview, package_manager_detection, sqlite_table_listing, sqlite_table_names_only, sqlite_database_kind_judgment, sqlite_schema_version, archive_list, archive_pack, archive_unpack, docker_ps, docker_images, docker_logs, docker_container_lifecycle.".to_string());
    parts.push("Allowed turn_type: task_request, task_append, task_replace, task_correct, task_scope_update, run_control, approval_decision, status_query, feedback_or_error, preference_or_memory, or empty string. clarify is a decision, never a turn_type or resume_behavior.".to_string());
    parts.push("state_patch must be a JSON object or null. Use null when there is no structured update; never output an empty string for state_patch. For ordered-entry follow-ups against an active ordered list, set state_patch.ordered_entry_ref to {\"index\":N,\"index_base\":1} for absolute item selection or {\"relative_offset\":K} for signed relative selection. When a standalone current REQUEST creates a new user-visible deliverable that later short corrections should edit, set state_patch.primary_task_update=\"replace\" and state_patch.active_task_boundary=\"new_deliverable\". For active-task visible corrections, set required_content_literals / replacement_pairs / forbidden_visible_literals as structured exact content literals, not language-specific phrase markers. Keep output-only/body-only/length/tone/count/format constraints in resolved_user_intent and output_contract, not in required_content_literals. For a clear deictic reference, set state_patch.deictic_reference={\"target\":\"current_action_result\"|\"current_turn_locator\"|\"comparison_result\"|\"unresolved_prior_object\"|\"missing_locator\"|\"ambiguous_locator\"}; unresolved/missing/ambiguous targets mean safe clarify. For runtime self-state questions about whether this assistant is waiting for user approval, set state_patch.runtime_status_query={\"kind\":\"approval_wait\",\"scope\":\"current_task\"}. The runtime consumes structured numbers/targets/status tokens, not language-specific ordinal words, pronouns, connectors, or status wording.".to_string());
    parts.push("Every enum field must be exactly one listed schema token. Do not output aliases, combined values, or explanatory prose in decision/output_contract/execution_recipe/turn_type/target_task_policy.".to_string());
    parts.push("Boolean fields must be JSON true/false, not prose. self_extension must be an object with mode/trigger/execute_now; use {\"mode\":\"none\",\"trigger\":\"none\",\"execute_now\":false} unless the user explicitly asks for self-extension. If locator_kind=\"none\", locator_hint must be \"\".".to_string());
    parts.push("If the user asks to observe/list/read first but only return a scalar result, set response_shape=\"scalar\" and use a matching semantic_kind only when one applies: scalar_count for generic counts, hidden_entries_check for hidden/dot-prefixed entry counts, scalar_path_only only for a path/current-directory/workspace-location answer, sqlite_schema_version for SQLite schema-version metadata. For config field values, package names, usernames, hostnames, titles, IDs, or other non-path scalar values, keep semantic_kind=\"none\" unless another specific enum applies. If the final answer must include both a structured field/key/path identifier and its value, it is not a scalar-only value response: use response_shape=\"strict\" and preserve the key/value shape in resolved_user_intent. If the request requires an exact non-scalar output format with fixed count, body-only delivery, one-line fixed format, placeholder format, or no-extra-output delivery, set response_shape=\"strict\" and preserve the exact format in resolved_user_intent. For any exact counted-sentence requirement, also set exact_sentence_count to that positive integer; use response_shape=\"strict\" when the count is greater than 1. Never put natural-language format descriptions in response_shape.".to_string());
    parts.push("For command/tool execution where the final answer is about execution failure itself, including a single failed command/action or ordered failed step(s), set response_shape=\"strict\", semantic_kind=\"execution_failed_step\", requires_content_evidence=true, delivery_required=false. This is a semantic judgment from the requested final answer shape, not a phrase-list trigger.".to_string());
    parts.push("For bounded file or log excerpt observations, choose the semantic_kind from the final answer, not from the tool used to gather evidence. A direct request to display a bounded line slice, head/tail slice, or exact range must use semantic_kind=\"raw_command_output\" with the exact slice/count preserved in resolved_user_intent and response_shape=\"strict\" when the final answer should paste/show the observed lines themselves. If the answer must only explain, summarize, conclude, judge, describe a phenomenon, or provide a one-sentence takeaway from the observed excerpt, use semantic_kind=\"content_excerpt_summary\" or semantic_kind=\"excerpt_kind_judgment\" for excerpt classification. If the final answer must include both the bounded observed slice and the requested synthesis, use semantic_kind=\"content_excerpt_with_summary\". Do not classify a plain bounded line read as content_excerpt_summary unless model-language interpretation is part of the requested deliverable.".to_string());
    parts.push("For requests to create/save/write a new artifact and then send/deliver it, set response_shape=\"file_token\", semantic_kind=\"generated_file_delivery\", delivery_required=true, delivery_intent=\"file_single\", requires_content_evidence=true. If the user did not supply a filename but the artifact type/content is clear, do not ask for one; let execution planning choose a safe workspace filename.".to_string());
    parts.push("For archive pack/create/compress or unpack/extract/decompress requests, use semantic_kind=\"archive_pack\" or semantic_kind=\"archive_unpack\" even when the final answer asks only for the resulting path or status. Do not classify archive operations as generated_file_delivery; they have dedicated archive contracts and actions.".to_string());
    parts.push("For requests to send/deliver/receive an existing or selected local file, including a file selected from an observed or target directory by ordinal/order such as first/last/newest/largest, set wants_file_delivery=true, response_shape=\"file_token\", delivery_required=true, delivery_intent=\"file_single\", requires_content_evidence=true. The final answer must be a file token, not a bare filename, file_path_and_content answer_candidate, or pasted file content. This is a semantic delivery contract, not a phrase list.".to_string());
    parts.push("Text drafting/composition is not file delivery by default. If REQUEST asks to write/draft/compose an article, note, proposal, summary, checklist, tutorial, guide, or long-form text for the chat, but does not explicitly ask to save it to a file/path/document or send/deliver it as an attachment/artifact, do not use response_shape=\"file_token\" or semantic_kind=\"generated_file_delivery\". Keep delivery_required=false, wants_file_delivery=false, and use response_shape=\"free\" or \"strict\" according to the requested prose format. If the text is project-bound and needs workspace facts, use decision=\"planner_execute\", requires_content_evidence=true, locator_kind=\"current_workspace\"; still keep file delivery disabled. Examples: \"帮我写一篇关于 RustClaw 的长文\" / \"Write a long article about RustClaw\" means pasted prose in chat, while \"帮我写成 md 文件并发给我\" / \"Create a markdown file and send it to me\" means generated file delivery.".to_string());
    parts.push("If REQUEST drafts or composes text for an external publishing channel or platform workflow owned by a visible publishing skill, use decision=\"planner_execute\", semantic_kind=\"publishing_preview\", requires_content_evidence=true even when the requested mode is preview-only, draft-only, dry-run, or no-publish. Keep delivery_required=false and preserve the preview/no-send constraint in resolved_user_intent; ordinary chat-only drafting still follows the direct-answer drafting rule.".to_string());
    parts.push("For exact same/different comparison of two scalar/field values that still need observation, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, response_shape=\"strict\", semantic_kind=\"recent_scalar_equality_check\". Keep the requested final line format in resolved_user_intent.".to_string());
    parts.push("For a comparison where one side is a scalar field/value from a structured manifest or config file and the other side is the corresponding value mentioned in a README/docs file, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, semantic_kind=\"recent_scalar_equality_check\", and response_shape=\"one_sentence\"/\"strict\" according to the requested final answer. This is a semantic contract for field/document evidence, not generic document summarization.".to_string());
    parts.push("For comparison or classification of prose excerpts/opening sections by audience, purpose, document role, or content type, use semantic_kind=\"excerpt_kind_judgment\" with content evidence. Do not route these as scalar equality checks because the compared evidence is prose, not scalar fields.".to_string());
    parts.push("For recent-file listings plus any grounded type, category, purpose, use, or role judgment about those selected recent entries, use semantic_kind=\"recent_artifacts_judgment\" with content evidence. Preserve both the recent-entry selection and the judgment/explanation deliverable in resolved_user_intent so planning can first observe the sorted entries and then read bounded content when needed.".to_string());
    parts.push("For file/path metadata comparisons across concrete local targets (for example size/大小, modified time/修改时间, existence state, or other observable path facts), use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, semantic_kind=\"quantity_comparison\", and response_shape=\"scalar\"/\"one_sentence\"/\"strict\" according to the requested final answer. This is a semantic contract decision, not a phrase-list trigger; do not treat metadata comparison as document content summarization just because the user also asks for a short explanation.".to_string());
    parts.push("For local project package-manager, dependency-manager, frontend package-manager, or build-tool detection, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, semantic_kind=\"package_manager_detection\", and locator_kind=\"current_workspace\" or \"path\" when the request names a project directory. This is a project capability contract based on manifest/lock-file observation; do not route it as generic file_names merely because marker filenames are inspected.".to_string());
    parts.push("For git commit subject/title requests, use decision=\"planner_execute\", requires_content_evidence=true, response_shape=\"scalar\" or \"strict\" according to the user's requested format, and semantic_kind=\"git_commit_subject\". Do not publish the raw git oneline hash when the final answer asks for the subject/title only.".to_string());
    parts.push("For read-only Git repository state observation such as current branch, branch list, status, remotes, changed files, or revision metadata, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, locator_kind=\"current_workspace\" or \"none\", semantic_kind=\"git_repository_state\", and response_shape=\"scalar\"/\"strict\"/\"free\" according to the requested final answer. This is a tool capability contract; do not require a file/path locator unless the Git action needs a concrete path, such as reading a file at a revision.".to_string());
    parts.push("For structured document key-name requests against JSON/TOML/YAML/config files, use decision=\"planner_execute\", requires_content_evidence=true, response_shape=\"strict\" when the user asks only for the keys, and semantic_kind=\"structured_keys\". Keep locator_kind/locator_hint pointed at the structured file; do not treat key-name requests as file excerpts.".to_string());
    parts.push("For hidden or dot-prefixed directory entry checks, use decision=\"planner_execute\", requires_content_evidence=true, locator_kind=\"current_workspace\" or \"path\", and semantic_kind=\"hidden_entries_check\". When the final answer is constrained to count only, use response_shape=\"scalar\" with this same semantic_kind. When the final answer is constrained to yes/no plus a limited set of entries, use response_shape=\"strict\" so later stages do not prepend execution traces.".to_string());
    parts.push("For existence checks whose final answer is a presence judgment over a concrete file, directory, path, or local artifact, use decision=\"planner_execute\", requires_content_evidence=true, semantic_kind=\"existence_with_path\", and the narrowest locator_kind that matches the target scope. If the final answer asks only for yes/no or exists/not-exists, use response_shape=\"scalar\"; if it asks to include a path/locator or other evidence fields, use response_shape=\"strict\". Do not use semantic_kind=\"scalar_count\" merely because the requested final answer is short or binary; presence judgment is not numeric counting. If the same request also asks for a brief content-grounded purpose, summary, role, or explanation when found, use semantic_kind=\"existence_with_path_summary\" instead so planning observes both the path and bounded content before synthesis. Preserve the final answer wording constraint in resolved_user_intent so later stages do not prepend execution traces.".to_string());
    parts.push("For directory/file inventory with name or extension filtering, set requires_content_evidence=true and locator_kind=\"current_workspace\" or \"path\". Use semantic_kind=\"file_names\" only when the final answer is an exact file or mixed entry names-only list. Use semantic_kind=\"directory_names\" when the final answer is exact folder/directory names only. Use semantic_kind=\"directory_entry_groups\" when the final answer must separate the same directory's files and directories into groups. Use semantic_kind=\"file_paths\" when the final answer must be file paths, especially repository/workspace-wide extension searches or representative file path lists. If the same request also asks for explanation, purpose, judgment, comparison, or a brief conclusion, do not use an exact names/paths contract; use directory_purpose_summary when it asks what entries are for / more like, otherwise keep semantic_kind=\"none\" and preserve the combined listing+synthesis requirement in resolved_user_intent/reason. If a nuance has no enum, keep response_shape=\"free\" or semantic_kind=\"none\" instead of inventing enum values.".to_string());
    parts.push("For bounded or ordered direct child inventory of a directory/workspace, including modification-time or recency ordering, keep the route executable with response_shape=\"strict\", requires_content_evidence=true, delivery_required=false, and semantic_kind=\"directory_entry_groups\" unless the final answer is restricted to files-only or directories-only. Preserve the ordering/count requirement in resolved_user_intent; do not downgrade such requests to semantic_kind=\"none\" or a generic tree/workspace overview.".to_string());
    parts.push("Use decision=\"planner_execute\" when the request inspects local/system/workspace state, whether the final answer is direct raw/scalar/list output or a narrative synthesis. For current-directory or workspace-location scalar answers, set output_contract.response_shape=\"scalar\" and output_contract.semantic_kind=\"scalar_path_only\" from the request meaning, not from local phrase-classifier hints.".to_string());
    parts.push("For recall questions, use exact values from RECENT/MEMORY. If found, put the value in answer_candidate and resolved_user_intent, set needs_clarify=false, and set decision=\"direct_answer\". Never invent recall-specific decisions. A request for a summary, recap, explanation, conclusion, judgment, or what something verifies/means is not a recall question; keep that deliverable in resolved_user_intent and leave answer_candidate empty unless the current request also explicitly asks for an exact scalar.".to_string());
    parts.push("For requests that depend on prior context, copy the relevant RECENT/MEMORY facts into resolved_user_intent so the next stage has enough context.".to_string());
    parts.push("Use ALIASES only for temporary references already defined in this session. When the current message mentions one, resolve it in resolved_user_intent and locator fields when relevant.".to_string());
    parts.push("For explicit temporary alias/reference mappings in the current turn, set state_patch.alias_bindings to objects with alias and target string fields. Do not infer aliases from vague references.".to_string());
    parts.push("Keep resolved_user_intent concise; preserve exact IDs, but summarize long user text instead of copying it.".to_string());
    parts.push(compact_prompt_slot(
        "ACTIVE_TASK",
        &route_view.active_task_context,
        760,
    ));
    parts.push(compact_prompt_slot(
        "ANCHOR",
        &route_view.active_execution_anchor_context,
        520,
    ));
    parts.push(compact_prompt_slot(
        "ALIASES",
        &route_view.session_alias_context,
        220,
    ));
    parts.push(compact_prompt_slot(
        "HINTS",
        &route_view.request_surface_hints,
        120,
    ));
    parts.push(compact_prompt_slot(
        "CAPABILITIES",
        &route_view.capability_map,
        1800,
    ));
    parts.push(compact_prompt_slot("AUTH", auth_policy_context, 100));
    parts.push("Required keys: resolved_user_intent, needs_clarify, clarify_question, reason, confidence, decision. If unsure: use decision=\"direct_answer\" only for non-observable discussion; use decision=\"planner_execute\" for clear observable local/system/workspace requests.".to_string());
    parts.push("For ordinary chat, greetings, and confirmations: decision=\"direct_answer\", needs_clarify=false, turn_type=\"\". Never use turn_type=\"chat\".".to_string());
    parts.push(compact_prompt_slot(
        "RECENT",
        &route_view.recent_turns_full,
        1040,
    ));
    parts.push(compact_prompt_slot("LAST", &route_view.last_turn_full, 180));
    let runtime_context = context_bundle
        .execution_view
        .as_ref()
        .map(|view| view.runtime_context.as_str())
        .filter(|runtime| {
            let trimmed = runtime.trim();
            !trimmed.is_empty() && trimmed != "<none>"
        })
        .map(str::to_string)
        .unwrap_or_else(|| compact_runtime_context_from_auth(auth_policy_context));
    parts.push(format!("LANG={}", request_language_hint));
    parts.push("CONTRACT: output_contract must be a JSON object. hidden/dot-entry check => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"hidden_entries_check\". yes/no-only existence check => response_shape=\"scalar\", semantic_kind=\"existence_with_path\"; existence check that must return a path/locator/evidence field => response_shape=\"strict\", semantic_kind=\"existence_with_path\"; if it also needs a content-grounded purpose/summary/explanation when found, use semantic_kind=\"existence_with_path_summary\". URL/web-page content or title summary => locator_kind=\"url\", requires_content_evidence=true, semantic_kind=\"web_page_summary\". Web search result summary => locator_kind=\"none\", requires_content_evidence=true, semantic_kind=\"web_search_summary\". Weather current/forecast observation => locator_kind=\"none\", requires_content_evidence=true, semantic_kind=\"weather_query\". Stock/crypto market quote observation => locator_kind=\"none\", requires_content_evidence=true, semantic_kind=\"market_quote\". Image/photo/screenshot understanding => requires_content_evidence=true, semantic_kind=\"image_understanding\", locator_kind=\"url\" only when a concrete image URL is supplied. External publishing-channel draft/preview => requires_content_evidence=true, semantic_kind=\"publishing_preview\", locator_kind=\"none\". exact file or mixed entry names list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"file_names\". exact folder/directory names list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"directory_names\". grouped files-vs-directories list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"directory_entry_groups\". exact file paths list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"file_paths\". exact bounded file/log line slice => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"raw_command_output\". local file/path metadata comparison => requires_content_evidence=true, semantic_kind=\"quantity_comparison\". git commit subject/title only => response_shape=\"scalar\", requires_content_evidence=true, semantic_kind=\"git_commit_subject\". read-only Git repository state => requires_content_evidence=true, semantic_kind=\"git_repository_state\". current path only => response_shape=\"scalar\", semantic_kind=\"scalar_path_only\"; never use scalar_path_only for directory listings.".to_string());
    // Keep memory and assistant recall context close to the current request so
    // compact head+tail truncation preserves both structure labels and goals.
    parts.push(compact_prompt_slot(
        "MEMORY",
        &route_view.memory_context,
        560,
    ));
    // Keep recent assistant replies closest to the request so exact scalar
    // recall can use the assistant's visible answer rather than memory scores.
    parts.push(compact_prompt_slot(
        "ASSISTANT",
        &route_view.recent_assistant_replies,
        260,
    ));
    parts.push(compact_prompt_slot("RUNTIME", &runtime_context, 260));
    parts.push("LOCAL_EXEC: local file/dir/command/count/metadata/read/list/summarize => planner_execute; no cannot-access-FS reply; never ask user to paste local files.".to_string());
    parts.push("SUMMARY_RECALL: summary != ID recall; answer_candidate only for exact scalar request; memory scores are metadata. RUNTIME_STATUS approval_wait=>direct_answer status_query. FOLLOWUP_ANCHOR_PRIORITY: ANCHOR/ACTIVE_TASK beat MEMORY for ordinal/deictic or active writing refinements unless REQUEST asks scalar recall.".to_string());
    parts.push(compact_prompt_slot("RUNTIME", &runtime_context, 240));
    // Keep memory, assistant replies, active-task state, and the request in the
    // compact tail together; small-context providers often preserve only
    // head+tail around the final request.
    parts.push(compact_prompt_slot(
        "MEMORY",
        &route_view.memory_context,
        560,
    ));
    // Keep recent assistant replies after memory so exact scalar recall can use
    // the assistant's visible answer rather than memory scores.
    parts.push(compact_prompt_slot(
        "ASSISTANT",
        &route_view.recent_assistant_replies,
        240,
    ));
    parts.push(compact_prompt_slot(
        "ACTIVE_TASK",
        &route_view.active_task_context,
        320,
    ));
    parts.push(compact_prompt_slot(
        "ANCHOR",
        &route_view.active_execution_anchor_context,
        280,
    ));
    parts.push(compact_prompt_slot("REQUEST", req, 480));
    parts.join("\n")
}

fn render_intent_normalizer_json_retry_prompt(
    route_view: &crate::task_context_builder::RouteContextView,
    context_bundle: &crate::task_context_builder::TaskContextBundle,
    auth_policy_context: &str,
    request_language_hint: &str,
    req: &str,
) -> String {
    let runtime_context = context_bundle
        .execution_view
        .as_ref()
        .map(|view| view.runtime_context.as_str())
        .filter(|runtime| {
            let trimmed = runtime.trim();
            !trimmed.is_empty() && trimmed != "<none>"
        })
        .map(str::to_string)
        .unwrap_or_else(|| compact_runtime_context_from_auth(auth_policy_context));
    let parts = vec![
        "JSON-only retry. Output one object now; start with `{` and stop after `}`. No reasoning, no markdown, no `<think>`.".to_string(),
        "Fill this route schema; use only listed machine tokens.".to_string(),
        "{\"resolved_user_intent\":\"...\",\"answer_candidate\":\"\",\"resume_behavior\":\"none\",\"schedule_kind\":\"none\",\"schedule_intent\":null,\"wants_file_delivery\":false,\"should_refresh_long_term_memory\":false,\"agent_display_name_hint\":\"\",\"needs_clarify\":false,\"clarify_question\":\"\",\"reason\":\"...\",\"confidence\":0.9,\"decision\":\"clarify|direct_answer|planner_execute\",\"output_contract\":{\"response_shape\":\"free|strict|scalar|one_sentence|file_token\",\"exact_sentence_count\":null,\"requires_content_evidence\":false,\"delivery_required\":false,\"locator_kind\":\"none|path|current_workspace|url|filename\",\"delivery_intent\":\"none|file_single|directory_lookup|directory_batch_files\",\"semantic_kind\":\"none|service_status|file_names|directory_names|directory_entry_groups|file_paths|raw_command_output|scalar_count|quantity_comparison|git_repository_state|structured_keys|config_validation|config_mutation|config_risk_assessment|rss_news_fetch|web_page_summary|web_search_summary|weather_query|market_quote|image_understanding|publishing_preview|package_manager_detection|existence_with_path|existence_with_path_summary\",\"locator_hint\":\"\",\"self_extension\":{\"mode\":\"none\",\"trigger\":\"none\",\"execute_now\":false}},\"execution_recipe\":{\"kind\":\"none\",\"profile\":\"none\",\"target_scope\":\"none\"},\"turn_type\":\"task_request|status_query|\",\"target_task_policy\":\"standalone|\",\"should_interrupt_active_run\":false,\"state_patch\":null,\"attachment_processing_required\":false}".to_string(),
        "Observable local/system/workspace inspection, command output, file/config reads, validation, risk assessment, listings, counts, and metadata => decision=\"planner_execute\" with requires_content_evidence=true.".to_string(),
        "Main application configuration risk/security/audit/guard assessment => semantic_kind=\"config_risk_assessment\", locator_kind=\"path\", locator_hint=\"configs/config.toml\" unless another concrete config path is named. Preserve no-secret-leak requirements in resolved_user_intent; do not expose secret values.".to_string(),
        "Only use decision=\"clarify\" when a required target/action is genuinely missing. Do not ask the user to paste local files when a local target is named or implied by the application config contract.".to_string(),
        format!("LANG={request_language_hint}"),
        compact_prompt_slot("RUNTIME", &runtime_context, 240),
        compact_prompt_slot("ACTIVE_TASK", &route_view.active_task_context, 180),
        compact_prompt_slot("ANCHOR", &route_view.active_execution_anchor_context, 180),
        compact_prompt_slot("CAPABILITIES", &route_view.capability_map, 600),
        compact_prompt_slot("RECENT", &route_view.recent_turns_full, 180),
        compact_prompt_slot("REQUEST", req, 680),
    ];
    parts.join("\n")
}

async fn retry_intent_normalizer_json_parse(
    state: &AppState,
    task: &ClaimedTask,
    route_view: &crate::task_context_builder::RouteContextView,
    context_bundle: &crate::task_context_builder::TaskContextBundle,
    auth_policy_context: &str,
    request_language_hint: &str,
    req: &str,
    prompt_source: &str,
    base_repair_report: &ContractRepairReport,
) -> Option<(IntentNormalizerOut, ContractRepairReport)> {
    let prompt = render_intent_normalizer_json_retry_prompt(
        route_view,
        context_bundle,
        auth_policy_context,
        request_language_hint,
        req,
    );
    let retry_prompt_source = format!("{prompt_source}#retry=json_only");
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "intent_normalizer_retry_prompt",
        &retry_prompt_source,
        None,
        None,
    );
    let retry_out = match llm_gateway::run_with_fallback_with_hints(
        state,
        task,
        &prompt,
        &retry_prompt_source,
        crate::ChatRequestHints {
            temperature: Some(0.0),
            max_tokens: Some(4096),
        },
    )
    .await
    {
        Ok(out) => out,
        Err(err) => {
            warn!(
                "intent_normalizer parse retry llm failed: task_id={} err={}",
                task.task_id, err
            );
            return None;
        }
    };
    let (retry_out_for_parse, retry_report) =
        normalize_intent_normalizer_raw_for_schema_with_report(&retry_out, req);
    let parsed = crate::prompt_utils::validate_against_schema::<IntentNormalizerOut>(
        &retry_out_for_parse,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    );
    match parsed {
        Ok(validated) => {
            let mut report = base_repair_report.clone();
            report.add("llm_retry", "normalizer_parse_retry");
            report.merge(&retry_report);
            if !validated.raw_parse_ok || validated.schema_normalized {
                info!(
                    "{} intent_normalizer task_id={} parse_retry_recovery raw_parse_ok={} schema_normalized={} repair_source={} repair_detail={} input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    validated.raw_parse_ok,
                    validated.schema_normalized,
                    report.source_csv(),
                    report.detail_csv(),
                    crate::truncate_for_log(req)
                );
            } else {
                info!(
                    "{} intent_normalizer task_id={} parse_retry_success input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(req)
                );
            }
            Some((validated.value, report))
        }
        Err(err) => {
            warn!(
                "intent_normalizer parse retry schema failed: task_id={} err={} normalized_raw={}",
                task.task_id,
                err,
                crate::truncate_for_log(&retry_out_for_parse)
            );
            None
        }
    }
}

#[cfg(test)]
fn normalize_intent_normalizer_raw_for_schema(raw: &str, req: &str) -> String {
    normalize_intent_normalizer_raw_for_schema_with_report(raw, req).0
}

fn normalize_intent_normalizer_raw_for_schema_with_report(
    raw: &str,
    req: &str,
) -> (String, ContractRepairReport) {
    let parsed_value = parse_top_level_json_object_preserving_meaningful_duplicates(raw)
        .or_else(|| crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw));
    let Some(mut value) = parsed_value else {
        let mut report = ContractRepairReport::default();
        report.add("conservative_none", "raw_parse_failed_safe_chat_schema");
        return (
            normalize_plain_intent_normalizer_text_for_schema(raw, req),
            report,
        );
    };
    let before = value.clone();
    let Some(obj) = value.as_object_mut() else {
        let text = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| raw.trim());
        let mut report = ContractRepairReport::default();
        report.add("conservative_none", "non_object_output_safe_chat_schema");
        return (
            normalize_plain_intent_normalizer_text_for_schema(text, req),
            report,
        );
    };
    let answer_like_payload = answer_like_normalizer_payload_text(obj);
    let explicit_answer_candidate = obj
        .get("answer_candidate")
        .and_then(answer_candidate_value_text)
        .or_else(|| scalar_output_contract_answer_candidate(obj));
    if let Some(candidate) = explicit_answer_candidate {
        let should_insert = obj
            .get("answer_candidate")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty());
        if should_insert {
            obj.insert("answer_candidate".to_string(), Value::String(candidate));
        }
    }
    match obj.get("resolved_user_intent") {
        Some(Value::String(value)) if value.trim().is_empty() && !req.trim().is_empty() => {
            obj.insert(
                "resolved_user_intent".to_string(),
                Value::String(
                    answer_like_payload
                        .clone()
                        .unwrap_or_else(|| req.trim().to_string()),
                ),
            );
        }
        Some(value) if !value.is_null() && !value.is_string() => {
            let text = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
            obj.insert("resolved_user_intent".to_string(), Value::String(text));
        }
        Some(_) => {}
        None if answer_like_payload.is_some() || !req.trim().is_empty() => {
            obj.insert(
                "resolved_user_intent".to_string(),
                Value::String(
                    answer_like_payload
                        .clone()
                        .unwrap_or_else(|| req.trim().to_string()),
                ),
            );
        }
        None => {}
    }
    normalize_intent_normalizer_top_level_for_schema(obj);
    normalize_intent_normalizer_scalar_types_for_schema(obj);
    normalize_execution_recipe_for_schema(obj, req);
    if let Some(turn_type) = obj.get("turn_type").and_then(|v| v.as_str()) {
        let normalized = normalize_schema_token(turn_type);
        let valid = matches!(
            normalized.as_str(),
            "" | "task_request"
                | "task_append"
                | "task_replace"
                | "task_correct"
                | "task_scope_update"
                | "run_control"
                | "approval_decision"
                | "status_query"
                | "feedback_or_error"
                | "preference_or_memory"
        );
        if !valid {
            obj.insert("turn_type".to_string(), Value::String(String::new()));
        } else {
            obj.insert("turn_type".to_string(), Value::String(normalized));
        }
    }
    if let Some(target_task_policy) = obj.get("target_task_policy").and_then(|v| v.as_str()) {
        let normalized = normalize_schema_token(target_task_policy);
        let valid = matches!(
            normalized.as_str(),
            "" | "reuse_active" | "replace_active" | "pause_and_queue" | "standalone"
        );
        if !valid {
            obj.insert(
                "target_task_policy".to_string(),
                Value::String(String::new()),
            );
        } else {
            obj.insert("target_task_policy".to_string(), Value::String(normalized));
        }
    }
    normalize_output_contract_for_schema(obj);
    normalize_decision_from_executable_output_contract(obj);
    let report = contract_repair_report_from_before_after(&before, &value);
    (
        serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string()),
        report,
    )
}

fn parse_top_level_json_object_preserving_meaningful_duplicates(raw: &str) -> Option<Value> {
    struct MeaningfulDuplicateVisitor;

    impl<'de> serde::de::Visitor<'de> for MeaningfulDuplicateVisitor {
        type Value = Value;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON object")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut map = serde_json::Map::new();
            while let Some(key) = access.next_key::<String>()? {
                let value = access.next_value::<Value>()?;
                match map.get(&key) {
                    Some(existing)
                        if route_duplicate_value_score(existing)
                            > route_duplicate_value_score(&value) => {}
                    _ => {
                        map.insert(key, value);
                    }
                }
            }
            Ok(Value::Object(map))
        }
    }

    let mut deserializer = serde_json::Deserializer::from_str(raw.trim());
    serde::de::Deserializer::deserialize_map(&mut deserializer, MeaningfulDuplicateVisitor).ok()
}

fn route_duplicate_value_score(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::String(raw) => {
            if raw.trim().is_empty() {
                0
            } else {
                3
            }
        }
        Value::Bool(false) => 1,
        Value::Bool(true) => 2,
        Value::Number(number) => {
            if number.as_i64() == Some(0) || number.as_u64() == Some(0) {
                1
            } else {
                2
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                0
            } else {
                3
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                0
            } else {
                4
            }
        }
    }
}

fn contract_repair_report_from_before_after(before: &Value, after: &Value) -> ContractRepairReport {
    let mut report = ContractRepairReport::default();
    let before_obj = before.as_object();
    let after_obj = after.as_object();

    if before_obj.is_some_and(normalizer_object_declares_tool_action_payload) {
        report.add("tool_payload", "normalizer_tool_payload_promoted_to_act");
    }

    let before_recipe = before_obj.and_then(|obj| obj.get("execution_recipe"));
    if execution_recipe_value_declares_command_payload(before_recipe) {
        report.add("command_payload", "execution_recipe_command_payload");
    } else if execution_recipe_value_declares_scalar_runtime_tool_observation(
        before_recipe,
        before_obj.and_then(|obj| obj.get("output_contract")),
    ) {
        report.add(
            "structured_recipe",
            "execution_recipe_scalar_runtime_tool_observation",
        );
    } else if execution_recipe_value_declares_structured_read_observation(before_recipe) {
        report.add(
            "structured_recipe",
            "execution_recipe_structured_read_observation",
        );
    } else if execution_recipe_value_declares_package_manager_detection(before_recipe) {
        report.add(
            "structured_recipe",
            "execution_recipe_package_manager_detection",
        );
    } else if execution_recipe_value_declares_service_status_observation(before_recipe) {
        report.add(
            "structured_recipe",
            "execution_recipe_service_status_observation",
        );
        if execution_recipe_value_declares_health_check_observation(before_recipe) {
            report.add(
                "structured_recipe",
                "execution_recipe_health_check_observation",
            );
        }
    } else if output_recipe_value_declares_execution(before_recipe) {
        report.add("enum_alias", "execution_recipe_enum");
    } else if execution_recipe_value_has_untrusted_text(before_recipe) {
        report.add(
            "conservative_none",
            "execution_recipe_untrusted_text_ignored",
        );
    }

    if schema_field_alias_or_normalization_changed(
        before_obj,
        after_obj,
        &["turn_type"],
        "turn_type",
        normalize_turn_type_schema_token_for_report,
    ) {
        report.add("enum_alias", "turn_type_enum_normalized");
    }
    if schema_field_alias_or_normalization_changed(
        before_obj,
        after_obj,
        &["target_task_policy"],
        "target_task_policy",
        normalize_target_task_policy_schema_token_for_report,
    ) {
        report.add("enum_alias", "target_task_policy_enum_normalized");
    }

    let before_contract = before_obj
        .and_then(|obj| obj.get("output_contract"))
        .and_then(Value::as_object);
    let after_contract = after_obj
        .and_then(|obj| obj.get("output_contract"))
        .and_then(Value::as_object);
    if output_contract_schema_field_changed(
        before_contract,
        after_contract,
        &[
            "response_shape",
            "shape",
            "answer_shape",
            "format",
            "response_format",
        ],
        "response_shape",
        normalize_output_response_shape_for_schema,
        "free",
    ) {
        report.add("enum_alias", "output_contract_response_shape_normalized");
    }
    if output_contract_schema_field_changed(
        before_contract,
        after_contract,
        &["locator_kind"],
        "locator_kind",
        normalize_output_locator_kind_for_schema,
        "none",
    ) {
        report.add("enum_alias", "output_contract_locator_kind_normalized");
    }
    if output_contract_schema_field_changed(
        before_contract,
        after_contract,
        &["delivery_intent"],
        "delivery_intent",
        normalize_output_delivery_intent_for_schema,
        "none",
    ) {
        report.add("enum_alias", "output_contract_delivery_intent_normalized");
    }
    if output_contract_schema_field_changed(
        before_contract,
        after_contract,
        &[
            "semantic_kind",
            "semantic",
            "kind",
            "answer_kind",
            "semantic_type",
        ],
        "semantic_kind",
        normalize_output_semantic_kind_for_schema,
        "none",
    ) {
        report.add("enum_alias", "output_contract_semantic_kind_normalized");
    }
    if output_contract_unknown_semantic_was_ignored(before_contract, after_contract) {
        report.add(
            "conservative_none",
            "output_contract_unknown_semantic_ignored",
        );
    }
    if output_contract_unknown_scalar_was_ignored(before_obj, after_contract) {
        report.add(
            "semantic_suspect",
            "executable_route_unknown_scalar_output_contract",
        );
    }
    if output_contract_requires_evidence_was_repaired(before_contract, after_contract) {
        report.add(
            "structured_contract",
            "output_contract_requires_evidence_repaired",
        );
    }
    if output_contract_has_executable_shape(after_contract) {
        let before_decision = before_obj
            .and_then(|obj| obj.get("decision"))
            .and_then(scalar_json_value_text)
            .and_then(|value| canonical_first_layer_decision_token(&value));
        let after_decision = after_obj
            .and_then(|obj| obj.get("decision"))
            .and_then(scalar_json_value_text)
            .and_then(|value| canonical_first_layer_decision_token(&value));
        if matches!(
            before_decision,
            None | Some(FirstLayerDecision::DirectAnswer)
        ) && matches!(after_decision, Some(FirstLayerDecision::PlannerExecute))
        {
            report.add(
                "structured_contract",
                "decision_promoted_by_output_contract",
            );
        }
    }
    if execution_recipe_schema_field_changed(
        before_recipe,
        after_obj.and_then(|obj| obj.get("execution_recipe")),
        "kind",
        |raw| Some(crate::execution_recipe::parse_execution_recipe_kind_text(raw).as_str()),
        "none",
    ) || execution_recipe_schema_field_changed(
        before_recipe,
        after_obj.and_then(|obj| obj.get("execution_recipe")),
        "profile",
        |raw| Some(crate::execution_recipe::parse_execution_recipe_profile_text(raw).as_str()),
        "none",
    ) || execution_recipe_schema_field_changed(
        before_recipe,
        after_obj.and_then(|obj| obj.get("execution_recipe")),
        "target_scope",
        |raw| Some(crate::execution_recipe::parse_execution_recipe_target_scope_text(raw).as_str()),
        "unknown",
    ) {
        report.add("enum_alias", "execution_recipe_fields_normalized");
    }

    report
}

fn semantic_suspect_detail_for_normalizer_output(
    out: &IntentNormalizerOut,
    req_surface: Option<&crate::intent::surface_signals::PromptSurfaceSignals>,
) -> Option<&'static str> {
    if out.needs_clarify {
        return None;
    }
    let Some(contract) = out.output_contract.as_ref() else {
        return None;
    };
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && contract.requires_content_evidence
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::FileNames
        )
    {
        return Some("file_names_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && contract.requires_content_evidence
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::FilePaths
        )
    {
        return Some("file_paths_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && contract.requires_content_evidence
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::ExistenceWithPathSummary
        )
    {
        return Some("existence_summary_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && !out.wants_file_delivery
        && !contract.delivery_required
        && matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::None
        )
        && req_surface.is_some_and(|surface| surface.locator_target_pair.is_some())
    {
        return Some("multi_path_generic_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && !out.wants_file_delivery
        && !contract.delivery_required
        && contract.requires_content_evidence
        && matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::None
        )
        && matches!(
            parse_output_response_shape(&contract.response_shape),
            OutputResponseShape::Scalar
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Free
                | OutputResponseShape::Strict
        )
        && contract_has_single_path_locator_target(contract, req_surface)
    {
        return Some("single_path_generic_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) == Some(FirstLayerDecision::PlannerExecute)
        && !out.wants_file_delivery
        && !contract.delivery_required
        && contract.requires_content_evidence
        && matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
        && matches!(
            parse_output_semantic_kind(&contract.semantic_kind),
            OutputSemanticKind::ScalarCount
        )
        && matches!(
            parse_output_response_shape(&contract.response_shape),
            OutputResponseShape::Scalar
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
        && contract_has_single_path_locator_target(contract, req_surface)
    {
        return Some("single_path_scalar_count_contract_needs_semantic_shape_review");
    }
    if parse_first_layer_decision_text(&out.decision) != Some(FirstLayerDecision::DirectAnswer) {
        return None;
    }
    if out.wants_file_delivery {
        return Some("chat_route_with_file_delivery_request");
    }
    if contract.requires_content_evidence {
        return Some("chat_route_requires_content_evidence");
    }
    if contract.delivery_required {
        return Some("chat_route_requires_delivery");
    }
    if !matches!(
        parse_output_semantic_kind(&contract.semantic_kind),
        OutputSemanticKind::None
    ) {
        return Some("chat_route_has_observable_semantic_kind");
    }
    if !matches!(
        parse_output_locator_kind(&contract.locator_kind),
        OutputLocatorKind::None
    ) && !contract.locator_hint.trim().is_empty()
    {
        return Some("chat_route_has_observable_locator");
    }
    None
}

fn contract_has_single_path_locator_target(
    contract: &IntentOutputContractOut,
    req_surface: Option<&crate::intent::surface_signals::PromptSurfaceSignals>,
) -> bool {
    if req_surface.is_some_and(|surface| surface.locator_target_pair.is_some()) {
        return false;
    }
    if req_surface.is_some_and(|surface| {
        surface.has_explicit_path_or_url() || surface.has_single_filename_candidate()
    }) {
        return true;
    }
    let locator_hint = contract.locator_hint.trim();
    !locator_hint.is_empty()
        && !locator_hint.contains('|')
        && matches!(
            parse_output_locator_kind(&contract.locator_kind),
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
}

fn normalize_turn_type_schema_token_for_report(raw: &str) -> Option<&'static str> {
    match normalize_schema_token(raw).as_str() {
        "task_request" => Some("task_request"),
        "task_append" => Some("task_append"),
        "task_replace" => Some("task_replace"),
        "task_correct" => Some("task_correct"),
        "task_scope_update" => Some("task_scope_update"),
        "run_control" => Some("run_control"),
        "approval_decision" => Some("approval_decision"),
        "status_query" => Some("status_query"),
        "feedback_or_error" => Some("feedback_or_error"),
        "preference_or_memory" => Some("preference_or_memory"),
        _ => None,
    }
}

fn normalize_target_task_policy_schema_token_for_report(raw: &str) -> Option<&'static str> {
    match normalize_schema_token(raw).as_str() {
        "reuse_active" => Some("reuse_active"),
        "replace_active" => Some("replace_active"),
        "pause_and_queue" => Some("pause_and_queue"),
        "standalone" => Some("standalone"),
        _ => None,
    }
}

fn schema_field_alias_or_normalization_changed(
    before_obj: Option<&serde_json::Map<String, Value>>,
    after_obj: Option<&serde_json::Map<String, Value>>,
    before_keys: &[&str],
    after_key: &str,
    normalize: fn(&str) -> Option<&'static str>,
) -> bool {
    let Some(after_text) = after_obj
        .and_then(|obj| obj.get(after_key))
        .and_then(scalar_json_value_text)
    else {
        return false;
    };
    let Some(after_normalized) = normalize(&after_text) else {
        return false;
    };
    before_keys.iter().any(|key| {
        let Some(before_text) = before_obj
            .and_then(|obj| obj.get(*key))
            .and_then(scalar_json_value_text)
        else {
            return false;
        };
        normalize(&before_text).is_some_and(|candidate| candidate == after_normalized)
            && normalize_schema_token(&before_text) != after_normalized
    })
}

fn output_contract_schema_field_changed(
    before_contract: Option<&serde_json::Map<String, Value>>,
    after_contract: Option<&serde_json::Map<String, Value>>,
    before_keys: &[&str],
    after_key: &str,
    normalize: fn(&str) -> &'static str,
    default: &str,
) -> bool {
    let Some(after_text) = after_contract
        .and_then(|obj| obj.get(after_key))
        .and_then(scalar_json_value_text)
    else {
        return false;
    };
    let after_normalized = normalize(&after_text);
    if after_normalized == default {
        return false;
    }
    before_keys.iter().any(|key| {
        let Some(before_text) = before_contract
            .and_then(|obj| obj.get(*key))
            .and_then(scalar_json_value_text)
        else {
            return false;
        };
        normalize(&before_text) == after_normalized
            && normalize_schema_token(&before_text) != after_normalized
    })
}

fn output_contract_unknown_semantic_was_ignored(
    before_contract: Option<&serde_json::Map<String, Value>>,
    after_contract: Option<&serde_json::Map<String, Value>>,
) -> bool {
    let before_text = before_contract
        .and_then(|obj| {
            [
                "semantic_kind",
                "semantic",
                "kind",
                "answer_kind",
                "semantic_type",
            ]
            .iter()
            .find_map(|key| obj.get(*key).and_then(scalar_json_value_text))
        })
        .unwrap_or_default();
    if before_text.trim().is_empty()
        || schema_text_is_neutral_none(&before_text)
        || normalize_output_semantic_kind_for_schema(&before_text)
            != OutputSemanticKind::None.as_str()
    {
        return false;
    }
    after_contract
        .and_then(|obj| obj.get("semantic_kind"))
        .and_then(scalar_json_value_text)
        .is_some_and(|text| text == OutputSemanticKind::None.as_str())
}

fn output_contract_unknown_scalar_was_ignored(
    before_obj: Option<&serde_json::Map<String, Value>>,
    after_contract: Option<&serde_json::Map<String, Value>>,
) -> bool {
    let Some(before_obj) = before_obj else {
        return false;
    };
    if !normalizer_object_declares_executable_route(before_obj) {
        return false;
    }
    let Some(raw) = before_obj
        .get("output_contract")
        .and_then(scalar_json_value_text)
    else {
        return false;
    };
    if raw.trim().is_empty()
        || schema_text_is_neutral_none(&raw)
        || output_contract_scalar_looks_like_schema_token(&raw)
    {
        return false;
    }
    let Some(after_contract) = after_contract else {
        return false;
    };
    let after_semantic_is_none = after_contract
        .get("semantic_kind")
        .and_then(scalar_json_value_text)
        .is_none_or(|text| text == OutputSemanticKind::None.as_str());
    let after_shape_is_free = after_contract
        .get("response_shape")
        .and_then(scalar_json_value_text)
        .is_none_or(|text| text == OutputResponseShape::Free.as_str());
    let after_locator_is_none = after_contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .is_none_or(|text| text == OutputLocatorKind::None.as_str());
    let after_delivery_is_none = after_contract
        .get("delivery_intent")
        .and_then(scalar_json_value_text)
        .is_none_or(|text| text == OutputDeliveryIntent::None.as_str());
    let after_requires_evidence = after_contract
        .get("requires_content_evidence")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let after_delivery_required = after_contract
        .get("delivery_required")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    after_semantic_is_none
        && after_shape_is_free
        && after_locator_is_none
        && after_delivery_is_none
        && !after_requires_evidence
        && !after_delivery_required
}

fn output_contract_requires_evidence_was_repaired(
    before_contract: Option<&serde_json::Map<String, Value>>,
    after_contract: Option<&serde_json::Map<String, Value>>,
) -> bool {
    let before_requires = before_contract
        .and_then(|obj| obj.get("requires_content_evidence"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let after_requires = after_contract
        .and_then(|obj| obj.get("requires_content_evidence"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    !before_requires && after_requires
}

fn output_contract_has_executable_shape(contract: Option<&serde_json::Map<String, Value>>) -> bool {
    let Some(contract) = contract else {
        return false;
    };
    contract
        .get("requires_content_evidence")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || contract
            .get("delivery_required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || contract
            .get("locator_kind")
            .and_then(scalar_json_value_text)
            .is_some_and(|value| normalize_output_locator_kind_for_schema(&value) != "none")
        || contract
            .get("semantic_kind")
            .and_then(scalar_json_value_text)
            .is_some_and(|value| normalize_output_semantic_kind_for_schema(&value) != "none")
}

fn execution_recipe_schema_field_changed(
    before_recipe: Option<&Value>,
    after_recipe: Option<&Value>,
    key: &str,
    normalize: fn(&str) -> Option<&'static str>,
    default: &str,
) -> bool {
    let Some(after_text) = after_recipe
        .and_then(Value::as_object)
        .and_then(|obj| obj.get(key))
        .and_then(scalar_json_value_text)
    else {
        return false;
    };
    let Some(after_normalized) = normalize(&after_text) else {
        return false;
    };
    if after_normalized == default {
        return false;
    }
    let before_text = before_recipe
        .and_then(Value::as_object)
        .and_then(|obj| obj.get(key))
        .and_then(scalar_json_value_text)
        .or_else(|| before_recipe.and_then(scalar_json_value_text));
    before_text.is_some_and(|text| {
        normalize(&text).is_some_and(|candidate| candidate == after_normalized)
            && normalize_schema_token(&text) != after_normalized
    })
}

fn execution_recipe_value_has_untrusted_text(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(raw)) => {
            !raw.trim().is_empty()
                && !schema_text_is_neutral_none(raw)
                && !schema_text_declares_execution_recipe(raw)
        }
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_has_untrusted_text(Some(value))),
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            if matches!(
                key.as_str(),
                "kind"
                    | "profile"
                    | "target_scope"
                    | "turn_type"
                    | "target_task_policy"
                    | "should_interrupt_active_run"
                    | "state_patch"
                    | "attachment_processing_required"
            ) {
                return false;
            }
            execution_recipe_value_has_untrusted_text(Some(value))
        }),
        Some(other) => scalar_json_value_text(other).is_some_and(|text| {
            !text.trim().is_empty()
                && !schema_text_is_neutral_none(&text)
                && !schema_text_declares_execution_recipe(&text)
        }),
        None => false,
    }
}

fn normalizer_object_declares_executable_route(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("decision")
        .and_then(scalar_json_value_text)
        .and_then(|text| canonical_first_layer_decision_token(&text))
        .is_some_and(|decision| decision == crate::FirstLayerDecision::PlannerExecute)
        || obj
            .get("execution_recipe")
            .is_some_and(|value| output_recipe_value_declares_execution(Some(value)))
}

fn schema_text_is_neutral_none(raw: &str) -> bool {
    matches!(
        normalize_schema_token(raw).as_str(),
        "" | "none" | "null" | "no" | "false"
    )
}

fn answer_like_normalizer_payload_text(obj: &serde_json::Map<String, Value>) -> Option<String> {
    for key in [
        "response_text",
        "response",
        "reply",
        "answer",
        "content",
        "summary",
    ] {
        if let Some(text) = obj.get(key).and_then(scalar_json_value_text) {
            return Some(text);
        }
    }
    if let Some(contract) = obj
        .get("output_contract")
        .and_then(|value| value.as_object())
    {
        for key in [
            "content",
            "scalar_content",
            "scalar_output",
            "answer",
            "response_text",
        ] {
            if let Some(text) = contract.get(key).and_then(scalar_json_value_text) {
                return Some(text);
            }
        }
    }

    const ROUTE_KEYS: &[&str] = &[
        "resolved_user_intent",
        "answer_candidate",
        "resume_behavior",
        "schedule_kind",
        "schedule_intent",
        "wants_file_delivery",
        "should_refresh_long_term_memory",
        "agent_display_name_hint",
        "needs_clarify",
        "clarify_question",
        "reason",
        "confidence",
        "decision",
        "output_contract",
        "execution_recipe",
        "turn_type",
        "target_task_policy",
        "should_interrupt_active_run",
        "state_patch",
        "attachment_processing_required",
    ];
    if obj.keys().any(|key| ROUTE_KEYS.contains(&key.as_str())) {
        return None;
    }
    let mut values = obj
        .values()
        .filter(|value| !value.is_null())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    if values.len() == 1 {
        return scalar_json_value_text(values.pop()?).or_else(|| {
            serde_json::to_string(&Value::Object(obj.clone()))
                .ok()
                .filter(|text| !text.trim().is_empty())
        });
    }
    serde_json::to_string(&Value::Object(obj.clone()))
        .ok()
        .filter(|text| !text.trim().is_empty())
}

fn scalar_json_value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_string()).filter(|text| !text.is_empty()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn answer_candidate_value_text(value: &Value) -> Option<String> {
    scalar_json_value_text(value).or_else(|| {
        let obj = value.as_object()?;
        for key in ["content", "value", "text", "answer", "result", "scalar"] {
            if let Some(text) = obj.get(key).and_then(answer_candidate_value_text) {
                return Some(text);
            }
        }
        if obj.len() == 1 {
            return obj.values().next().and_then(answer_candidate_value_text);
        }
        None
    })
}

fn scalar_output_contract_answer_candidate(obj: &serde_json::Map<String, Value>) -> Option<String> {
    if normalizer_object_declares_executable_route(obj) {
        return None;
    }
    let raw = obj
        .get("output_contract")
        .and_then(scalar_json_value_text)?;
    if output_contract_scalar_looks_like_schema_token(&raw) {
        None
    } else {
        Some(raw)
    }
}

fn output_contract_scalar_looks_like_schema_token(raw: &str) -> bool {
    let token = normalize_schema_token(raw);
    if token.is_empty() {
        return true;
    }
    matches!(
        token.as_str(),
        "free"
            | "text"
            | "plain_text"
            | "string"
            | "message"
            | "answer"
            | "response"
            | "clarification"
            | "json"
            | "json_object"
            | "raw_json"
            | "structured"
            | "structured_data"
            | "text_plain"
            | "text/plain"
            | "text_markdown"
            | "text/markdown"
            | "application_json"
            | "application/json"
            | "application_xml"
            | "application/xml"
            | "text_csv"
            | "text/csv"
            | "text_html"
            | "text/html"
    ) || matches!(
        normalize_output_response_shape_for_schema(&token),
        "one_sentence" | "strict" | "scalar" | "file_token"
    ) || !matches!(normalize_output_locator_kind_for_schema(&token), "none")
        || !matches!(normalize_output_delivery_intent_for_schema(&token), "none")
        || !matches!(normalize_output_semantic_kind_for_schema(&token), "none")
}

fn normalize_plain_intent_normalizer_text_for_schema(raw: &str, req: &str) -> String {
    let text = raw.trim();
    if text.is_empty() {
        return raw.to_string();
    }
    let mut obj = serde_json::Map::new();
    obj.insert(
        "resolved_user_intent".to_string(),
        Value::String(if text.is_empty() { req.trim() } else { text }.to_string()),
    );
    normalize_intent_normalizer_top_level_for_schema(&mut obj);
    normalize_intent_normalizer_scalar_types_for_schema(&mut obj);
    normalize_execution_recipe_for_schema(&mut obj, req);
    normalize_output_contract_for_schema(&mut obj);
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| raw.to_string())
}

fn normalize_intent_normalizer_scalar_types_for_schema(obj: &mut serde_json::Map<String, Value>) {
    normalize_answer_candidate_field(obj);
    normalize_optional_string_field(obj, "clarify_question");
    normalize_optional_string_field(obj, "agent_display_name_hint");
    normalize_optional_string_field(obj, "reason");
    normalize_optional_string_field(obj, "turn_type");
    normalize_optional_string_field(obj, "target_task_policy");
    normalize_confidence_field(obj, "confidence");
}

fn normalize_answer_candidate_field(obj: &mut serde_json::Map<String, Value>) {
    match obj.get("answer_candidate") {
        Some(Value::String(_)) => {}
        Some(Value::Null) | None => {
            obj.insert("answer_candidate".to_string(), Value::String(String::new()));
        }
        Some(value) => {
            let text = answer_candidate_value_text(value).unwrap_or_default();
            obj.insert("answer_candidate".to_string(), Value::String(text));
        }
    }
}

fn normalize_string_field_with_default(
    obj: &mut serde_json::Map<String, Value>,
    key: &str,
    default: &str,
) {
    match obj.get(key) {
        Some(Value::String(_)) => {}
        Some(Value::Null) | None => {
            obj.insert(key.to_string(), Value::String(default.to_string()));
        }
        Some(value) => {
            let text = scalar_json_value_text(value).unwrap_or_else(|| default.to_string());
            obj.insert(key.to_string(), Value::String(text));
        }
    }
}

fn normalize_optional_string_field(obj: &mut serde_json::Map<String, Value>, key: &str) {
    match obj.get(key) {
        Some(Value::String(_)) => {}
        Some(Value::Null) | None => {
            obj.insert(key.to_string(), Value::String(String::new()));
        }
        Some(value) => {
            let text = scalar_json_value_text(value).unwrap_or_else(|| {
                serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
            });
            obj.insert(key.to_string(), Value::String(text));
        }
    }
}

fn normalize_schema_token(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .trim_matches('_')
        .to_string()
}

fn normalize_output_response_shape_for_schema(raw: &str) -> &'static str {
    let trimmed = raw.trim();
    if trimmed.contains('{') && trimmed.contains('}') {
        return "strict";
    }
    if trimmed.split_whitespace().nth(1).is_some() {
        return "free";
    }
    match normalize_schema_token(raw).as_str() {
        "one_sentence" | "single_sentence" | "sentence" | "short_sentence" => "one_sentence",
        "strict"
        | "exact"
        | "exact_text"
        | "strict_text"
        | "exact_format"
        | "one_line"
        | "single_line"
        | "line_only"
        | "one_line_string"
        | "single_line_string"
        | "one_line_text"
        | "single_line_text"
        | "one_line_result"
        | "single_line_result"
        | "one_line_comparison"
        | "single_line_comparison"
        | "list"
        | "array"
        | "string_list"
        | "strings_list"
        | "list_of_strings" => "strict",
        "scalar" | "value" | "value_only" | "single_value" | "field_value" => "scalar",
        "file_token" | "file" | "delivery_token" => "file_token",
        // Model-side shape descriptions are not runtime
        // answer contracts. Preserve the request as executable and let the planner
        // produce the requested final form instead of failing schema validation.
        _ => "free",
    }
}

fn normalize_output_locator_kind_for_schema(raw: &str) -> &'static str {
    match normalize_schema_token(raw).as_str() {
        "path" | "file_path" | "directory" | "directory_path" | "dir" => "path",
        "current_workspace" | "workspace" | "repo" | "repository" => "current_workspace",
        "url" | "uri" | "link" => "url",
        "filename" | "file_name" | "basename" | "file" | "file_locator" => "filename",
        _ => "none",
    }
}

fn contract_value_token(contract: &serde_json::Map<String, Value>, key: &str) -> String {
    contract
        .get(key)
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .unwrap_or_default()
}

fn looks_like_current_workspace_path_alias(token: &str) -> bool {
    matches!(
        token,
        "current_working_directory"
            | "current_directory"
            | "working_directory"
            | "current_workspace"
            | "workspace_root"
            | "cwd"
            | "pwd"
    )
}

fn normalize_output_delivery_intent_for_schema(raw: &str) -> &'static str {
    match normalize_schema_token(raw).as_str() {
        "file_single" | "single_file" | "file" | "deliver_file" | "file_delivery" => "file_single",
        "directory_lookup" | "dir_lookup" | "directory" | "list_directory" => "directory_lookup",
        "directory_batch_files" | "batch_directory_delivery" | "dir_batch" => {
            "directory_batch_files"
        }
        _ => "none",
    }
}

fn normalize_output_semantic_kind_for_schema(raw: &str) -> &'static str {
    match normalize_schema_token(raw).as_str() {
        "raw"
        | "raw_output"
        | "command_output"
        | "command_result"
        | "command_execution_result"
        | "shell_output"
        | "terminal_output" => OutputSemanticKind::RawCommandOutput.as_str(),
        "service_state"
        | "service_running_status"
        | "process_status"
        | "process_state"
        | "process_running_status"
        | "daemon_status"
        | "daemon_state" => OutputSemanticKind::ServiceStatus.as_str(),
        "existence_boolean_with_path" | "boolean_with_path" | "exists_boolean_with_path" => {
            OutputSemanticKind::ExistenceWithPath.as_str()
        }
        "hidden_files"
        | "hidden_entries"
        | "hidden_file_check"
        | "hidden_files_check"
        | "hidden_entry_check"
        | "hidden_entries_check"
        | "hidden_files_example"
        | "hidden_entries_example" => OutputSemanticKind::HiddenEntriesCheck.as_str(),
        "file_names"
        | "file_names_only"
        | "file_name_only"
        | "files_listing"
        | "files_list"
        | "names_only"
        | "entry_names"
        | "directory_entry_names"
        | "file_listing"
        | "file_list"
        | "filename_listing"
        | "filename_list"
        | "filename_only"
        | "filenames_list"
        | "filenames_only"
        | "list_filenames"
        | "list_file_names" => OutputSemanticKind::FileNames.as_str(),
        "directory_names"
        | "directory_names_only"
        | "directory_name_only"
        | "dir_names"
        | "dir_names_only"
        | "folder_names"
        | "folder_names_only"
        | "folders_only" => OutputSemanticKind::DirectoryNames.as_str(),
        "directory_entry_groups"
        | "directory_file_groups"
        | "file_directory_groups"
        | "entry_kind_groups"
        | "entries_by_kind"
        | "grouped_entries"
        | "grouped_entry_names" => OutputSemanticKind::DirectoryEntryGroups.as_str(),
        "file_paths"
        | "file_paths_only"
        | "path_list"
        | "paths_list"
        | "file_path_list"
        | "repository_file_paths"
        | "workspace_file_paths" => OutputSemanticKind::FilePaths.as_str(),
        "git_commit_subject"
        | "git_commit_title"
        | "commit_subject"
        | "commit_title"
        | "latest_commit_subject"
        | "latest_commit_title" => OutputSemanticKind::GitCommitSubject.as_str(),
        "git_repository_state"
        | "git_workspace_state"
        | "git_state"
        | "git_status"
        | "git_branch"
        | "git_current_branch"
        | "git_remote"
        | "git_changed_files"
        | "git_rev_parse" => OutputSemanticKind::GitRepositoryState.as_str(),
        "one_line_comparison" | "single_line_comparison" => {
            OutputSemanticKind::RecentScalarEqualityCheck.as_str()
        }
        "failed_step" | "failed_command_step" | "execution_failure_step" => {
            OutputSemanticKind::ExecutionFailedStep.as_str()
        }
        "new_file_delivery" | "created_file_delivery" | "write_then_send_file" => {
            OutputSemanticKind::GeneratedFileDelivery.as_str()
        }
        "value_only" | "file_field_value" | "field_value" => OutputSemanticKind::None.as_str(),
        normalized => parse_output_semantic_kind(normalized).as_str(),
    }
}

fn canonical_first_layer_decision_token(raw: &str) -> Option<crate::FirstLayerDecision> {
    match normalize_schema_token(raw).as_str() {
        "clarify" => Some(crate::FirstLayerDecision::Clarify),
        "direct_answer" => Some(crate::FirstLayerDecision::DirectAnswer),
        "planner_execute" => Some(crate::FirstLayerDecision::PlannerExecute),
        _ => None,
    }
}

fn parse_first_layer_decision_text(raw: &str) -> Option<crate::FirstLayerDecision> {
    canonical_first_layer_decision_token(raw)
}

fn route_label_from_first_layer_decision(
    decision: FirstLayerDecision,
    finalize_style: ActFinalizeStyle,
) -> &'static str {
    crate::AskMode::from_first_layer_decision_with_finalize(decision, finalize_style).route_label()
}

fn execution_finalize_style_for_contract(contract: &IntentOutputContract) -> ActFinalizeStyle {
    if matches!(
        contract.response_shape,
        OutputResponseShape::Scalar | OutputResponseShape::FileToken
    ) || matches!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput)
    {
        ActFinalizeStyle::Plain
    } else {
        ActFinalizeStyle::ChatWrapped
    }
}

fn normalize_bool_field_with_default(
    obj: &mut serde_json::Map<String, Value>,
    key: &str,
    default: bool,
) {
    let normalized = match obj.get(key) {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::Null) | None => Some(default),
        Some(Value::String(value)) => match normalize_schema_token(value).as_str() {
            "true" | "yes" | "required" => Some(true),
            "false" | "no" | "none" | "final" | "filename_list" | "confirmation" => Some(false),
            _ => Some(default),
        },
        Some(value) => value.as_bool().or(Some(default)),
    };
    if let Some(value) = normalized {
        obj.insert(key.to_string(), Value::Bool(value));
    }
}

fn normalize_confidence_field(obj: &mut serde_json::Map<String, Value>, key: &str) {
    let numeric = match obj.get(key) {
        Some(Value::String(confidence)) => {
            let normalized = confidence.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "high" => Some(0.9),
                "medium" => Some(0.6),
                "low" => Some(0.3),
                _ => normalized.parse::<f64>().ok(),
            }
        }
        Some(value) => value.as_f64(),
        None => None,
    };
    if let Some(numeric) = numeric.filter(|value| value.is_finite()) {
        let normalized = if numeric > 1.0 && numeric <= 100.0 {
            numeric / 100.0
        } else {
            numeric
        };
        obj.insert(key.to_string(), Value::from(normalized.clamp(0.0, 1.0)));
    }
}

fn normalize_intent_normalizer_top_level_for_schema(obj: &mut serde_json::Map<String, Value>) {
    obj.remove("mode");
    obj.entry("resume_behavior".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    normalize_string_field_with_default(obj, "resume_behavior", "none");
    normalize_resume_behavior_for_schema(obj);
    obj.entry("schedule_kind".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    normalize_string_field_with_default(obj, "schedule_kind", "none");
    normalize_schedule_kind_for_schema(obj);
    normalize_schedule_intent_for_schema(obj);
    obj.entry("wants_file_delivery".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "wants_file_delivery", false);
    obj.entry("should_refresh_long_term_memory".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "should_refresh_long_term_memory", false);
    obj.entry("agent_display_name_hint".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("answer_candidate".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("needs_clarify".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "needs_clarify", false);
    obj.entry("clarify_question".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("reason".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("confidence".to_string())
        .or_insert_with(|| Value::from(0.8));
    normalize_string_field_with_default(obj, "decision", "");
    let canonical_decision = obj
        .get("decision")
        .and_then(|v| v.as_str())
        .and_then(canonical_first_layer_decision_token);
    let decision = canonical_decision.unwrap_or(crate::FirstLayerDecision::DirectAnswer);
    obj.insert(
        "decision".to_string(),
        Value::String(decision.as_str().to_string()),
    );
    obj.entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    obj.entry("execution_recipe".to_string())
        .or_insert_with(|| {
            serde_json::json!({
                "kind": "none",
                "profile": "none",
                "target_scope": "none"
            })
        });
    obj.entry("turn_type".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("target_task_policy".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("should_interrupt_active_run".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "should_interrupt_active_run", false);
    obj.entry("state_patch".to_string()).or_insert(Value::Null);
    normalize_state_patch_for_schema(obj);
    obj.entry("attachment_processing_required".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(obj, "attachment_processing_required", false);
}

fn normalize_schedule_kind_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let Some(value) = obj.get_mut("schedule_kind") else {
        obj.insert(
            "schedule_kind".to_string(),
            Value::String("none".to_string()),
        );
        return;
    };
    let raw = value.as_str().unwrap_or("none");
    let normalized = normalize_schema_token(raw);
    let canonical = match normalized.as_str() {
        "" | "none" => "none",
        "create" => "create",
        "update" | "pause" | "resume" => normalized.as_str(),
        "delete" => "delete",
        "query" | "list" => normalized.as_str(),
        _ => "none",
    };
    *value = Value::String(canonical.to_string());
}

fn normalize_resume_behavior_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let Some(value) = obj.get_mut("resume_behavior") else {
        obj.insert(
            "resume_behavior".to_string(),
            Value::String("none".to_string()),
        );
        return;
    };
    let raw = value.as_str().unwrap_or("none");
    let canonical = match normalize_schema_token(raw).as_str() {
        "resume_execute" | "resume" => "resume_execute",
        "resume_discuss" | "defer" => "resume_discuss",
        _ => "none",
    };
    *value = Value::String(canonical.to_string());
}

fn normalize_schedule_intent_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let schedule_kind_is_none = obj
        .get("schedule_kind")
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .map(|value| value == "none" || value.is_empty())
        .unwrap_or(true);
    let Some(value) = obj.get_mut("schedule_intent") else {
        obj.insert("schedule_intent".to_string(), Value::Null);
        return;
    };
    match value {
        Value::Null => {}
        Value::Object(intent) => {
            if schedule_kind_is_none {
                *value = Value::Null;
                return;
            }
            for field in ["schedule", "task"] {
                match intent.get_mut(field) {
                    Some(Value::Object(_)) => {}
                    Some(slot @ Value::String(_)) => {
                        let raw = slot.as_str().unwrap_or_default();
                        if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                            *slot = if parsed.is_object() {
                                parsed
                            } else {
                                Value::Object(serde_json::Map::new())
                            };
                        } else {
                            *slot = Value::Object(serde_json::Map::new());
                        }
                    }
                    Some(slot) => {
                        *slot = Value::Object(serde_json::Map::new());
                    }
                    None => {
                        intent.insert(field.to_string(), Value::Object(serde_json::Map::new()));
                    }
                }
            }
        }
        Value::String(raw) => {
            let normalized = normalize_schema_token(raw);
            if normalized.is_empty() || matches!(normalized.as_str(), "none" | "null" | "no") {
                *value = Value::Null;
                return;
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                *value = if parsed.is_object() {
                    parsed
                } else {
                    Value::Null
                };
            } else {
                *value = Value::Null;
            }
        }
        _ => {
            *value = Value::Null;
        }
    }
}

fn normalize_state_patch_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let Some(value) = obj.get_mut("state_patch") else {
        obj.insert("state_patch".to_string(), Value::Null);
        return;
    };
    match value {
        Value::Null | Value::Object(_) => {}
        Value::String(raw) => {
            let normalized = normalize_schema_token(raw);
            if normalized.is_empty() || matches!(normalized.as_str(), "none" | "null" | "no") {
                *value = Value::Null;
                return;
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                *value = if parsed.is_object() {
                    parsed
                } else {
                    Value::Null
                };
            } else {
                *value = Value::Null;
            }
        }
        _ => {
            *value = Value::Null;
        }
    }
}

fn normalize_decision_from_executable_output_contract(obj: &mut serde_json::Map<String, Value>) {
    if obj
        .get("needs_clarify")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return;
    }
    let current_decision = obj
        .get("decision")
        .and_then(Value::as_str)
        .and_then(canonical_first_layer_decision_token)
        .unwrap_or(FirstLayerDecision::DirectAnswer);
    if current_decision != FirstLayerDecision::DirectAnswer {
        return;
    }
    let Some(contract) = obj
        .get("output_contract")
        .and_then(|value| value.as_object())
    else {
        return;
    };
    let has_executable_contract = contract
        .get("locator_kind")
        .and_then(|value| value.as_str())
        .map(normalize_output_locator_kind_for_schema)
        .is_some_and(|kind| kind != "none")
        || contract
            .get("semantic_kind")
            .and_then(|value| value.as_str())
            .map(normalize_output_semantic_kind_for_schema)
            .is_some_and(|kind| kind != "none")
        || contract
            .get("delivery_required")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        || contract
            .get("requires_content_evidence")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    if has_executable_contract {
        obj.insert(
            "decision".to_string(),
            Value::String(FirstLayerDecision::PlannerExecute.as_str().to_string()),
        );
    }
}

fn normalize_execution_recipe_for_schema(obj: &mut serde_json::Map<String, Value>, req: &str) {
    promote_misnested_turn_analysis_from_execution_recipe(obj);
    if normalizer_object_declares_tool_action_payload(obj) {
        mark_output_contract_requires_content_evidence(obj);
        mark_decision_planner_execute_from_execution_recipe(obj);
    }
    let execution_recipe_value = obj.get("execution_recipe").cloned();
    let execution_recipe = execution_recipe_value.as_ref();
    if execution_recipe_value_declares_command_payload(execution_recipe) {
        mark_output_contract_requires_content_evidence(obj);
        let locator_hint = execution_recipe_value_locator_hint(execution_recipe);
        normalize_output_contract_for_command_payload(obj, locator_hint.as_deref());
        mark_decision_planner_execute_from_execution_recipe(obj);
    } else if execution_recipe_value_declares_scalar_runtime_tool_observation(
        execution_recipe,
        obj.get("output_contract"),
    ) {
        mark_output_contract_requires_content_evidence(obj);
        normalize_output_contract_for_command_payload(obj, None);
        force_output_contract_semantic_kind(obj, OutputSemanticKind::RawCommandOutput);
        if let Some(kind) = scalar_runtime_status_kind_from_execution_recipe(execution_recipe)
            .or_else(|| scalar_runtime_status_kind_from_output_contract(obj.get("output_contract")))
        {
            upsert_runtime_status_query_state_patch(obj, kind);
        }
        mark_decision_planner_execute_from_execution_recipe(obj);
    } else if execution_recipe_value_declares_structured_read_observation(execution_recipe) {
        let locator_hint = execution_recipe_value_structured_locator_hint(execution_recipe);
        let scalar_extraction =
            execution_recipe_value_declares_structured_scalar_extraction(execution_recipe);
        normalize_output_contract_for_structured_read_recipe(
            obj,
            locator_hint.as_deref(),
            scalar_extraction,
            request_uses_filename_only_schema_token(req),
        );
        mark_decision_planner_execute_from_execution_recipe(obj);
    } else if execution_recipe_value_declares_package_manager_detection(execution_recipe) {
        normalize_output_contract_for_package_manager_detection(obj);
        mark_decision_planner_execute_from_execution_recipe(obj);
    } else if execution_recipe_value_declares_service_status_observation(execution_recipe) {
        normalize_output_contract_for_service_status_recipe(obj);
        mark_decision_planner_execute_from_execution_recipe(obj);
    } else if output_recipe_value_declares_execution(obj.get("execution_recipe")) {
        mark_output_contract_requires_content_evidence(obj);
        mark_decision_planner_execute_from_execution_recipe(obj);
    }
    let value = obj
        .entry("execution_recipe".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let Some(recipe) = value.as_object_mut() else {
        return;
    };
    recipe.retain(|key, _| matches!(key.as_str(), "kind" | "profile" | "target_scope"));
    normalize_string_field_with_default(recipe, "kind", "none");
    normalize_string_field_with_default(recipe, "profile", "none");
    normalize_string_field_with_default(recipe, "target_scope", "none");
    if let Some(raw) = recipe.get("kind").and_then(Value::as_str) {
        let kind = crate::execution_recipe::parse_execution_recipe_kind_text(raw);
        recipe.insert("kind".to_string(), Value::String(kind.as_str().to_string()));
    }
    if let Some(raw) = recipe.get("profile").and_then(Value::as_str) {
        let profile = crate::execution_recipe::parse_execution_recipe_profile_text(raw);
        recipe.insert(
            "profile".to_string(),
            Value::String(profile.as_str().to_string()),
        );
    }
    if let Some(raw) = recipe.get("target_scope").and_then(Value::as_str) {
        let target_scope = crate::execution_recipe::parse_execution_recipe_target_scope_text(raw);
        recipe.insert(
            "target_scope".to_string(),
            Value::String(target_scope.as_str().to_string()),
        );
    }
}

fn output_recipe_value_declares_execution(value: Option<&Value>) -> bool {
    if execution_recipe_value_explicitly_declares_none_kind(value) {
        return false;
    }
    execution_recipe_value_has_text(value, schema_text_declares_execution_recipe)
}

fn execution_recipe_value_declares_structured_read_observation(value: Option<&Value>) -> bool {
    execution_recipe_value_structured_locator_hint(value).is_some()
        && (execution_recipe_value_declares_structured_read_action(value)
            || execution_recipe_value_declares_structured_scalar_field_request(value))
}

fn execution_recipe_value_declares_structured_read_action(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            let key = normalize_schema_token(key);
            (matches!(
                key.as_str(),
                "kind" | "action" | "operation" | "op" | "tool"
            ) && value_has_schema_token(value, schema_token_is_read_observation_action))
                || execution_recipe_value_declares_structured_read_action(Some(value))
        }),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_structured_read_action(Some(value))),
        _ => false,
    }
}

fn execution_recipe_value_declares_structured_scalar_extraction(value: Option<&Value>) -> bool {
    if execution_recipe_value_declares_structured_scalar_field_request(value) {
        return true;
    }
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            let key = normalize_schema_token(key);
            (matches!(
                key.as_str(),
                "action"
                    | "kind"
                    | "operation"
                    | "op"
                    | "method"
                    | "extract"
                    | "extraction"
                    | "extractor"
                    | "schema"
                    | "output"
                    | "content"
            ) && value_has_schema_token(value, schema_token_is_scalar_extraction_action))
                || execution_recipe_value_declares_structured_scalar_extraction(Some(value))
        }),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_structured_scalar_extraction(Some(value))),
        _ => false,
    }
}

fn execution_recipe_value_declares_structured_scalar_field_request(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            let key = normalize_schema_token(key);
            (schema_key_is_structured_scalar_field_selector(&key)
                && value_has_nonempty_scalar_text(value))
                || execution_recipe_value_declares_structured_scalar_field_request(Some(value))
        }),
        Some(Value::Array(items)) => items.iter().any(|value| {
            execution_recipe_value_declares_structured_scalar_field_request(Some(value))
        }),
        _ => false,
    }
}

fn schema_key_is_structured_scalar_field_selector(key: &str) -> bool {
    matches!(
        key,
        "target_key"
            | "target_field"
            | "field_path"
            | "key_path"
            | "field_selector"
            | "json_pointer"
            | "json_path"
    )
}

fn execution_recipe_value_declares_package_manager_detection(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(map)) => {
            let mut has_package_manager_target = false;
            let mut has_detect_action = false;
            for (key, value) in map {
                let key = normalize_schema_token(key);
                if matches!(
                    key.as_str(),
                    "capability" | "capability_name" | "planner_capability"
                ) && value_has_schema_token(
                    value,
                    schema_token_is_package_manager_detect_capability,
                ) {
                    return true;
                }
                if matches!(
                    key.as_str(),
                    "name" | "skill" | "skill_name" | "runner" | "runner_name" | "tool"
                ) && value_has_schema_token(value, schema_token_is_package_manager_skill)
                {
                    has_package_manager_target = true;
                }
                if matches!(
                    key.as_str(),
                    "action" | "operation" | "op" | "method" | "intent"
                ) && value_has_schema_token(value, schema_token_is_package_manager_detect_action)
                {
                    has_detect_action = true;
                }
                if execution_recipe_value_declares_package_manager_detection(Some(value)) {
                    return true;
                }
            }
            has_package_manager_target && has_detect_action
        }
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_package_manager_detection(Some(value))),
        _ => false,
    }
}

fn execution_recipe_value_declares_service_status_observation(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(map)) => {
            let mut has_service_status_tool = false;
            let mut has_service_status_action = false;
            for (key, value) in map {
                let key = normalize_schema_token(key);
                if matches!(
                    key.as_str(),
                    "capability" | "capability_name" | "planner_capability"
                ) && value_has_schema_token(value, schema_token_is_service_status_capability)
                {
                    return true;
                }
                if matches!(
                    key.as_str(),
                    "name" | "skill" | "skill_name" | "runner" | "runner_name" | "tool"
                ) && value_has_schema_token(value, schema_token_is_service_status_tool)
                {
                    has_service_status_tool = true;
                    if value_has_schema_token(value, schema_token_is_standalone_service_status_tool)
                    {
                        return true;
                    }
                }
                if matches!(
                    key.as_str(),
                    "name" | "skill" | "skill_name" | "runner" | "runner_name" | "tool"
                ) && value_has_schema_token(value, schema_token_is_port_status_tool)
                {
                    return true;
                }
                if matches!(
                    key.as_str(),
                    "action" | "operation" | "op" | "method" | "intent"
                ) && value_has_schema_token(value, schema_token_is_service_status_action)
                {
                    has_service_status_action = true;
                }
                if execution_recipe_value_declares_service_status_observation(Some(value)) {
                    return true;
                }
            }
            has_service_status_tool && has_service_status_action
        }
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_service_status_observation(Some(value))),
        _ => false,
    }
}

fn execution_recipe_value_declares_health_check_observation(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            let key = normalize_schema_token(key);
            (matches!(
                key.as_str(),
                "name" | "skill" | "skill_name" | "runner" | "runner_name" | "tool"
            ) && value_has_schema_token(value, schema_token_is_standalone_service_status_tool))
                || (matches!(
                    key.as_str(),
                    "capability" | "capability_name" | "planner_capability"
                ) && value_has_schema_token(
                    value,
                    schema_token_is_standalone_service_status_tool,
                ))
                || execution_recipe_value_declares_health_check_observation(Some(value))
        }),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_health_check_observation(Some(value))),
        _ => false,
    }
}

fn schema_token_is_service_status_tool(token: &str) -> bool {
    matches!(token, "health_check" | "process_basic" | "service_control")
}

fn schema_token_is_standalone_service_status_tool(token: &str) -> bool {
    matches!(token, "health_check")
}

fn schema_token_is_port_status_tool(token: &str) -> bool {
    matches!(token, "netstat_ss_ports")
}

fn schema_token_is_service_status_action(token: &str) -> bool {
    matches!(
        token,
        "status" | "health_check" | "ps" | "port_list" | "diagnose_runtime"
    )
}

fn schema_token_is_service_status_capability(token: &str) -> bool {
    matches!(
        token,
        "health_check"
            | "service_status"
            | "service.status"
            | "runtime_health"
            | "process.ps"
            | "process.port_list"
    )
}

fn schema_token_is_package_manager_skill(token: &str) -> bool {
    matches!(token, "package_manager" | "package_manager_skill")
}

fn schema_token_is_package_manager_detect_action(token: &str) -> bool {
    matches!(
        token,
        "detect"
            | "detection"
            | "detect_manager"
            | "manager_detection"
            | "package_detect_manager"
            | "package_manager_detect"
            | "package_manager_detection"
    )
}

fn schema_token_is_package_manager_detect_capability(token: &str) -> bool {
    matches!(
        token,
        "package.detect_manager"
            | "package_detect_manager"
            | "package_manager_detect"
            | "package_manager_detection"
    )
}

fn value_has_schema_token(value: &Value, predicate: fn(&str) -> bool) -> bool {
    match value {
        Value::String(raw) => predicate(&normalize_schema_token(raw)),
        Value::Array(items) => items
            .iter()
            .any(|value| value_has_schema_token(value, predicate)),
        Value::Object(map) => map
            .values()
            .any(|value| value_has_schema_token(value, predicate)),
        other => scalar_json_value_text(other)
            .is_some_and(|text| predicate(&normalize_schema_token(&text))),
    }
}

fn schema_token_is_read_observation_action(token: &str) -> bool {
    matches!(
        token,
        "read"
            | "file_read"
            | "read_file"
            | "read_text"
            | "read_range"
            | "read_text_range"
            | "file_read_title"
            | "file_read_extract_title"
            | "read_file_title"
            | "read_file_extract_title"
            | "read_file_and_extract_title"
    )
}

fn schema_token_is_scalar_extraction_action(token: &str) -> bool {
    matches!(
        token,
        "extract_scalar"
            | "scalar"
            | "file_read_title"
            | "file_read_extract_title"
            | "read_file_title"
            | "read_file_extract_title"
            | "read_file_and_extract_title"
            | "extract_title"
            | "title"
            | "title_only"
            | "first_heading_line"
            | "markdown_heading"
    )
}

fn execution_recipe_value_structured_locator_hint(value: Option<&Value>) -> Option<String> {
    let mut hints = Vec::new();
    collect_execution_recipe_locator_hints(value?, &mut hints);
    hints.sort();
    hints.dedup();
    (hints.len() == 1).then(|| hints.remove(0))
}

fn collect_execution_recipe_locator_hints(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let normalized_key = normalize_schema_token(key);
                if matches!(
                    normalized_key.as_str(),
                    "target"
                        | "path"
                        | "file_path"
                        | "target_path"
                        | "input_path"
                        | "source_path"
                        | "read_path"
                        | "filepath"
                ) {
                    if let Some(hint) = scalar_json_value_text(value)
                        .and_then(|text| {
                            crate::intent::locator_extractor::extract_explicit_locator_for_fallback(
                                &text,
                            )
                            .map(|locator| locator.locator_hint)
                        })
                        .filter(|hint| !hint.trim().is_empty())
                    {
                        out.push(hint);
                    }
                }
                collect_execution_recipe_locator_hints(value, out);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_execution_recipe_locator_hints(value, out);
            }
        }
        _ => {}
    }
}

fn execution_recipe_value_explicitly_declares_none_kind(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_object)
        .and_then(|map| map.get("kind"))
        .and_then(scalar_json_value_text)
        .is_some_and(|kind| {
            matches!(
                crate::execution_recipe::parse_execution_recipe_kind_text(&kind),
                crate::execution_recipe::ExecutionRecipeKind::None
            )
        })
}

fn execution_recipe_value_has_text(value: Option<&Value>, predicate: fn(&str) -> bool) -> bool {
    match value {
        Some(Value::String(raw)) => predicate(raw),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_has_text(Some(value), predicate)),
        Some(Value::Object(map)) => map
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "target_scope"
                        | "turn_type"
                        | "target_task_policy"
                        | "should_interrupt_active_run"
                        | "state_patch"
                        | "attachment_processing_required"
                )
            })
            .any(|(_, value)| execution_recipe_value_has_text(Some(value), predicate)),
        Some(other) => scalar_json_value_text(other).is_some_and(|text| predicate(&text)),
        None => false,
    }
}

fn execution_recipe_value_declares_command_payload(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            (matches!(
                normalize_schema_token(key).as_str(),
                "command" | "commands" | "cmd" | "cmds" | "shell_command" | "shell_commands"
            ) && value_has_nonempty_scalar_text(value))
                || execution_recipe_value_declares_command_payload(Some(value))
        }),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_command_payload(Some(value))),
        _ => false,
    }
}

fn execution_recipe_value_declares_scalar_runtime_tool_observation(
    value: Option<&Value>,
    output_contract: Option<&Value>,
) -> bool {
    if !output_contract_declares_scalar_locatorless_observation(output_contract) {
        return false;
    }
    let Some(map) = value.and_then(Value::as_object) else {
        return false;
    };
    if map
        .get("action")
        .or_else(|| map.get("operation"))
        .or_else(|| map.get("op"))
        .or_else(|| map.get("method"))
        .is_some_and(|value| {
            value_has_nonempty_scalar_text(value)
                && !value_has_schema_token(value, schema_token_is_runtime_status_operation)
        })
    {
        return false;
    }
    [
        "name",
        "tool",
        "tool_name",
        "runner",
        "runner_name",
        "skill",
        "skill_name",
        "capability",
        "capability_name",
    ]
    .iter()
    .any(|key| {
        map.get(*key).is_some_and(|value| {
            value_has_schema_token(value, schema_token_is_runtime_observation_tool)
        })
    })
}

fn upsert_runtime_status_query_state_patch(
    obj: &mut serde_json::Map<String, Value>,
    kind: &'static str,
) {
    let value = obj
        .entry("state_patch".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let Some(patch) = value.as_object_mut() else {
        return;
    };
    patch.insert(
        "runtime_status_query".to_string(),
        serde_json::json!({
            "kind": kind,
            "scope": "system"
        }),
    );
}

fn upsert_runtime_status_query_state_patch_value(
    state_patch: &mut Option<Value>,
    kind: &'static str,
) {
    let value = state_patch.get_or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let Some(patch) = value.as_object_mut() else {
        return;
    };
    patch.insert(
        "runtime_status_query".to_string(),
        serde_json::json!({
            "kind": kind,
            "scope": "system"
        }),
    );
}

fn volatile_runtime_status_kind_for_answer_candidate(
    answer_candidate: &str,
) -> Option<&'static str> {
    let candidate = answer_candidate.trim();
    if candidate.is_empty() || candidate.len() > 128 || candidate.contains(['\n', '\r']) {
        return None;
    }
    for key in ["USER", "LOGNAME", "USERNAME"] {
        if std::env::var(key)
            .ok()
            .map(|value| value.trim() == candidate)
            .unwrap_or(false)
        {
            return Some("current_user");
        }
    }
    None
}

fn apply_unobserved_runtime_status_answer_candidate_repair(
    output_contract: &mut IntentOutputContract,
    answer_candidate: &mut String,
    state_patch: &mut Option<Value>,
    needs_clarify: bool,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    first_layer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    turn_type: &mut Option<TurnType>,
    target_task_policy: &mut Option<TargetTaskPolicy>,
) -> Option<&'static str> {
    let execution_recipe_hint_is_neutral = execution_recipe_hint
        .is_none_or(|spec| spec.kind.as_str() == "none" && spec.profile.as_str() == "none");
    if needs_clarify
        || wants_file_delivery
        || !matches!(schedule_kind, ScheduleKind::None)
        || !execution_recipe_hint_is_neutral
        || !matches!(*first_layer_decision, FirstLayerDecision::DirectAnswer)
        || !matches!(output_contract.response_shape, OutputResponseShape::Scalar)
        || output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !matches!(output_contract.locator_kind, OutputLocatorKind::None)
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        || matches!(*turn_type, Some(TurnType::PreferenceOrMemory))
    {
        return None;
    }
    let kind = volatile_runtime_status_kind_for_answer_candidate(answer_candidate)?;

    // Guard against unobserved volatile runtime facts. This consumes only the
    // model's structured scalar candidate and the process environment; it does
    // not inspect or branch on the user's natural-language wording.
    output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    output_contract.requires_content_evidence = true;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.delivery_required = false;
    *first_layer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
    *turn_type = Some(TurnType::StatusQuery);
    *target_task_policy = Some(TargetTaskPolicy::Standalone);
    upsert_runtime_status_query_state_patch_value(state_patch, kind);
    answer_candidate.clear();
    Some("unobserved_runtime_status_answer_candidate_requires_evidence")
}

fn scalar_runtime_status_kind_from_execution_recipe(value: Option<&Value>) -> Option<&'static str> {
    let mut tokens = Vec::new();
    collect_runtime_status_operation_tokens(value?, &mut tokens);
    tokens
        .into_iter()
        .find_map(|token| runtime_status_kind_for_operation_token(&token))
}

fn scalar_runtime_status_kind_from_output_contract(value: Option<&Value>) -> Option<&'static str> {
    let contract = value?.as_object()?;
    let locator_kind = contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_locator_kind(&value))
        .unwrap_or_default();
    if locator_kind != OutputLocatorKind::CurrentWorkspace {
        return None;
    }
    let hint = contract
        .get("locator_hint")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let hint_path = Path::new(hint.trim());
    if !hint_path.is_absolute() {
        return None;
    }
    std::env::var("HOME").ok().and_then(|home| {
        let home_path = Path::new(home.trim());
        (home_path.is_absolute() && hint_path == home_path).then_some("current_user")
    })
}

fn collect_runtime_status_operation_tokens(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key = normalize_schema_token(key);
                if matches!(
                    key.as_str(),
                    "action"
                        | "operation"
                        | "op"
                        | "method"
                        | "intent"
                        | "query_kind"
                        | "field"
                        | "field_name"
                        | "target_field"
                ) {
                    if let Some(token) = scalar_json_value_text(value)
                        .map(|raw| normalize_schema_token(&raw))
                        .filter(|token| !token.is_empty())
                    {
                        out.push(token);
                    }
                }
                if matches!(
                    key.as_str(),
                    "arg"
                        | "args"
                        | "argument"
                        | "arguments"
                        | "param"
                        | "params"
                        | "parameter"
                        | "parameters"
                ) {
                    collect_runtime_status_arg_tokens(value, out);
                }
                collect_runtime_status_operation_tokens(value, out);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_runtime_status_operation_tokens(value, out);
            }
        }
        _ => {}
    }
}

fn collect_runtime_status_arg_tokens(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let token = normalize_schema_token(text);
            if runtime_status_kind_for_operation_token(&token).is_some() {
                out.push(token);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_runtime_status_arg_tokens(value, out);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_runtime_status_arg_tokens(value, out);
            }
        }
        _ => {}
    }
}

fn schema_token_is_runtime_status_operation(token: &str) -> bool {
    runtime_status_kind_for_operation_token(token).is_some()
}

fn runtime_status_kind_for_operation_token(token: &str) -> Option<&'static str> {
    match normalize_schema_token(token).as_str() {
        "whoami" | "current_user" | "current_username" | "os_user" | "system_user"
        | "runtime_user" => Some("current_user"),
        "hostname" | "host_name" | "current_hostname" | "current_host" | "machine_name" => {
            Some("host_name")
        }
        "pwd"
        | "cwd"
        | "current_working_directory"
        | "current_directory"
        | "process_cwd"
        | "current_process_cwd" => Some("current_working_directory"),
        _ => None,
    }
}

fn output_contract_declares_scalar_locatorless_observation(value: Option<&Value>) -> bool {
    let Some(contract) = value.and_then(Value::as_object) else {
        return false;
    };
    let response_shape = contract
        .get("response_shape")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_response_shape(&value))
        .unwrap_or_default();
    let locator_kind = contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_locator_kind(&value))
        .unwrap_or_default();
    let delivery_intent = contract
        .get("delivery_intent")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_delivery_intent(&value))
        .unwrap_or_default();
    let semantic_kind = contract
        .get("semantic_kind")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_semantic_kind(&value))
        .unwrap_or_default();
    matches!(response_shape, OutputResponseShape::Scalar)
        && contract
            .get("requires_content_evidence")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && !contract
            .get("delivery_required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && matches!(
            locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        && matches!(delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            semantic_kind,
            OutputSemanticKind::None | OutputSemanticKind::ScalarPathOnly
        )
}

fn schema_token_is_runtime_observation_tool(token: &str) -> bool {
    matches!(
        token,
        "system_basic" | "system" | "system_query" | "run_cmd"
    )
}

fn execution_recipe_value_locator_hint(value: Option<&Value>) -> Option<String> {
    let map = value?.as_object()?;
    for key in [
        "path",
        "file_path",
        "target_path",
        "input_path",
        "source_path",
        "read_path",
        "filepath",
    ] {
        let Some(hint) = map
            .get(key)
            .and_then(scalar_json_value_text)
            .map(|hint| hint.trim().to_string())
            .filter(|hint| !hint.is_empty())
        else {
            continue;
        };
        return Some(hint);
    }
    None
}

fn normalizer_object_declares_tool_action_payload(obj: &serde_json::Map<String, Value>) -> bool {
    let has_action_args_payload = obj
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(|action| !action.trim().is_empty())
        && obj.get("args").is_some_and(value_has_nonempty_scalar_text);
    if has_action_args_payload {
        return true;
    }

    obj.get("steps")
        .and_then(Value::as_array)
        .is_some_and(|steps| {
            steps.iter().any(|step| {
                let Some(step) = step.as_object() else {
                    return false;
                };
                step.get("type")
                    .or_else(|| step.get("action"))
                    .and_then(Value::as_str)
                    .is_some_and(|kind| !kind.trim().is_empty())
                    && (step.get("args").is_some_and(value_has_nonempty_scalar_text)
                        || step.get("tool").is_some_and(value_has_nonempty_scalar_text)
                        || step
                            .get("skill")
                            .is_some_and(value_has_nonempty_scalar_text))
            })
        })
}

fn value_has_nonempty_scalar_text(value: &Value) -> bool {
    match value {
        Value::String(raw) => !raw.trim().is_empty(),
        Value::Array(items) => items.iter().any(value_has_nonempty_scalar_text),
        Value::Object(map) => map.values().any(value_has_nonempty_scalar_text),
        other => scalar_json_value_text(other).is_some_and(|text| !text.trim().is_empty()),
    }
}

fn schema_text_declares_execution_recipe(raw: &str) -> bool {
    !matches!(
        crate::execution_recipe::parse_execution_recipe_kind_text(raw),
        crate::execution_recipe::ExecutionRecipeKind::None
    ) || !matches!(
        crate::execution_recipe::parse_execution_recipe_profile_text(raw),
        crate::execution_recipe::ExecutionRecipeProfile::None
    )
}

fn mark_output_contract_requires_content_evidence(obj: &mut serde_json::Map<String, Value>) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    if let Some(contract) = value.as_object_mut() {
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    }
}

fn force_output_contract_semantic_kind(
    obj: &mut serde_json::Map<String, Value>,
    semantic_kind: OutputSemanticKind,
) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    if let Some(contract) = value.as_object_mut() {
        contract.insert(
            "semantic_kind".to_string(),
            Value::String(semantic_kind.as_str().to_string()),
        );
    }
}

fn normalize_output_contract_for_structured_read_recipe(
    obj: &mut serde_json::Map<String, Value>,
    locator_hint_from_recipe: Option<&str>,
    scalar_extraction: bool,
    request_declares_filename_only_schema_token: bool,
) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    let Some(contract) = value.as_object_mut() else {
        return;
    };

    contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    contract.insert("delivery_required".to_string(), Value::Bool(false));
    contract.insert(
        "delivery_intent".to_string(),
        Value::String("none".to_string()),
    );
    let model_only_filename_semantic = !request_declares_filename_only_schema_token
        && contract
            .get("semantic_kind")
            .and_then(scalar_json_value_text)
            .is_some_and(|value| {
                parse_output_semantic_kind(&value) == OutputSemanticKind::FileNames
            });

    if scalar_extraction || model_only_filename_semantic {
        contract.insert(
            "response_shape".to_string(),
            Value::String("scalar".to_string()),
        );
    }
    if scalar_extraction || model_only_filename_semantic {
        contract.insert(
            "semantic_kind".to_string(),
            Value::String(OutputSemanticKind::None.as_str().to_string()),
        );
    }
    if let Some(hint) = locator_hint_from_recipe
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
    {
        contract.insert(
            "locator_kind".to_string(),
            Value::String("path".to_string()),
        );
        contract.insert("locator_hint".to_string(), Value::String(hint.to_string()));
    }
}

fn normalize_output_contract_for_package_manager_detection(
    obj: &mut serde_json::Map<String, Value>,
) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    let Some(contract) = value.as_object_mut() else {
        return;
    };

    contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    contract.insert("delivery_required".to_string(), Value::Bool(false));
    contract.insert(
        "delivery_intent".to_string(),
        Value::String("none".to_string()),
    );
    contract.insert(
        "locator_kind".to_string(),
        Value::String("none".to_string()),
    );
    contract.insert("locator_hint".to_string(), Value::String(String::new()));
    contract.insert(
        "semantic_kind".to_string(),
        Value::String(
            OutputSemanticKind::PackageManagerDetection
                .as_str()
                .to_string(),
        ),
    );
}

fn normalize_output_contract_for_service_status_recipe(obj: &mut serde_json::Map<String, Value>) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    let Some(contract) = value.as_object_mut() else {
        return;
    };

    contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    contract.insert("delivery_required".to_string(), Value::Bool(false));
    contract.insert(
        "delivery_intent".to_string(),
        Value::String("none".to_string()),
    );
    contract.insert(
        "locator_kind".to_string(),
        Value::String("none".to_string()),
    );
    contract.insert("locator_hint".to_string(), Value::String(String::new()));
    contract.insert(
        "semantic_kind".to_string(),
        Value::String(OutputSemanticKind::ServiceStatus.as_str().to_string()),
    );
}

fn normalize_output_contract_for_command_payload(
    obj: &mut serde_json::Map<String, Value>,
    locator_hint_from_recipe: Option<&str>,
) {
    let value = obj
        .entry("output_contract".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        coerce_output_contract_value_for_schema(value);
    }
    let Some(contract) = value.as_object_mut() else {
        return;
    };

    let locator_hint = contract
        .get("locator_hint")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let locator_kind = contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .map(|value| normalize_output_locator_kind_for_schema(&value))
        .unwrap_or("none");
    if locator_hint.trim().is_empty() {
        if let Some(hint) = locator_hint_from_recipe
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
        {
            contract.insert(
                "locator_kind".to_string(),
                Value::String("path".to_string()),
            );
            contract.insert("locator_hint".to_string(), Value::String(hint.to_string()));
        } else if locator_kind != "none" {
            contract.insert(
                "locator_kind".to_string(),
                Value::String("none".to_string()),
            );
            contract.insert("locator_hint".to_string(), Value::String(String::new()));
        }
    } else if locator_kind == "none" {
        contract.insert(
            "locator_kind".to_string(),
            Value::String("path".to_string()),
        );
    }

    let semantic_kind = contract
        .get("semantic_kind")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    if matches!(
        parse_output_semantic_kind(&semantic_kind),
        OutputSemanticKind::None
    ) {
        contract.insert(
            "semantic_kind".to_string(),
            Value::String(OutputSemanticKind::RawCommandOutput.as_str().to_string()),
        );
    }
    contract.insert("delivery_required".to_string(), Value::Bool(false));
}

fn apply_raw_output_explicit_locator_repair(
    output_contract: &mut IntentOutputContract,
    request: &str,
    command_runtime: &crate::CommandIntentRuntime,
) -> Option<&'static str> {
    if !output_contract.requires_content_evidence
        || output_contract.delivery_required
        || output_contract.semantic_kind != OutputSemanticKind::RawCommandOutput
        || output_contract.locator_kind != OutputLocatorKind::None
        || !output_contract.locator_hint.trim().is_empty()
        || crate::agent_engine::explicit_command_segment_for_policy(command_runtime, request)
            .is_some()
    {
        return None;
    }
    let locator = crate::intent::locator_extractor::extract_explicit_locator_for_fallback(request)?;
    if !matches!(
        locator.locator_kind,
        OutputLocatorKind::Path | OutputLocatorKind::Url
    ) {
        return None;
    }
    output_contract.locator_kind = locator.locator_kind;
    output_contract.locator_hint = locator.locator_hint;
    Some("raw_output_explicit_locator_contract_repair")
}

fn mark_decision_planner_execute_from_execution_recipe(obj: &mut serde_json::Map<String, Value>) {
    if obj
        .get("needs_clarify")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return;
    }
    let current = obj
        .get("decision")
        .and_then(scalar_json_value_text)
        .and_then(|value| canonical_first_layer_decision_token(&value));
    if current.is_none() || matches!(current, Some(FirstLayerDecision::DirectAnswer)) {
        obj.insert(
            "decision".to_string(),
            Value::String(FirstLayerDecision::PlannerExecute.as_str().to_string()),
        );
    }
}

fn promote_misnested_turn_analysis_from_execution_recipe(obj: &mut serde_json::Map<String, Value>) {
    let Some(recipe) = obj.get("execution_recipe").and_then(Value::as_object) else {
        return;
    };
    let misplaced_turn_type = recipe
        .get("turn_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let misplaced_target_policy = recipe
        .get("target_task_policy")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let misplaced_interrupt = recipe
        .get("should_interrupt_active_run")
        .and_then(Value::as_bool)
        .filter(|value| *value);
    let misplaced_attachment = recipe
        .get("attachment_processing_required")
        .and_then(Value::as_bool)
        .filter(|value| *value);
    let misplaced_state_patch = recipe
        .get("state_patch")
        .filter(|value| is_meaningful_state_patch(value))
        .cloned();

    if let Some(turn_type) = misplaced_turn_type {
        if obj
            .get("turn_type")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            obj.insert("turn_type".to_string(), Value::String(turn_type));
        }
    }
    if let Some(target_policy) = misplaced_target_policy {
        if obj
            .get("target_task_policy")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            obj.insert(
                "target_task_policy".to_string(),
                Value::String(target_policy),
            );
        }
    }
    if misplaced_interrupt.is_some()
        && !obj
            .get("should_interrupt_active_run")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        obj.insert("should_interrupt_active_run".to_string(), Value::Bool(true));
    }
    if misplaced_attachment.is_some()
        && !obj
            .get("attachment_processing_required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        obj.insert(
            "attachment_processing_required".to_string(),
            Value::Bool(true),
        );
    }
    if let Some(state_patch) = misplaced_state_patch {
        if obj
            .get("state_patch")
            .is_none_or(|value| !is_meaningful_state_patch(value))
        {
            obj.insert("state_patch".to_string(), state_patch);
        }
    }
}

fn coerce_output_contract_value_for_schema(value: &mut Value) {
    if value.is_object() {
        return;
    }

    let mut contract = serde_json::Map::new();
    if let Some(raw) = value.as_str().map(str::trim).filter(|raw| !raw.is_empty()) {
        let response_shape = normalize_output_response_shape_for_schema(raw);
        if response_shape != "free" {
            contract.insert(
                "response_shape".to_string(),
                Value::String(response_shape.to_string()),
            );
        }
        let semantic_kind = normalize_output_semantic_kind_for_schema(raw);
        if semantic_kind != "none" {
            contract.insert(
                "semantic_kind".to_string(),
                Value::String(semantic_kind.to_string()),
            );
        }
    }
    *value = Value::Object(contract);
}

fn normalize_output_contract_aliases(contract: &mut serde_json::Map<String, Value>) {
    let raw_type_token = contract
        .get("type")
        .and_then(scalar_json_value_text)
        .map(|value| normalize_schema_token(&value));
    let file_delivery_type = raw_type_token.as_deref().is_some_and(|token| {
        matches!(
            token,
            "file" | "file_token" | "delivery_file" | "attachment" | "artifact_file"
        )
    });
    if !contract.contains_key("response_shape") {
        for alias in ["shape", "answer_shape", "format", "response_format"] {
            if let Some(value) = contract.get(alias).cloned() {
                contract.insert("response_shape".to_string(), value);
                break;
            }
        }
    }
    if !contract.contains_key("response_shape") && file_delivery_type {
        contract.insert(
            "response_shape".to_string(),
            Value::String("file_token".to_string()),
        );
    }
    if !contract.contains_key("response_shape")
        && contract
            .get("type")
            .and_then(Value::as_str)
            .map(normalize_schema_token)
            .is_some_and(|token| matches!(token.as_str(), "list" | "array"))
    {
        contract.insert(
            "response_shape".to_string(),
            Value::String("strict".to_string()),
        );
    }
    if !contract.contains_key("locator_hint") {
        for alias in [
            "filename",
            "file_name",
            "file_path",
            "path",
            "target_path",
            "locator",
            "locator_value",
        ] {
            if let Some(value) = contract
                .get(alias)
                .and_then(scalar_json_value_text)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                contract.insert("locator_hint".to_string(), Value::String(value));
                break;
            }
        }
    }
    if file_delivery_type {
        if !contract.contains_key("delivery_required") {
            contract.insert("delivery_required".to_string(), Value::Bool(true));
        }
        if !contract.contains_key("delivery_intent") {
            contract.insert(
                "delivery_intent".to_string(),
                Value::String("file_single".to_string()),
            );
        }
        if !contract.contains_key("locator_kind")
            && contract
                .get("locator_hint")
                .and_then(scalar_json_value_text)
                .is_some_and(|hint| !hint.trim().is_empty())
        {
            let hint = contract
                .get("locator_hint")
                .and_then(scalar_json_value_text)
                .unwrap_or_default();
            let kind = if hint.contains('/') || hint.contains('\\') {
                "path"
            } else {
                "filename"
            };
            contract.insert("locator_kind".to_string(), Value::String(kind.to_string()));
        }
    }
    if !contract.contains_key("exact_sentence_count") {
        for alias in ["sentence_count", "sentences", "exact_sentences"] {
            if let Some(value) = contract.get(alias).cloned() {
                contract.insert("exact_sentence_count".to_string(), value);
                break;
            }
        }
    }
    if !contract.contains_key("semantic_kind") {
        for alias in ["semantic", "kind", "answer_kind", "semantic_type"] {
            if let Some(value) = contract.get(alias).cloned() {
                contract.insert("semantic_kind".to_string(), value);
                break;
            }
        }
    }
}

fn normalize_output_contract_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let direct_answer_decision = obj
        .get("decision")
        .and_then(scalar_json_value_text)
        .and_then(|text| canonical_first_layer_decision_token(&text))
        .is_some_and(|decision| decision == FirstLayerDecision::DirectAnswer);
    let Some(value) = obj.get_mut("output_contract") else {
        return;
    };
    let raw_scalar_output_contract_token = value.as_str().map(normalize_schema_token);
    coerce_output_contract_value_for_schema(value);
    let Some(contract) = value.as_object_mut() else {
        return;
    };
    normalize_output_contract_aliases(contract);
    let raw_response_shape_token = contract_value_token(contract, "response_shape");
    let raw_locator_kind_token = contract_value_token(contract, "locator_kind");
    let raw_delivery_required_token = contract_value_token(contract, "delivery_required");
    let raw_semantic_kind_token = contract_value_token(contract, "semantic_kind");
    let raw_locator_hint_token = contract_value_token(contract, "locator_hint");
    contract.retain(|key, _| {
        matches!(
            key.as_str(),
            "response_shape"
                | "exact_sentence_count"
                | "requires_content_evidence"
                | "delivery_required"
                | "locator_kind"
                | "delivery_intent"
                | "semantic_kind"
                | "locator_hint"
                | "self_extension"
        )
    });
    contract
        .entry("response_shape".to_string())
        .or_insert_with(|| Value::String("free".to_string()));
    let response_shape = contract
        .get("response_shape")
        .and_then(|value| value.as_str())
        .map(normalize_output_response_shape_for_schema)
        .unwrap_or("free");
    contract.insert(
        "response_shape".to_string(),
        Value::String(response_shape.to_string()),
    );
    if let Some(value) = contract.get("exact_sentence_count").cloned() {
        if let Some(count) = parse_positive_usize_value(&value) {
            contract.insert(
                "exact_sentence_count".to_string(),
                Value::Number(serde_json::Number::from(count as u64)),
            );
            if count > 1 && response_shape == "one_sentence" {
                contract.insert(
                    "response_shape".to_string(),
                    Value::String("strict".to_string()),
                );
            }
        } else {
            contract.remove("exact_sentence_count");
        }
    }
    contract
        .entry("requires_content_evidence".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(contract, "requires_content_evidence", false);
    contract
        .entry("delivery_required".to_string())
        .or_insert(Value::Bool(false));
    normalize_bool_field_with_default(contract, "delivery_required", false);
    contract
        .entry("locator_kind".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    let locator_kind = contract
        .get("locator_kind")
        .and_then(|value| value.as_str())
        .map(normalize_output_locator_kind_for_schema)
        .unwrap_or("none");
    contract.insert(
        "locator_kind".to_string(),
        Value::String(locator_kind.to_string()),
    );
    contract
        .entry("delivery_intent".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    let delivery_intent = contract
        .get("delivery_intent")
        .and_then(|value| value.as_str())
        .map(normalize_output_delivery_intent_for_schema)
        .unwrap_or("none");
    contract.insert(
        "delivery_intent".to_string(),
        Value::String(delivery_intent.to_string()),
    );
    contract
        .entry("semantic_kind".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    let semantic_kind = contract
        .get("semantic_kind")
        .and_then(|value| value.as_str())
        .map(normalize_output_semantic_kind_for_schema)
        .unwrap_or("none");
    let semantic_kind =
        if raw_scalar_output_contract_token.as_deref() == Some("raw") && direct_answer_decision {
            "none"
        } else {
            semantic_kind
        };
    contract.insert(
        "semantic_kind".to_string(),
        Value::String(semantic_kind.to_string()),
    );
    let semantic_kind_enum = parse_output_semantic_kind(semantic_kind);
    if matches!(
        semantic_kind,
        kind if kind == OutputSemanticKind::FileNames.as_str()
            || kind == OutputSemanticKind::DirectoryNames.as_str()
            || kind == OutputSemanticKind::DirectoryEntryGroups.as_str()
            || kind == OutputSemanticKind::FilePaths.as_str()
    ) && response_shape == "free"
    {
        contract.insert(
            "response_shape".to_string(),
            Value::String("strict".to_string()),
        );
    }
    if semantic_kind == OutputSemanticKind::FileNames.as_str()
        || semantic_kind == OutputSemanticKind::DirectoryNames.as_str()
        || semantic_kind == OutputSemanticKind::DirectoryEntryGroups.as_str()
        || semantic_kind == OutputSemanticKind::FilePaths.as_str()
        || semantic_kind == OutputSemanticKind::ContentPresenceCheck.as_str()
        || semantic_kind == OutputSemanticKind::GitCommitSubject.as_str()
        || semantic_kind == OutputSemanticKind::GitRepositoryState.as_str()
    {
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    }
    if output_semantic_kind_requires_fresh_evidence(semantic_kind_enum) {
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
    }
    if semantic_kind == OutputSemanticKind::HiddenEntriesCheck.as_str() {
        if matches!(response_shape, "free" | "one_sentence") {
            contract.insert(
                "response_shape".to_string(),
                Value::String("strict".to_string()),
            );
        }
        contract.insert("delivery_required".to_string(), Value::Bool(false));
    }
    if semantic_kind == OutputSemanticKind::ExistenceWithPath.as_str()
        || semantic_kind == OutputSemanticKind::ExistenceWithPathSummary.as_str()
    {
        if matches!(response_shape, "free" | "one_sentence") {
            contract.insert(
                "response_shape".to_string(),
                Value::String("strict".to_string()),
            );
        }
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
        contract.insert("delivery_required".to_string(), Value::Bool(false));
    }
    if semantic_kind == OutputSemanticKind::ExecutionFailedStep.as_str() {
        if matches!(response_shape, "free" | "one_sentence") {
            contract.insert(
                "response_shape".to_string(),
                Value::String("strict".to_string()),
            );
        }
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
        contract.insert("delivery_required".to_string(), Value::Bool(false));
    }
    if semantic_kind == OutputSemanticKind::GeneratedFileDelivery.as_str() {
        contract.insert(
            "response_shape".to_string(),
            Value::String("file_token".to_string()),
        );
        contract.insert("requires_content_evidence".to_string(), Value::Bool(true));
        contract.insert("delivery_required".to_string(), Value::Bool(true));
        contract.insert(
            "delivery_intent".to_string(),
            Value::String("file_single".to_string()),
        );
        if locator_kind == "none" {
            contract.insert(
                "locator_kind".to_string(),
                Value::String("current_workspace".to_string()),
            );
        }
    }
    contract
        .entry("locator_hint".to_string())
        .or_insert_with(|| Value::String(String::new()));
    normalize_optional_string_field(contract, "locator_hint");
    if locator_kind == "none" {
        contract.insert("locator_hint".to_string(), Value::String(String::new()));
    }
    let current_workspace_path_alias = raw_semantic_kind_token == "filesystem_locator"
        && (looks_like_current_workspace_path_alias(&raw_locator_hint_token)
            || looks_like_current_workspace_path_alias(&raw_delivery_required_token)
            || looks_like_current_workspace_path_alias(&raw_locator_kind_token));
    if current_workspace_path_alias {
        contract.insert(
            "response_shape".to_string(),
            Value::String("scalar".to_string()),
        );
        contract.insert(
            "semantic_kind".to_string(),
            Value::String("scalar_path_only".to_string()),
        );
        contract.insert(
            "locator_kind".to_string(),
            Value::String("current_workspace".to_string()),
        );
        contract.insert(
            "delivery_intent".to_string(),
            Value::String("none".to_string()),
        );
        contract.insert("delivery_required".to_string(), Value::Bool(false));
        contract.insert("requires_content_evidence".to_string(), Value::Bool(false));
        contract.insert("locator_hint".to_string(), Value::String(String::new()));
    } else if raw_response_shape_token == "plain_text"
        && looks_like_current_workspace_path_alias(&raw_locator_hint_token)
        && response_shape == "free"
    {
        contract.insert(
            "response_shape".to_string(),
            Value::String("scalar".to_string()),
        );
        contract.insert(
            "semantic_kind".to_string(),
            Value::String("scalar_path_only".to_string()),
        );
        contract.insert(
            "locator_kind".to_string(),
            Value::String("current_workspace".to_string()),
        );
        contract.insert("locator_hint".to_string(), Value::String(String::new()));
    }
    let self_extension = contract
        .entry("self_extension".to_string())
        .or_insert_with(|| {
            serde_json::json!({
                "mode": "none",
                "trigger": "none",
                "execute_now": false
            })
        });
    if !self_extension.is_object() {
        *self_extension = serde_json::json!({
            "mode": "none",
            "trigger": "none",
            "execute_now": false
        });
    }
    if let Some(self_extension) = self_extension.as_object_mut() {
        self_extension.retain(|key, _| matches!(key.as_str(), "mode" | "trigger" | "execute_now"));
        self_extension
            .entry("mode".to_string())
            .or_insert_with(|| Value::String("none".to_string()));
        self_extension
            .entry("trigger".to_string())
            .or_insert_with(|| Value::String("none".to_string()));
        self_extension
            .entry("execute_now".to_string())
            .or_insert(Value::Bool(false));
    }
}

fn cap_intent_normalizer_prompt_for_llm_budget(
    state: &AppState,
    task: &ClaimedTask,
    prompt: String,
) -> String {
    let max_bytes = intent_normalizer_max_prompt_bytes(state, task);
    if prompt.len() <= max_bytes {
        return prompt;
    }
    warn!(
        "intent_normalizer_prompt oversized vs provider budget — truncating head+tail task_id={} bytes_before={} bytes_budget={}",
        task.task_id,
        prompt.len(),
        max_bytes
    );
    let head_take = (max_bytes * 35) / 100;
    let tail_take = (max_bytes * 55) / 100;
    let note_budget = max_bytes
        .saturating_sub(head_take)
        .saturating_sub(tail_take)
        .max(32);
    let note = format!(
        "\n\n[RustClaw: omitted {} bytes of middle context to fit provider window]\n\n",
        prompt
            .len()
            .saturating_sub(head_take.saturating_add(tail_take))
    );
    let head = crate::providers::utf8_safe_prefix(&prompt, head_take);
    let note = crate::providers::utf8_safe_prefix(&note, note_budget);
    let tail = crate::providers::utf8_safe_suffix(&prompt, tail_take);
    let capped = format!("{head}{note}{tail}");
    state.note_task_prompt_truncation_with_label(
        &task.task_id,
        "normalizer",
        prompt.len(),
        max_bytes,
        capped.len(),
    );
    capped
}

async fn run_contract_repair_judge(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    raw_normalizer_output: &str,
    normalized_route_json: &str,
    repair_report: &ContractRepairReport,
    repair_context: &str,
) -> Option<ContractRepairJudgeOut> {
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        CONTRACT_REPAIR_JUDGE_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            info!(
                "{} contract_repair_judge prompt_missing task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            ("__REQUEST__", user_request.trim()),
            (
                "__NORMALIZED_ROUTE_JSON__",
                &crate::truncate_for_log(normalized_route_json),
            ),
            ("__CONTRACT_REPAIR_SOURCE__", &repair_report.source_csv()),
            ("__CONTRACT_REPAIR_DETAIL__", &repair_report.detail_csv()),
            ("__CONTRACT_REPAIR_CONTEXT__", repair_context),
            (
                "__RAW_NORMALIZER_OUTPUT__",
                &crate::truncate_for_log(raw_normalizer_output),
            ),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "contract_repair_judge_prompt",
        &resolved.source,
        resolved.version.as_deref(),
        None,
    );
    let prompt_source = resolved.source;
    let llm_out = match llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            info!(
                "{} contract_repair_judge llm_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    match crate::prompt_utils::validate_against_schema::<ContractRepairJudgeOut>(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::ContractRepairJudge,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok || validated.schema_normalized {
                info!(
                    "{} contract_repair_judge schema_parse_recovery task_id={} raw_parse_ok={} schema_normalized={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    validated.raw_parse_ok,
                    validated.schema_normalized
                );
            }
            Some(validated.value)
        }
        Err(err) => {
            info!(
                "{} contract_repair_judge schema_validation_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            None
        }
    }
}

fn apply_contract_repair_judge_output(
    out: &mut IntentNormalizerOut,
    repair: ContractRepairJudgeOut,
) -> bool {
    if !repair.apply || repair.confidence < 0.60 {
        return false;
    }
    let Some(mut decision) = parse_first_layer_decision_text(&repair.decision) else {
        return false;
    };
    let Some(mut output_contract) = repair.output_contract else {
        return false;
    };
    let Some(mut execution_recipe) = repair.execution_recipe else {
        return false;
    };
    let preserved_structured_config_keys = preserve_structured_config_key_contract_during_repair(
        out.output_contract.as_ref(),
        &mut output_contract,
    );
    let missing_turn_binding_for_content_read =
        contract_repair_reason_requires_missing_locator_clarify(&repair.reason);
    let mut needs_clarify = repair.needs_clarify;
    let mut clarify_question = repair.clarify_question;

    if missing_turn_binding_for_content_read {
        decision = FirstLayerDecision::Clarify;
        needs_clarify = true;
        clarify_question.clear();
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.locator_kind = "none".to_string();
        output_contract.delivery_intent = "none".to_string();
        output_contract.locator_hint.clear();
        execution_recipe = IntentExecutionRecipeOut::default();
    }

    out.decision = decision.as_str().to_string();
    out.needs_clarify = needs_clarify;
    out.clarify_question = clarify_question;
    if !repair.resolved_user_intent.trim().is_empty() {
        out.resolved_user_intent = repair.resolved_user_intent;
    }
    out.wants_file_delivery = repaired_contract_wants_file_delivery(&output_contract);
    out.output_contract = Some(output_contract);
    out.execution_recipe = Some(execution_recipe);
    if missing_turn_binding_for_content_read {
        out.state_patch = Some(json!({"deictic_reference": {"target": "missing_locator"}}));
    } else if repair
        .state_patch
        .as_ref()
        .is_some_and(is_meaningful_state_patch)
    {
        out.state_patch = repair.state_patch;
    }
    out.confidence = repair.confidence.clamp(0.0, 1.0);
    let repaired_turn_type = normalize_schema_token(&repair.turn_type);
    if repaired_turn_type.is_empty() {
        if parse_turn_type(&out.turn_type).is_none() {
            out.turn_type.clear();
        }
    } else if parse_turn_type(&repaired_turn_type).is_some() {
        out.turn_type = repaired_turn_type;
    }
    let repaired_target_task_policy = normalize_schema_token(&repair.target_task_policy);
    if repaired_target_task_policy.is_empty() {
        if parse_target_task_policy(&out.target_task_policy).is_none() {
            out.target_task_policy.clear();
        }
    } else if parse_target_task_policy(&repaired_target_task_policy).is_some() {
        out.target_task_policy = repaired_target_task_policy;
    }
    if repair.reason.trim().is_empty() {
        append_route_reason(&mut out.reason, "llm_semantic_contract_repair");
    } else {
        append_route_reason(
            &mut out.reason,
            &format!("llm_semantic_contract_repair:{}", repair.reason.trim()),
        );
    }
    if preserved_structured_config_keys {
        append_route_reason(&mut out.reason, "structured_config_key_contract_preserved");
    }
    true
}

fn preserve_structured_config_key_contract_during_repair(
    current: Option<&IntentOutputContractOut>,
    repair: &mut IntentOutputContractOut,
) -> bool {
    let Some(current) = current else {
        return false;
    };
    let current_contract = parse_output_contract(Some(current.clone()), false);
    let repaired_contract = parse_output_contract(Some(repair.clone()), false);
    if current_contract.semantic_kind != OutputSemanticKind::StructuredKeys
        || repaired_contract.semantic_kind != OutputSemanticKind::None
        || current_contract.delivery_required
        || repaired_contract.delivery_required
        || !matches!(current_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            repaired_contract.delivery_intent,
            OutputDeliveryIntent::None
        )
        || output_contract_structured_config_path(&current_contract).is_none()
        || output_contract_structured_config_path(&repaired_contract).is_none()
    {
        return false;
    }
    repair.semantic_kind = OutputSemanticKind::StructuredKeys.as_str().to_string();
    repair.requires_content_evidence = true;
    repair.delivery_required = false;
    repair.delivery_intent = OutputDeliveryIntent::None.as_str().to_string();
    if matches!(
        repaired_contract.response_shape,
        OutputResponseShape::Free | OutputResponseShape::OneSentence
    ) {
        repair.response_shape = OutputResponseShape::Strict.as_str().to_string();
    }
    true
}

fn contract_repair_reason_requires_missing_locator_clarify(reason: &str) -> bool {
    reason
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .any(|token| token == "execution_recipe_untrusted_text_ignored_and_turn_binding_missing_for_content_read")
}

fn repaired_contract_wants_file_delivery(contract: &IntentOutputContractOut) -> bool {
    contract.delivery_required
        || matches!(
            parse_output_response_shape(&contract.response_shape),
            OutputResponseShape::FileToken
        )
        || !matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
}

fn append_route_reason(reason: &mut String, addition: &str) {
    let addition = addition.trim();
    if addition.is_empty() || reason.contains(addition) {
        return;
    }
    if reason.trim().is_empty() {
        *reason = addition.to_string();
    } else {
        reason.push_str("; ");
        reason.push_str(addition);
    }
}

/// Unified intent normalizer: one LLM call for resume decision, intent completion,
/// schedule classification, clarify state, and the first-layer decision.
pub(crate) async fn run_intent_normalizer(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    resume_context: Option<&Value>,
    binding_context: Option<&Value>,
    now_iso: &str,
    timezone: &str,
    schedule_rules: &str,
) -> IntentNormalizerOutput {
    let req = user_request.trim();
    let surface_req = request_without_contract_test_hint(req);
    let req_surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    if contract_test_hint_semantic_kind(req).is_some() {
        // This is a machine-readable contract-matrix test hook, not a
        // natural-language intent classifier. Parse it before the normalizer so
        // large NL suites do not spend a model call rediscovering the contract.
        if let Some(fallback) = contract_hint_fallback_decision(
            req,
            &req_surface,
            &state.skill_rt.workspace_root,
            "contract_hint_fast_path",
        ) {
            info!(
                "{} intent_normalizer task_id={} contract_hint_fast_path reason={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                fallback.reason,
                crate::truncate_for_log(req)
            );
            return normalizer_output_from_fallback(
                req,
                "structured_contract_hint_fast_path",
                fallback,
                None,
            );
        }
    }
    let context_bundle = crate::task_context_builder::build_route_task_context_bundle(
        state,
        task,
        user_request,
        session_snapshot,
        resume_context,
        binding_context,
        now_iso,
        timezone,
        schedule_rules,
    );
    let route_view = context_bundle
        .route_view
        .as_ref()
        .expect("route context bundle should include route_view");
    let resolved_prompt = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        INTENT_NORMALIZER_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            warn!(
                "intent_normalizer prompt load failed, falling back to safe clarify: task_id={} err={}",
                task.task_id, err
            );
            if let Some(fallback) = inline_json_transform_fallback_decision(req) {
                info!(
                    "{} intent_normalizer task_id={} prompt_missing_inline_json_transform_fallback input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(req)
                );
                return normalizer_output_from_fallback(
                    req,
                    "prompt_missing_inline_json_transform_fallback",
                    fallback,
                    None,
                );
            }
            if let Some(fallback) = directory_pair_fallback_decision(state, &surface_req) {
                info!(
                    "{} intent_normalizer task_id={} prompt_missing_directory_pair_fallback reason={} input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    fallback.reason,
                    crate::truncate_for_log(req)
                );
                return normalizer_output_from_fallback(
                    req,
                    "prompt_missing_directory_pair_fallback",
                    fallback,
                    None,
                );
            }
            let fallback = empty_clarify_decision(req, "normalizer_prompt_missing");
            return normalizer_output_from_fallback(
                req,
                "prompt_missing_safe_clarify",
                fallback,
                None,
            );
        }
    };
    let prompt_template = resolved_prompt.template;
    let prompt_source = resolved_prompt.source;
    let prompt_version = resolved_prompt.version;
    let request_language_hint = crate::language_policy::preferred_response_language_hint(
        req,
        session_snapshot
            .and_then(|snapshot| snapshot.conversation_state.as_ref())
            .and_then(|conversation_state| conversation_state.locale_hint.as_deref()),
    );
    let auth_policy_context = render_auth_policy_context(state, task);
    let max_prompt_bytes = intent_normalizer_max_prompt_bytes(state, task);
    let compact_prompt_required = intent_normalizer_uses_compact_prompt(state, task);
    let mut prompt = if compact_prompt_required {
        warn!(
            "intent_normalizer using compact prompt for small-context provider: task_id={}",
            task.task_id
        );
        render_compact_intent_normalizer_prompt(
            route_view,
            &context_bundle,
            &auth_policy_context,
            &request_language_hint,
            req,
        )
    } else {
        crate::render_prompt_template(
            &prompt_template,
            &[
                ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
                ("__AUTH_POLICY_CONTEXT__", &auth_policy_context),
                ("__CAPABILITY_MAP__", &route_view.capability_map),
                (
                    "__SELF_EXTENSION_RUNTIME__",
                    &render_self_extension_runtime(state),
                ),
                (
                    "__RESUME_CONTEXT__",
                    &context_bundle.raw_sources.resume_context,
                ),
                (
                    "__BINDING_CONTEXT__",
                    &context_bundle.raw_sources.binding_context,
                ),
                ("__ACTIVE_TASK_CONTEXT__", &route_view.active_task_context),
                (
                    "__ACTIVE_EXECUTION_ANCHOR__",
                    &route_view.active_execution_anchor_context,
                ),
                (
                    "__SESSION_ALIAS_CONTEXT__",
                    &route_view.session_alias_context,
                ),
                (
                    "__REQUEST_SURFACE_HINTS__",
                    &route_view.request_surface_hints,
                ),
                (
                    "__RECENT_EXECUTION_CONTEXT__",
                    &route_view.recent_execution_context,
                ),
                ("__MEMORY_CONTEXT__", &route_view.memory_context),
                ("__RECENT_TURNS_FULL__", &route_view.recent_turns_full),
                ("__LAST_TURN_FULL__", &route_view.last_turn_full),
                (
                    "__RECENT_ASSISTANT_REPLIES__",
                    &route_view.recent_assistant_replies,
                ),
                ("__NOW__", &context_bundle.raw_sources.now_iso),
                ("__TIMEZONE__", &context_bundle.raw_sources.timezone),
                (
                    "__SCHEDULE_RULES__",
                    &context_bundle.raw_sources.schedule_rules,
                ),
                ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
                ("__REQUEST__", req),
            ],
        )
    };
    if !compact_prompt_required && prompt.len() > max_prompt_bytes {
        warn!(
            "intent_normalizer full prompt exceeds provider budget, switching to compact prompt: task_id={} bytes_before={} bytes_budget={}",
            task.task_id,
            prompt.len(),
            max_prompt_bytes
        );
        prompt = render_compact_intent_normalizer_prompt(
            route_view,
            &context_bundle,
            &auth_policy_context,
            &request_language_hint,
            req,
        );
    }
    let prompt = cap_intent_normalizer_prompt_for_llm_budget(state, task, prompt);
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "intent_normalizer_prompt",
        &prompt_source,
        prompt_version.as_deref(),
        None,
    );
    let llm_out = match llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "intent_normalizer llm failed, falling back to safe clarify: task_id={} err={}",
                task.task_id, err
            );
            if let Some(fallback) = contract_hint_fallback_decision(
                req,
                &req_surface,
                &state.skill_rt.workspace_root,
                "normalizer_unavailable_contract_hint",
            ) {
                info!(
                    "{} intent_normalizer task_id={} llm_failed_contract_hint_fallback reason={} input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    fallback.reason,
                    crate::truncate_for_log(req)
                );
                return normalizer_output_from_fallback(
                    req,
                    "llm_failed_contract_hint_fallback",
                    fallback,
                    None,
                );
            }
            if let Some(fallback) = inline_json_transform_fallback_decision(req) {
                info!(
                    "{} intent_normalizer task_id={} llm_failed_inline_json_transform_fallback input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(req)
                );
                return normalizer_output_from_fallback(
                    req,
                    "llm_failed_inline_json_transform_fallback",
                    fallback,
                    None,
                );
            }
            if let Some(fallback) = directory_pair_fallback_decision(state, &surface_req) {
                info!(
                    "{} intent_normalizer task_id={} llm_failed_directory_pair_fallback reason={} input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    fallback.reason,
                    crate::truncate_for_log(req)
                );
                return normalizer_output_from_fallback(
                    req,
                    "llm_failed_directory_pair_fallback",
                    fallback,
                    None,
                );
            }
            if let Some(fallback) = parse_failed_explicit_capability_fallback_decision(
                &surface_req,
                &req_surface,
                &state.skill_rt.workspace_root,
            ) {
                info!(
                    "{} intent_normalizer task_id={} llm_failed_explicit_capability_fallback reason={} input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    fallback.reason,
                    crate::truncate_for_log(req)
                );
                return normalizer_output_from_fallback(
                    req,
                    "llm_failed_structured_capability_fallback",
                    fallback,
                    None,
                );
            }
            if let Some(fallback) = explicit_surface_path_facts_fallback_decision(
                &surface_req,
                &req_surface,
                &state.skill_rt.workspace_root,
            ) {
                info!(
                    "{} intent_normalizer task_id={} llm_failed_explicit_surface_fallback reason={} input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    fallback.reason,
                    crate::truncate_for_log(req)
                );
                return normalizer_output_from_fallback(
                    req,
                    "llm_failed_structured_surface_fallback",
                    fallback,
                    None,
                );
            }
            // Planner-first: do not recover semantic execution locally when the
            // normalizer LLM is unavailable unless the request surface carries a
            // narrow structured fallback handled above.
            let fallback = empty_clarify_decision(req, "normalizer_llm_failed");
            return normalizer_output_from_fallback(
                req,
                "llm_failed_safe_clarify",
                fallback,
                Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable),
            );
        }
    };
    let (llm_out_for_parse, contract_repair_report) =
        normalize_intent_normalizer_raw_for_schema_with_report(&llm_out, req);
    let parsed = crate::prompt_utils::validate_against_schema::<IntentNormalizerOut>(
        &llm_out_for_parse,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    );
    if let Ok(validated) = &parsed {
        if !validated.raw_parse_ok {
            info!(
                "{} intent_normalizer task_id={} parse_recovery=schema_repair contract_repair_source={} contract_repair_detail={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                contract_repair_report.source_csv(),
                contract_repair_report.detail_csv(),
                crate::truncate_for_log(req)
            );
        }
    }
    let parsed = match parsed {
        Ok(validated) => Some(validated.value),
        Err(err) => {
            warn!(
                "intent_normalizer schema parse failed, falling back to safe clarify: task_id={} err={} normalized_raw={}",
                task.task_id,
                err,
                crate::truncate_for_log(&llm_out_for_parse)
            );
            None
        }
    };
    let (parsed, contract_repair_report) = match parsed {
        Some(parsed) => (Some(parsed), contract_repair_report),
        None => {
            if let Some((retry_out, retry_report)) = retry_intent_normalizer_json_parse(
                state,
                task,
                route_view,
                &context_bundle,
                &auth_policy_context,
                &request_language_hint,
                req,
                &prompt_source,
                &contract_repair_report,
            )
            .await
            {
                (Some(retry_out), retry_report)
            } else {
                (None, contract_repair_report)
            }
        }
    };
    if let Some(mut out) = parsed {
        let mut contract_repair_report = contract_repair_report;
        let answer_candidate_binding =
            analyze_answer_candidate_binding(req, &out.answer_candidate, route_view);
        let mut contract_repair_context = String::from("none");
        let mut active_text_answer_candidate_conflict = false;
        let cleared_memory_update_candidate = clear_memory_update_answer_candidate_if_memory_only(
            &mut out,
            answer_candidate_binding.as_ref(),
        )
        .is_some();
        if cleared_memory_update_candidate {
            contract_repair_report.add(
                "structural_cleanup",
                "memory_update_unbound_answer_candidate_cleared",
            );
        } else if rebind_memory_only_answer_candidate_to_recent_user_memory(
            state,
            task,
            &mut out,
            answer_candidate_binding.as_ref(),
        )
        .is_some()
        {
            contract_repair_report.add(
                "structural_cleanup",
                "memory_only_answer_candidate_rebound_to_recent_user_memory",
            );
        } else if clear_memory_only_answer_candidate_if_recent_context_conflicts(
            &mut out,
            answer_candidate_binding.as_ref(),
            route_view,
        )
        .is_some()
        {
            contract_repair_report.add(
                "structural_cleanup",
                "memory_only_answer_candidate_recent_scalar_conflict_cleared",
            );
        } else if let Some(binding) = answer_candidate_binding
            .as_ref()
            .filter(|binding| binding.is_memory_only_binding() && binding.is_distinctive())
        {
            contract_repair_context = answer_candidate_binding_repair_context(
                binding,
                out.should_refresh_long_term_memory,
            );
            contract_repair_report.add("semantic_suspect", "answer_candidate_memory_only_binding");
        }
        if let Some(active_conflict_context) = active_text_answer_candidate_conflict_context(
            answer_candidate_binding.as_ref(),
            session_snapshot,
            &req_surface,
            out.should_refresh_long_term_memory,
        ) {
            active_text_answer_candidate_conflict = true;
            append_contract_repair_context(&mut contract_repair_context, active_conflict_context);
            contract_repair_report.add("semantic_suspect", "active_task_answer_candidate_conflict");
        }
        if let Some(invalid_binding_context) = active_task_invalid_turn_binding_context(
            &llm_out,
            session_snapshot,
            &req_surface,
            out.should_refresh_long_term_memory,
        ) {
            append_contract_repair_context(&mut contract_repair_context, invalid_binding_context);
            contract_repair_report.add("semantic_suspect", "active_task_invalid_turn_binding");
        }
        if let Some(detail) =
            semantic_suspect_detail_for_normalizer_output(&out, Some(&req_surface))
        {
            contract_repair_report.add("semantic_suspect", detail);
        }
        let mut active_text_answer_candidate_repair_applied = false;
        if contract_repair_report.needs_llm_semantic_repair() {
            if let Some(repair) = run_contract_repair_judge(
                state,
                task,
                req,
                &llm_out,
                &llm_out_for_parse,
                &contract_repair_report,
                &contract_repair_context,
            )
            .await
            {
                if apply_contract_repair_judge_output(&mut out, repair) {
                    if active_text_answer_candidate_conflict {
                        active_text_answer_candidate_repair_applied = true;
                    }
                    let mut repair_applied = ContractRepairReport::default();
                    repair_applied.add("llm_semantic", "contract_repair_judge_applied");
                    contract_repair_report.merge(&repair_applied);
                }
            }
        }
        let resolved = out.resolved_user_intent.trim();
        let mut resume_behavior = parse_resume_behavior(&out.resume_behavior);
        if resume_context.is_none() && resume_behavior != ResumeBehavior::None {
            warn!(
                "intent_normalizer override resume_behavior to none: task_id={} raw_resume_behavior={}",
                task.task_id, out.resume_behavior
            );
            resume_behavior = ResumeBehavior::None;
        }
        let mut schedule_kind = parse_schedule_kind(&out.schedule_kind);
        let confidence = out.confidence.clamp(0.0, 1.0);
        let parsed_decision = parse_first_layer_decision_text(&out.decision);
        let parsed_turn_type = parse_turn_type(&out.turn_type);
        let parsed_target_task_policy = parse_target_task_policy(&out.target_task_policy);
        let command_payload_declared =
            contract_repair_report.has_detail("execution_recipe_command_payload");
        let mut wants_file_delivery = out.wants_file_delivery;
        let mut output_contract =
            parse_output_contract(out.output_contract.clone(), wants_file_delivery);
        let mut clarify_question = out.clarify_question.trim().to_string();
        let mut execution_recipe_hint = parse_execution_recipe_hint(out.execution_recipe.clone());
        let mut needs_clarify = out.needs_clarify;
        let mut first_layer_decision = parsed_decision.unwrap_or_else(|| {
            if needs_clarify {
                FirstLayerDecision::Clarify
            } else if route_has_structured_execution_signal(
                &output_contract,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
            ) {
                FirstLayerDecision::PlannerExecute
            } else {
                FirstLayerDecision::DirectAnswer
            }
        });
        let mut execution_finalize_style = execution_finalize_style_for_contract(&output_contract);
        let structured_contract_hint_repair = apply_structured_contract_hint_repair(
            &mut output_contract,
            req,
            &req_surface,
            &state.skill_rt.workspace_root,
            &mut wants_file_delivery,
            &mut needs_clarify,
            &mut clarify_question,
            &mut first_layer_decision,
            &mut execution_finalize_style,
        );
        if let Some(fallback) = parsed_inline_json_transform_repair_decision(
            req,
            needs_clarify,
            first_layer_decision,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        ) {
            info!(
                "{} intent_normalizer task_id={} parsed_inline_json_transform_repair reason={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                fallback.reason,
                crate::truncate_for_log(req)
            );
            return normalizer_output_from_fallback(
                req,
                "parsed_inline_json_transform_repair",
                fallback,
                None,
            );
        }
        if let Some(fallback) = explicit_surface_path_metadata_clarify_repair_decision(
            req,
            &req_surface,
            &state.skill_rt.workspace_root,
            needs_clarify,
            first_layer_decision,
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        ) {
            info!(
                "{} intent_normalizer task_id={} clarify_explicit_surface_metadata_fallback reason={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                fallback.reason,
                crate::truncate_for_log(req)
            );
            return normalizer_output_from_fallback(
                req,
                "clarify_structured_surface_metadata_fallback",
                fallback,
                None,
            );
        }
        if let Some(fallback) = explicit_surface_path_facts_clarify_repair_decision(
            req,
            &req_surface,
            &state.skill_rt.workspace_root,
            needs_clarify,
            first_layer_decision,
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        ) {
            info!(
                "{} intent_normalizer task_id={} clarify_explicit_surface_fallback reason={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                fallback.reason,
                crate::truncate_for_log(req)
            );
            return normalizer_output_from_fallback(
                req,
                "clarify_structured_surface_fallback",
                fallback,
                None,
            );
        }
        let mut synced_route_label =
            route_label_from_first_layer_decision(first_layer_decision, execution_finalize_style);
        let structural_contract_repair = apply_current_turn_structural_contract_repair(
            &mut output_contract,
            &surface_req,
            &req_surface,
            &state.skill_rt.workspace_root,
            first_layer_decision,
            &out.answer_candidate,
            parsed_turn_type,
            parsed_target_task_policy,
        );
        let self_contained_payload_repair =
            apply_self_contained_payload_direct_answer_contract_repair(
                &mut output_contract,
                req,
                &req_surface,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
                needs_clarify,
                &out.answer_candidate,
                &mut first_layer_decision,
                &mut execution_finalize_style,
            );
        let inline_structured_transform_direct_answer_repair =
            apply_inline_structured_transform_direct_answer_repair(
                &mut output_contract,
                &req_surface,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
                needs_clarify,
                &out.answer_candidate,
                &mut first_layer_decision,
                &mut execution_finalize_style,
            );
        if matches!(first_layer_decision, FirstLayerDecision::PlannerExecute) {
            execution_finalize_style = execution_finalize_style_for_contract(&output_contract);
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
        }
        let command_payload_contract_repair = apply_command_payload_contract_repair(
            command_payload_declared,
            &mut output_contract,
            &mut needs_clarify,
            &mut clarify_question,
            &mut first_layer_decision,
            &mut execution_finalize_style,
        );
        if command_payload_contract_repair.is_some() {
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
        }
        let raw_output_explicit_locator_repair = apply_raw_output_explicit_locator_repair(
            &mut output_contract,
            &surface_req,
            &state.policy.command_intent,
        );
        if raw_output_explicit_locator_repair.is_some() {
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
        }
        let state_patch_for_decision = out
            .state_patch
            .as_ref()
            .filter(|value| is_meaningful_state_patch(value));
        let active_ordered_scalar_path_chat_repair = apply_active_ordered_scalar_path_chat_repair(
            session_snapshot,
            state_patch_for_decision,
            &out.answer_candidate,
            needs_clarify,
            &mut first_layer_decision,
            &mut execution_finalize_style,
            &mut output_contract,
        );
        if active_ordered_scalar_path_chat_repair.is_some() {
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
        }
        let active_observed_output_chat_repair = apply_active_observed_output_chat_repair(
            req,
            session_snapshot,
            parsed_turn_type,
            parsed_target_task_policy,
            out.attachment_processing_required,
            out.should_refresh_long_term_memory,
            schedule_kind,
            execution_recipe_hint,
            wants_file_delivery,
            &out.answer_candidate,
            needs_clarify,
            &mut first_layer_decision,
            &mut execution_finalize_style,
            &mut output_contract,
        );
        if active_observed_output_chat_repair.is_some() {
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
        }
        let decision_contract_conflict_repair = if !needs_clarify
            && matches!(first_layer_decision, FirstLayerDecision::DirectAnswer)
            && route_has_structured_execution_signal(
                &output_contract,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
            ) {
            first_layer_decision = FirstLayerDecision::PlannerExecute;
            execution_finalize_style = execution_finalize_style_for_contract(&output_contract);
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
            Some("direct_answer_decision_overridden_by_executable_contract")
        } else {
            None
        };
        let explicit_command_execution_repair = apply_explicit_command_execution_contract_repair(
            &state.policy.command_intent,
            req,
            &mut needs_clarify,
            &mut clarify_question,
            &mut output_contract,
            &mut first_layer_decision,
            &mut execution_finalize_style,
        );
        if explicit_command_execution_repair.is_some() {
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
        }
        let current_turn_anchor_repair_allowed = current_turn_anchor_drift_repair_allowed(
            first_layer_decision,
            needs_clarify,
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
            &state.skill_rt.workspace_root,
        );
        let current_turn_anchor_path = current_turn_anchor_repair_allowed
            .then(|| resolve_current_turn_anchor_path(state, req))
            .flatten();
        let current_turn_anchor_drift_repair =
            current_turn_anchor_path.as_deref().and_then(|anchor_path| {
                apply_current_turn_anchor_drift_repair(
                    &mut output_contract,
                    resolved,
                    anchor_path,
                    &state.skill_rt.workspace_root,
                )
            });
        if current_turn_anchor_drift_repair.is_some() {
            schedule_kind = ScheduleKind::None;
            wants_file_delivery = output_contract.delivery_required
                || matches!(
                    output_contract.response_shape,
                    OutputResponseShape::FileToken
                )
                || matches!(
                    output_contract.delivery_intent,
                    OutputDeliveryIntent::FileSingle
                );
            execution_recipe_hint = None;
        }
        if let Some(finalize_style) =
            crate::post_route_policy::content_evidence_execution_finalize_style(
                &output_contract,
                needs_clarify,
            )
        {
            first_layer_decision = FirstLayerDecision::PlannerExecute;
            execution_finalize_style = finalize_style;
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
        }
        let mut state_patch = out.state_patch.clone().filter(is_meaningful_state_patch);
        let answer_candidate_path_repair = apply_answer_candidate_path_evidence_repair(
            &mut output_contract,
            &out.answer_candidate,
            state_patch.as_ref(),
            &state.skill_rt.workspace_root,
            needs_clarify,
            &mut first_layer_decision,
            &mut execution_finalize_style,
        );
        let archive_unpack_missing_archive_locator_clarify_repair =
            apply_archive_unpack_missing_archive_locator_clarify(
                &mut output_contract,
                &req_surface,
                session_snapshot,
                &mut needs_clarify,
                &mut clarify_question,
                &mut first_layer_decision,
                &mut execution_finalize_style,
            );
        let structured_clarify_repair =
            if archive_unpack_missing_archive_locator_clarify_repair.is_none() {
                apply_spurious_structured_observation_clarify_repair(
                    &mut output_contract,
                    req,
                    &req_surface,
                    &state.skill_rt.workspace_root,
                    state_patch.as_ref(),
                    &mut needs_clarify,
                    &mut clarify_question,
                    &mut first_layer_decision,
                    &mut execution_finalize_style,
                )
            } else {
                None
            };
        let workspace_default_clarify_repair =
            if archive_unpack_missing_archive_locator_clarify_repair.is_none()
                && structured_clarify_repair.is_none()
            {
                apply_workspace_default_observation_clarify_repair(
                    &mut output_contract,
                    &state.skill_rt.workspace_root,
                    state_patch.as_ref(),
                    &mut needs_clarify,
                    &mut clarify_question,
                    &mut first_layer_decision,
                    &mut execution_finalize_style,
                )
            } else {
                None
            };
        let resolved_directory_clarify_repair =
            if archive_unpack_missing_archive_locator_clarify_repair.is_none()
                && structured_clarify_repair.is_none()
                && workspace_default_clarify_repair.is_none()
            {
                apply_resolved_directory_observation_clarify_repair(
                    state,
                    &mut output_contract,
                    req,
                    &req_surface,
                    state_patch.as_ref(),
                    &mut needs_clarify,
                    &mut clarify_question,
                    &mut first_layer_decision,
                    &mut execution_finalize_style,
                )
            } else {
                None
            };
        let unbound_workspace_generic_content_clarify_repair = if structured_clarify_repair
            .is_none()
            && archive_unpack_missing_archive_locator_clarify_repair.is_none()
            && workspace_default_clarify_repair.is_none()
            && resolved_directory_clarify_repair.is_none()
        {
            apply_unbound_workspace_generic_content_clarify_repair(
                &mut output_contract,
                req,
                &req_surface,
                &mut needs_clarify,
                &mut clarify_question,
                &mut first_layer_decision,
                &mut execution_finalize_style,
            )
        } else {
            None
        };
        let executionless_route_repair = downgrade_executionless_route_to_direct_answer(
            &mut first_layer_decision,
            &mut execution_finalize_style,
            needs_clarify,
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        );
        let schedule_intent = normalize_schedule_intent_from_normalizer(
            schedule_kind,
            out.schedule_intent.clone(),
            if resolved.is_empty() { req } else { resolved },
            &out.reason,
            needs_clarify,
            &clarify_question,
            confidence,
        );
        let mut target_task_policy = infer_missing_target_policy_from_contract(
            parsed_target_task_policy,
            parsed_turn_type,
            first_layer_decision,
            needs_clarify,
            schedule_kind,
            out.should_refresh_long_term_memory,
            &output_contract,
        );
        let mut turn_type = infer_missing_turn_type_from_policy(
            parsed_turn_type,
            target_task_policy,
            first_layer_decision,
            needs_clarify,
            schedule_kind,
            out.should_refresh_long_term_memory,
        );
        let mut reason = out.reason;
        let mut force_current_request_resolved_intent = current_turn_anchor_drift_repair.is_some();
        if current_turn_anchor_drift_repair.is_some() {
            turn_type = Some(TurnType::TaskRequest);
            target_task_policy = Some(TargetTaskPolicy::Standalone);
        }
        let unobserved_runtime_status_answer_candidate_repair =
            apply_unobserved_runtime_status_answer_candidate_repair(
                &mut output_contract,
                &mut out.answer_candidate,
                &mut state_patch,
                needs_clarify,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
                &mut first_layer_decision,
                &mut execution_finalize_style,
                &mut turn_type,
                &mut target_task_policy,
            );
        if unobserved_runtime_status_answer_candidate_repair.is_some() {
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
        }
        if should_detach_bare_acknowledgement_from_active_task(
            turn_type,
            target_task_policy,
            first_layer_decision,
            &output_contract,
            state_patch.as_ref(),
            out.should_refresh_long_term_memory,
        ) {
            turn_type = None;
            target_task_policy = None;
            force_current_request_resolved_intent = true;
            if reason.trim().is_empty() {
                reason = "bare_acknowledgement_standalone_chat".to_string();
            } else if !reason.contains("bare_acknowledgement_standalone_chat") {
                reason.push_str("; bare_acknowledgement_standalone_chat");
            }
            info!(
                "{} intent_normalizer task_id={} bare_acknowledgement_standalone_chat input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = structural_contract_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = self_contained_payload_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = inline_structured_transform_direct_answer_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = active_ordered_scalar_path_chat_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = active_observed_output_chat_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = structured_contract_hint_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = current_turn_anchor_drift_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
            if let Some(anchor_path) = current_turn_anchor_path.as_deref() {
                info!(
                    "{} intent_normalizer task_id={} current_turn_anchor_overrides_contextual_target anchor={} input={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(anchor_path),
                    crate::truncate_for_log(req)
                );
            }
        }
        if let Some(repair_reason) = answer_candidate_path_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
            info!(
                "{} intent_normalizer task_id={} answer_candidate_path_requires_evidence input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = archive_unpack_missing_archive_locator_clarify_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = structured_clarify_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = workspace_default_clarify_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = resolved_directory_clarify_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = unbound_workspace_generic_content_clarify_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = executionless_route_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = unobserved_runtime_status_answer_candidate_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if contract_repair_report.has_detail("execution_recipe_scalar_runtime_tool_observation") {
            append_route_reason(
                &mut reason,
                "execution_recipe_scalar_runtime_tool_observation",
            );
        }
        if contract_repair_report.has_detail("execution_recipe_service_status_observation") {
            append_route_reason(&mut reason, "execution_recipe_service_status_observation");
        }
        if contract_repair_report.has_detail("execution_recipe_health_check_observation") {
            append_route_reason(&mut reason, "execution_recipe_health_check_observation");
        }
        if let Some(repair_reason) = command_payload_contract_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = raw_output_explicit_locator_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = explicit_command_execution_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(repair_reason) = decision_contract_conflict_repair {
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
        }
        if let Some(scope_hint) = apply_workspace_scope_patch_to_contract(
            &mut output_contract,
            turn_type,
            target_task_policy,
            state_patch.as_ref(),
        ) {
            if reason.trim().is_empty() {
                reason = "workspace_scope_patch_locator_hint".to_string();
            } else if !reason.contains("workspace_scope_patch_locator_hint") {
                reason.push_str("; workspace_scope_patch_locator_hint");
            }
            info!(
                "{} intent_normalizer task_id={} workspace_scope_patch_locator_hint={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(&scope_hint)
            );
        }
        if should_downgrade_orphan_output_shape_clarify_to_direct_answer(
            session_snapshot,
            turn_type,
            target_task_policy,
            first_layer_decision,
            &output_contract,
            state_patch.as_ref(),
            out.should_refresh_long_term_memory,
            out.attachment_processing_required,
        ) {
            needs_clarify = false;
            clarify_question.clear();
            first_layer_decision = FirstLayerDecision::DirectAnswer;
            execution_finalize_style = ActFinalizeStyle::Plain;
            turn_type = None;
            target_task_policy = None;
            force_current_request_resolved_intent = true;
            if reason.trim().is_empty() {
                reason = "orphan_output_shape_ack_chat".to_string();
            } else if !reason.contains("orphan_output_shape_ack_chat") {
                reason.push_str("; orphan_output_shape_ack_chat");
            }
            info!(
                "{} intent_normalizer task_id={} orphan_output_shape_ack_chat input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = apply_active_text_followup_route_repair(
            req,
            session_snapshot,
            &mut turn_type,
            &mut target_task_policy,
            out.attachment_processing_required,
            &mut first_layer_decision,
            &mut execution_finalize_style,
            &mut needs_clarify,
            schedule_kind,
            out.should_refresh_long_term_memory,
            &mut wants_file_delivery,
            &mut output_contract,
            state_patch.as_ref(),
            resolved_existing_directory_from_current_request(state, req).is_some()
                || resolved_directory_pair_from_current_request(state, req).is_some(),
            active_text_answer_candidate_repair_applied,
            &mut out.answer_candidate,
        ) {
            clarify_question.clear();
            force_current_request_resolved_intent = true;
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
            info!(
                "{} intent_normalizer task_id={} active_text_followup_route_repair input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        let resolved_user_intent = if force_current_request_resolved_intent || resolved.is_empty() {
            req.to_string()
        } else {
            resolved.to_string()
        };
        let answer_candidate_for_resolved = if output_contract.requires_content_evidence {
            ""
        } else {
            out.answer_candidate.as_str()
        };
        let mut resolved_user_intent = merge_answer_candidate_into_resolved_intent(
            resolved_user_intent,
            answer_candidate_for_resolved,
        );
        if let Some(current_turn_intent) = sanitize_resolved_intent_for_current_turn_locator(
            &resolved_user_intent,
            req,
            &req_surface,
        ) {
            resolved_user_intent = current_turn_intent;
            if reason.trim().is_empty() {
                reason = "current_turn_locator_overrides_contextual_path".to_string();
            } else if !reason.contains("current_turn_locator_overrides_contextual_path") {
                reason.push_str("; current_turn_locator_overrides_contextual_path");
            }
            info!(
                "{} intent_normalizer task_id={} current_turn_locator_overrides_contextual_path input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = apply_active_task_structured_patch_repair(
            req,
            session_snapshot,
            &mut turn_type,
            &mut target_task_policy,
            out.attachment_processing_required,
            &mut first_layer_decision,
            &mut execution_finalize_style,
            &mut needs_clarify,
            schedule_kind,
            out.should_refresh_long_term_memory,
            &mut output_contract,
            state_patch.as_ref(),
        ) {
            clarify_question.clear();
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
            info!(
                "{} intent_normalizer task_id={} active_task_structured_patch_repair input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if let Some(repair_reason) = apply_active_task_scope_refinement_repair(
            req,
            session_snapshot,
            &mut turn_type,
            &mut target_task_policy,
            out.attachment_processing_required,
            &mut first_layer_decision,
            &mut execution_finalize_style,
            &mut needs_clarify,
            schedule_kind,
            out.should_refresh_long_term_memory,
            &mut output_contract,
            state_patch.as_ref(),
        ) {
            clarify_question.clear();
            if reason.trim().is_empty() {
                reason = repair_reason.to_string();
            } else if !reason.contains(repair_reason) {
                reason.push_str("; ");
                reason.push_str(repair_reason);
            }
            info!(
                "{} intent_normalizer task_id={} active_task_scope_refinement_repair input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if should_resolve_task_scope_update_clarify_with_active_task(
            req,
            session_snapshot,
            turn_type,
            target_task_policy,
            out.attachment_processing_required,
            first_layer_decision,
            &output_contract,
            state_patch.as_ref(),
        ) {
            needs_clarify = false;
            clarify_question.clear();
            first_layer_decision = FirstLayerDecision::DirectAnswer;
            execution_finalize_style = ActFinalizeStyle::Plain;
            if reason.trim().is_empty() {
                reason = "active_task_scope_update_resolves_clarify".to_string();
            } else if !reason.contains("active_task_scope_update_resolves_clarify") {
                reason.push_str("; active_task_scope_update_resolves_clarify");
            }
            info!(
                "{} intent_normalizer task_id={} turn_analysis_override=active_task_scope_update_resolves_clarify input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if should_route_active_task_mutation_to_direct_answer(
            req,
            session_snapshot,
            turn_type,
            target_task_policy,
            out.attachment_processing_required,
            first_layer_decision,
            &output_contract,
            state_patch.as_ref(),
        ) {
            first_layer_decision = FirstLayerDecision::DirectAnswer;
            execution_finalize_style = ActFinalizeStyle::Plain;
            if reason.trim().is_empty() {
                reason = "active_task_mutation_to_direct_answer".to_string();
            } else if !reason.contains("active_task_mutation_to_direct_answer") {
                reason.push_str("; active_task_mutation_to_direct_answer");
            }
            info!(
                "{} intent_normalizer task_id={} turn_analysis_override=active_task_mutation_to_direct_answer input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if should_resolve_task_replace_clarify_with_active_task(
            req,
            session_snapshot,
            turn_type,
            target_task_policy,
            out.attachment_processing_required,
            first_layer_decision,
            &output_contract,
            state_patch.as_ref(),
        ) {
            needs_clarify = false;
            clarify_question.clear();
            first_layer_decision = FirstLayerDecision::DirectAnswer;
            execution_finalize_style = ActFinalizeStyle::Plain;
            if reason.trim().is_empty() {
                reason = "active_task_replace_resolves_clarify".to_string();
            } else if !reason.contains("active_task_replace_resolves_clarify") {
                reason.push_str("; active_task_replace_resolves_clarify");
            }
            info!(
                "{} intent_normalizer task_id={} turn_analysis_override=active_task_replace_resolves_clarify input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        if should_resolve_task_append_clarify_with_active_task(
            req,
            session_snapshot,
            turn_type,
            target_task_policy,
            out.attachment_processing_required,
            first_layer_decision,
            &output_contract,
            state_patch.as_ref(),
        ) {
            needs_clarify = false;
            clarify_question.clear();
            first_layer_decision = FirstLayerDecision::DirectAnswer;
            execution_finalize_style = ActFinalizeStyle::Plain;
            if reason.trim().is_empty() {
                reason = "active_task_append_resolves_clarify".to_string();
            } else if !reason.contains("active_task_append_resolves_clarify") {
                reason.push_str("; active_task_append_resolves_clarify");
            }
            info!(
                "{} intent_normalizer task_id={} turn_analysis_override=active_task_append_resolves_clarify input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req)
            );
        }
        let turn_analysis = if turn_type.is_some()
            || target_task_policy.is_some()
            || out.should_interrupt_active_run
            || state_patch.is_some()
            || out.attachment_processing_required
        {
            Some(TurnAnalysis {
                turn_type,
                target_task_policy,
                should_interrupt_active_run: out.should_interrupt_active_run,
                state_patch,
                attachment_processing_required: out.attachment_processing_required,
            })
        } else {
            None
        };
        let turn_analysis_log = turn_analysis
            .as_ref()
            .map(|analysis| {
                format!(
                    "type={:?},policy={:?},interrupt={},state_patch={},attachments={}",
                    analysis.turn_type,
                    analysis.target_task_policy,
                    analysis.should_interrupt_active_run,
                    analysis.state_patch.is_some(),
                    analysis.attachment_processing_required
                )
            })
            .unwrap_or_else(|| "none".to_string());
        let derived_route_label =
            route_label_from_first_layer_decision(first_layer_decision, execution_finalize_style);
        if derived_route_label != synced_route_label {
            info!(
                "{} intent_normalizer task_id={} derived_route_label_override={} -> {} reason=content_evidence_requires_execution locator_kind={:?} shape={:?}",
                crate::highlight_tag("routing"),
                task.task_id,
                synced_route_label,
                derived_route_label,
                output_contract.locator_kind,
                output_contract.response_shape
            );
        }
        info!(
            "{} intent_normalizer task_id={} input={} resolved_user_intent={} resume_behavior={:?} schedule_kind={:?} decision={:?} derived_route_label={} wants_file_delivery={} needs_clarify={} reason={} confidence={} output_contract.shape={:?} output_contract.delivery_required={} output_contract.requires_content_evidence={} output_contract.locator_kind={:?} execution_recipe_hint={} contract_repair_source={} contract_repair_detail={} turn_analysis={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req),
            crate::truncate_for_log(&resolved_user_intent),
            resume_behavior,
            schedule_kind,
            first_layer_decision,
            derived_route_label,
            wants_file_delivery,
            needs_clarify,
            crate::truncate_for_log(&reason),
            confidence,
            output_contract.response_shape,
            output_contract.delivery_required,
            output_contract.requires_content_evidence,
            output_contract.locator_kind,
            execution_recipe_hint
                .map(|spec| format!(
                    "{}:{}:{}",
                    spec.kind.as_str(),
                    spec.profile.as_str(),
                    spec.target_scope.as_str()
                ))
                .unwrap_or_else(|| "none".to_string()),
            contract_repair_report.source_csv(),
            contract_repair_report.detail_csv(),
            turn_analysis_log,
        );
        // Structural safety guard only: a single path/file token has no action verb, so
        // ask for the missing operation instead of letting the planner repair a non-actionable
        // instruction. This does not classify natural-language intent.
        let bare_path_only = is_bare_path_only_input_for_clarify(req, &req_surface);
        let bare_path_fills_active_observable_task = bare_path_only
            && bare_path_only_input_can_fill_active_observable_task(
                session_snapshot,
                turn_type,
                target_task_policy,
                first_layer_decision,
                &output_contract,
            );
        let (needs_clarify_eff, clarify_question_eff) = if bare_path_only
            && bare_path_fills_active_observable_task
        {
            if reason.trim().is_empty() {
                reason = "bare_path_fills_active_observable_task".to_string();
            } else if !reason.contains("bare_path_fills_active_observable_task") {
                reason.push_str("; bare_path_fills_active_observable_task");
            }
            info!(
                    "{} intent_normalizer task_id={} bare_path_active_observable_fill needs_clarify=false path_token={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(req.trim())
                );
            (false, String::new())
        } else if !needs_clarify && bare_path_only {
            if reason.trim().is_empty() {
                reason = "bare_path_no_verb".to_string();
            } else if !reason.contains("bare_path_no_verb") {
                reason.push_str("; bare_path_no_verb");
            }
            info!(
                "{} intent_normalizer task_id={} bare_path_no_verb_override needs_clarify=true path_token={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req.trim())
            );
            (true, String::new())
        } else {
            (needs_clarify, clarify_question)
        };
        let bare_path_promotes_to_execute = !needs_clarify_eff
            && bare_path_only
            && bare_path_fills_active_observable_task
            && matches!(first_layer_decision, FirstLayerDecision::Clarify);
        let structured_execution_signal = route_has_structured_execution_signal(
            &output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        );
        let first_layer_decision_eff = if needs_clarify_eff {
            FirstLayerDecision::Clarify
        } else if bare_path_promotes_to_execute
            || structured_execution_signal
            || matches!(first_layer_decision, FirstLayerDecision::PlannerExecute)
        {
            FirstLayerDecision::PlannerExecute
        } else {
            FirstLayerDecision::DirectAnswer
        };
        let execution_finalize_style_eff =
            if matches!(first_layer_decision_eff, FirstLayerDecision::PlannerExecute) {
                if bare_path_promotes_to_execute {
                    crate::post_route_policy::content_evidence_execution_finalize_style(
                        &output_contract,
                        false,
                    )
                    .unwrap_or_else(|| execution_finalize_style_for_contract(&output_contract))
                } else if structured_execution_signal {
                    execution_finalize_style_for_contract(&output_contract)
                } else {
                    execution_finalize_style
                }
            } else {
                ActFinalizeStyle::Plain
            };
        return IntentNormalizerOutput {
            resolved_user_intent,
            resume_behavior,
            schedule_kind,
            schedule_intent,
            wants_file_delivery,
            should_refresh_long_term_memory: out.should_refresh_long_term_memory,
            agent_display_name_hint: out.agent_display_name_hint.trim().to_string(),
            needs_clarify: needs_clarify_eff,
            clarify_question: clarify_question_eff,
            reason,
            confidence,
            output_contract,
            execution_recipe_hint,
            first_layer_decision: first_layer_decision_eff,
            execution_finalize_style: execution_finalize_style_eff,
            turn_analysis,
            fallback_source: None,
        };
    }
    warn!(
        "intent_normalizer parse failed, falling back to safe clarify: task_id={} raw={}",
        task.task_id,
        crate::truncate_for_log(&llm_out)
    );
    if let Some(fallback) = inline_json_transform_fallback_decision(req) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_inline_json_transform_fallback input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_inline_json_transform_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = directory_pair_fallback_decision(state, &surface_req) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_directory_pair_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_directory_pair_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = contract_hint_fallback_decision(
        req,
        &req_surface,
        &state.skill_rt.workspace_root,
        "normalizer_parse_failed_contract_hint",
    ) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_contract_hint_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_contract_hint_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = parse_failed_explicit_capability_fallback_decision(
        &surface_req,
        &req_surface,
        &state.skill_rt.workspace_root,
    ) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_explicit_capability_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_structured_capability_fallback",
            fallback,
            None,
        );
    }
    if let Some(fallback) = explicit_surface_path_facts_fallback_decision(
        &surface_req,
        &req_surface,
        &state.skill_rt.workspace_root,
    ) {
        info!(
            "{} intent_normalizer task_id={} parse_failed_explicit_surface_fallback reason={} input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            fallback.reason,
            crate::truncate_for_log(req)
        );
        return normalizer_output_from_fallback(
            req,
            "parse_failed_structured_surface_fallback",
            fallback,
            None,
        );
    }
    // Planner-first: do not synthesize Act/ChatAct locally on parser failure unless
    // the request has an explicit structured capability token handled above.
    let _ = (resume_context, binding_context);
    let fallback = empty_clarify_decision(req, "normalizer_parse_failed");
    normalizer_output_from_fallback(req, "parse_failed_safe_clarify", fallback, None)
}

fn inline_json_transform_fallback_decision(req: &str) -> Option<RouteDecision> {
    if !inline_structural_transform_candidate(req) {
        return None;
    }

    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_unavailable_inline_json_transform".to_string(),
        confidence: Some(0.82),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            ..Default::default()
        },
    })
}

fn inline_structural_transform_candidate(req: &str) -> bool {
    crate::intent::surface_signals::inline_json_transform_request(req)
        || inline_object_rename_transform_candidate(req)
}

fn inline_object_rename_transform_candidate(req: &str) -> bool {
    let Some(raw) = crate::extract_first_json_value_any(req) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };
    if obj.is_empty()
        || obj.contains_key("action")
        || obj.contains_key("skill")
        || obj.contains_key("operation")
    {
        return false;
    }
    let input_keys = obj.keys().map(String::as_str).collect::<Vec<_>>();
    let instruction = req
        .rfind(&raw)
        .map(|start| {
            let end = start.saturating_add(raw.len());
            format!("{} {}", &req[..start], &req[end..])
        })
        .unwrap_or_else(|| req.to_string());
    let tokens = inline_transform_schema_tokens(&instruction);
    let mut source_positions = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| input_keys.iter().any(|key| key == &token.as_str()))
        .collect::<Vec<_>>();
    source_positions.dedup_by(|(_, left), (_, right)| left == right);
    if source_positions.len() != 1 {
        return false;
    }
    let (source_index, source_token) = source_positions[0];
    let target_candidates = tokens
        .iter()
        .skip(source_index + 1)
        .filter(|token| !input_keys.iter().any(|key| key == &token.as_str()))
        .filter(|token| inline_transform_schema_shaped_target_token(token, source_token))
        .fold(Vec::<&String>::new(), |mut acc, token| {
            if !acc
                .iter()
                .any(|existing| existing.as_str() == token.as_str())
            {
                acc.push(token);
            }
            acc
        });
    target_candidates.len() == 1
}

fn inline_transform_schema_field_token(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '-' || ch.is_ascii_alphanumeric())
}

fn inline_transform_schema_shaped_target_token(candidate: &str, source: &str) -> bool {
    inline_transform_schema_field_token(candidate)
        && candidate != source
        && !candidate.chars().all(|ch| ch.is_ascii_uppercase())
        && (candidate.contains('_')
            || candidate.contains('-')
            || candidate.chars().any(|ch| ch.is_ascii_digit())
            || source.contains('_')
            || source.contains('-')
            || source.chars().any(|ch| ch.is_ascii_digit()))
}

fn inline_transform_schema_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch == '_' || ch == '-' || ch.is_ascii_alphanumeric() {
            current.push(ch);
            continue;
        }
        if inline_transform_schema_field_token(&current) {
            tokens.push(std::mem::take(&mut current));
        } else {
            current.clear();
        }
    }
    if inline_transform_schema_field_token(&current) {
        tokens.push(current);
    }
    tokens
}

fn parsed_inline_json_transform_repair_decision(
    req: &str,
    needs_clarify: bool,
    first_layer_decision: FirstLayerDecision,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> Option<RouteDecision> {
    if !needs_clarify && !matches!(first_layer_decision, FirstLayerDecision::Clarify) {
        return None;
    }
    if wants_file_delivery || !matches!(schedule_kind, ScheduleKind::None) {
        return None;
    }
    if execution_recipe_hint.is_some_and(|spec| {
        !matches!(
            spec.kind,
            crate::execution_recipe::ExecutionRecipeKind::None
        )
    }) {
        return None;
    }

    let mut decision = inline_json_transform_fallback_decision(req)?;
    decision.reason = "parsed_inline_json_transform_contract_repair".to_string();
    Some(decision)
}

fn directory_pair_fallback_decision(state: &AppState, req: &str) -> Option<RouteDecision> {
    let enabled_skills = state.get_skills_list();
    if !enabled_skills.is_empty() && !enabled_skills.contains("system_basic") {
        return None;
    }
    let (left, right) = resolved_directory_pair_from_current_request(state, req)?;
    if left.eq_ignore_ascii_case(&right) {
        return None;
    }

    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_unavailable_directory_pair".to_string(),
        confidence: Some(0.62),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: format!("{left} | {right}"),
            ..Default::default()
        },
    })
}

fn contract_hint_fallback_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    reason: &'static str,
) -> Option<RouteDecision> {
    let semantic_kind = contract_test_hint_semantic_kind(req)?;
    let surface_req = request_without_contract_test_hint(req);
    let mut wants_file_delivery = false;
    let mut output_contract = IntentOutputContract {
        response_shape: response_shape_for_contract_hint_fallback(semantic_kind),
        requires_content_evidence: output_semantic_kind_requires_fresh_evidence(semantic_kind),
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind,
        locator_hint: String::new(),
        ..Default::default()
    };
    apply_contract_hint_delivery_defaults(&mut output_contract, &mut wants_file_delivery);
    apply_contract_hint_locator_defaults(
        &mut output_contract,
        &surface_req,
        req_surface,
        workspace_root,
    );

    let resolved_user_intent = if surface_req.trim().is_empty() {
        req.trim().to_string()
    } else {
        surface_req.trim().to_string()
    };
    Some(RouteDecision {
        resolved_user_intent,
        needs_clarify: false,
        clarify_question: String::new(),
        reason: reason.to_string(),
        confidence: Some(0.70),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract,
    })
}

fn response_shape_for_contract_hint_fallback(kind: OutputSemanticKind) -> OutputResponseShape {
    match kind {
        OutputSemanticKind::RawCommandOutput
        | OutputSemanticKind::ServiceStatus
        | OutputSemanticKind::DirectoryPurposeSummary
        | OutputSemanticKind::ContentExcerptSummary
        | OutputSemanticKind::ContentPresenceCheck
        | OutputSemanticKind::ExcerptKindJudgment
        | OutputSemanticKind::RecentArtifactsJudgment
        | OutputSemanticKind::WorkspaceProjectSummary
        | OutputSemanticKind::ExecutionFailedStep
        | OutputSemanticKind::ExistenceWithPathSummary
        | OutputSemanticKind::GitRepositoryState
        | OutputSemanticKind::ConfigValidation
        | OutputSemanticKind::ConfigMutation
        | OutputSemanticKind::ConfigRiskAssessment
        | OutputSemanticKind::RssNewsFetch
        | OutputSemanticKind::WebPageSummary
        | OutputSemanticKind::WebSearchSummary
        | OutputSemanticKind::WeatherQuery
        | OutputSemanticKind::MarketQuote
        | OutputSemanticKind::ImageUnderstanding
        | OutputSemanticKind::PublishingPreview
        | OutputSemanticKind::PackageManagerDetection
        | OutputSemanticKind::SqliteDatabaseKindJudgment
        | OutputSemanticKind::DockerContainerLifecycle
        | OutputSemanticKind::ArchiveUnpack => OutputResponseShape::OneSentence,
        OutputSemanticKind::ScalarCount
        | OutputSemanticKind::ScalarPathOnly
        | OutputSemanticKind::RecentScalarEqualityCheck
        | OutputSemanticKind::GitCommitSubject
        | OutputSemanticKind::SqliteSchemaVersion
        | OutputSemanticKind::ArchivePack => OutputResponseShape::Scalar,
        OutputSemanticKind::GeneratedFileDelivery => OutputResponseShape::FileToken,
        OutputSemanticKind::None
        | OutputSemanticKind::HiddenEntriesCheck
        | OutputSemanticKind::FileNames
        | OutputSemanticKind::DirectoryNames
        | OutputSemanticKind::DirectoryEntryGroups
        | OutputSemanticKind::FilePaths
        | OutputSemanticKind::ContentExcerptWithSummary
        | OutputSemanticKind::QuantityComparison
        | OutputSemanticKind::ExistenceWithPath
        | OutputSemanticKind::StructuredKeys
        | OutputSemanticKind::SqliteTableListing
        | OutputSemanticKind::SqliteTableNamesOnly
        | OutputSemanticKind::ArchiveList
        | OutputSemanticKind::ArchiveRead
        | OutputSemanticKind::DockerPs
        | OutputSemanticKind::DockerImages
        | OutputSemanticKind::DockerLogs => OutputResponseShape::Strict,
    }
}

fn apply_contract_hint_delivery_defaults(
    output_contract: &mut IntentOutputContract,
    wants_file_delivery: &mut bool,
) {
    if output_contract.semantic_kind != OutputSemanticKind::GeneratedFileDelivery {
        return;
    }
    output_contract.delivery_required = true;
    output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    output_contract.response_shape = OutputResponseShape::FileToken;
    *wants_file_delivery = true;
}

fn apply_contract_hint_locator_defaults(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) {
    match output_contract.semantic_kind {
        OutputSemanticKind::RawCommandOutput
        | OutputSemanticKind::ServiceStatus
        | OutputSemanticKind::PackageManagerDetection
        | OutputSemanticKind::DockerPs
        | OutputSemanticKind::DockerImages
        | OutputSemanticKind::DockerLogs
        | OutputSemanticKind::DockerContainerLifecycle => {
            output_contract.locator_kind = OutputLocatorKind::None;
            output_contract.locator_hint.clear();
        }
        OutputSemanticKind::GitCommitSubject
        | OutputSemanticKind::GitRepositoryState
        | OutputSemanticKind::HiddenEntriesCheck
        | OutputSemanticKind::RecentScalarEqualityCheck => {
            output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
            output_contract.locator_hint = workspace_root.display().to_string();
        }
        OutputSemanticKind::WorkspaceProjectSummary => {
            apply_path_locator_defaults_for_contract_hint(
                output_contract,
                req,
                req_surface,
                workspace_root,
            );
        }
        _ => apply_path_locator_defaults_for_contract_hint(
            output_contract,
            req,
            req_surface,
            workspace_root,
        ),
    }
}

fn apply_path_locator_defaults_for_contract_hint(
    output_contract: &mut IntentOutputContract,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) {
    if matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::ArchivePack | OutputSemanticKind::ArchiveUnpack
    ) {
        if let Some((semantic_kind, locator_hint)) =
            archive_pair_contract_from_surface(output_contract, req_surface)
        {
            output_contract.semantic_kind = semantic_kind;
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = locator_hint;
            return;
        }
    }
    if output_contract.semantic_kind == OutputSemanticKind::ArchiveRead {
        if let Some(locator_hint) = archive_read_contract_from_surface(output_contract, req_surface)
        {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = locator_hint;
            return;
        }
    }
    if output_contract.semantic_kind == OutputSemanticKind::QuantityComparison {
        if let Some((left, right)) = req_surface.locator_target_pair.as_ref() {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = format!("{} | {}", left.trim(), right.trim());
            return;
        }
        let targets = explicit_surface_path_fact_targets(req_surface);
        if targets.len() >= 2 {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = format!("{} | {}", targets[0].trim(), targets[1].trim());
            return;
        }
    }
    if let Some(locator) =
        crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req)
    {
        output_contract.locator_kind = locator.locator_kind;
        output_contract.locator_hint = locator.locator_hint;
        return;
    }
    let filename_candidates = req_surface.filename_candidates_excluding_field_selectors();
    if filename_candidates.len() == 1 {
        output_contract.locator_kind = OutputLocatorKind::Filename;
        output_contract.locator_hint = filename_candidates[0].clone();
        return;
    }
    if !filename_candidates.is_empty() {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
        return;
    }
    if output_contract.requires_content_evidence {
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        output_contract.locator_hint = workspace_root.display().to_string();
    }
}

fn parse_failed_explicit_capability_fallback_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) -> Option<RouteDecision> {
    if !git_repository_state_surface_token_present(req)
        || req_surface.has_explicit_path_or_url()
        || req_surface.has_single_filename_candidate()
        || req_surface.has_filename_candidates()
        || req_surface.has_structured_target_refinement()
        || req_surface.has_delivery_token_reference()
    {
        return None;
    }

    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_parse_failed_explicit_git_repository_state".to_string(),
        confidence: Some(0.55),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::GitRepositoryState,
            locator_hint: workspace_root.display().to_string(),
            ..Default::default()
        },
    })
}

fn git_repository_state_surface_token_present(req: &str) -> bool {
    ascii_token_present(req, "git")
        || ascii_token_present(req, "remote")
        || ascii_token_present(req, "HEAD")
        || ascii_token_present(req, "branch")
}

fn explicit_surface_path_facts_fallback_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
) -> Option<RouteDecision> {
    let targets = explicit_surface_path_fact_targets(req_surface);
    if req_surface.inline_json_shape.is_some()
        || req_surface.has_delivery_token_reference()
        || req_surface.has_deictic_reference()
        || structured_target_refinement_blocks_explicit_path_facts(req_surface, &targets)
    {
        return None;
    }
    if targets.len() < 2 {
        return None;
    }
    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_unavailable_explicit_multi_path_facts".to_string(),
        confidence: Some(0.50),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: workspace_root.display().to_string(),
            ..Default::default()
        },
    })
}

fn explicit_surface_path_metadata_clarify_repair_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    needs_clarify: bool,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> Option<RouteDecision> {
    if !(needs_clarify || matches!(first_layer_decision, FirstLayerDecision::Clarify)) {
        return None;
    }
    let targets = explicit_surface_path_fact_targets(req_surface);
    if req_surface.inline_json_shape.is_some()
        || req_surface.has_delivery_token_reference()
        || req_surface.has_deictic_reference()
        || structured_target_refinement_blocks_explicit_path_facts(req_surface, &targets)
        || output_contract.semantic_kind != OutputSemanticKind::QuantityComparison
        || wants_file_delivery
        || !matches!(schedule_kind, ScheduleKind::None)
        || output_contract.delivery_required
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || !matches!(output_contract.self_extension.mode, SelfExtensionMode::None)
        || !matches!(
            output_contract.self_extension.trigger,
            SelfExtensionTrigger::None
        )
        || output_contract.self_extension.execute_now
        || execution_recipe_hint.is_some_and(|spec| {
            !matches!(
                spec.kind,
                crate::execution_recipe::ExecutionRecipeKind::None
            )
        })
    {
        return None;
    }
    if targets.len() < 2 {
        return None;
    }
    Some(RouteDecision {
        resolved_user_intent: req.trim().to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer_clarify_explicit_multi_path_metadata".to_string(),
        confidence: Some(0.55),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::QuantityComparison,
            locator_hint: workspace_root.display().to_string(),
            ..Default::default()
        },
    })
}

fn explicit_surface_path_facts_clarify_repair_decision(
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    workspace_root: &Path,
    needs_clarify: bool,
    first_layer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    _execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> Option<RouteDecision> {
    if !(needs_clarify || matches!(first_layer_decision, FirstLayerDecision::Clarify)) {
        return None;
    }
    if route_has_structured_execution_signal(
        output_contract,
        wants_file_delivery,
        schedule_kind,
        None,
    ) {
        return None;
    }
    let mut decision =
        explicit_surface_path_facts_fallback_decision(req, req_surface, workspace_root)?;
    decision.reason = "normalizer_clarify_explicit_multi_path_facts".to_string();
    decision.confidence = Some(0.55);
    Some(decision)
}

fn explicit_surface_path_fact_targets(
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> Vec<String> {
    let pair_targets = req_surface
        .locator_target_pair
        .as_ref()
        .map(|(left, right)| vec![left.clone(), right.clone()])
        .unwrap_or_default();
    let candidates = if pair_targets.len() >= 2 {
        pair_targets
    } else {
        req_surface.filename_candidates.clone()
    };
    let mut out = Vec::new();
    for candidate in candidates {
        let candidate = trim_structural_path_fact_candidate(&candidate);
        if !candidate.is_empty()
            && token_looks_like_supported_path_fact_target(&candidate)
            && !out
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&candidate))
        {
            out.push(candidate);
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn structured_target_refinement_blocks_explicit_path_facts(
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    targets: &[String],
) -> bool {
    let mentions_block = !req_surface.field_selector_mentions.is_empty()
        && !req_surface
            .field_selector_mentions
            .iter()
            .all(|selector| selector_matches_explicit_path_target(selector, targets));
    mentions_block
        || req_surface
            .dotted_field_selector
            .as_deref()
            .is_some_and(|selector| !selector_matches_explicit_path_target(selector, targets))
}

fn selector_matches_explicit_path_target(selector: &str, targets: &[String]) -> bool {
    let selector = selector.trim();
    !selector.is_empty()
        && targets.iter().any(|target| {
            Path::new(target)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .is_some_and(|name| name.eq_ignore_ascii_case(selector))
        })
}

fn trim_structural_path_fact_candidate(candidate: &str) -> String {
    let trimmed = candidate
        .trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | ','
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '《'
                    | '》'
            )
        })
        .to_string();
    if let Some(stripped) = trimmed.strip_suffix('.') {
        if token_looks_like_supported_path_fact_target(stripped) {
            return stripped.to_string();
        }
    }
    trimmed
}

fn token_looks_like_supported_path_fact_target(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty()
        || token.starts_with("http://")
        || token.starts_with("https://")
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
    {
        return false;
    }
    let Some(name) = Path::new(token)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return false;
    };
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && matches!(
            extension.to_ascii_lowercase().as_str(),
            "md" | "txt"
                | "json"
                | "toml"
                | "yaml"
                | "yml"
                | "rs"
                | "log"
                | "sqlite"
                | "db"
                | "csv"
        )
}

fn ascii_token_present(text: &str, token: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .any(|candidate| candidate.eq_ignore_ascii_case(token))
}

fn is_bare_path_only_input_for_clarify(
    text: &str,
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 60 {
        return false;
    }
    if trimmed.contains(['?', '？', '!', '！']) {
        return false;
    }
    if surface.inline_json_shape.is_some() || surface.has_structured_target_refinement() {
        return false;
    }
    if trimmed.split_whitespace().count() != 1 {
        return false;
    }
    surface.has_explicit_path_or_url()
        || surface.has_single_filename_candidate()
        || token_looks_like_pathish_filename(trimmed)
}

fn token_looks_like_pathish_filename(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() || token.starts_with('.') || token.contains('/') || token.contains('\\') {
        return false;
    }
    let Some((stem, extension)) = token.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !extension.is_empty()
        && extension.len() <= 8
        && extension
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

/// Fallback `RouteDecision` used when normalizer LLM fails or its output cannot be parsed.
/// It intentionally stays on AskClarify instead of using local semantic heuristics as
/// a substitute planner.
fn empty_clarify_decision(user_request: &str, reason: &str) -> RouteDecision {
    RouteDecision {
        resolved_user_intent: user_request.trim().to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        reason: reason.to_string(),
        confidence: None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    }
}

pub(crate) async fn generate_clarify_question(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resolver_reason: &str,
    candidate_context: Option<&str>,
) -> String {
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let (prompt_template, prompt_source) =
        match crate::bootstrap::load_required_prompt_template_for_state(
            state,
            CLARIFY_QUESTION_PROMPT_LOGICAL_PATH,
        ) {
            Ok(resolved) => resolved,
            Err(err) => {
                warn!(
                    "generate_clarify_question prompt load failed, fallback default: task_id={} err={}",
                    task.task_id, err
                );
                return crate::fallback::render_clarify_fallback_with_language_hint(
                    state,
                    &task.task_id,
                    crate::fallback::ClarifyFallbackSource::LlmUnavailable,
                    Some(&err.to_string()),
                    &request_language_hint,
                );
            }
        };
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__REQUEST__", user_request.trim()),
            ("__RESOLVER_REASON__", resolver_reason.trim()),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            (
                "__CANDIDATE_CONTEXT__",
                candidate_context
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("<none>"),
            ),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "clarify_question_prompt",
        &prompt_source,
        None,
    );
    match llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
        .await
    {
        Ok(v) => {
            let out = v.trim();
            if out.is_empty() {
                // §7.2: LLM 调用 OK 但内容为空 → EmptyResponse 特化文案 + tracing 上报。
                crate::fallback::render_clarify_fallback_with_language_hint(
                    state,
                    &task.task_id,
                    crate::fallback::ClarifyFallbackSource::EmptyResponse,
                    None,
                    &request_language_hint,
                )
            } else {
                out.to_string()
            }
        }
        Err(err) => {
            warn!(
                "generate_clarify_question llm failed, fallback default: task_id={} err={}",
                task.task_id, err
            );
            // §7.2: LLM 直接 Err（401 / 熔断 / 超时 / 网络）→ LlmUnavailable 特化文案。
            // err 概要写进 context_hint，便于 inspect_task.sh 关联。
            let hint = format!("err={err}");
            crate::fallback::render_clarify_fallback_with_language_hint(
                state,
                &task.task_id,
                crate::fallback::ClarifyFallbackSource::LlmUnavailable,
                Some(&hint),
                &request_language_hint,
            )
        }
    }
}

pub(crate) async fn generate_or_reuse_clarify_question(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resolver_reason: &str,
    candidate_context: Option<&str>,
    preferred_question: Option<&str>,
    policy: ClarifyQuestionPolicy,
    // 上游必须显式声明"我现在为什么走 SafeFallback"。
    // policy=SafeFallback 时优先让 LLM 按结构化上下文生成用户可见澄清；
    // 只有 LLM 本身不可用等硬失败才落到该 source 的最小安全模板。
    // policy=AllowModel 时本参数仅作为诊断上下文。
    default_source: crate::fallback::ClarifyFallbackSource,
) -> String {
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let preferred = preferred_question
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .filter(|question| {
            !crate::language_policy::text_language_conflicts_with_hint(
                question,
                &request_language_hint,
            )
        })
        .map(ToString::to_string);
    if let Some(question) = preferred {
        return question;
    }
    if matches!(policy, ClarifyQuestionPolicy::SafeFallback)
        && !safe_fallback_source_should_try_llm(default_source)
    {
        return crate::fallback::render_clarify_fallback_with_language_hint(
            state,
            &task.task_id,
            default_source,
            None,
            &request_language_hint,
        );
    }
    if matches!(policy, ClarifyQuestionPolicy::SafeFallback) {
        tracing::info!(
            task_id = %task.task_id,
            fallback_source = default_source.as_metric_label(),
            "safe_fallback_try_llm_response_composer"
        );
        let contract = crate::fallback::UserResponseContract::clarify_from_fallback_source(
            default_source,
            user_request,
            resolver_reason,
            candidate_context,
            &request_language_hint,
        );
        let default_text = crate::fallback::clarify_fallback_text_with_language_hint(
            state,
            default_source,
            None,
            &request_language_hint,
        );
        let answer = crate::fallback::compose_user_response_from_contract_with_default(
            state,
            task,
            &contract,
            default_source,
            &default_text,
        )
        .await;
        if crate::language_policy::text_language_conflicts_with_hint(
            &answer,
            &request_language_hint,
        ) {
            tracing::info!(
                task_id = %task.task_id,
                fallback_source = default_source.as_metric_label(),
                language_hint = %request_language_hint,
                "clarify_generated_language_mismatch_fallback"
            );
            return default_text;
        }
        return answer;
    }
    let answer = generate_clarify_question(
        state,
        task,
        user_request,
        resolver_reason,
        candidate_context,
    )
    .await;
    if crate::language_policy::text_language_conflicts_with_hint(&answer, &request_language_hint) {
        return crate::fallback::render_clarify_fallback_with_language_hint(
            state,
            &task.task_id,
            default_source,
            None,
            &request_language_hint,
        );
    }
    answer
}

fn safe_fallback_source_should_try_llm(source: crate::fallback::ClarifyFallbackSource) -> bool {
    !matches!(
        source,
        crate::fallback::ClarifyFallbackSource::LlmUnavailable
    )
}

pub(crate) async fn try_handle_schedule_request(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    precompiled_intent: Option<&crate::ScheduleIntentOutput>,
) -> Result<Option<String>, String> {
    schedule_service::try_handle_schedule_request(state, task, prompt, precompiled_intent).await
}

#[cfg(test)]
#[path = "intent_router_tests.rs"]
mod tests;
