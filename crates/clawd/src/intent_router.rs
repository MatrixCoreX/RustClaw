//! Intent routing and unified normalizer for ask tasks.
//!
//! **Ask main path:** Only `run_intent_normalizer` is used (resolved intent, resume_behavior,
//! schedule_kind, first-layer decision, needs_clarify, and output contract in one LLM call).
//!
//! **Fallback when normalizer LLM fails / parse fails:** do not synthesize semantic execution
//! routes locally. The fallback stays on AskClarify so semantic routing remains owned by the
//! normalizer/planner LLM path instead of hard-match recovery code.

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
        "service_status" => OutputSemanticKind::ServiceStatus,
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
        "structured_keys"
        | "structured_key_names"
        | "structured_top_level_keys"
        | "top_level_keys"
        | "object_keys"
        | "config_keys" => OutputSemanticKind::StructuredKeys,
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
        "archive_list" | "archive_listing" | "archive_contents" => OutputSemanticKind::ArchiveList,
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

    if output_contract.semantic_kind == OutputSemanticKind::ScalarPathOnly
        && req_surface.has_structured_target_refinement()
    {
        output_contract.semantic_kind = OutputSemanticKind::None;
        output_contract.requires_content_evidence = true;
        reason = Some("structured_file_scalar_repair");
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
        && (req_surface.has_explicit_path_or_url() || req_surface.has_filename_candidates())
        && !req_surface.is_structural_locator_only_reply()
    {
        output_contract.requires_content_evidence = true;
        reason = reason.or(Some("planner_locator_requires_evidence"));
    }

    if output_contract.requires_content_evidence
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
    {
        if let Some(locator) =
            crate::intent::locator_extractor::extract_explicit_locator_for_fallback(req)
        {
            output_contract.locator_kind = locator.locator_kind;
            output_contract.locator_hint = locator.locator_hint;
            reason = reason.or(Some("structured_locator_contract_repair"));
        } else if req_surface.has_filename_candidates() {
            output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
            output_contract.locator_hint = workspace_root.display().to_string();
            reason = reason.or(Some("workspace_filename_targets_contract_repair"));
        }
    }

    reason
}

fn apply_self_contained_payload_direct_answer_contract_repair(
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
        || answer_candidate.trim().is_empty()
        || req_surface.inline_json_shape.is_none()
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
    if output_contract.locator_hint.trim().is_empty() {
        if let Some(filename) = req_surface.single_filename_candidate() {
            output_contract.locator_kind = OutputLocatorKind::Filename;
            output_contract.locator_hint = filename.to_string();
        }
    }
    if output_contract.locator_hint.trim().is_empty()
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && req_surface.locator_target_pair.is_some()
    {
        if let Some((left, right)) = req_surface.locator_target_pair.as_ref() {
            output_contract.locator_kind = OutputLocatorKind::Path;
            output_contract.locator_hint = format!("{left}, {right}");
        }
    } else if let Some(locator) = fallback_locator {
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

    output_contract.response_shape = if preserve_file_delivery {
        OutputResponseShape::FileToken
    } else {
        OutputResponseShape::Free
    };
    output_contract.exact_sentence_count = None;
    output_contract.requires_content_evidence = !preserve_file_delivery;
    output_contract.delivery_required = preserve_file_delivery;
    output_contract.locator_kind = OutputLocatorKind::Path;
    output_contract.delivery_intent = if preserve_file_delivery {
        OutputDeliveryIntent::FileSingle
    } else {
        OutputDeliveryIntent::None
    };
    output_contract.semantic_kind = OutputSemanticKind::None;
    output_contract.locator_hint = current_anchor_path.trim().to_string();
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
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
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
        execution_recipe_hint,
    ) {
        return None;
    }
    *first_layer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("executionless_route_downgraded_to_direct_answer")
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
            | OutputSemanticKind::ExcerptKindJudgment
            | OutputSemanticKind::RecentArtifactsJudgment
            | OutputSemanticKind::WorkspaceProjectSummary
            | OutputSemanticKind::ScalarCount
            | OutputSemanticKind::ExecutionFailedStep
            | OutputSemanticKind::GeneratedFileDelivery
            | OutputSemanticKind::ExistenceWithPath
            | OutputSemanticKind::ExistenceWithPathSummary
            | OutputSemanticKind::GitCommitSubject
            | OutputSemanticKind::StructuredKeys
            | OutputSemanticKind::SqliteTableListing
            | OutputSemanticKind::SqliteTableNamesOnly
            | OutputSemanticKind::SqliteDatabaseKindJudgment
            | OutputSemanticKind::SqliteSchemaVersion
            | OutputSemanticKind::ArchiveList
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
    if !surface.has_deictic_reference() || !output_contract.requires_content_evidence {
        return false;
    }
    let current_turn_has_concrete_locator = surface.has_explicit_path_or_url()
        || surface.has_single_filename_candidate()
        || surface.has_concrete_locator_hint()
        || surface.has_structured_target_refinement()
        || surface.has_delivery_token_reference();
    if current_turn_has_concrete_locator {
        return false;
    }
    let contract_has_concrete_locator = !matches!(
        output_contract.locator_kind,
        OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
    ) && !output_contract.locator_hint.trim().is_empty();
    !contract_has_concrete_locator
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

    *turn_type = Some(TurnType::TaskScopeUpdate);
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
    Some("active_task_scope_refinement_repair")
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
    !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            output_contract.locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
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
    semantic_active_text_candidate_repair: bool,
    answer_candidate: &mut String,
) -> Option<&'static str> {
    active_primary_text_context(session_snapshot)?;
    if attachment_processing_required
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
        || *wants_file_delivery
        || !output_contract_looks_like_contextual_text_followup(output_contract)
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
    parts.push("Always include all top-level schema keys: resolved_user_intent, answer_candidate, resume_behavior, schedule_kind, schedule_intent, wants_file_delivery, should_refresh_long_term_memory, agent_display_name_hint, needs_clarify, clarify_question, reason, confidence, decision, output_contract, execution_recipe, turn_type, target_task_policy, should_interrupt_active_run, state_patch, attachment_processing_required.".to_string());
    parts.push("Use decision as the only first-layer semantic gate: clarify, direct_answer, planner_execute.".to_string());
    parts.push("Prefer decision=direct_answer for greetings, confirmations, memory-only requests, and pure discussion. Use decision=clarify only when a required target/action is truly missing.".to_string());
    parts.push("High-priority: if REQUEST asks to summarize, explain, conclude, judge, or state what a current topic/test/conversation mainly verifies or means, do not treat a prior exact ID/value as the answer. Keep answer_candidate empty unless REQUEST explicitly asks for that scalar, and copy any relevant recent background/goals/purpose into resolved_user_intent. In MEMORY lists, leading decimal numbers are retrieval scores, never user facts or answer candidates.".to_string());
    parts.push("If ACTIVE_TASK is <none>, do not use task_append, task_correct, or task_scope_update. Classify a fresh user goal as task_request or leave turn_type empty for pure chat/memory/status turns.".to_string());
    parts.push("A complete current REQUEST with its own deliverable, topic, object, audience, scope, or factual constraints is standalone unless it semantically modifies the active deliverable. Shared chat identity, similar task type, same product name, or nearby memory is not enough to merge independent tasks.".to_string());
    parts.push("If ACTIVE_TASK or LAST shows an active writing/drafting/planning task and REQUEST only adds audience, tone, length, body-only, wording, count, format, scope, or presentation constraints, keep it attached: turn_type=\"task_append\", target_task_policy=\"reuse_active\", decision=\"direct_answer\", execution_recipe.kind=\"none\", requires_content_evidence=false, locator_kind=\"none\". Do not route such presentation-only follow-ups to planner_execute unless the REQUEST explicitly requires fresh local/system/file/web evidence.".to_string());
    parts.push("Do not treat a bare acknowledgement request as active-task output refinement. If REQUEST only asks for a short acknowledgement/confirmation and does not explicitly reference ACTIVE_TASK/LAST/the prior answer/result/rewrite target, use standalone chat with no state_patch; answer the acknowledgement itself, not the active task output.".to_string());
    parts.push("If REQUEST's apparent missing topic is only the generic acknowledgement/short-reply target itself, do not ask what topic to answer. Use standalone chat, needs_clarify=false, empty turn_type/target_task_policy, no evidence/delivery, and put the minimal acknowledgement/short reply in answer_candidate when inferable from REQUEST. This is semantic, not phrase-list based.".to_string());
    parts.push("If the same active writing/drafting task is still missing its topic or core subject, keep the new constraint in resolved_user_intent and ask one concise clarification with decision=\"clarify\"; never force planner_execute for a presentation-only constraint.".to_string());
    parts.push("Do not ask optional preference clarifications for harmless creative/chat requests; answer generically when the deliverable is clear. For a negative constraint plus positive deliverable, preserve the constraint and route the positive deliverable.".to_string());
    parts.push("If CAPABILITIES or a visible skill contract says a missing target/parameter can be handled by safe discovery, default behavior, bounded lookup, or a candidate-returning prepare step, keep the request executable instead of asking a front-door clarification; execution can return observed candidates when it cannot choose uniquely.".to_string());
    parts.push("Example pattern: if a photo-organization capability declares external-drive discovery, route the request to execution without a source_dir so the skill can inspect mounts and either preview the unique candidate or return observed candidates. This is a contract example, not a phrase trigger.".to_string());
    parts.push("Current REQUEST overrides RECENT/MEMORY. Prior assistant refusals, tool failures, exact IDs, scalar values, or capability claims in history are background only unless the current request explicitly asks for them.".to_string());
    parts.push("Do not import a prior directory/path scope from RECENT/MEMORY into the current REQUEST when the current REQUEST names its own file/dir target. Reuse prior scope only for explicit follow-ups like same directory, that file, or previous result.".to_string());
    parts.push("If REQUEST asks for observable local/system/workspace state, filesystem inspection, command output, file content, directory listing, counts, or extracting a value, choose decision=\"planner_execute\". Do not claim the assistant cannot execute; the runtime has tools and the AUTH block describes permission.".to_string());
    parts.push("If REQUEST asks about the assistant/runtime's current unfinished task queue, running tasks, queued tasks, or canceling those tasks, use the existing task_control capability with its default current-user/current-chat scope; do not ask the user to choose a queue type just because no separate system name was supplied.".to_string());
    parts.push("Never ask the user to paste local file contents when REQUEST names a local file/dir/workspace target; route the request for tool execution. Capability refusals are only valid after an actual tool failure, not inside this normalizer.".to_string());
    parts.push("Always include output_contract as a JSON object, never as a string token. It is the final answer contract, not a place to invent a task-specific schema. Put exact scalar recall/direct-answer values in answer_candidate as a string only when the current request itself asks for that exact value; never put answer_candidate as an object or inside output_contract. If unsure, still emit the full default output_contract object with response_shape=\"free\", requires_content_evidence=false, delivery_required=false, locator_kind=\"none\", delivery_intent=\"none\", semantic_kind=\"none\", locator_hint=\"\", and self_extension set to none.".to_string());
    parts.push("Allowed output_contract keys only: response_shape, exact_sentence_count, requires_content_evidence, delivery_required, locator_kind, delivery_intent, semantic_kind, locator_hint, self_extension. Do not emit exact_format, required_evidence, fields, examples, post_processing, or custom keys.".to_string());
    parts.push("locator_hint must be a clean concrete locator value or concrete target pair, not a full instruction sentence and not explanatory prose. If no clean locator is known, leave it empty and let needs_clarify/decision express the missing target.".to_string());
    parts.push("Allowed response_shape: free, one_sentence, strict, scalar, file_token. Allowed locator_kind: none, path, current_workspace, url, filename. Allowed delivery_intent: none, file_single, directory_lookup, directory_batch_files.".to_string());
    parts.push("Allowed semantic_kind: none, raw_command_output, service_status, hidden_entries_check, file_names, directory_names, directory_entry_groups, file_paths, directory_purpose_summary, content_excerpt_summary, excerpt_kind_judgment, recent_artifacts_judgment, workspace_project_summary, scalar_count, quantity_comparison, execution_failed_step, generated_file_delivery, scalar_path_only, existence_with_path, existence_with_path_summary, recent_scalar_equality_check, git_commit_subject, structured_keys, sqlite_table_listing, sqlite_table_names_only, sqlite_database_kind_judgment, sqlite_schema_version, archive_list, archive_pack, archive_unpack, docker_ps, docker_images, docker_logs, docker_container_lifecycle.".to_string());
    parts.push("Allowed turn_type: task_request, task_append, task_replace, task_correct, task_scope_update, run_control, approval_decision, status_query, feedback_or_error, preference_or_memory, or empty string. clarify is a decision, never a turn_type or resume_behavior.".to_string());
    parts.push("state_patch must be a JSON object or null. Use null when there is no structured update; never output an empty string for state_patch. For ordered-entry follow-ups against an active ordered list, set state_patch.ordered_entry_ref to {\"index\":N,\"index_base\":1} for absolute item selection or {\"relative_offset\":K} for signed relative selection. When a standalone current REQUEST creates a new user-visible deliverable that later short corrections should edit, set state_patch.primary_task_update=\"replace\" and state_patch.active_task_boundary=\"new_deliverable\". For a clear deictic reference, set state_patch.deictic_reference={\"target\":\"current_action_result\"|\"current_turn_locator\"|\"comparison_result\"|\"unresolved_prior_object\"|\"missing_locator\"|\"ambiguous_locator\"}; unresolved/missing/ambiguous targets mean safe clarify. The runtime consumes structured numbers/targets, not language-specific ordinal words, pronouns, or connectors.".to_string());
    parts.push("Every enum field must be exactly one listed schema token. Do not output aliases, combined values, or explanatory prose in decision/output_contract/execution_recipe/turn_type/target_task_policy.".to_string());
    parts.push("Boolean fields must be JSON true/false, not prose. self_extension must be an object with mode/trigger/execute_now; use {\"mode\":\"none\",\"trigger\":\"none\",\"execute_now\":false} unless the user explicitly asks for self-extension. If locator_kind=\"none\", locator_hint must be \"\".".to_string());
    parts.push("If the user asks to observe/list/read first but only return a scalar result, set response_shape=\"scalar\" and use a matching semantic_kind only when one applies: scalar_count for generic counts, hidden_entries_check for hidden/dot-prefixed entry counts, scalar_path_only only for a path/current-directory/workspace-location answer, sqlite_schema_version for SQLite schema-version metadata. For config field values, package names, usernames, hostnames, titles, IDs, or other non-path scalar values, keep semantic_kind=\"none\" unless another specific enum applies. If the final answer must include both a structured field/key/path identifier and its value, it is not a scalar-only value response: use response_shape=\"strict\" and preserve the key/value shape in resolved_user_intent. If the request requires an exact non-scalar output format with fixed count, body-only delivery, one-line fixed format, placeholder format, or no-extra-output delivery, set response_shape=\"strict\" and preserve the exact format in resolved_user_intent. For any exact counted-sentence requirement, also set exact_sentence_count to that positive integer; use response_shape=\"strict\" when the count is greater than 1. Never put natural-language format descriptions in response_shape.".to_string());
    parts.push("For ordered command/tool execution where the final answer should identify only the failed step(s), set response_shape=\"strict\", semantic_kind=\"execution_failed_step\", requires_content_evidence=true, delivery_required=false. This is a semantic judgment from the requested final answer shape, not a phrase-list trigger.".to_string());
    parts.push("For requests to create/save/write a new artifact and then send/deliver it, set response_shape=\"file_token\", semantic_kind=\"generated_file_delivery\", delivery_required=true, delivery_intent=\"file_single\", requires_content_evidence=true. If the user did not supply a filename but the artifact type/content is clear, do not ask for one; let execution planning choose a safe workspace filename.".to_string());
    parts.push("For requests to send/deliver/receive an existing or selected local file, including a file selected from an observed or target directory by ordinal/order such as first/last/newest/largest, set wants_file_delivery=true, response_shape=\"file_token\", delivery_required=true, delivery_intent=\"file_single\", requires_content_evidence=true. The final answer must be a file token, not a bare filename, file_path_and_content answer_candidate, or pasted file content. This is a semantic delivery contract, not a phrase list.".to_string());
    parts.push("Text drafting/composition is not file delivery by default. If REQUEST asks to write/draft/compose an article, note, proposal, summary, checklist, tutorial, guide, or long-form text for the chat, but does not explicitly ask to save it to a file/path/document or send/deliver it as an attachment/artifact, do not use response_shape=\"file_token\" or semantic_kind=\"generated_file_delivery\". Keep delivery_required=false, wants_file_delivery=false, and use response_shape=\"free\" or \"strict\" according to the requested prose format. If the text is project-bound and needs workspace facts, use decision=\"planner_execute\", requires_content_evidence=true, locator_kind=\"current_workspace\"; still keep file delivery disabled. Examples: \"帮我写一篇关于 RustClaw 的长文\" / \"Write a long article about RustClaw\" means pasted prose in chat, while \"帮我写成 md 文件并发给我\" / \"Create a markdown file and send it to me\" means generated file delivery.".to_string());
    parts.push("For exact same/different comparison of two scalar/field values that still need observation, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, response_shape=\"strict\", semantic_kind=\"recent_scalar_equality_check\". Keep the requested final line format in resolved_user_intent.".to_string());
    parts.push("For a comparison where one side is a scalar field/value from a structured manifest or config file and the other side is the corresponding value mentioned in a README/docs file, use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, semantic_kind=\"recent_scalar_equality_check\", and response_shape=\"one_sentence\"/\"strict\" according to the requested final answer. This is a semantic contract for field/document evidence, not generic document summarization.".to_string());
    parts.push("For file/path metadata comparisons across concrete local targets (for example size/大小, modified time/修改时间, existence state, or other observable path facts), use decision=\"planner_execute\", requires_content_evidence=true, delivery_required=false, semantic_kind=\"quantity_comparison\", and response_shape=\"scalar\"/\"one_sentence\"/\"strict\" according to the requested final answer. This is a semantic contract decision, not a phrase-list trigger; do not treat metadata comparison as document content summarization just because the user also asks for a short explanation.".to_string());
    parts.push("For git commit subject/title requests, use decision=\"planner_execute\", requires_content_evidence=true, response_shape=\"scalar\" or \"strict\" according to the user's requested format, and semantic_kind=\"git_commit_subject\". Do not publish the raw git oneline hash when the final answer asks for the subject/title only.".to_string());
    parts.push("For structured document key-name requests against JSON/TOML/YAML/config files, use decision=\"planner_execute\", requires_content_evidence=true, response_shape=\"strict\" when the user asks only for the keys, and semantic_kind=\"structured_keys\". Keep locator_kind/locator_hint pointed at the structured file; do not treat key-name requests as file excerpts.".to_string());
    parts.push("For hidden or dot-prefixed directory entry checks, use decision=\"planner_execute\", requires_content_evidence=true, locator_kind=\"current_workspace\" or \"path\", and semantic_kind=\"hidden_entries_check\". When the final answer is constrained to count only, use response_shape=\"scalar\" with this same semantic_kind. When the final answer is constrained to yes/no plus a limited set of entries, use response_shape=\"strict\" so later stages do not prepend execution traces.".to_string());
    parts.push("For existence checks whose final answer is a presence judgment over a concrete file, directory, path, or local artifact, use decision=\"planner_execute\", requires_content_evidence=true, semantic_kind=\"existence_with_path\", and the narrowest locator_kind that matches the target scope. If the final answer asks only for yes/no or exists/not-exists, use response_shape=\"scalar\"; if it asks to include a path/locator or other evidence fields, use response_shape=\"strict\". Do not use semantic_kind=\"scalar_count\" merely because the requested final answer is short or binary; presence judgment is not numeric counting. If the same request also asks for a brief content-grounded purpose, summary, role, or explanation when found, use semantic_kind=\"existence_with_path_summary\" instead so planning observes both the path and bounded content before synthesis. Preserve the final answer wording constraint in resolved_user_intent so later stages do not prepend execution traces.".to_string());
    parts.push("For directory/file inventory with name or extension filtering, set requires_content_evidence=true and locator_kind=\"current_workspace\" or \"path\". Use semantic_kind=\"file_names\" only when the final answer is an exact file or mixed entry names-only list. Use semantic_kind=\"directory_names\" when the final answer is exact folder/directory names only. Use semantic_kind=\"directory_entry_groups\" when the final answer must separate the same directory's files and directories into groups. Use semantic_kind=\"file_paths\" when the final answer must be file paths, especially repository/workspace-wide extension searches or representative file path lists. If the same request also asks for explanation, purpose, judgment, comparison, or a brief conclusion, do not use an exact names/paths contract; use directory_purpose_summary when it asks what entries are for / more like, otherwise keep semantic_kind=\"none\" and preserve the combined listing+synthesis requirement in resolved_user_intent/reason. If a nuance has no enum, keep response_shape=\"free\" or semantic_kind=\"none\" instead of inventing enum values.".to_string());
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
    parts.push("CONTRACT: output_contract must be a JSON object. hidden/dot-entry check => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"hidden_entries_check\". yes/no-only existence check => response_shape=\"scalar\", semantic_kind=\"existence_with_path\"; existence check that must return a path/locator/evidence field => response_shape=\"strict\", semantic_kind=\"existence_with_path\"; if it also needs a content-grounded purpose/summary/explanation when found, use semantic_kind=\"existence_with_path_summary\". exact file or mixed entry names list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"file_names\". exact folder/directory names list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"directory_names\". grouped files-vs-directories list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"directory_entry_groups\". exact file paths list => response_shape=\"strict\", requires_content_evidence=true, semantic_kind=\"file_paths\". local file/path metadata comparison => requires_content_evidence=true, semantic_kind=\"quantity_comparison\". git commit subject/title only => response_shape=\"scalar\", requires_content_evidence=true, semantic_kind=\"git_commit_subject\". current path only => response_shape=\"scalar\", semantic_kind=\"scalar_path_only\"; never use scalar_path_only for directory listings.".to_string());
    // Keep memory immediately before the current request so providers with
    // compact head+tail truncation preserve recent goals with the query.
    parts.push(compact_prompt_slot(
        "MEMORY",
        &route_view.memory_context,
        900,
    ));
    // Keep recent assistant replies closest to the request so exact scalar
    // recall can use the assistant's visible answer rather than memory scores.
    parts.push(compact_prompt_slot(
        "ASSISTANT",
        &route_view.recent_assistant_replies,
        420,
    ));
    parts.push(compact_prompt_slot("RUNTIME", &runtime_context, 260));
    parts.push("LOCAL_EXEC: local file/dir/command/count/metadata/read/list/summarize => decision=\"planner_execute\"; no cannot-access-FS reply; do not ask user to paste local files; current target beats prior directory.".to_string());
    parts.push("SUMMARY_RECALL: summary != ID recall; answer_candidate empty unless exact scalar requested; memory scores are metadata.".to_string());
    // Keep the immediate structured follow-up anchor next to REQUEST so
    // ordinal/deictic resolution does not fall back to older memory or prose
    // when a provider truncates the middle context.
    parts.push("FOLLOWUP_ANCHOR_PRIORITY: ANCHOR is the latest structured execution state and overrides MEMORY/ASSISTANT/RECENT for ordinal or deictic follow-ups unless REQUEST names a new target.".to_string());
    parts.push("ACTIVE_TASK_PRIORITY: if REQUEST semantically refines the active writing/drafting/planning deliverable, ACTIVE_TASK overrides MEMORY/ASSISTANT scalar recall candidates unless REQUEST explicitly asks to recall that exact scalar.".to_string());
    parts.push(compact_prompt_slot(
        "ACTIVE_TASK",
        &route_view.active_task_context,
        900,
    ));
    parts.push(compact_prompt_slot(
        "ANCHOR",
        &route_view.active_execution_anchor_context,
        700,
    ));
    parts.push(compact_prompt_slot("REQUEST", req, 480));
    parts.join("\n")
}

#[cfg(test)]
fn normalize_intent_normalizer_raw_for_schema(raw: &str, req: &str) -> String {
    normalize_intent_normalizer_raw_for_schema_with_report(raw, req).0
}

fn normalize_intent_normalizer_raw_for_schema_with_report(
    raw: &str,
    req: &str,
) -> (String, ContractRepairReport) {
    let parsed_value = crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw);
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
) -> Option<&'static str> {
    if parse_first_layer_decision_text(&out.decision) != Some(FirstLayerDecision::DirectAnswer)
        || out.needs_clarify
    {
        return None;
    }
    if out.wants_file_delivery {
        return Some("chat_route_with_file_delivery_request");
    }
    let Some(contract) = out.output_contract.as_ref() else {
        return None;
    };
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
    // Schema repair only: some small-context models put a localized
    // one-line description into the enum field. Treat that as the
    // canonical exact-format contract instead of routing from user text.
    if trimmed.contains("一行") || trimmed.contains("单行") {
        return "strict";
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

fn normalize_execution_recipe_for_schema(obj: &mut serde_json::Map<String, Value>, _req: &str) {
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
    let Some(Value::Object(map)) = value else {
        return false;
    };
    map.iter().any(|(key, value)| {
        matches!(
            normalize_schema_token(key).as_str(),
            "command" | "commands" | "cmd" | "cmds" | "shell_command" | "shell_commands"
        ) && value_has_nonempty_scalar_text(value)
    })
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

    contract.insert("delivery_required".to_string(), Value::Bool(false));
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
        || semantic_kind == OutputSemanticKind::GitCommitSubject.as_str()
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
    let Some(decision) = parse_first_layer_decision_text(&repair.decision) else {
        return false;
    };
    let Some(output_contract) = repair.output_contract else {
        return false;
    };
    let Some(execution_recipe) = repair.execution_recipe else {
        return false;
    };

    out.decision = decision.as_str().to_string();
    out.needs_clarify = repair.needs_clarify;
    out.clarify_question = repair.clarify_question;
    if !repair.resolved_user_intent.trim().is_empty() {
        out.resolved_user_intent = repair.resolved_user_intent;
    }
    out.wants_file_delivery = repaired_contract_wants_file_delivery(&output_contract);
    out.output_contract = Some(output_contract);
    out.execution_recipe = Some(execution_recipe);
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
    true
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
    let req_surface = crate::intent::surface_signals::analyze_prompt_surface(req);
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
            // Planner-first: do not recover semantic execution locally when the
            // normalizer LLM is unavailable. Stay on AskClarify until the LLM path
            // can classify the request.
            warn!(
                "intent_normalizer llm failed, falling back to safe clarify: task_id={} err={}",
                task.task_id, err
            );
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
        if let Some(detail) = semantic_suspect_detail_for_normalizer_output(&out) {
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
        let mut wants_file_delivery = out.wants_file_delivery;
        let mut output_contract =
            parse_output_contract(out.output_contract.clone(), wants_file_delivery);
        let mut clarify_question = out.clarify_question.trim().to_string();
        let mut execution_recipe_hint = parse_execution_recipe_hint(out.execution_recipe.clone());
        let mut first_layer_decision = parsed_decision.unwrap_or_else(|| {
            if out.needs_clarify {
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
        let mut synced_route_label =
            route_label_from_first_layer_decision(first_layer_decision, execution_finalize_style);
        let structural_contract_repair = apply_current_turn_structural_contract_repair(
            &mut output_contract,
            req,
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
                &req_surface,
                wants_file_delivery,
                schedule_kind,
                execution_recipe_hint,
                out.needs_clarify,
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
        let decision_contract_conflict_repair = if !out.needs_clarify
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
        let current_turn_anchor_repair_allowed = current_turn_anchor_drift_repair_allowed(
            first_layer_decision,
            out.needs_clarify,
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
                out.needs_clarify,
            )
        {
            first_layer_decision = FirstLayerDecision::PlannerExecute;
            execution_finalize_style = finalize_style;
            synced_route_label = route_label_from_first_layer_decision(
                first_layer_decision,
                execution_finalize_style,
            );
        }
        let mut needs_clarify = out.needs_clarify;
        let state_patch = out.state_patch.clone().filter(is_meaningful_state_patch);
        let answer_candidate_path_repair = apply_answer_candidate_path_evidence_repair(
            &mut output_contract,
            &out.answer_candidate,
            state_patch.as_ref(),
            &state.skill_rt.workspace_root,
            needs_clarify,
            &mut first_layer_decision,
            &mut execution_finalize_style,
        );
        let structured_clarify_repair = apply_spurious_structured_observation_clarify_repair(
            &mut output_contract,
            req,
            &req_surface,
            &state.skill_rt.workspace_root,
            state_patch.as_ref(),
            &mut needs_clarify,
            &mut clarify_question,
            &mut first_layer_decision,
            &mut execution_finalize_style,
        );
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
            out.needs_clarify,
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
        if let Some(repair_reason) = structured_clarify_repair {
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
    // Planner-first: do not synthesize Act/ChatAct locally on parser failure.
    let _ = (resume_context, binding_context);
    let fallback = empty_clarify_decision(req, "normalizer_parse_failed");
    normalizer_output_from_fallback(req, "parse_failed_safe_clarify", fallback, None)
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
mod tests {
    use super::{
        apply_self_contained_payload_direct_answer_contract_repair,
        normalizer_output_from_fallback, parse_execution_recipe_hint, ClarifyQuestionPolicy,
        IntentExecutionRecipeOut, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
        OutputResponseShape, OutputSemanticKind, RouteDecision, ScheduleKind, TargetTaskPolicy,
        TurnType,
    };
    use crate::{
        execution_recipe::{
            ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeTargetScope,
        },
        FirstLayerDecision,
    };

    #[test]
    fn parse_execution_recipe_hint_accepts_explicit_ops_service_contract() {
        let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
            kind: "ops_closed_loop".to_string(),
            profile: "ops_service".to_string(),
            target_scope: "system".to_string(),
        }))
        .expect("execution recipe spec");
        assert_eq!(spec.profile, ExecutionRecipeProfile::OpsService);
        assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::System);
        assert!(spec.inspect_first);
        assert!(spec.validation_required);
    }

    #[test]
    fn inline_json_answer_candidate_can_repair_to_direct_answer_contract() {
        let request = r#"把这个 JSON 数组按 score 从高到低排序，只输出 name 顺序：[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
        let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
        let mut contract = IntentOutputContract {
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            locator_hint: "/tmp/rustclaw".to_string(),
            semantic_kind: OutputSemanticKind::StructuredKeys,
            ..IntentOutputContract::default()
        };
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize = crate::ActFinalizeStyle::ChatWrapped;

        let repair = apply_self_contained_payload_direct_answer_contract_repair(
            &mut contract,
            &surface,
            false,
            ScheduleKind::None,
            None,
            false,
            "beta, alpha",
            &mut decision,
            &mut finalize,
        );

        assert_eq!(
            repair,
            Some("self_contained_payload_direct_answer_contract")
        );
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert_eq!(finalize, crate::ActFinalizeStyle::Plain);
        assert!(!contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
        assert!(contract.locator_hint.is_empty());
    }

    #[test]
    fn inline_json_direct_answer_repair_rejects_explicit_path() {
        let request = r#"读取 data/input.json 并按 [{"field":"score"}] 排序"#;
        let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
        let mut contract = IntentOutputContract {
            requires_content_evidence: true,
            semantic_kind: OutputSemanticKind::StructuredKeys,
            ..IntentOutputContract::default()
        };
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize = crate::ActFinalizeStyle::ChatWrapped;

        let repair = apply_self_contained_payload_direct_answer_contract_repair(
            &mut contract,
            &surface,
            false,
            ScheduleKind::None,
            None,
            false,
            "beta, alpha",
            &mut decision,
            &mut finalize,
        );

        assert_eq!(repair, None);
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert!(contract.requires_content_evidence);
    }

    #[test]
    fn normalizer_schema_normalization_preserves_direct_answer_decision() {
        let raw = r#"{"resolved_user_intent":"client-like-continuous-123","needs_clarify":false,"clarify_question":"","reason":"recent memory recall","confidence":1.0,"decision":"direct_answer"}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value.get("resolved_user_intent").and_then(|v| v.as_str()),
            Some("client-like-continuous-123")
        );
    }

    #[test]
    fn normalizer_schema_normalization_preserves_planner_and_direct_decisions() {
        let raw = r#"{"resolved_user_intent":"check then explain","needs_clarify":false,"decision":"planner_execute"}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );

        let raw = r#"{"resolved_user_intent":"not an execution request","needs_clarify":false,"decision":"direct_answer"}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("direct_answer")
        );
    }

    #[test]
    fn normalizer_schema_normalization_preserves_object_resolved_intent() {
        let raw = r#"{"resolved_user_intent":{"test_id":"client-like-continuous-123"},"needs_clarify":false,"decision":"direct_answer"}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("resolved_user_intent").and_then(|v| v.as_str()),
            Some(r#"{"test_id":"client-like-continuous-123"}"#)
        );
    }

    #[test]
    fn normalizer_schema_normalization_accepts_percent_confidence() {
        let raw = r#"{"resolved_user_intent":"检查当前目录隐藏文件","needs_clarify":false,"clarify_question":"","reason":"local inspection","confidence":100,"decision":"planner_execute"}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(value.get("confidence").and_then(|v| v.as_f64()), Some(1.0));
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
    }

    #[test]
    fn normalizer_schema_normalization_recovers_stray_quote_after_bool() {
        let raw = r#"{"resolved_user_intent":"检查仓库中是否存在 rustclaw.service","needs_clarify":false,"clarify_question":"","reason":"repo inspection","confidence":0.95,"decision":"planner_execute","should_refresh_long_term_memory":false","agent_display_name_hint":"","output_contract":{"response_shape":"strict","requires_content_evidence":true,"delivery_required":false,"locator_kind":"current_workspace","delivery_intent":"none","semantic_kind":"existence_with_path","locator_hint":"rustclaw.service","self_extension":{"mode":"none","trigger":"none","execute_now":false}},"execution_recipe":{"kind":"none","profile":"none","target_scope":"none"}}"#;
        assert!(serde_json::from_str::<serde_json::Value>(raw).is_err());
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("existence_with_path")
        );
        assert_eq!(
            value
                .get("should_refresh_long_term_memory")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn normalizer_schema_normalization_recovers_minimax_output_contract_only_payload() {
        let raw = r#"{"output_contract":{"response_shape":"free","requires_content_evidence":false,"delivery_required":true,"locator_kind":"path","delivery_intent":"list_filenames","semantic_kind":"file_listing","locator_hint":"logs"}}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 logs 目录下的前 10 个文件名，不要读内容",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("resolved_user_intent").and_then(|v| v.as_str()),
            Some("列出 logs 目录下的前 10 个文件名，不要读内容")
        );
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("file_names")
        );
        assert_eq!(
            value
                .pointer("/output_contract/delivery_required")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn contract_repair_report_marks_structured_alias_repair() {
        let raw = r#"{"output_contract":{"response_shape":"free","requires_content_evidence":false,"delivery_required":true,"locator_kind":"path","delivery_intent":"list_filenames","semantic_kind":"file_listing","locator_hint":"logs"}}"#;
        let (_normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
            raw,
            "列出 logs 目录下的前 10 个文件名，不要读内容",
        );

        assert!(report.source_csv().contains("enum_alias"));
        assert!(report.source_csv().contains("structured_contract"));
        assert!(report
            .detail_csv()
            .contains("output_contract_semantic_kind_normalized"));
        assert!(report
            .detail_csv()
            .contains("decision_promoted_by_output_contract"));
    }

    #[test]
    fn contract_repair_report_marks_command_payload_without_semantic_guess() {
        let raw = r#"{
          "resolved_user_intent":"列出 logs 目录下前 10 个文件名，不读取内容",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":null,
          "execution_recipe":{
            "command":"ls -1 logs/ | head -n 10",
            "working_dir":"/home/guagua/rustclaw"
          }
        }"#;
        let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
            raw,
            "列出 logs 目录下的前 10 个文件名",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert!(report.source_csv().contains("command_payload"));
        assert!(report
            .detail_csv()
            .contains("execution_recipe_command_payload"));
    }

    #[test]
    fn command_payload_path_becomes_observable_locator_contract() {
        let raw = r#"{
          "resolved_user_intent":"读取 /tmp/clawd.log 文件的最后 2 行内容",
          "needs_clarify":false,
          "decision":"direct_answer",
          "decision":"direct_answer",
          "output_contract":"file_tail_lines",
          "execution_recipe":{
            "command":"tail -n 2 /tmp/clawd.log",
            "encoding":"utf-8",
            "path":"/tmp/clawd.log"
          }
        }"#;
        let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
            raw,
            "就第二个，看看最后 2 行",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            value
                .pointer("/output_contract/locator_kind")
                .and_then(|v| v.as_str()),
            Some("path")
        );
        assert_eq!(
            value
                .pointer("/output_contract/locator_hint")
                .and_then(|v| v.as_str()),
            Some("/tmp/clawd.log")
        );
        assert!(report
            .detail_csv()
            .contains("execution_recipe_command_payload"));
    }

    #[test]
    fn contract_repair_report_marks_untrusted_recipe_as_conservative_none() {
        let raw = r#"{
          "resolved_user_intent":"总结刚才的对话",
          "needs_clarify":false,
          "decision":"direct_answer",
          "output_contract":"summary",
          "execution_recipe":["SUMMARIZE"]
        }"#;
        let (_normalized, report) =
            super::normalize_intent_normalizer_raw_for_schema_with_report(raw, "总结刚才的对话");

        assert!(report.source_csv().contains("conservative_none"));
        assert!(report
            .detail_csv()
            .contains("execution_recipe_untrusted_text_ignored"));
        assert!(
            !report.needs_llm_semantic_repair(),
            "dropping an untrusted execution_recipe payload is schema cleanup; it must not rewrite an otherwise valid route contract"
        );
    }

    #[test]
    fn contract_repair_report_still_repairs_unknown_semantic_contracts() {
        let raw = r#"{
          "resolved_user_intent":"检查当前目录状态",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "semantic_kind":"unknown_observable_semantic",
            "locator_hint":""
          },
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"unknown"}
        }"#;
        let (_normalized, report) =
            super::normalize_intent_normalizer_raw_for_schema_with_report(raw, "检查当前目录状态");

        assert!(report.source_csv().contains("conservative_none"));
        assert!(report
            .detail_csv()
            .contains("output_contract_unknown_semantic_ignored"));
        assert!(report.needs_llm_semantic_repair());
    }

    #[test]
    fn current_turn_anchor_drift_repair_discards_contextual_path_contract() {
        let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
        let mut contract = IntentOutputContract {
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            semantic_kind: OutputSemanticKind::SqliteSchemaVersion,
            locator_hint: "/tmp/rustclaw-anchor-test/data/db-basic-contract.sqlite".to_string(),
            ..Default::default()
        };

        let repair = super::apply_current_turn_anchor_drift_repair(
            &mut contract,
            "查询 /tmp/rustclaw-anchor-test/data/db-basic-contract.sqlite 的 schema version",
            "/tmp/rustclaw-anchor-test/logs",
            workspace,
        );

        assert_eq!(
            repair,
            Some("current_turn_anchor_overrides_contextual_target")
        );
        assert_eq!(contract.response_shape, OutputResponseShape::Free);
        assert!(contract.requires_content_evidence);
        assert!(!contract.delivery_required);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
        assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
        assert_eq!(contract.locator_hint, "/tmp/rustclaw-anchor-test/logs");
    }

    #[test]
    fn current_turn_anchor_drift_repair_preserves_file_delivery_contract() {
        let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
        let mut contract = IntentOutputContract {
            response_shape: OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            locator_kind: OutputLocatorKind::Path,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/tmp/rustclaw-anchor-test/old.md".to_string(),
            ..Default::default()
        };

        let repair = super::apply_current_turn_anchor_drift_repair(
            &mut contract,
            "Send me /tmp/rustclaw-anchor-test/old.md",
            "/tmp/rustclaw-anchor-test/LICENSE.zh-CN.md",
            workspace,
        );

        assert_eq!(
            repair,
            Some("current_turn_anchor_overrides_contextual_target")
        );
        assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
        assert!(!contract.requires_content_evidence);
        assert!(contract.delivery_required);
        assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
        assert_eq!(
            contract.locator_hint,
            "/tmp/rustclaw-anchor-test/LICENSE.zh-CN.md"
        );
    }

    #[test]
    fn current_turn_anchor_drift_repair_keeps_compatible_child_path() {
        let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
        let mut contract = IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "/tmp/rustclaw-anchor-test/logs/clawd.log".to_string(),
            ..Default::default()
        };

        let repair = super::apply_current_turn_anchor_drift_repair(
            &mut contract,
            "列出 /tmp/rustclaw-anchor-test/logs/clawd.log 的基本信息",
            "/tmp/rustclaw-anchor-test/logs",
            workspace,
        );

        assert_eq!(repair, None);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::FileNames);
        assert_eq!(
            contract.locator_hint,
            "/tmp/rustclaw-anchor-test/logs/clawd.log"
        );
    }

    #[test]
    fn current_turn_anchor_drift_repair_keeps_multi_target_locator_contract() {
        let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
        let mut contract = IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "README.md, README.zh-CN.md, Cargo.toml".to_string(),
            ..Default::default()
        };

        let repair = super::apply_current_turn_anchor_drift_repair(
            &mut contract,
            "Check README.md, README.zh-CN.md, and Cargo.toml in the current workspace",
            "/tmp/rustclaw-anchor-test/README.md",
            workspace,
        );

        assert_eq!(repair, None);
        assert_eq!(contract.response_shape, OutputResponseShape::Strict);
        assert_eq!(
            contract.semantic_kind,
            OutputSemanticKind::ExistenceWithPath
        );
        assert_eq!(
            contract.locator_hint,
            "README.md, README.zh-CN.md, Cargo.toml"
        );
    }

    #[test]
    fn current_turn_anchor_repair_stays_off_for_executionless_chat() {
        let contract = IntentOutputContract::default();
        let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

        assert!(!super::current_turn_anchor_drift_repair_allowed(
            FirstLayerDecision::DirectAnswer,
            false,
            &contract,
            false,
            crate::ScheduleKind::None,
            None,
            workspace,
        ));
    }

    #[test]
    fn current_turn_anchor_repair_allowed_for_structured_evidence_contract() {
        let contract = IntentOutputContract {
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };
        let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

        assert!(super::current_turn_anchor_drift_repair_allowed(
            FirstLayerDecision::DirectAnswer,
            false,
            &contract,
            false,
            crate::ScheduleKind::None,
            None,
            workspace,
        ));
    }

    #[test]
    fn current_turn_anchor_repair_allowed_for_explicit_act_route() {
        let contract = IntentOutputContract::default();
        let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

        assert!(super::current_turn_anchor_drift_repair_allowed(
            FirstLayerDecision::PlannerExecute,
            false,
            &contract,
            false,
            crate::ScheduleKind::None,
            None,
            workspace,
        ));
    }

    #[test]
    fn current_turn_anchor_repair_stays_off_for_current_workspace_root_identity() {
        let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test/rustclaw");
        let contract = IntentOutputContract {
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            locator_hint: "RustClaw".to_string(),
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };

        assert!(!super::current_turn_anchor_drift_repair_allowed(
            FirstLayerDecision::PlannerExecute,
            false,
            &contract,
            false,
            crate::ScheduleKind::None,
            None,
            workspace,
        ));
    }

    #[test]
    fn current_turn_anchor_repair_stays_off_for_current_workspace_absolute_hint() {
        let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test/rustclaw");
        let contract = IntentOutputContract {
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            locator_hint: "/tmp/rustclaw-anchor-test/rustclaw".to_string(),
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };

        assert!(!super::current_turn_anchor_drift_repair_allowed(
            FirstLayerDecision::PlannerExecute,
            false,
            &contract,
            false,
            crate::ScheduleKind::None,
            None,
            workspace,
        ));
    }

    #[test]
    fn contract_repair_judge_schema_accepts_canonical_payload() {
        let raw = r#"{
          "apply": true,
          "reason": "malformed_contract_semantically_requires_directory_listing",
          "confidence": 0.91,
          "decision": "planner_execute",
          "needs_clarify": false,
          "clarify_question": "",
          "resolved_user_intent": "列出 logs 目录下前 3 个文件名，不读取内容",
          "output_contract": {
            "response_shape": "strict",
            "exact_sentence_count": null,
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "path",
            "delivery_intent": "none",
            "semantic_kind": "file_names",
            "locator_hint": "logs",
            "self_extension": {"mode": "none", "trigger": "none", "execute_now": false}
          },
          "execution_recipe": {"kind": "none", "profile": "none", "target_scope": "unknown"},
          "turn_type": "task_request",
          "target_task_policy": "standalone"
        }"#;

        crate::prompt_utils::validate_against_schema::<super::ContractRepairJudgeOut>(
            raw,
            crate::prompt_utils::PromptSchemaId::ContractRepairJudge,
        )
        .expect("contract repair judge payload should validate");
    }

    #[test]
    fn contract_repair_judge_output_applies_semantic_contract() {
        let mut out = super::IntentNormalizerOut {
            resolved_user_intent: "列出 document 目录下的所有文件名".to_string(),
            answer_candidate: String::new(),
            resume_behavior: "none".to_string(),
            schedule_kind: "none".to_string(),
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: "malformed recipe text was ignored".to_string(),
            confidence: 0.5,
            decision: "direct_answer".to_string(),
            schedule_intent: None,
            output_contract: Some(super::IntentOutputContractOut::default()),
            execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
            turn_type: "task_request".to_string(),
            target_task_policy: "standalone".to_string(),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let repair = super::ContractRepairJudgeOut {
            apply: true,
            reason: "malformed_contract_semantically_requires_directory_listing".to_string(),
            confidence: 0.91,
            decision: "planner_execute".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            resolved_user_intent: "列出 document 目录下所有文件名，只输出文件名列表".to_string(),
            output_contract: Some(super::IntentOutputContractOut {
                response_shape: "strict".to_string(),
                exact_sentence_count: None,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: "path".to_string(),
                delivery_intent: "none".to_string(),
                semantic_kind: "file_names".to_string(),
                locator_hint: "document".to_string(),
                self_extension: None,
            }),
            execution_recipe: Some(super::IntentExecutionRecipeOut {
                kind: "none".to_string(),
                profile: "none".to_string(),
                target_scope: "unknown".to_string(),
            }),
            turn_type: "task_request".to_string(),
            target_task_policy: "standalone".to_string(),
        };

        assert!(super::apply_contract_repair_judge_output(&mut out, repair));

        assert_eq!(out.decision, "planner_execute");
        assert_eq!(out.confidence, 0.91);
        assert!(out.reason.contains("llm_semantic_contract_repair"));
        let contract = super::parse_output_contract(out.output_contract, false);
        assert_eq!(contract.response_shape, OutputResponseShape::Strict);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::FileNames);
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_hint, "document");
    }

    #[test]
    fn contract_repair_judge_output_clears_stale_file_delivery_flag() {
        let mut out = super::IntentNormalizerOut {
            resolved_user_intent: "Write a short release note for RustClaw".to_string(),
            answer_candidate: String::new(),
            resume_behavior: "none".to_string(),
            schedule_kind: "none".to_string(),
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: "RustClaw".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: "malformed output contract".to_string(),
            confidence: 0.5,
            decision: "planner_execute".to_string(),
            schedule_intent: None,
            output_contract: Some(super::IntentOutputContractOut {
                response_shape: "file_token".to_string(),
                exact_sentence_count: None,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: "path".to_string(),
                delivery_intent: "file_single".to_string(),
                semantic_kind: "none".to_string(),
                locator_hint: String::new(),
                self_extension: None,
            }),
            execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
            turn_type: String::new(),
            target_task_policy: String::new(),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let repair = super::ContractRepairJudgeOut {
            apply: true,
            reason: "inline_text_contract".to_string(),
            confidence: 0.85,
            decision: "direct_answer".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            resolved_user_intent: "Write a short release note for RustClaw as inline content."
                .to_string(),
            output_contract: Some(super::IntentOutputContractOut {
                response_shape: "free".to_string(),
                exact_sentence_count: None,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: "none".to_string(),
                delivery_intent: "none".to_string(),
                semantic_kind: "none".to_string(),
                locator_hint: String::new(),
                self_extension: None,
            }),
            execution_recipe: Some(super::IntentExecutionRecipeOut {
                kind: "none".to_string(),
                profile: "none".to_string(),
                target_scope: "none".to_string(),
            }),
            turn_type: String::new(),
            target_task_policy: String::new(),
        };

        assert!(super::apply_contract_repair_judge_output(&mut out, repair));

        assert!(!out.wants_file_delivery);
        let contract = super::parse_output_contract(out.output_contract, out.wants_file_delivery);
        assert_eq!(contract.response_shape, OutputResponseShape::Free);
        assert!(!contract.delivery_required);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    }

    #[test]
    fn semantic_suspect_flags_chat_with_observable_contract() {
        let out = super::IntentNormalizerOut {
            resolved_user_intent: "check README.md exists".to_string(),
            answer_candidate: String::new(),
            resume_behavior: "none".to_string(),
            schedule_kind: "none".to_string(),
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: String::new(),
            confidence: 0.8,
            decision: "direct_answer".to_string(),
            schedule_intent: None,
            output_contract: Some(super::IntentOutputContractOut {
                response_shape: "scalar".to_string(),
                exact_sentence_count: None,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: "filename".to_string(),
                delivery_intent: "none".to_string(),
                semantic_kind: "existence_with_path".to_string(),
                locator_hint: "README.md".to_string(),
                self_extension: None,
            }),
            execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
            turn_type: String::new(),
            target_task_policy: String::new(),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };

        assert_eq!(
            super::semantic_suspect_detail_for_normalizer_output(&out),
            Some("chat_route_requires_content_evidence")
        );
    }

    #[test]
    fn contract_repair_judge_output_rejects_low_confidence() {
        let mut out = super::IntentNormalizerOut {
            resolved_user_intent: "总结刚才的对话".to_string(),
            answer_candidate: String::new(),
            resume_behavior: "none".to_string(),
            schedule_kind: "none".to_string(),
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: String::new(),
            confidence: 0.8,
            decision: "direct_answer".to_string(),
            schedule_intent: None,
            output_contract: Some(super::IntentOutputContractOut::default()),
            execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
            turn_type: String::new(),
            target_task_policy: String::new(),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };
        let repair = super::ContractRepairJudgeOut {
            apply: true,
            reason: "uncertain".to_string(),
            confidence: 0.59,
            decision: "planner_execute".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            resolved_user_intent: "bad".to_string(),
            output_contract: Some(super::IntentOutputContractOut {
                response_shape: "strict".to_string(),
                exact_sentence_count: None,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: "current_workspace".to_string(),
                delivery_intent: "none".to_string(),
                semantic_kind: "file_names".to_string(),
                locator_hint: String::new(),
                self_extension: None,
            }),
            execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
            turn_type: String::new(),
            target_task_policy: String::new(),
        };

        assert!(!super::apply_contract_repair_judge_output(&mut out, repair));
        assert_eq!(out.decision, "direct_answer");
        assert_eq!(out.resolved_user_intent, "总结刚才的对话");
    }

    #[test]
    fn observed_context_summary_followup_does_not_force_fresh_evidence() {
        let mut contract = IntentOutputContract::default();
        contract.response_shape = OutputResponseShape::OneSentence;
        contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
        contract.locator_kind = OutputLocatorKind::Filename;
        contract.locator_hint = "app.log".to_string();

        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "in one sentence tell me if anything looks abnormal",
        );
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            "in one sentence tell me if anything looks abnormal",
            &surface,
            std::path::Path::new("/workspace"),
            FirstLayerDecision::DirectAnswer,
            "",
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
        );

        assert_eq!(reason, Some("existing_observed_context_synthesis"));
        assert!(!contract.requires_content_evidence);
    }

    #[test]
    fn explicit_locator_summary_still_requires_fresh_evidence() {
        let mut contract = IntentOutputContract::default();
        contract.response_shape = OutputResponseShape::OneSentence;
        contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
        contract.locator_kind = OutputLocatorKind::Filename;
        contract.locator_hint = "app.log".to_string();

        let surface =
            crate::intent::surface_signals::analyze_prompt_surface("summarize app.log briefly");
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            "summarize app.log briefly",
            &surface,
            std::path::Path::new("/workspace"),
            FirstLayerDecision::DirectAnswer,
            "",
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
        );

        assert_eq!(reason, Some("semantic_contract_requires_evidence"));
        assert!(contract.requires_content_evidence);
    }

    #[test]
    fn normalizer_schema_normalization_recovers_minimax_file_list_search_payload() {
        let raw = r#"{
          "resolved_user_intent":"列出 document 目录中所有 .md 文件，排除 README，返回剩余的 .md 文件列表",
          "answer_candidate":[],
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"directory listing",
          "confidence":0.7,
          "decision":"planner_execute",
          "output_contract":{"response_shape":"list","semantic_kind":"file_list"},
          "execution_recipe":"list_md_files_excluding_readme",
          "turn_type":"file_query",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 document 目录里所有 .md 文件，但排除 README，告诉我还剩哪些",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("file_names")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_infer_malformed_recipe_array() {
        let raw = r#"{
          "resolved_user_intent":"用户请求读取 README 文件的开头内容，并用通俗易懂的非技术语言进行一句话总结",
          "answer_candidate":"",
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"need file evidence",
          "confidence":0.78,
          "decision":"direct_answer",
          "output_contract":"summary",
          "execution_recipe":["READ","SUMMARIZE"],
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "读取 README.md 的开头，用一句非技术语言总结",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_infer_string_recipe_locator() {
        let raw = r#"{
          "resolved_user_intent":"读取 README.md 开头部分并用非技术语言一句话总结其核心含义",
          "answer_candidate":"",
          "resume_behavior":"continue",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"Assistant",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"clear file read and summary request",
          "confidence":0.92,
          "decision":"direct_answer",
          "output_contract":"非技术用户可理解的一句话总结",
          "execution_recipe":"read_file:/home/guagua/rustclaw/README.md,line_range:[1-50],summarize:one_sentence_non_technical",
          "turn_type":"execute",
          "target_task_policy":"read_and_summarize",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "读取 README 开头内容，再用非技术用户能听懂的话做一句总结",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .pointer("/output_contract/locator_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/locator_hint")
                .and_then(|v| v.as_str()),
            Some("")
        );
    }

    #[test]
    fn malformed_recipe_array_without_locator_stays_chat() {
        let raw = r#"{
          "resolved_user_intent":"总结刚才的对话",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"chat summary",
          "confidence":0.8,
          "decision":"direct_answer",
          "output_contract":"summary",
          "execution_recipe":["SUMMARIZE"]
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "总结刚才的对话");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn parse_output_contract_clears_inconsistent_inline_delivery_flag() {
        let contract = super::parse_output_contract(
            Some(super::IntentOutputContractOut {
                response_shape: "strict".to_string(),
                exact_sentence_count: None,
                requires_content_evidence: true,
                delivery_required: true,
                locator_kind: "path".to_string(),
                delivery_intent: "response".to_string(),
                semantic_kind: "file_names".to_string(),
                locator_hint: "logs".to_string(),
                self_extension: None,
            }),
            false,
        );

        assert!(!contract.delivery_required);
        assert_eq!(contract.response_shape, OutputResponseShape::Strict);
        assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::FileNames);
    }

    #[test]
    fn parse_output_contract_clears_file_delivery_for_inline_raw_output_contract() {
        let contract = super::parse_output_contract(
            Some(super::IntentOutputContractOut {
                response_shape: "strict".to_string(),
                exact_sentence_count: None,
                requires_content_evidence: true,
                delivery_required: true,
                locator_kind: "path".to_string(),
                delivery_intent: "file_single".to_string(),
                semantic_kind: "raw_command_output".to_string(),
                locator_hint: "/tmp/app.log".to_string(),
                self_extension: None,
            }),
            false,
        );

        assert!(!contract.delivery_required);
        assert_eq!(contract.response_shape, OutputResponseShape::Strict);
        assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    }

    #[test]
    fn parse_output_contract_preserves_exact_sentence_count_as_strict_contract() {
        let contract = super::parse_output_contract(
            Some(super::IntentOutputContractOut {
                response_shape: "one_sentence".to_string(),
                exact_sentence_count: Some(serde_json::json!(3)),
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: "path".to_string(),
                delivery_intent: "none".to_string(),
                semantic_kind: "content_excerpt_summary".to_string(),
                locator_hint: "/tmp/report.md".to_string(),
                self_extension: None,
            }),
            false,
        );

        assert_eq!(contract.exact_sentence_count, Some(3));
        assert_eq!(contract.response_shape, OutputResponseShape::Strict);
        assert_eq!(
            contract.semantic_kind,
            OutputSemanticKind::ContentExcerptSummary
        );
    }

    #[test]
    fn normalizer_schema_normalization_coerces_string_bool_contract_fields() {
        let raw = r#"{"resolved_user_intent":"列出 document 目录文件名","needs_clarify":false,"decision":"planner_execute","output_contract":{"response_shape":"strict","requires_content_evidence":"true","delivery_required":"filename_list","locator_kind":"path","delivery_intent":"返回 document 目录下的文件名列表","semantic_kind":"filename_list","locator_hint":"document"}}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 document 目录下有哪些文件，只输出文件名列表",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            value
                .pointer("/output_contract/delivery_required")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("file_names")
        );
    }

    #[test]
    fn normalizer_schema_normalization_does_not_infer_custom_recipe_text() {
        let raw = r#"{
          "resolved_user_intent":"列出 document 目录下的所有文件名，仅输出文件名列表。",
          "answer_candidate":"",
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw测试助手",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"User explicitly requests file listing for the 'document' directory.",
          "confidence":0.98,
          "decision":"planner_execute",
          "output_contract":"list_of_strings",
          "execution_recipe":"list_files(directory='document', include_subdirs=False)",
          "turn_type":"command",
          "target_task_policy":"list_files",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 document 目录下有哪些文件，只输出文件名列表",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_infer_shell_file_listing_recipe_text() {
        let raw = r#"{
          "resolved_user_intent":"列出 document 目录下所有文件的文件名列表",
          "answer_candidate":"",
          "resume_behavior":false,
          "schedule_kind":"immediate",
          "schedule_intent":"execute",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户明确要求列出 document 目录下的文件列表",
          "confidence":"high",
          "decision":"direct_answer",
          "output_contract":"text",
          "execution_recipe":"执行 ls -1 document/ 获取文件名列表",
          "turn_type":"act",
          "target_task_policy":"browse_local_fs",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 document 目录下有哪些文件，只输出文件名列表",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("free")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_infer_hidden_entries_recipe_text() {
        let raw = r#"{
          "resolved_user_intent":"检查当前工作目录是否存在隐藏文件，回答有或没有，并提供3个具体例子",
          "answer_candidate":"",
          "resume_behavior":"",
          "schedule_kind":"",
          "schedule_intent":"",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"clear local filesystem observation",
          "confidence":0.92,
          "decision":"planner_execute",
          "output_contract":"",
          "execution_recipe":"list_hidden_files",
          "turn_type":"task",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("free")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_infer_hidden_entries_recipe_array() {
        let raw = r#"{
          "resolved_user_intent":"检查当前工作目录是否存在隐藏文件，只回答有或没有，并提供3个具体例子",
          "answer_candidate":"有",
          "needs_clarify":false,
          "reason":"local filesystem observation",
          "confidence":0.92,
          "decision":"planner_execute",
          "output_contract":"json",
          "execution_recipe":[
            "ls -a /home/guagua/rustclaw | grep '^\\.'",
            "Check if any hidden files exist",
            "Return answer '有' and examples: .git, .gitignore, .rustfmt.toml"
          ]
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("free")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_keeps_structured_contract_over_recipe_text() {
        let raw = r#"{
          "resolved_user_intent":"检查当前目录 /home/guagua/rustclaw 是否有隐藏文件（以点开头的文件），若存在则回答“有”并提供3个示例",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"requires actual directory observation",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":{"response_shape":"strict","semantic_kind":"existence_with_path","requires_content_evidence":true},
          "execution_recipe":{
            "command":"ls -la /home/guagua/rustclaw | grep '^\\.' | head -3",
            "action_type":"list_hidden_files"
          },
          "turn_type":"task",
          "target_task_policy":"check_hidden_entries"
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("existence_with_path")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_keeps_command_payload_as_execution_without_semantic_guess() {
        let raw = r#"{
          "resolved_user_intent":"列出 logs 目录下前 10 个文件名，不读取内容",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"local command can list directory entries",
          "confidence":0.91,
          "decision":"planner_execute",
          "output_contract":null,
          "execution_recipe":{
            "action":"local_exec",
            "command":"ls -1 logs/ | head -n 10",
            "working_dir":"/home/guagua/rustclaw"
          }
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 logs 目录下的前 10 个文件名",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("free")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_maps_legacy_command_result_contract_to_raw_output() {
        let raw = r#"{
          "resolved_user_intent":"执行pwd命令获取当前工作目录",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"local command execution",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":"command_execution_result",
          "execution_recipe":{
            "command":"pwd",
            "capture_stdout":true
          }
        }"#;
        let normalized =
            super::normalize_intent_normalizer_raw_for_schema(raw, "请执行 pwd，只输出命令结果");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("raw_command_output")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_clears_empty_path_locator_for_command_payload() {
        let raw = r#"{
          "resolved_user_intent":"Run a local shell command and summarize the command output.",
          "answer_candidate":"",
          "resume_behavior":"",
          "schedule_kind":"",
          "schedule_intent":"",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"A local command payload supplies the evidence source.",
          "confidence":0.95,
          "decision":"planner_execute",
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"path",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":"",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"run_cmd","command":"df -h"},
          "turn_type":"task_request",
          "target_task_policy":"new",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "Run a local shell command and summarize the output",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .pointer("/output_contract/locator_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/locator_hint")
                .and_then(|v| v.as_str()),
            Some("")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_tool_payload_as_execution_signal() {
        let raw = r#"{
          "action": "search_files",
          "args": {
            "pattern": "README.md",
            "search_path": ".",
            "max_results": 1
          }
        }"#;
        let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
            raw,
            "检查 README.md 是否存在，并只用 JSON 返回 path 和 size_bytes 两个字段。",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert!(report.source_csv().contains("tool_payload"));
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_infer_check_file_recipe_text() {
        let raw = r#"{
          "resolved_user_intent":"检查仓库中是否存在 rustclaw.service 文件",
          "answer_candidate":"没有",
          "needs_clarify":false,
          "reason":"repo inspection",
          "confidence":0.87,
          "decision":"direct_answer",
          "output_contract":"strict",
          "execution_recipe":"check_file"
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查仓库里有没有 rustclaw.service",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_infer_shell_find_recipe_text() {
        let raw = r#"{
          "resolved_user_intent":"检查仓库目录是否存在 rustclaw.service 文件，报告有或没有，并给出找到的路径",
          "answer_candidate":"有: /home/guagua/rustclaw/rustclaw.service",
          "needs_clarify":false,
          "reason":"file existence check",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":"raw_text",
          "execution_recipe":"find /home/guagua/rustclaw -name 'rustclaw.service' 2>/dev/null",
          "turn_type":"ask",
          "target_task_policy":"filesystem_existence"
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("free")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_file_names_only_contract() {
        let raw = r#"{
          "resolved_user_intent":"列出 document 目录下的文件名",
          "needs_clarify":false,
          "reason":"local file listing",
          "confidence":0.9,
          "decision":"direct_answer",
          "output_contract":"file_names_only",
          "execution_recipe":"ls document/"
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 document 目录下有哪些文件，只输出文件名列表",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("file_names")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_repair_file_listing_from_recipe_text() {
        let raw = r#"{
          "resolved_user_intent":"列出 /home/guagua/rustclaw/document 目录下的所有文件名",
          "answer_candidate":null,
          "schedule_kind":"immediate",
          "needs_clarify":false,
          "reason":"用户明确请求列出 document 目录下的文件，目标清晰，属于简单文件列表操作",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"scalar","semantic_kind":"scalar_path_only","requires_content_evidence":false},
          "execution_recipe":"LIST_FILES",
          "turn_type":"act"
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 document 目录下有哪些文件，只输出文件名列表",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("scalar")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("scalar_path_only")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_repairs_exact_format_scalar_comparison_contract() {
        let raw = r#"{"resolved_user_intent":"读取 UI/package.json 的 name 字段和 crates/clawd/Cargo.toml 的 package.name 字段，对比后单行输出：{UI名}, {Cargo名}, {一样|不一样}","needs_clarify":false,"decision":"planner_execute","output_contract":{"response_shape":"一行字符串，格式为：{UI_name}, {Cargo_name}, {一样|不一样}","requires_content_evidence":false,"delivery_required":true,"locator_kind":"none","delivery_intent":"直接返回对比结果","semantic_kind":"key_value_comparison","locator_hint":""}}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name，最后只用一行输出：前者、后者、一样或不一样",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|v| v.as_str()),
            Some("recent_scalar_equality_check")
        );
        assert_eq!(
            value
                .pointer("/output_contract/delivery_intent")
                .and_then(|v| v.as_str()),
            Some("none")
        );

        let raw = r#"{"resolved_user_intent":"比较UI/package.json的name字段与crates/clawd/Cargo.toml的package.name字段，输出一行格式：<UI名>, <Cargo名>, <一样|不一样>","needs_clarify":false,"decision":"planner_execute","output_contract":{"response_shape":"一行文字","requires_content_evidence":false,"delivery_required":"执行文件系统读取后输出单行结果","locator_hint":"UI/package.json, crates/clawd/Cargo.toml"}}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name，最后只用一行输出：前者、后者、一样或不一样",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            value
                .pointer("/output_contract/delivery_required")
                .and_then(|v| v.as_bool()),
            Some(false)
        );

        let raw = r#"{"resolved_user_intent":"compare two names","needs_clarify":false,"decision":"planner_execute","output_contract":{"response_shape":"single line string","requires_content_evidence":false,"delivery_required":true,"locator_kind":"file","delivery_intent":"comparison result line","semantic_kind":"comparison","locator_hint":"UI/package.json crates/clawd/Cargo.toml"}}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "compare two fields in one line",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|v| v.as_str()),
            Some("strict")
        );
    }

    #[test]
    fn normalizer_schema_normalization_coerces_non_object_self_extension_contract() {
        let raw = r#"{
          "resolved_user_intent": "用户请求一个 harmless chat deliverable",
          "needs_clarify": false,
          "clarify_question": "",
          "reason": "task is clear",
          "confidence": "high",
          "decision":"direct_answer",
          "output_contract": {
            "response_shape": "free",
            "requires_content_evidence": false,
            "delivery_required": "deliverable content",
            "locator_kind": "none",
            "delivery_intent": "provide answer",
            "semantic_kind": "entertainment",
            "locator_hint": "no locator is needed",
            "self_extension": "not requested"
          }
        }"#;
        let normalized =
            super::normalize_intent_normalizer_raw_for_schema(raw, "tell me something short");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value.get("needs_clarify").and_then(|value| value.as_bool()),
            Some(false)
        );
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract
                .get("delivery_required")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        assert_eq!(
            contract
                .get("semantic_kind")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        assert_eq!(
            contract
                .get("locator_hint")
                .and_then(|value| value.as_str()),
            Some("")
        );
        assert_eq!(
            contract
                .get("self_extension")
                .and_then(|value| value.get("mode"))
                .and_then(|value| value.as_str()),
            Some("none")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_coerces_model_contract_synonyms() {
        let raw = r#"{
          "resolved_user_intent": {"action":"find_file","target":"rustclaw.service","scope":"repository"},
          "needs_clarify": false,
          "reason": "repo inspection",
          "confidence": 0.9,
          "decision":"planner_execute",
          "output_contract": {
            "response_shape": "inline",
            "semantic_kind": "existence_boolean_with_path",
            "locator_kind": "repository",
            "delivery_intent": "list_directory",
            "extra_model_field": "ignored"
          },
          "action": "find_file"
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查仓库里有没有 rustclaw.service",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        let resolved_value: serde_json::Value = serde_json::from_str(
            value
                .get("resolved_user_intent")
                .and_then(|v| v.as_str())
                .expect("resolved intent string"),
        )
        .expect("resolved intent json");
        assert_eq!(
            resolved_value
                .get("target")
                .and_then(|value| value.as_str()),
            Some("rustclaw.service")
        );
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("response_shape").and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            contract.get("semantic_kind").and_then(|v| v.as_str()),
            Some("existence_with_path")
        );
        assert_eq!(
            contract.get("locator_kind").and_then(|v| v.as_str()),
            Some("current_workspace")
        );
        assert!(!contract.contains_key("extra_model_field"));
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_repairs_current_workspace_path_alias_contract() {
        let raw = r#"{
          "resolved_user_intent":"只输出当前工作目录的绝对路径，不要解释",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"plain_text",
            "semantic_kind":"filesystem_locator",
            "locator_kind":"directory_path",
            "locator_hint":"current_working_directory",
            "delivery_intent":"show",
            "requires_content_evidence":false,
            "delivery_required":"current_working_directory",
            "self_extension":"none"
          }
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "只输出当前工作目录的绝对路径，不要解释",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("response_shape").and_then(|v| v.as_str()),
            Some("scalar")
        );
        assert_eq!(
            contract.get("semantic_kind").and_then(|v| v.as_str()),
            Some("scalar_path_only")
        );
        assert_eq!(
            contract.get("locator_kind").and_then(|v| v.as_str()),
            Some("current_workspace")
        );
        assert_eq!(
            contract.get("locator_hint").and_then(|v| v.as_str()),
            Some("")
        );
        assert_eq!(
            contract.get("delivery_required").and_then(|v| v.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_coerces_null_locator_hint() {
        let raw = r#"{
          "turn_type": "standalone",
          "needs_clarify": false,
          "decision":"direct_answer",
          "output_contract": {
            "response_shape": "scalar",
            "semantic_kind": "none",
            "requires_content_evidence": false,
            "delivery_required": false,
            "locator_hint": null,
            "locator_kind": "none",
            "self_extension": {"mode":"none","trigger":"none","execute_now":false},
            "delivery_intent": "none"
          },
          "schedule_kind": null,
          "execution_recipe": {
            "kind":"none",
            "profile": null,
            "target_scope":"none",
            "repair_policies":[]
          },
          "resolved_user_intent": "输出测试编号 mimo-small-20260429_203108。",
          "clarify_question": null
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "只回答测试编号");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract
                .get("locator_hint")
                .and_then(|value| value.as_str()),
            Some("")
        );
        assert_eq!(
            value
                .get("clarify_question")
                .and_then(|value| value.as_str()),
            Some("")
        );
        assert_eq!(
            value.get("schedule_kind").and_then(|value| value.as_str()),
            Some("none")
        );
        let recipe = value
            .get("execution_recipe")
            .and_then(|value| value.as_object())
            .expect("execution recipe");
        assert_eq!(
            recipe.get("profile").and_then(|value| value.as_str()),
            Some("none")
        );
        assert!(!recipe.contains_key("repair_policies"));
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_coerces_none_schedule_intent_string() {
        let raw = r#"{
          "resolved_user_intent":"用户想获取刚才记住的测试编号 RC-CONT-CN-0428-A",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户请求已记住的编号",
          "confidence":0.99,
          "decision":"direct_answer",
          "output_contract":"text",
          "execution_recipe":{"kind":"none"},
          "turn_type":"",
          "target_task_policy":"none",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false,
          "answer":"RC-CONT-CN-0428-A"
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "刚才让你记住的连续测试编号是什么？只回答编号。",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert!(value
            .get("schedule_intent")
            .is_some_and(|value| value.is_null()));
        assert_eq!(
            value.get("decision").and_then(|value| value.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value
                .get("resolved_user_intent")
                .and_then(|value| value.as_str()),
            Some("用户想获取刚才记住的测试编号 RC-CONT-CN-0428-A")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_drops_none_schedule_intent_object_with_string_nested_fields()
    {
        let raw = r#"{
          "resolved_user_intent":"logs ディレクトリのファイル名を3つだけ一覧して。",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":{
            "kind":"none",
            "timezone":"",
            "schedule":"",
            "task":"",
            "target_job_id":"",
            "raw":"",
            "reason":"",
            "needs_clarify":false,
            "clarify_question":"",
            "confidence":0.0
          },
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"",
          "confidence":0.98,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"strict",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "semantic_kind":"file_names",
            "locator_hint":"logs",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"unknown"},
          "turn_type":"task_request",
          "target_task_policy":"standalone",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert!(value
            .get("schedule_intent")
            .is_some_and(|value| value.is_null()));
        assert_eq!(
            value.get("decision").and_then(|value| value.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .get("output_contract")
                .and_then(|value| value.get("semantic_kind"))
                .and_then(|value| value.as_str()),
            Some("file_names")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_scalar_output_contract_answer_candidate() {
        let raw = r#"{
          "resolved_user_intent":"查询之前记住的测试编号",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户请求回忆之前存储的信息",
          "confidence":1.0,
          "decision":"direct_answer",
          "output_contract":"client-like-continuous-20260430_094246",
          "execution_recipe":{"kind":"none"},
          "turn_type":"",
          "target_task_policy":"none",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "刚才我让你记住的测试编号是什么？只回答编号。",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .get("answer_candidate")
                .and_then(|value| value.as_str()),
            Some("client-like-continuous-20260430_094246")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|value| value.as_str()),
            Some("free")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_object_answer_candidate_and_ignores_json_contract()
    {
        let raw = r#"{
          "resolved_user_intent":"retrieve test ID",
          "answer_candidate":{"content":"client-like-continuous-20260430_095834"},
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"User asked for the stored test ID, which is available in short-term memory.",
          "confidence":0.99,
          "decision":"direct_answer",
          "output_contract":"json",
          "execution_recipe":{"kind":"none"},
          "turn_type":"memory",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "刚才我让你记住的测试编号是什么？只回答编号。",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .get("answer_candidate")
                .and_then(|value| value.as_str()),
            Some("client-like-continuous-20260430_095834")
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|value| value.as_str()),
            Some("free")
        );
        assert_eq!(
            value.get("turn_type").and_then(|value| value.as_str()),
            Some("")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_command_output_contract_with_unknown_recipe_kind() {
        let raw = r#"{
          "resolved_user_intent":"execute pwd command to get current working directory",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"User explicitly requests direct command execution with raw output, no summary.",
          "confidence":0.98,
          "decision":"planner_execute",
          "output_contract":"raw",
          "execution_recipe":{"kind":"shell","command":"pwd","requires_content_evidence":false,"locator_kind":"none"},
          "turn_type":"task_request",
          "target_task_policy":"none",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "执行 pwd，直接输出命令结果，不要总结",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|value| value.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|value| value.as_str()),
            Some("raw_command_output")
        );
        assert_eq!(
            value
                .pointer("/execution_recipe/kind")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .get("answer_candidate")
                .and_then(|value| value.as_str()),
            Some("")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_keeps_chat_raw_contract_non_executing() {
        let raw = r#"{
          "resolved_user_intent":"User wants a very short joke and explicitly requests no execution.",
          "answer_candidate":"Why don't scientists trust atoms? Because they make up everything.",
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"MiniMax-M2.1",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"User explicitly requested a joke and no execution.",
          "confidence":0.99,
          "decision":"direct_answer",
          "output_contract":"raw",
          "execution_recipe":null,
          "turn_type":"chat",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "do not run anything, just tell me a very short joke",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|value| value.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_trusts_explicit_none_recipe_for_skill_plan() {
        let raw = r#"{
          "resolved_user_intent":"只生成一个全新可复用技能方案，不执行、不启用。",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"纯设计交付物；execution_recipe.kind=none。",
          "confidence":0.92,
          "decision":"direct_answer",
          "output_contract":{
            "response_shape":"strict",
            "requires_content_evidence":false,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":"",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"none","profile":"skill_authoring","target_scope":"none"},
          "turn_type":"task_request",
          "target_task_policy":"standalone",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "请为一个 action=ping 的全新可复用能力生成技能方案，先不要执行，也不要启用。",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|value| value.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .pointer("/execution_recipe/kind")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/execution_recipe/profile")
                .and_then(|value| value.as_str()),
            Some("skill_authoring")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_resolved_intent_includes_answer_candidate_for_chat_stage() {
        let resolved = super::merge_answer_candidate_into_resolved_intent(
            "查询之前记住的测试编号".to_string(),
            "client-like-continuous-20260430_094246",
        );
        assert_eq!(
            resolved,
            "查询之前记住的测试编号\nanswer_candidate: client-like-continuous-20260430_094246"
        );
        assert_eq!(
            super::merge_answer_candidate_into_resolved_intent(
                resolved.clone(),
                "client-like-continuous-20260430_094246",
            ),
            resolved
        );
    }

    #[test]
    fn answer_candidate_binding_reports_memory_only_without_phrase_matching() {
        let request = "Return the marker value only.";
        let answer = "client-like-continuous-20260501_054730";
        let memory_only = crate::task_context_builder::RouteContextView {
            memory_context: format!("#### RELEVANT_FACTS\n- remembered marker {answer}"),
            recent_assistant_replies: "<none>".to_string(),
            recent_turns_full: "<none>".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_context: "<none>".to_string(),
            ..Default::default()
        };

        let report = super::analyze_answer_candidate_binding(request, answer, &memory_only)
            .expect("candidate should produce binding report");
        assert!(report.is_memory_only_binding());
        assert!(report.is_distinctive());
        assert!(!report.in_current_request);

        let recent_bound = crate::task_context_builder::RouteContextView {
            memory_context: memory_only.memory_context,
            recent_assistant_replies: format!("已记录。测试编号 `{answer}` 已记住。"),
            ..Default::default()
        };

        let report = super::analyze_answer_candidate_binding(request, answer, &recent_bound)
            .expect("candidate should produce binding report");
        assert!(!report.is_memory_only_binding());
        assert!(report.has_current_or_recent_binding());
    }

    #[test]
    fn answer_candidate_binding_context_is_structural_not_language_specific() {
        let request = "For this continuous test, remember marker RC-CONT-EN-0428-B. Reply with one short confirmation.";
        let answer = "client-like-continuous-20260501_054730";
        let route_view = crate::task_context_builder::RouteContextView {
            memory_context: format!("older marker {answer}"),
            ..Default::default()
        };

        let report = super::analyze_answer_candidate_binding(request, answer, &route_view)
            .expect("candidate should produce binding report");
        let context = super::answer_candidate_binding_repair_context(&report, true);
        assert!(context.contains("should_refresh_long_term_memory: true"));
        assert!(context.contains("memory_only_binding: true"));
        assert!(context.contains("distinctive_candidate: true"));
    }

    #[test]
    fn normalizer_schema_normalization_coerces_invalid_schedule_kind_to_none() {
        let raw = r#"{
          "resolved_user_intent":"修改方案目标用户为开发者，输出正文",
          "resume_behavior":"resume",
          "schedule_kind":"immediate",
          "schedule_intent":"deliver",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户修正目标受众约束并明确要求输出正文",
          "confidence":1.0,
          "decision":"direct_answer",
          "output_contract":{"kind":"text","text_content":"示例正文","media_type":"text/plain"},
          "execution_recipe":{"kind":"none"},
          "turn_type":"task_append",
          "target_task_policy":"reuse_active",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "不对，目标用户改成开发者，不是老板。只输出修正后的正文。",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("schedule_kind").and_then(|value| value.as_str()),
            Some("none")
        );
        assert!(value
            .get("schedule_intent")
            .is_some_and(|value| value.is_null()));
        assert_eq!(
            value.get("turn_type").and_then(|value| value.as_str()),
            Some("task_append")
        );
        assert_eq!(
            value
                .get("target_task_policy")
                .and_then(|value| value.as_str()),
            Some("reuse_active")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_minimax_text_only_append_payload() {
        let raw = r#"{
          "resolved_user_intent":"调整字数约束：RustClaw 连续会话可靠性技术博客风格，100字以内",
          "answer_candidate":"RustClaw 以多层容错与自动状态恢复机制，在网络抖动或进程异常时快速回到上一状态，保障关键业务链路不中断。",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"none",
          "needs_clarify":false,
          "clarify_question":"none",
          "reason":"字数约束更新，主题、受众、语气不变，直接输出精简版本",
          "confidence":"0.97",
          "decision":"direct_answer",
          "output_contract":"text_only",
          "execution_recipe":{"kind":"none","requires_content_evidence":false,"locator_kind":"none"},
          "turn_type":"task_append",
          "target_task_policy":"reuse_active",
          "should_interrupt_active_run":"no",
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "不对，不是 200 字，是 100 字以内",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("turn_type").and_then(|value| value.as_str()),
            Some("task_append")
        );
        assert_eq!(
            value
                .get("target_task_policy")
                .and_then(|value| value.as_str()),
            Some("reuse_active")
        );
        assert_eq!(
            value
                .get("should_interrupt_active_run")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|value| value.as_str()),
            Some("free")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_treats_one_line_comparison_as_strict_shape() {
        let raw = r#"{
          "resolved_user_intent":"比较两个字段并输出一行",
          "resume_behavior":null,
          "schedule_kind":"immediate",
          "schedule_intent":"read_and_compare",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"clear comparison",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":"one_line_comparison",
          "execution_recipe":{"kind":"file_read_two"},
          "turn_type":"task_request",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "读取两个字段，最后只用一行输出：前者、后者、一样或不一样",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .pointer("/output_contract/response_shape")
                .and_then(|value| value.as_str()),
            Some("strict")
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|value| value.as_str()),
            Some("recent_scalar_equality_check")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_rejects_invalid_resume_and_turn_tokens() {
        let raw = r#"{
          "resolved_user_intent":"用户希望在80字以内生成一份面向开发者的简短方案，缺少主题信息。",
          "resume_behavior":"unsupported_resume_token",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"assistant",
          "needs_clarify":true,
          "clarify_question":"请告诉我这份方案的主题是什么？",
          "reason":"缺少核心主题",
          "confidence":0.95,
          "decision":"direct_answer",
          "output_contract":"clarification",
          "execution_recipe":{"kind":"none"},
          "turn_type":"unsupported_turn_token",
          "target_task_policy":"reuse active",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "不对，目标用户改成开发者，不是老板。只输出修正后的正文。",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .get("resume_behavior")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        assert_eq!(
            value.get("turn_type").and_then(|value| value.as_str()),
            Some("")
        );
        assert_eq!(
            value
                .get("target_task_policy")
                .and_then(|value| value.as_str()),
            Some("reuse_active")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_coerces_empty_state_patch_string() {
        let raw = r#"{
          "resolved_user_intent":"用户询问刚才记住的测试编号",
          "resume_behavior":"",
          "schedule_kind":"",
          "schedule_intent":"",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"上下文中已有编号",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":"",
          "execution_recipe":{"kind":"none"},
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":"",
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "刚才让你记住的连续测试编号是什么？只回答编号。",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert!(value
            .get("state_patch")
            .is_some_and(|value| value.is_null()));
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_promotes_misnested_turn_analysis_fields() {
        let raw = r#"{
          "resolved_user_intent":"用一句中文确认当前测试正在进行",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"pure confirmation",
          "confidence":0.99,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"one_sentence"},
          "execution_recipe":{
            "kind":"none",
            "profile":"none",
            "target_scope":"none",
            "turn_type":"task_request",
            "target_task_policy":"standalone",
            "state_patch":{"constraints":{"tone":"brief"}},
            "attachment_processing_required":true
          },
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized =
            super::normalize_intent_normalizer_raw_for_schema(raw, "请用一句中文回复确认。");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("turn_type").and_then(|value| value.as_str()),
            Some("task_request")
        );
        assert_eq!(
            value
                .get("target_task_policy")
                .and_then(|value| value.as_str()),
            Some("standalone")
        );
        assert_eq!(
            value
                .pointer("/state_patch/constraints/tone")
                .and_then(|value| value.as_str()),
            Some("brief")
        );
        assert_eq!(
            value
                .get("attachment_processing_required")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert!(value
            .get("execution_recipe")
            .and_then(|value| value.as_object())
            .is_some_and(|recipe| !recipe.contains_key("turn_type")
                && !recipe.contains_key("target_task_policy")
                && !recipe.contains_key("state_patch")));
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_commands_payload_as_execution_signal() {
        let raw = r#"{
          "resolved_user_intent":"User wants to know approximate size of the target directory",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"User requested local directory size and provided a command payload.",
          "confidence":0.95,
          "decision":"direct_answer",
          "output_contract":"json",
          "execution_recipe":{"commands":[{"executor":"local","command":"du -sh target","purpose":"Get approximate size"}]},
          "turn_type":"task_request",
          "target_task_policy":"none",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
        let normalized =
            super::normalize_intent_normalizer_raw_for_schema(raw, "看一下 target 大概多大");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|value| value.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            value
                .pointer("/execution_recipe/kind")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_ignores_response_recipe_as_execution_signal() {
        let raw = r#"{
          "resolved_user_intent":"确认用户正在进行 RustClaw 真实客户端连续会话测试",
          "answer_candidate":"好的，我已确认你正在进行 RustClaw 的真实客户端连续会话测试。",
          "resume_behavior":null,
          "schedule_kind":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户明确要求用一句中文回复确认其正在进行 RustClaw 真实客户端连续会话测试",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"text","semantic_kind":"confirmation"},
          "execution_recipe":"respond_with_simple_chinese_confirmation",
          "turn_type":"greeting",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "你好，我正在做 RustClaw 的真实客户端连续会话测试，请用一句中文回复确认。",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|value| value.as_str()),
            Some("direct_answer")
        );
        assert_eq!(
            value
                .pointer("/output_contract/requires_content_evidence")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .pointer("/output_contract/semantic_kind")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        assert_eq!(
            value
                .pointer("/execution_recipe/kind")
                .and_then(|value| value.as_str()),
            Some("none")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn structured_preference_ack_is_detached_from_active_task() {
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        };
        assert!(super::should_detach_bare_acknowledgement_from_active_task(
            Some(TurnType::PreferenceOrMemory),
            Some(TargetTaskPolicy::ReuseActive),
            FirstLayerDecision::DirectAnswer,
            &contract,
            None,
            false,
        ));
        assert!(!super::should_detach_bare_acknowledgement_from_active_task(
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            FirstLayerDecision::DirectAnswer,
            &contract,
            None,
            false,
        ));
        assert!(!super::should_detach_bare_acknowledgement_from_active_task(
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            FirstLayerDecision::DirectAnswer,
            &contract,
            Some(&serde_json::json!({"output_refinement":"one_sentence_only"})),
            false,
        ));
    }

    #[test]
    fn orphan_output_shape_clarify_downgrades_to_standalone_chat() {
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        };
        let snapshot_without_primary = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState::default()),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(
            super::should_downgrade_orphan_output_shape_clarify_to_direct_answer(
                Some(&snapshot_without_primary),
                Some(TurnType::TaskAppend),
                Some(TargetTaskPolicy::ReuseActive),
                FirstLayerDecision::Clarify,
                &contract,
                None,
                false,
                false,
            )
        );

        let snapshot_with_primary = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我写个方案".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(
            !super::should_downgrade_orphan_output_shape_clarify_to_direct_answer(
                Some(&snapshot_with_primary),
                Some(TurnType::TaskAppend),
                Some(TargetTaskPolicy::ReuseActive),
                FirstLayerDecision::Clarify,
                &contract,
                None,
                false,
                false,
            )
        );
    }

    #[test]
    fn missing_turn_type_with_standalone_policy_infers_primary_task_request() {
        assert_eq!(
            super::infer_missing_turn_type_from_policy(
                None,
                Some(TargetTaskPolicy::Standalone),
                FirstLayerDecision::DirectAnswer,
                false,
                crate::ScheduleKind::None,
                false,
            ),
            Some(TurnType::TaskRequest)
        );
        assert_eq!(
            super::infer_missing_turn_type_from_policy(
                Some(TurnType::PreferenceOrMemory),
                Some(TargetTaskPolicy::Standalone),
                FirstLayerDecision::DirectAnswer,
                false,
                crate::ScheduleKind::None,
                true,
            ),
            Some(TurnType::PreferenceOrMemory)
        );
        assert_eq!(
            super::infer_missing_turn_type_from_policy(
                None,
                Some(TargetTaskPolicy::Standalone),
                FirstLayerDecision::Clarify,
                true,
                crate::ScheduleKind::None,
                false,
            ),
            None
        );
    }

    #[test]
    fn missing_turn_type_with_active_task_policy_infers_mutation_type() {
        assert_eq!(
            super::infer_missing_turn_type_from_policy(
                None,
                Some(TargetTaskPolicy::ReuseActive),
                FirstLayerDecision::DirectAnswer,
                false,
                crate::ScheduleKind::None,
                false,
            ),
            Some(TurnType::TaskAppend)
        );
        assert_eq!(
            super::infer_missing_turn_type_from_policy(
                None,
                Some(TargetTaskPolicy::ReplaceActive),
                FirstLayerDecision::DirectAnswer,
                false,
                crate::ScheduleKind::None,
                false,
            ),
            Some(TurnType::TaskReplace)
        );
        assert_eq!(
            super::infer_missing_turn_type_from_policy(
                None,
                Some(TargetTaskPolicy::ReuseActive),
                FirstLayerDecision::DirectAnswer,
                false,
                crate::ScheduleKind::None,
                true,
            ),
            None
        );
    }

    #[test]
    fn missing_policy_with_strict_chat_deliverable_infers_standalone_task() {
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        };
        let policy = super::infer_missing_target_policy_from_contract(
            None,
            None,
            FirstLayerDecision::DirectAnswer,
            false,
            crate::ScheduleKind::None,
            false,
            &contract,
        );
        assert_eq!(policy, Some(TargetTaskPolicy::Standalone));
        assert_eq!(
            super::infer_missing_turn_type_from_policy(
                None,
                policy,
                FirstLayerDecision::DirectAnswer,
                false,
                crate::ScheduleKind::None,
                false,
            ),
            Some(TurnType::TaskRequest)
        );
    }

    #[test]
    fn missing_policy_with_non_strict_chat_does_not_promote_generic_chat() {
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        };
        assert_eq!(
            super::infer_missing_target_policy_from_contract(
                None,
                None,
                FirstLayerDecision::DirectAnswer,
                false,
                crate::ScheduleKind::None,
                false,
                &contract,
            ),
            None
        );
    }

    #[test]
    fn empty_nested_state_patch_is_not_meaningful() {
        assert!(!super::is_meaningful_state_patch(&serde_json::json!({
            "alias_bindings": [],
            "notes": ""
        })));
        assert!(super::is_meaningful_state_patch(&serde_json::json!({
            "audience": "developers"
        })));
    }

    #[test]
    fn compact_normalizer_prompt_pins_output_contract_schema() {
        let route_view = crate::task_context_builder::RouteContextView {
            request_surface_hints: "locator_target_pair: Cargo.toml | Cargo.lock".to_string(),
            ..Default::default()
        };
        let context_bundle = crate::task_context_builder::TaskContextBundle {
            raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
            planner_view: crate::task_context_builder::PlannerContextView::default(),
            route_view: Some(route_view.clone()),
            execution_view: None,
        };
        let prompt = super::render_compact_intent_normalizer_prompt(
            &route_view,
            &context_bundle,
            "admin=true current_process_cwd=/home/guagua/rustclaw",
            "zh-CN",
            "list current toml files and briefly explain them",
        );

        assert!(prompt.contains("Allowed output_contract keys only"));
        assert!(prompt.contains("output_contract as a JSON object, never as a string token"));
        assert!(prompt.contains("Use ALIASES only for temporary references"));
        assert!(prompt.contains("ALIASES: <none>"));
        assert!(prompt.contains("CAPABILITIES:"));
        assert!(prompt
            .contains("Allowed response_shape: free, one_sentence, strict, scalar, file_token"));
        assert!(prompt.contains("Allowed semantic_kind: none, raw_command_output"));
        assert!(prompt.contains("semantic_kind=\"hidden_entries_check\""));
        assert!(prompt.contains("semantic_kind=\"existence_with_path\""));
        assert!(prompt.contains("file/path metadata comparisons"));
        assert!(prompt.contains("semantic_kind=\"quantity_comparison\""));
        assert!(prompt.contains("Text drafting/composition is not file delivery by default"));
        assert!(prompt.contains("Write a long article about RustClaw"));
        assert!(prompt.contains("presence judgment is not numeric counting"));
        assert!(prompt.contains("Do not emit exact_format, required_evidence, fields"));
        assert!(prompt.contains("instead of inventing enum values"));
        assert!(prompt.contains("Every enum field must be exactly one listed schema token"));
        assert!(prompt.contains("clarify is a decision, never a turn_type or resume_behavior"));
        assert!(prompt.contains("state_patch must be a JSON object or null"));
        assert!(prompt.contains("Use decision=\"planner_execute\" when the request inspects"));
        assert!(prompt.contains("Never ask the user to paste local file contents"));
        assert!(prompt.contains("Output exactly one raw JSON object and then stop"));
        assert!(prompt.contains("Always include all top-level schema keys"));
        assert!(prompt.contains("If ACTIVE_TASK is <none>, do not use task_append"));
        assert!(prompt.contains("turn_type=\"task_append\", target_task_policy=\"reuse_active\""));
        assert!(prompt.contains("never force planner_execute for a presentation-only constraint"));
        assert!(prompt.contains("Current REQUEST overrides RECENT/MEMORY"));
        assert!(prompt.contains("Do not import a prior directory/path scope"));
    }

    #[test]
    fn compact_prompt_slot_preserves_head_and_tail_when_truncated() {
        let value = format!(
            "project background: {}\nvalidation goal: continuous state memory context should remain usable",
            "long middle context ".repeat(80)
        );
        let slot = super::compact_prompt_slot("MEMORY", &value, 180);

        assert!(slot.contains("MEMORY: project background"));
        assert!(slot.contains("...<snip>..."));
        assert!(slot.contains("validation goal:"));
        assert!(slot.contains("state memory context"));
    }

    #[test]
    fn compact_normalizer_prompt_keeps_followup_anchor_next_to_request_tail() {
        let route_view = crate::task_context_builder::RouteContextView {
            active_execution_anchor_context:
                "### ACTIVE_EXECUTION_ANCHOR\nfollowup_source_request: list logs\nfollowup_ordered_entries: 1:act_plan.log | 2:clawd.log | 3:clawd.run.log"
                    .to_string(),
            memory_context: "older document list memory ".repeat(160),
            recent_assistant_replies: "older assistant document list ".repeat(160),
            ..Default::default()
        };
        let context_bundle = crate::task_context_builder::TaskContextBundle {
            raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
            planner_view: crate::task_context_builder::PlannerContextView::default(),
            route_view: Some(route_view.clone()),
            execution_view: None,
        };
        let prompt = super::render_compact_intent_normalizer_prompt(
            &route_view,
            &context_bundle,
            "admin=true current_process_cwd=/home/guagua/rustclaw",
            "zh-CN",
            "inspect the second item from the latest list",
        );
        let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1485);

        assert!(compact_tail.contains("FOLLOWUP_ANCHOR_PRIORITY"));
        assert!(compact_tail.contains("followup_ordered_entries"));
        assert!(compact_tail.contains("2:clawd.log"));
        assert!(compact_tail.contains("REQUEST: inspect the second item from the latest list"));
    }

    #[test]
    fn compact_normalizer_prompt_keeps_summary_recall_guard_in_head_and_tail() {
        let route_view = crate::task_context_builder::RouteContextView {
            recent_turns_full: "recent turn noise ".repeat(120),
            memory_context: "memory noise ".repeat(120),
            ..Default::default()
        };
        let context_bundle = crate::task_context_builder::TaskContextBundle {
            raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
            planner_view: crate::task_context_builder::PlannerContextView::default(),
            route_view: Some(route_view.clone()),
            execution_view: None,
        };
        let request = "请用一句话总结这个连续会话测试主要验证什么。";
        let prompt = super::render_compact_intent_normalizer_prompt(
            &route_view,
            &context_bundle,
            "admin=true current_process_cwd=/home/guagua/rustclaw",
            "zh-CN",
            request,
        );
        let compact_head = crate::providers::utf8_safe_prefix(&prompt, 1485);
        let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1485);

        assert!(compact_head.contains("High-priority"));
        assert!(compact_head.contains("mainly verifies or means"));
        assert!(compact_tail.contains("SUMMARY_RECALL"));
        assert!(compact_tail.contains(request));
    }

    #[test]
    fn compact_normalizer_prompt_tail_preserves_memory_recall_near_request() {
        let test_id = "client-like-continuous-20260430_134427";
        let route_view = crate::task_context_builder::RouteContextView {
            memory_context: format!("STABLE_FACTS: test number is {test_id}"),
            recent_turns_full: "recent turn noise ".repeat(120),
            last_turn_full: "last turn noise ".repeat(40),
            recent_assistant_replies: "assistant noise ".repeat(20),
            ..Default::default()
        };
        let context_bundle = crate::task_context_builder::TaskContextBundle {
            raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
            planner_view: crate::task_context_builder::PlannerContextView::default(),
            route_view: Some(route_view.clone()),
            execution_view: None,
        };
        let request = "刚才我让你记住的测试编号是什么？只回答编号。";
        let prompt = super::render_compact_intent_normalizer_prompt(
            &route_view,
            &context_bundle,
            "admin=true current_process_cwd=/home/guagua/rustclaw",
            "zh-CN",
            request,
        );
        let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1485);

        assert!(compact_tail.contains(test_id));
        assert!(compact_tail.contains(request));
        assert!(compact_tail.find("MEMORY:").is_some_and(|memory_idx| {
            compact_tail
                .find("REQUEST:")
                .is_some_and(|request_idx| memory_idx < request_idx)
        }));
    }

    #[test]
    fn compact_normalizer_prompt_tail_keeps_assistant_scalar_and_marks_scores_metadata() {
        let test_id = "client-like-continuous-20260430_174102";
        let route_view = crate::task_context_builder::RouteContextView {
            memory_context: "### MEMORY_CONTEXT\n#### RECENT_RELATED_EVENTS\n- 0.55 user asked to remember a long context\n- 0.70 unrelated relevance score".to_string(),
            recent_assistant_replies: format!(
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] relative_index=-1 short_preview=已收到 has_code_block=false\n- turn_id=assistant[-2] relative_index=-2 short_preview=已记录。测试编号 `{test_id}` 已记住，后续询问时可直接使用。 has_code_block=false"
            ),
            recent_turns_full: "recent turn noise ".repeat(120),
            last_turn_full: "last turn noise ".repeat(40),
            ..Default::default()
        };
        let context_bundle = crate::task_context_builder::TaskContextBundle {
            raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
            planner_view: crate::task_context_builder::PlannerContextView::default(),
            route_view: Some(route_view.clone()),
            execution_view: None,
        };
        let request = "刚才我让你记住的测试编号是什么？只回答编号。";
        let prompt = super::render_compact_intent_normalizer_prompt(
            &route_view,
            &context_bundle,
            "admin=true current_process_cwd=/home/guagua/rustclaw",
            "zh-CN",
            request,
        );
        let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1815);

        assert!(compact_tail.contains("memory scores are metadata"));
        assert!(compact_tail.contains("ASSISTANT:"));
        assert!(compact_tail.contains(test_id));
        assert!(compact_tail.contains(request));
        assert!(compact_tail.find("MEMORY:").is_some_and(|memory_idx| {
            compact_tail
                .find("ASSISTANT:")
                .is_some_and(|assistant_idx| memory_idx < assistant_idx)
        }));
        assert!(compact_tail
            .find("ASSISTANT:")
            .is_some_and(|assistant_idx| {
                compact_tail
                    .find("REQUEST:")
                    .is_some_and(|request_idx| assistant_idx < request_idx)
            }));
    }

    #[test]
    fn compact_normalizer_prompt_tail_preserves_long_memory_goal_near_request() {
        let goal = "validation goal: continuous messages should keep recent turns, memory context, and clarification state usable";
        let route_view = crate::task_context_builder::RouteContextView {
            memory_context: format!(
                "project background: {}\n{goal}",
                "multi-channel agent console context ".repeat(80)
            ),
            recent_turns_full: "recent turn noise ".repeat(120),
            last_turn_full: "last turn noise ".repeat(40),
            recent_assistant_replies: "assistant noise ".repeat(20),
            ..Default::default()
        };
        let context_bundle = crate::task_context_builder::TaskContextBundle {
            raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
            planner_view: crate::task_context_builder::PlannerContextView::default(),
            route_view: Some(route_view.clone()),
            execution_view: None,
        };
        let request = "Please summarize what this continuous conversation test validates.";
        let prompt = super::render_compact_intent_normalizer_prompt(
            &route_view,
            &context_bundle,
            "admin=true current_process_cwd=/home/guagua/rustclaw",
            "en",
            request,
        );
        let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1700);

        assert!(compact_tail.contains("MEMORY:"));
        assert!(compact_tail.contains("validation goal:"));
        assert!(compact_tail.contains("clarification state usable"));
        assert!(compact_tail.contains(request));
    }

    #[test]
    fn compact_normalizer_prompt_tail_preserves_runtime_context_near_request() {
        let route_view = crate::task_context_builder::RouteContextView {
            recent_turns_full: "recent turn noise ".repeat(120),
            last_turn_full: "last turn noise ".repeat(40),
            recent_assistant_replies: "assistant noise ".repeat(20),
            memory_context: "memory noise ".repeat(40),
            ..Default::default()
        };
        let runtime_context = "### RUNTIME_CONTEXT\ncurrent_process_cwd: /tmp/rustclaw-workspace\nworkspace_root: /tmp/rustclaw-workspace";
        let context_bundle = crate::task_context_builder::TaskContextBundle {
            raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
            planner_view: crate::task_context_builder::PlannerContextView::default(),
            route_view: Some(route_view.clone()),
            execution_view: Some(crate::task_context_builder::ExecutionContextView {
                budget_tier: crate::task_context_builder::ExecutionContextBudgetTier::Full,
                memory_ctx: crate::memory::service::PromptMemoryContext {
                    prompt_with_memory: String::new(),
                    chat_prompt_context: String::new(),
                    long_term_summary: None,
                    preferences: Vec::new(),
                    recalled: Vec::new(),
                    similar_triggers: Vec::new(),
                    relevant_facts: Vec::new(),
                    recent_related_events: Vec::new(),
                },
                runtime_context: runtime_context.to_string(),
                session_alias_context: "<none>".to_string(),
                recent_turns_full: "<none>".to_string(),
                last_turn_full: "<none>".to_string(),
                recent_execution_anchor: "<none>".to_string(),
                recent_execution_context: "<none>".to_string(),
                image_context: None,
            }),
        };
        let request = "只输出当前工作目录的绝对路径，不要解释";
        let prompt = super::render_compact_intent_normalizer_prompt(
            &route_view,
            &context_bundle,
            "admin=true",
            "zh-CN",
            request,
        );
        let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1700);

        assert!(prompt.contains("CONTRACT: output_contract must be a JSON object"));
        assert!(compact_tail.contains("LOCAL_EXEC"));
        assert!(compact_tail.contains("no cannot-access-FS reply"));
        assert!(compact_tail.contains("RUNTIME:"));
        assert!(compact_tail.contains("current_process_cwd: /tmp/rustclaw-workspace"));
        assert!(compact_tail.contains("workspace_root: /tmp/rustclaw-workspace"));
        assert!(compact_tail.contains(request));
        assert!(compact_tail.find("RUNTIME:").is_some_and(|runtime_idx| {
            compact_tail
                .find("REQUEST:")
                .is_some_and(|request_idx| runtime_idx < request_idx)
        }));
    }

    #[test]
    fn compact_normalizer_prompt_falls_back_to_auth_runtime_context() {
        let route_view = crate::task_context_builder::RouteContextView {
            recent_turns_full: "recent turn noise ".repeat(120),
            memory_context: "memory noise ".repeat(40),
            ..Default::default()
        };
        let context_bundle = crate::task_context_builder::TaskContextBundle {
            raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
            planner_view: crate::task_context_builder::PlannerContextView::default(),
            route_view: Some(route_view.clone()),
            execution_view: None,
        };
        let request = "只输出当前工作目录的绝对路径，不要解释";
        let prompt = super::render_compact_intent_normalizer_prompt(
            &route_view,
            &context_bundle,
            "current_auth_role: admin\nallow_path_outside_workspace_for_task: true\nworkspace_root: /home/guagua/rustclaw\ncurrent_process_cwd: /home/guagua/rustclaw",
            "zh-CN",
            request,
        );
        let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1700);

        assert!(compact_tail.contains("RUNTIME:"));
        assert!(compact_tail.contains("current_process_cwd: /home/guagua/rustclaw"));
        assert!(compact_tail.contains("workspace_root: /home/guagua/rustclaw"));
        assert!(!compact_tail.contains("RUNTIME: <none>"));
        assert!(compact_tail.contains(request));
    }

    #[test]
    fn normalizer_schema_normalization_coerces_hidden_files_check_synonym() {
        let raw = r#"{
          "resolved_user_intent": "检查当前目录是否存在隐藏文件并提供3个示例",
          "needs_clarify": false,
          "reason": "local hidden entries check",
          "confidence": 1.0,
          "decision":"planner_execute",
          "output_contract": {
            "response_shape": "object",
            "semantic_kind": "hidden_files_check"
          }
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("semantic_kind").and_then(|v| v.as_str()),
            Some("hidden_entries_check")
        );
        assert_eq!(
            contract.get("response_shape").and_then(|v| v.as_str()),
            Some("strict")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_planner_decision_and_filename_listing_contract() {
        let raw = r#"{
          "resolved_user_intent":"List first 10 filenames in logs directory without reading content",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"directory listing action",
          "confidence":0.98,
          "decision":"planner_execute",
          "output_contract":"filename_listing",
          "execution_recipe":{"bash":"ls -1 logs/ | head -10"}
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 logs 目录下的前 10 个文件名，不要读内容",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value.get("answer_candidate").and_then(|v| v.as_str()),
            Some("")
        );
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("semantic_kind").and_then(|v| v.as_str()),
            Some("file_names")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_files_listing_contract_from_chat_drift() {
        let raw = r#"{
          "resolved_user_intent":"列出 /home/guagua/rustclaw/logs 目录下的前 10 个文件名，仅文件名，不读取文件内容",
          "answer_candidate":"",
          "resume_behavior":"proceed",
          "schedule_kind":"immediate",
          "schedule_intent":"list_directory",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"directory listing action",
          "confidence":1.0,
          "decision":"direct_answer",
          "output_contract":"files_listing",
          "execution_recipe":"ls -1 /home/guagua/rustclaw/logs | head -n 10",
          "turn_type":"request",
          "target_task_policy":"list_directory",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 logs 目录下的前 10 个文件名，不要读内容",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("answer_candidate").and_then(|v| v.as_str()),
            Some("")
        );
        let validated = crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation")
        .value;
        let contract = super::parse_output_contract(validated.output_contract, false);
        assert_eq!(contract.semantic_kind, crate::OutputSemanticKind::FileNames);
        assert!(contract.requires_content_evidence);
        assert_eq!(
            super::parse_first_layer_decision_text(&validated.decision),
            Some(crate::FirstLayerDecision::PlannerExecute)
        );
    }

    #[test]
    fn executable_unknown_scalar_output_contract_triggers_semantic_repair_not_answer_candidate() {
        let raw = r#"{
          "resolved_user_intent":"查找当前仓库里所有 sh 脚本所在的目录，去重后列出来",
          "answer_candidate":null,
          "resume_behavior":"none",
          "schedule_kind":"immediate",
          "schedule_intent":"find_sh_scripts_directories",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"file_finder",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户请求在当前仓库中搜索脚本文件，提取其所在目录并去重列出。这是明确且可执行的本地 FS 搜索任务。",
          "confidence":0.98,
          "decision":"planner_execute",
          "decision":"planner_execute",
          "output_contract":"structured_exec_state",
          "execution_recipe":"find /workspace -name \"*.sh\" -type f -exec dirname {} \\; | sort -u",
          "turn_type":"exec",
          "target_task_policy":"local_fs_search",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
        let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
            raw,
            "查找当前仓库里所有 sh 脚本所在的目录，去重后列出来",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value
                .get("answer_candidate")
                .and_then(|value| value.as_str()),
            Some("")
        );
        assert!(report
            .details
            .contains("executable_route_unknown_scalar_output_contract"));
        assert!(report.needs_llm_semantic_repair());
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_recovers_detection_payload_as_planner_execute() {
        let raw = r#"{
          "resolved_user_intent":"检查仓库中是否存在 rustclaw.service 文件，只回答有或没有，并给出完整路径",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"repo existence check",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":{"response_shape":"strict","semantic_kind":"existence_with_path","requires_content_evidence":true}
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_preserves_recognized_execution_recipe_signal() {
        let raw = r#"{
          "resolved_user_intent":"列出 document 目录下所有文件的文件名列表",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"directory filename listing",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":{"type":"list","items":{"type":"string"}},
          "execution_recipe":{"kind":"ops_closed_loop","profile":"ops_service","target_scope":"current_repo"}
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 document 目录下有哪些文件，只输出文件名列表",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("response_shape").and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            contract
                .get("requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_treat_target_scope_as_execution() {
        let raw = r#"{
          "resolved_user_intent":"简单解释一下这个项目是什么",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"chat explanation",
          "confidence":0.88,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"free"},
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"current_repo"}
        }"#;
        let normalized =
            super::normalize_intent_normalizer_raw_for_schema(raw, "简单解释一下这个项目是什么");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract
                .get("requires_content_evidence")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn structural_contract_repair_routes_file_field_scalar_to_evidence() {
        let req = "读取 Cargo.toml 的 package.name，只输出值";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            ..IntentOutputContract::default()
        };
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root");
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            workspace_root,
            FirstLayerDecision::PlannerExecute,
            "",
            None,
            None,
        );

        assert_eq!(reason, Some("structured_file_scalar_repair"));
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
        assert_eq!(contract.locator_hint, "Cargo.toml");
    }

    #[test]
    fn structural_contract_repair_keeps_workspace_summary_on_workspace_root_name() {
        let req = "把 RustClaw 当成当前项目来介绍，先查证 README 和 Cargo.toml";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            locator_hint: "RustClaw".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        };
        let workspace_root = std::path::Path::new("/tmp/rustclaw");
        let _ = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            workspace_root,
            FirstLayerDecision::PlannerExecute,
            "",
            None,
            None,
        );

        assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
        assert_eq!(contract.locator_hint, "RustClaw");
    }

    #[test]
    fn structural_contract_repair_preserves_chat_workspace_name_without_evidence() {
        let req = "用一句话介绍 RustClaw 是什么，不要查询文件";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: Some(1),
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        };
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new("/tmp/rustclaw"),
            FirstLayerDecision::DirectAnswer,
            "RustClaw 是一个面向自然语言自动化的本地 agent 项目。",
            None,
            None,
        );

        assert_eq!(reason, None);
        assert!(!contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
        assert!(contract.locator_hint.is_empty());
    }

    #[test]
    fn structural_contract_repair_preserves_file_path_only_delivery() {
        let req =
            "Run pwd, write one short line based on it into pwd_line.txt, and output only the file path.";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: "pwd_line.txt".to_string(),
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root");
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            workspace_root,
            FirstLayerDecision::PlannerExecute,
            "",
            None,
            None,
        );

        assert_eq!(reason, Some("scalar_locator_requires_evidence"));
        assert_eq!(contract.semantic_kind, OutputSemanticKind::ScalarPathOnly);
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
        assert_eq!(contract.locator_hint, "pwd_line.txt");
    }

    #[test]
    fn semantic_contract_repair_ignores_invented_answer_candidate_for_observation() {
        let req = "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            ..IntentOutputContract::default()
        };
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            FirstLayerDecision::PlannerExecute,
            "没有 (路径未找到)",
            None,
            None,
        );

        assert_eq!(reason, Some("semantic_contract_requires_evidence"));
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
        assert_eq!(contract.locator_hint, "rustclaw.service");
    }

    #[test]
    fn scalar_file_contract_repair_ignores_invented_answer_candidate() {
        let req = "读取 package.json 里的 name 字段，只输出值";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            ..IntentOutputContract::default()
        };
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            FirstLayerDecision::PlannerExecute,
            "rustclaw",
            None,
            None,
        );

        assert_eq!(reason, Some("scalar_locator_requires_evidence"));
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
        assert_eq!(contract.locator_hint, "package.json");
    }

    #[test]
    fn planner_locator_contract_repair_requires_evidence_for_sparse_contract() {
        let req = "Read configs/config.toml and output the selected_vendor field and value";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            ..IntentOutputContract::default()
        };
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            FirstLayerDecision::PlannerExecute,
            "",
            None,
            None,
        );

        assert_eq!(reason, Some("planner_locator_requires_evidence"));
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
        assert_eq!(contract.locator_hint, "configs/config.toml");
    }

    #[test]
    fn scalar_direct_answer_candidate_is_not_promoted_by_filename_like_text() {
        let req = "Literal text: app.log, answer only acknowledged.";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            ..IntentOutputContract::default()
        };
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            FirstLayerDecision::DirectAnswer,
            "acknowledged",
            None,
            None,
        );

        assert_eq!(reason, None);
        assert!(!contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert!(contract.locator_hint.is_empty());
    }

    #[test]
    fn answer_candidate_existing_relative_path_routes_to_evidence() {
        let workspace = make_temp_workspace_with_child("answer_candidate_path", "docs");
        let relative = "docs/report.md";
        std::fs::write(workspace.join(relative), "report").expect("write report");
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            ..IntentOutputContract::default()
        };
        let mut decision = FirstLayerDecision::DirectAnswer;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;

        let reason = super::apply_answer_candidate_path_evidence_repair(
            &mut contract,
            &format!("`{relative}`"),
            None,
            &workspace,
            false,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, Some("answer_candidate_path_requires_evidence"));
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
        assert_eq!(contract.locator_hint, relative);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::ScalarPathOnly);
        std::fs::remove_dir_all(workspace).ok();
    }

    #[test]
    fn answer_candidate_existing_bare_filename_stays_chat_without_evidence_repair() {
        let workspace = make_temp_workspace_with_child("answer_candidate_bare_filename", "docs");
        std::fs::write(workspace.join("README.md"), "readme").expect("write readme");
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            ..IntentOutputContract::default()
        };
        let mut decision = FirstLayerDecision::DirectAnswer;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;

        let reason = super::apply_answer_candidate_path_evidence_repair(
            &mut contract,
            "README.md",
            None,
            &workspace,
            false,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, None);
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert!(!contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
        std::fs::remove_dir_all(workspace).ok();
    }

    #[test]
    fn answer_candidate_plain_scalar_stays_chat_without_evidence() {
        let workspace = make_temp_workspace_with_child("answer_candidate_scalar", "docs");
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            ..IntentOutputContract::default()
        };
        let mut decision = FirstLayerDecision::DirectAnswer;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;

        let reason = super::apply_answer_candidate_path_evidence_repair(
            &mut contract,
            "my_abcd",
            None,
            &workspace,
            false,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, None);
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert!(!contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        std::fs::remove_dir_all(workspace).ok();
    }

    #[test]
    fn deictic_filename_answer_candidate_stays_chat_without_path_evidence_repair() {
        let workspace = make_temp_workspace_with_child("answer_candidate_deictic", "docs");
        std::fs::write(workspace.join("README.md"), "readme").expect("write readme");
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            ..IntentOutputContract::default()
        };
        let mut decision = FirstLayerDecision::DirectAnswer;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let patch = serde_json::json!({"output_format":"filename_only"});

        let reason = super::apply_answer_candidate_path_evidence_repair(
            &mut contract,
            "README.md",
            Some(&patch),
            &workspace,
            false,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, None);
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert!(!contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
        std::fs::remove_dir_all(workspace).ok();
    }

    #[test]
    fn structural_contract_repair_does_not_bind_workspace_child_mentions() {
        let workspace_root = make_temp_workspace_with_child("workspace_child_mentions", "document");
        let req = "列出document目录下有哪些文件，只输出文件名列表";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            &workspace_root,
            FirstLayerDecision::PlannerExecute,
            "",
            None,
            None,
        );

        assert_eq!(reason, None);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert!(contract.locator_hint.is_empty());
        std::fs::remove_dir_all(workspace_root).ok();
    }

    #[test]
    fn structural_contract_repair_does_not_bind_case_mismatched_product_name() {
        let workspace_root =
            make_temp_workspace_with_child("workspace_child_product_name", "rustclaw");
        let req = "你好，我正在做 RustClaw 的真实客户端连续会话测试，请用一句中文回复确认。";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };
        let reason = super::apply_current_turn_structural_contract_repair(
            &mut contract,
            req,
            &surface,
            &workspace_root,
            FirstLayerDecision::DirectAnswer,
            "",
            None,
            None,
        );

        assert_eq!(reason, None);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert!(contract.locator_hint.is_empty());
        std::fs::remove_dir_all(workspace_root).ok();
    }

    #[test]
    fn executionless_chat_wrapped_execute_is_downgraded_to_direct_answer() {
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            ..IntentOutputContract::default()
        };

        let reason = super::downgrade_executionless_route_to_direct_answer(
            &mut decision,
            &mut finalize_style,
            false,
            &contract,
            false,
            crate::ScheduleKind::None,
            None,
        );

        assert_eq!(
            reason,
            Some("executionless_route_downgraded_to_direct_answer")
        );
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    }

    #[test]
    fn plain_execute_is_not_downgraded_when_contract_is_sparse() {
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            ..IntentOutputContract::default()
        };

        let reason = super::downgrade_executionless_route_to_direct_answer(
            &mut decision,
            &mut finalize_style,
            false,
            &contract,
            false,
            crate::ScheduleKind::None,
            None,
        );

        assert_eq!(reason, None);
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    }

    #[test]
    fn execution_signal_act_route_stays_executable() {
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            semantic_kind: OutputSemanticKind::FileNames,
            ..IntentOutputContract::default()
        };

        let reason = super::downgrade_executionless_route_to_direct_answer(
            &mut decision,
            &mut finalize_style,
            false,
            &contract,
            false,
            crate::ScheduleKind::None,
            None,
        );

        assert_eq!(reason, None);
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    }

    #[test]
    fn structured_observation_clarify_repair_routes_concrete_file_request_to_act() {
        let req = "读取 package.json 里的 name 字段，只输出值";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            ..IntentOutputContract::default()
        };
        let mut needs_clarify = true;
        let mut clarify_question = "请提供 package.json 文件内容".to_string();
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let reason = super::apply_spurious_structured_observation_clarify_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            None,
            &mut needs_clarify,
            &mut clarify_question,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, Some("structured_observation_clarify_repair"));
        assert!(!needs_clarify);
        assert!(clarify_question.is_empty());
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
        assert_eq!(contract.locator_hint, "package.json");
    }

    #[test]
    fn structured_observation_clarify_repair_fills_file_delivery_filename_locator() {
        let req = "把 definitely_missing_named_file_phase0_runtime_20260515.txt 发给我";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::FileToken,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        };
        let mut needs_clarify = true;
        let mut clarify_question =
            "请提供 definitely_missing_named_file_phase0_runtime_20260515.txt 的完整路径"
                .to_string();
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let reason = super::apply_spurious_structured_observation_clarify_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            None,
            &mut needs_clarify,
            &mut clarify_question,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, Some("structured_observation_clarify_repair"));
        assert!(!needs_clarify);
        assert!(clarify_question.is_empty());
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
        assert_eq!(
            contract.locator_hint,
            "definitely_missing_named_file_phase0_runtime_20260515.txt"
        );
        assert!(contract.requires_content_evidence);
    }

    #[test]
    fn structured_observation_clarify_repair_routes_multi_filename_request_to_workspace_act() {
        let req = "检查 README.md, README.zh-CN.md, Cargo.toml, and no_such_file_20260513.txt 是否存在，用表格返回";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root");
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            ..IntentOutputContract::default()
        };
        let mut needs_clarify = true;
        let mut clarify_question = "请提供具体的文件夹路径".to_string();
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let reason = super::apply_spurious_structured_observation_clarify_repair(
            &mut contract,
            req,
            &surface,
            workspace_root,
            None,
            &mut needs_clarify,
            &mut clarify_question,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, Some("structured_observation_clarify_repair"));
        assert!(!needs_clarify);
        assert!(clarify_question.is_empty());
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
        assert_eq!(contract.locator_hint, workspace_root.display().to_string());
    }

    #[test]
    fn structured_observation_clarify_repair_routes_two_explicit_targets_to_act() {
        let req = "比较 configs/skills_registry.toml 和 docker/config/skills_registry.toml 哪个文件更大，只回答文件名和大小差";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root");
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            ..IntentOutputContract::default()
        };
        let mut needs_clarify = true;
        let mut clarify_question = "您希望我执行文件大小比较操作吗？".to_string();
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let reason = super::apply_spurious_structured_observation_clarify_repair(
            &mut contract,
            req,
            &surface,
            workspace_root,
            None,
            &mut needs_clarify,
            &mut clarify_question,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, Some("structured_observation_clarify_repair"));
        assert!(!needs_clarify);
        assert!(clarify_question.is_empty());
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
        assert_eq!(
            contract.locator_hint,
            "configs/skills_registry.toml, docker/config/skills_registry.toml"
        );
    }

    #[test]
    fn structured_observation_clarify_repair_preserves_named_target_without_clean_locator() {
        let req = "读一下 README 然后用恰好三句话总结，不要多也不要少";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            ..IntentOutputContract::default()
        };
        let mut needs_clarify = true;
        let mut clarify_question = "请提供 README 的具体内容或文件路径".to_string();
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let reason = super::apply_spurious_structured_observation_clarify_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            None,
            &mut needs_clarify,
            &mut clarify_question,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, None);
        assert!(needs_clarify);
        assert_eq!(clarify_question, "请提供 README 的具体内容或文件路径");
        assert_eq!(decision, FirstLayerDecision::Clarify);
        assert!(!contract.requires_content_evidence);
    }

    #[test]
    fn structured_observation_clarify_repair_preserves_deictic_bare_target_clarify() {
        let req = "读一下那个 README 开头并用一句话总结";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            ..IntentOutputContract::default()
        };
        let mut needs_clarify = true;
        let mut clarify_question = "请确认具体 README 路径".to_string();
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let patch = serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}});

        let reason = super::apply_spurious_structured_observation_clarify_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            Some(&patch),
            &mut needs_clarify,
            &mut clarify_question,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, None);
        assert!(needs_clarify);
        assert_eq!(clarify_question, "请确认具体 README 路径");
        assert_eq!(decision, FirstLayerDecision::Clarify);
        assert!(!contract.requires_content_evidence);
    }

    #[test]
    fn structured_observation_clarify_repair_preserves_version_correction_clarify() {
        let req = "Correction: mention Python 3.11, not Python 3.10.";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            ..IntentOutputContract::default()
        };
        let mut needs_clarify = true;
        let mut clarify_question = "请确认要修正哪段内容".to_string();
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;

        let reason = super::apply_spurious_structured_observation_clarify_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            None,
            &mut needs_clarify,
            &mut clarify_question,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, None);
        assert!(needs_clarify);
        assert_eq!(clarify_question, "请确认要修正哪段内容");
        assert_eq!(decision, FirstLayerDecision::Clarify);
        assert!(!contract.requires_content_evidence);
    }

    #[test]
    fn structured_observation_clarify_repair_preserves_deictic_with_destination_path_clarify() {
        let req = "把那个压缩包解压到 /tmp/unpack_dest 然后告诉我结果";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "/tmp/unpack_dest".to_string(),
            ..IntentOutputContract::default()
        };
        let mut needs_clarify = true;
        let mut clarify_question = "请提供压缩包路径".to_string();
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let patch = serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}});

        let reason = super::apply_spurious_structured_observation_clarify_repair(
            &mut contract,
            req,
            &surface,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root"),
            Some(&patch),
            &mut needs_clarify,
            &mut clarify_question,
            &mut decision,
            &mut finalize_style,
        );

        assert_eq!(reason, None);
        assert!(needs_clarify);
        assert_eq!(clarify_question, "请提供压缩包路径");
        assert_eq!(decision, FirstLayerDecision::Clarify);
        assert!(!contract.requires_content_evidence);
    }

    #[test]
    fn current_turn_locator_sanitizer_drops_contextual_path_prefix() {
        let req = "读一下 README 然后用恰好三句话总结，不要多也不要少";
        let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
        let cleaned = super::sanitize_resolved_intent_for_current_turn_locator(
            "读取 document 目录下的 README.md 文件内容并用恰好三句话进行总结",
            req,
            &surface,
        );

        assert_eq!(cleaned.as_deref(), Some(req));
    }

    fn make_temp_workspace_with_child(test_name: &str, child_name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rustclaw_intent_router_{test_name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join(child_name)).expect("create child directory");
        root
    }

    #[test]
    fn normalizer_schema_normalization_does_not_invent_contract_from_surface() {
        let raw = r#"{
          "resolved_user_intent": "检查当前目录是否有隐藏文件，如有则列出3个例子",
          "needs_clarify": false,
          "reason": "local hidden entries check",
          "confidence": 0.98,
          "decision":"planner_execute"
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("semantic_kind").and_then(|v| v.as_str()),
            Some("none")
        );
        assert_eq!(
            contract.get("response_shape").and_then(|v| v.as_str()),
            Some("free")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_extracts_fenced_json() {
        let raw = r#"```json
{
  "resolved_user_intent": "检查当前目录有没有隐藏文件，只回答有或没有，并补3个例子",
  "needs_clarify": false,
  "reason": "local hidden entries check",
  "confidence": 0.95,
  "decision":"planner_execute",
  "output_contract": {
    "response_shape": "scalar",
    "requires_content_evidence": true,
    "semantic_kind": "hidden_files_example",
    "locator_kind": "current_workspace"
  }
}
```"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("semantic_kind").and_then(|v| v.as_str()),
            Some("hidden_entries_check")
        );
        assert_eq!(
            contract.get("response_shape").and_then(|v| v.as_str()),
            Some("scalar")
        );
        assert_eq!(
            contract.get("locator_kind").and_then(|v| v.as_str()),
            Some("current_workspace")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_preserves_act_when_shape_is_descriptive() {
        let raw = r#"{
          "resolved_user_intent": "列出 logs 目录下前 10 个文件名，不读取内容",
          "needs_clarify": false,
          "reason": "workspace directory listing",
          "confidence": 0.9,
          "decision":"planner_execute",
          "output_contract": {
            "response_shape": "list_of_strings",
            "semantic_kind": "file_names"
          },
          "action": {"tool":"list_directory","path":"logs","limit":10}
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "列出 logs 目录下的前 10 个文件名",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("decision").and_then(|v| v.as_str()),
            Some("planner_execute")
        );
        assert_eq!(
            value.get("needs_clarify").and_then(|v| v.as_bool()),
            Some(false)
        );
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("response_shape").and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            contract.get("semantic_kind").and_then(|v| v.as_str()),
            Some("file_names")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_coerces_output_contract_scalar_and_aliases() {
        let raw = r#"{
          "resolved_user_intent": "列出 README.md 和 AGENTS.md，只输出文件名",
          "needs_clarify": false,
          "reason": "names-only inventory",
          "confidence": 0.9,
          "decision":"planner_execute",
          "output_contract": "file_names"
        }"#;
        let normalized =
            super::normalize_intent_normalizer_raw_for_schema(raw, "列出文件，只输出文件名");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("semantic_kind").and_then(|v| v.as_str()),
            Some("file_names")
        );

        let raw = r#"{
          "resolved_user_intent": "严格输出两行",
          "needs_clarify": false,
          "reason": "exact output",
          "confidence": 0.9,
          "decision":"direct_answer",
          "output_contract": {"shape":"exact_format","semantic":"sqlite_table_names"}
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "严格输出两行");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("response_shape").and_then(|v| v.as_str()),
            Some("strict")
        );
        assert_eq!(
            contract.get("semantic_kind").and_then(|v| v.as_str()),
            Some("sqlite_table_names_only")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_maps_file_type_filename_to_locator_hint() {
        let raw = r#"{
          "resolved_user_intent": "User wants to receive the file from the current workspace.",
          "needs_clarify": false,
          "reason": "file delivery",
          "confidence": 0.95,
          "decision": "planner_execute",
          "wants_file_delivery": true,
          "output_contract": {
            "type": "file",
            "filename": "definitely_missing_named_file_phase0_runtime_20260515.txt"
          }
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "把 definitely_missing_named_file_phase0_runtime_20260515.txt 发给我",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        let contract = value
            .get("output_contract")
            .and_then(|value| value.as_object())
            .expect("output contract");
        assert_eq!(
            contract.get("response_shape").and_then(|v| v.as_str()),
            Some("file_token")
        );
        assert_eq!(
            contract.get("delivery_required").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            contract.get("locator_kind").and_then(|v| v.as_str()),
            Some("filename")
        );
        assert_eq!(
            contract.get("locator_hint").and_then(|v| v.as_str()),
            Some("definitely_missing_named_file_phase0_runtime_20260515.txt")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn safe_fallback_tries_llm_except_when_model_unavailable() {
        assert!(!super::safe_fallback_source_should_try_llm(
            crate::fallback::ClarifyFallbackSource::LlmUnavailable
        ));
        assert!(super::safe_fallback_source_should_try_llm(
            crate::fallback::ClarifyFallbackSource::IntentUnresolved
        ));
        assert!(super::safe_fallback_source_should_try_llm(
            crate::fallback::ClarifyFallbackSource::SynthesisEmpty
        ));
    }

    #[test]
    fn parse_execution_recipe_hint_missing_profile_falls_back_to_default_spec() {
        // 历史语义：profile 缺失 → None（曾让下游 fallback 到 keyword detect）
        // B1 修复后：normalizer 显式回了 execution_recipe 字段（即使 profile 缺）就视为
        // 已分类，返回 default spec（kind=None, inactive），不再触发本地补判。
        // 这样可以避免 legacy local detector 因 STABLE_FACTS 污染而误升级 read-only 任务。
        let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
            kind: "ops_closed_loop".to_string(),
            profile: String::new(),
            target_scope: "current_repo".to_string(),
        }))
        .expect("normalizer-classified hint should yield Some, even with missing profile");
        assert_eq!(spec.kind, ExecutionRecipeKind::None);
    }

    #[test]
    fn parse_execution_recipe_hint_explicit_none_is_trusted() {
        // 这是修复 B1 的核心回归测试。
        // 场景：normalizer 已经基于完整上下文判定"这不是 ops loop"（kind=none）。
        // 期望：返回 Some(default spec) → initial_execution_recipe_spec 用 default spec
        // → runtime.is_active()=false → plan_repair_reason 不会触发
        // ops_closed_loop_apply_requires_mutation。
        // 历史风险：返回 None 会让下游 fallback 到 keyword 启发式；
        // 长期记忆里残留的 "configs/" "verify" 关键字会把任务误升级为
        // OpsClosedLoop config_change，让 read-only 的 `pwd` 任务跑挂。
        let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
        }))
        .expect("explicit kind=none should still be Some so local fallback remains bypassed");
        assert_eq!(spec.kind, ExecutionRecipeKind::None);
        assert!(
            !crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(spec).is_active(),
            "default spec must produce an inactive runtime state"
        );
    }

    #[test]
    fn parse_execution_recipe_hint_missing_field_leaves_no_recipe_hint() {
        // 当 normalizer 完全没在响应里给出 execution_recipe 字段时（None），
        // 只表示 LLM 没给出 recipe hint；主链不再用本地关键词检测补判。
        assert!(parse_execution_recipe_hint(None).is_none());
    }

    #[test]
    fn fallback_normalizer_output_still_enforces_content_evidence_planner_execute() {
        let out = normalizer_output_from_fallback(
            "把当前目录有没有隐藏文件看一下",
            "parse_failed_fallback_router",
            RouteDecision {
                resolved_user_intent: "看一下当前目录有没有隐藏文件".to_string(),
                needs_clarify: false,
                clarify_question: String::new(),
                reason: "current workspace executable request".to_string(),
                confidence: Some(0.72),
                schedule_kind: super::ScheduleKind::None,
                schedule_intent: None,
                wants_file_delivery: false,
                should_refresh_long_term_memory: false,
                agent_display_name_hint: String::new(),
                output_contract: IntentOutputContract {
                    exact_sentence_count: None,
                    response_shape: OutputResponseShape::Scalar,
                    requires_content_evidence: true,
                    delivery_required: false,
                    locator_kind: OutputLocatorKind::CurrentWorkspace,
                    delivery_intent: OutputDeliveryIntent::None,
                    semantic_kind: Default::default(),
                    locator_hint: String::new(),
                    self_extension: crate::SelfExtensionContract::default(),
                },
            },
            None,
        );
        assert_eq!(out.first_layer_decision, FirstLayerDecision::PlannerExecute);
        assert!(!out.needs_clarify);
        assert_eq!(
            out.output_contract.locator_kind,
            OutputLocatorKind::CurrentWorkspace
        );
        assert_eq!(out.fallback_source, None);
    }

    #[test]
    fn workspace_scope_patch_sets_locator_hint_from_structured_scope() {
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            locator_hint: "/home/guagua/rustclaw".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        };
        let applied = super::apply_workspace_scope_patch_to_contract(
            &mut contract,
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            Some(&serde_json::json!({"scope": "UI_only"})),
        );

        assert_eq!(applied.as_deref(), Some("UI"));
        assert_eq!(contract.locator_hint, "UI");
        assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    }

    #[test]
    fn workspace_scope_patch_keeps_specific_locator_hint() {
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            locator_hint: "UI".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        };
        let applied = super::apply_workspace_scope_patch_to_contract(
            &mut contract,
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            Some(&serde_json::json!({"scope": "pi_app_only"})),
        );

        assert_eq!(applied, None);
        assert_eq!(contract.locator_hint, "UI");
    }

    #[test]
    fn fallback_normalizer_keeps_llm_failure_on_safe_clarify() {
        let out = normalizer_output_from_fallback(
            "read scripts/nl_tests/fixtures/device_local/package.json and output only the name field",
            "llm_failed_safe_clarify",
            RouteDecision {
                resolved_user_intent: String::new(),
                needs_clarify: true,
                clarify_question: String::new(),
                reason: "fallback_router_llm_failed".to_string(),
                confidence: None,
                schedule_kind: super::ScheduleKind::None,
                schedule_intent: None,
                wants_file_delivery: false,
                should_refresh_long_term_memory: false,
                agent_display_name_hint: String::new(),
                output_contract: IntentOutputContract::default(),
            },
            Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable),
        );
        assert_eq!(out.first_layer_decision, FirstLayerDecision::Clarify);
        assert!(out.needs_clarify);
        assert!(matches!(
            out.output_contract.response_shape,
            OutputResponseShape::Free
        ));
        assert!(!out.output_contract.requires_content_evidence);
        assert!(!out.output_contract.delivery_required);
        assert!(matches!(
            out.output_contract.locator_kind,
            OutputLocatorKind::None
        ));
        assert!(matches!(
            out.output_contract.delivery_intent,
            OutputDeliveryIntent::None
        ));
        assert!(out
            .reason
            .contains("llm_failed_safe_clarify; fallback_router_llm_failed"));
        assert_eq!(
            out.fallback_source,
            Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable)
        );
    }

    #[test]
    fn clarify_question_policy_defaults_to_allow_model() {
        assert_eq!(
            ClarifyQuestionPolicy::default(),
            ClarifyQuestionPolicy::AllowModel
        );
    }

    #[test]
    fn scope_update_clarify_is_resolved_when_active_task_exists() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(
            super::should_resolve_task_scope_update_clarify_with_active_task(
                "先只看登录模块",
                Some(&snapshot),
                Some(TurnType::TaskScopeUpdate),
                Some(TargetTaskPolicy::ReuseActive),
                false,
                FirstLayerDecision::Clarify,
                &IntentOutputContract::default(),
                None,
            )
        );
    }

    #[test]
    fn scope_update_clarify_reuses_active_task_without_keyword_detector() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Help me create a rollout plan".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(
            super::should_resolve_task_scope_update_clarify_with_active_task(
                "Keep it limited to the onboarding flow",
                Some(&snapshot),
                Some(TurnType::TaskScopeUpdate),
                Some(TargetTaskPolicy::ReuseActive),
                false,
                FirstLayerDecision::Clarify,
                &IntentOutputContract::default(),
                None,
            )
        );
    }

    #[test]
    fn task_replace_clarify_is_resolved_when_active_task_exists() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Write a long article about RustClaw".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_resolve_task_replace_clarify_with_active_task(
            "Actually, replace it with an X thread",
            Some(&snapshot),
            Some(TurnType::TaskReplace),
            Some(TargetTaskPolicy::ReplaceActive),
            false,
            FirstLayerDecision::Clarify,
            &IntentOutputContract::default(),
            None,
        ));
    }

    #[test]
    fn task_replace_clarify_reuses_active_task_without_keyword_detector() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Write a launch memo about RustClaw".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_resolve_task_replace_clarify_with_active_task(
            "Make it a shorter internal memo instead",
            Some(&snapshot),
            Some(TurnType::TaskReplace),
            Some(TargetTaskPolicy::ReplaceActive),
            false,
            FirstLayerDecision::Clarify,
            &IntentOutputContract::default(),
            None,
        ));
    }

    #[test]
    fn active_task_scope_update_is_routed_back_to_direct_answer() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_route_active_task_mutation_to_direct_answer(
            "先只看登录模块",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::PlannerExecute,
            &IntentOutputContract::default(),
            None,
        ));
    }

    #[test]
    fn structured_replacement_patch_repairs_active_task_correction_metadata() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "Write a short setup checklist for RustClaw".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let patch = serde_json::json!({
            "replacements": [
                {"old": "Python 3.10", "new": "Python 3.11"}
            ]
        });
        let mut turn_type = None;
        let mut target_task_policy = None;
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
        let mut needs_clarify = false;
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: Default::default(),
        };

        let reason = super::apply_active_task_structured_patch_repair(
            "Use Python 3.11 instead of Python 3.10.",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut contract,
            Some(&patch),
        );

        assert_eq!(reason, Some("active_task_structured_patch_repair"));
        assert_eq!(turn_type, Some(TurnType::TaskCorrect));
        assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
        assert!(!needs_clarify);
        assert!(!contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    }

    #[test]
    fn structured_patch_repair_does_not_override_explicit_filename_target() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Write a release note".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let patch = serde_json::json!({
            "replacements": [
                {"old": "Python 3.10", "new": "Python 3.11"}
            ]
        });
        let mut turn_type = None;
        let mut target_task_policy = None;
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
        let mut needs_clarify = false;
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: Default::default(),
        };

        let reason = super::apply_active_task_structured_patch_repair(
            "In README.md, replace Python 3.10 with Python 3.11.",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut contract,
            Some(&patch),
        );

        assert_eq!(reason, None);
        assert_eq!(turn_type, None);
        assert_eq!(target_task_policy, None);
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert!(contract.requires_content_evidence);
    }

    #[test]
    fn scalar_patch_with_locator_hint_requires_active_binding_for_repair() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "Write a short release note for RustClaw".to_string(),
                ),
                last_primary_task_output: Some(
                    "RustClaw Release Notes - Your Quick Checklist".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let patch = serde_json::json!({"release_notes_python_version": "Python 3.11"});
        let mut turn_type = None;
        let mut target_task_policy = None;
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
        let mut needs_clarify = false;
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "release notes".to_string(),
            self_extension: Default::default(),
        };

        let reason = super::apply_active_task_structured_patch_repair(
            "Use Python 3.11 instead of Python 3.10.",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut contract,
            Some(&patch),
        );

        assert_eq!(reason, None);
        assert_eq!(turn_type, None);
        assert_eq!(target_task_policy, None);
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_hint, "release notes");

        let mut turn_type = Some(TurnType::TaskCorrect);
        let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
        let mut needs_clarify = false;
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "release notes".to_string(),
            self_extension: Default::default(),
        };
        let reason = super::apply_active_task_structured_patch_repair(
            "Use Python 3.11 instead of Python 3.10.",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut contract,
            Some(&patch),
        );

        assert_eq!(reason, Some("active_task_structured_patch_repair"));
        assert_eq!(turn_type, Some(TurnType::TaskCorrect));
        assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert!(!contract.requires_content_evidence);
        assert!(contract.locator_hint.is_empty());
    }

    #[test]
    fn standalone_execution_target_misroute_is_repaired_to_active_scope_update() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let mut turn_type = Some(TurnType::TaskRequest);
        let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let mut needs_clarify = true;
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "current workspace".to_string(),
            self_extension: Default::default(),
        };

        let reason = super::apply_active_task_scope_refinement_repair(
            "先只看登录模块",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut contract,
            None,
        );

        assert_eq!(reason, Some("active_task_scope_refinement_repair"));
        assert_eq!(turn_type, Some(TurnType::TaskScopeUpdate));
        assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
        assert!(!needs_clarify);
        assert!(!contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert!(contract.locator_hint.is_empty());
    }

    #[test]
    fn scope_refinement_repair_does_not_override_explicit_locator() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let mut turn_type = Some(TurnType::TaskRequest);
        let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let mut needs_clarify = true;
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "UI/src".to_string(),
            self_extension: Default::default(),
        };

        let reason = super::apply_active_task_scope_refinement_repair(
            "先只看 UI/src",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut contract,
            None,
        );

        assert_eq!(reason, None);
        assert_eq!(turn_type, Some(TurnType::TaskRequest));
        assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
        assert_eq!(decision, FirstLayerDecision::Clarify);
        assert!(needs_clarify);
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
        assert_eq!(contract.locator_hint, "UI/src");
    }

    #[test]
    fn scope_refinement_repair_preserves_standalone_observation_contract() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("生成一个 JSON 文件".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let mut turn_type = Some(TurnType::TaskRequest);
        let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
        let mut needs_clarify = false;
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: Default::default(),
        };

        let reason = super::apply_active_task_scope_refinement_repair(
            "检查当前运行环境并只返回关键值",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut contract,
            None,
        );

        assert_eq!(reason, None);
        assert_eq!(turn_type, Some(TurnType::TaskRequest));
        assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
        assert!(!needs_clarify);
        assert!(contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    }

    #[test]
    fn active_task_scope_update_en_remains_direct_answer_from_chat_wrapped_execution() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Help me create a test plan".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_route_active_task_mutation_to_direct_answer(
            "Only focus on the login module first",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::PlannerExecute,
            &IntentOutputContract::default(),
            None,
        ));
    }

    #[test]
    fn active_task_output_table_refinement_is_routed_back_to_direct_answer() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Summarize the release checklist".to_string()),
                last_primary_task_output: Some(
                    "1. Build\n2. Run tests\n3. Publish release notes".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_route_active_task_mutation_to_direct_answer(
            "把结果改成 markdown table 输出",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::PlannerExecute,
            &IntentOutputContract::default(),
            None,
        ));
    }

    #[test]
    fn active_task_correct_is_routed_back_to_direct_answer() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "Write one deployment note that mentions Python 3.10".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_route_active_task_mutation_to_direct_answer(
            "Correction: not Python 3.10, use Python 3.11",
            Some(&snapshot),
            Some(TurnType::TaskCorrect),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::PlannerExecute,
            &IntentOutputContract::default(),
            None,
        ));
    }

    #[test]
    fn active_text_followup_clears_stale_scalar_answer_candidate() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "Write a short release note for RustClaw".to_string(),
                ),
                last_primary_task_output: Some(
                    "RustClaw v0.1.7 ships with clearer configuration controls.".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let binding = super::AnswerCandidateBindingReport {
            candidate: "RC-CONT-EN-0428-B".to_string(),
            in_current_request: false,
            in_recent_assistant_replies: true,
            in_recent_turns_full: true,
            in_last_turn_full: true,
            in_recent_execution_context: false,
            in_memory_context: true,
        };
        let mut turn_type = Some(TurnType::PreferenceOrMemory);
        let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
        let mut decision = FirstLayerDecision::DirectAnswer;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let mut needs_clarify = false;
        let mut wants_file_delivery = false;
        let mut answer_candidate = binding.candidate.clone();
        let mut contract = IntentOutputContract::default();

        let reason = super::apply_active_text_followup_route_repair(
            "Make it for non-technical users.",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut wants_file_delivery,
            &mut contract,
            None,
            true,
            &mut answer_candidate,
        );

        assert_eq!(reason, Some("active_text_followup_route_repair"));
        assert_eq!(turn_type, Some(TurnType::TaskScopeUpdate));
        assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
        assert!(answer_candidate.is_empty());
        assert!(!contract.requires_content_evidence);
    }

    #[test]
    fn active_task_invalid_turn_binding_context_uses_schema_tokens_not_user_phrases() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "Write a short release note for RustClaw".to_string(),
                ),
                last_primary_task_output: Some(
                    "RustClaw v0.1.7 ships with clearer configuration controls.".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "Make it easier for non-technical readers.",
        );
        let raw = serde_json::json!({
            "turn_type": "response",
            "target_task_policy": "release_note_rewrite_non_technical"
        })
        .to_string();

        let context =
            super::active_task_invalid_turn_binding_context(&raw, Some(&snapshot), &surface, false)
                .unwrap();

        assert!(context.contains("active_task_invalid_turn_binding"));
        assert!(context.contains("turn_type_invalid: true"));
        assert!(context.contains("target_task_policy_invalid: true"));
    }

    #[test]
    fn active_text_correction_clears_stale_workspace_evidence_contract() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "Write a short release note for RustClaw".to_string(),
                ),
                last_primary_task_output: Some(
                    "RustClaw v0.1.7 supports Python 3.10 setup notes.".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let mut turn_type = Some(TurnType::TaskCorrect);
        let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
        let mut needs_clarify = false;
        let mut wants_file_delivery = false;
        let mut answer_candidate = String::new();
        let mut contract = IntentOutputContract {
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "release notes".to_string(),
            ..IntentOutputContract::default()
        };

        let reason = super::apply_active_text_followup_route_repair(
            "Correction: mention Python 3.11, not Python 3.10.",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut wants_file_delivery,
            &mut contract,
            None,
            false,
            &mut answer_candidate,
        );

        assert_eq!(reason, Some("active_text_followup_route_repair"));
        assert_eq!(turn_type, Some(TurnType::TaskCorrect));
        assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
        assert_eq!(decision, FirstLayerDecision::DirectAnswer);
        assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
        assert!(!contract.requires_content_evidence);
        assert_eq!(contract.locator_kind, OutputLocatorKind::None);
        assert!(contract.locator_hint.is_empty());
    }

    #[test]
    fn active_task_mutation_with_content_evidence_stays_executable() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Summarize this repository".to_string()),
                last_primary_task_output: Some("It has a web UI and backend services.".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            ..IntentOutputContract::default()
        };
        assert!(!super::should_route_active_task_mutation_to_direct_answer(
            "Focus only on the UI part",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::PlannerExecute,
            &contract,
            None,
        ));
    }

    #[test]
    fn active_text_followup_repair_preserves_real_workspace_summary_contract() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Summarize this repository".to_string()),
                last_primary_task_output: Some("It has a web UI and backend services.".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let mut turn_type = Some(TurnType::TaskScopeUpdate);
        let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
        let mut decision = FirstLayerDecision::PlannerExecute;
        let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
        let mut needs_clarify = false;
        let mut wants_file_delivery = false;
        let mut answer_candidate = String::new();
        let mut contract = IntentOutputContract {
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            ..IntentOutputContract::default()
        };

        let reason = super::apply_active_text_followup_route_repair(
            "Focus only on the UI part",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut wants_file_delivery,
            &mut contract,
            None,
            false,
            &mut answer_candidate,
        );

        assert_eq!(reason, None);
        assert_eq!(decision, FirstLayerDecision::PlannerExecute);
        assert!(contract.requires_content_evidence);
        assert_eq!(
            contract.semantic_kind,
            OutputSemanticKind::WorkspaceProjectSummary
        );
    }

    #[test]
    fn unresolved_deictic_observation_clarify_is_not_downgraded_to_direct_answer() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我检查这个文件".to_string()),
                last_primary_task_output: Some("需要一个具体文件目标。".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::None,
            semantic_kind: OutputSemanticKind::None,
            ..IntentOutputContract::default()
        };
        assert!(
            !super::should_resolve_task_scope_update_clarify_with_active_task(
                "看看那个文件最后 5 行",
                Some(&snapshot),
                Some(TurnType::TaskScopeUpdate),
                Some(TargetTaskPolicy::ReuseActive),
                false,
                FirstLayerDecision::Clarify,
                &contract,
                None,
            )
        );
    }

    #[test]
    fn structured_deictic_unresolved_target_blocks_non_chinese_pronoun_fallback_gap() {
        let surface =
            crate::intent::surface_signals::analyze_prompt_surface("それの最後の2行を見せて");
        assert!(!surface.has_deictic_reference());
        let contract = IntentOutputContract {
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };
        let patch = serde_json::json!({
            "deictic_reference": {"target": "unresolved_prior_object"}
        });

        assert!(super::unresolved_deictic_observable_target_should_clarify(
            &surface,
            &contract,
            Some(&patch),
        ));
        assert!(!super::active_task_turn_can_reuse_semantic_patch(
            &surface,
            Some(&patch),
        ));
    }

    #[test]
    fn structured_deictic_resolved_target_overrides_local_pronoun_fallback() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface("看看那个最后 5 行");
        assert!(surface.has_deictic_reference());
        let contract = IntentOutputContract {
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };
        let patch = serde_json::json!({
            "deictic_reference": {"target": "current_action_result"}
        });

        assert!(!super::unresolved_deictic_observable_target_should_clarify(
            &surface,
            &contract,
            Some(&patch),
        ));
    }

    #[test]
    fn scope_refinement_repair_keeps_unresolved_deictic_observation_clarify() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我整理一个方案".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let mut turn_type = Some(TurnType::TaskRequest);
        let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
        let mut decision = FirstLayerDecision::Clarify;
        let mut finalize_style = crate::ActFinalizeStyle::Plain;
        let mut needs_clarify = true;
        let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: Default::default(),
        };

        let reason = super::apply_active_task_scope_refinement_repair(
            "看看那个文件最后 5 行",
            Some(&snapshot),
            &mut turn_type,
            &mut target_task_policy,
            false,
            &mut decision,
            &mut finalize_style,
            &mut needs_clarify,
            super::ScheduleKind::None,
            false,
            &mut contract,
            None,
        );

        assert_eq!(reason, None);
        assert_eq!(turn_type, Some(TurnType::TaskRequest));
        assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
        assert_eq!(decision, FirstLayerDecision::Clarify);
        assert!(needs_clarify);
        assert!(contract.requires_content_evidence);
    }

    #[test]
    fn active_task_output_refinement_clarify_is_resolved() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Summarize this repository".to_string()),
                last_primary_task_output: Some(
                    "The UI is a web-based frontend for RustClaw.".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_resolve_task_append_clarify_with_active_task(
            "Output a two-row markdown table",
            Some(&snapshot),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::Clarify,
            &IntentOutputContract::default(),
            None,
        ));
    }

    #[test]
    fn active_task_append_clarify_without_output_is_resolved() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我写个方案".to_string()),
                last_primary_task_output: None,
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_resolve_task_append_clarify_with_active_task(
            "控制在 80 字内，只输出正文",
            Some(&snapshot),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::Clarify,
            &IntentOutputContract::default(),
            None,
        ));
    }

    #[test]
    fn active_task_append_clarify_keeps_file_locator_guard() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我检查这个文件".to_string()),
                last_primary_task_output: None,
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!super::should_resolve_task_append_clarify_with_active_task(
            "README.md",
            Some(&snapshot),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::Clarify,
            &IntentOutputContract::default(),
            None,
        ));
    }

    #[test]
    fn bare_path_correction_can_fill_active_observable_task() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "读一下 configs/config.toml 里的名字字段，只输出值".to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };

        assert!(super::bare_path_only_input_can_fill_active_observable_task(
            Some(&snapshot),
            Some(TurnType::TaskCorrect),
            Some(TargetTaskPolicy::ReuseActive),
            FirstLayerDecision::PlannerExecute,
            &contract,
        ));
    }

    #[test]
    fn bare_path_clarify_with_observable_scalar_contract_can_fill_active_task() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some(
                    "Extract the name field from the package file and output only the value"
                        .to_string(),
                ),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "scripts/nl_tests/fixtures/device_local/package.json".to_string(),
            ..IntentOutputContract::default()
        };

        assert!(super::bare_path_only_input_can_fill_active_observable_task(
            Some(&snapshot),
            None,
            None,
            FirstLayerDecision::Clarify,
            &contract,
        ));
    }

    #[test]
    fn bare_path_active_clarify_state_can_fill_standalone_task_request() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: Some(crate::clarify_state::ClarifyState {
                missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
                pending_question: "Provide the file path".to_string(),
                candidate_targets: Vec::new(),
                delivery_required: false,
                output_shape: Some(OutputResponseShape::Scalar.as_str().to_string()),
                semantic_kind: Some(OutputSemanticKind::StructuredKeys.as_str().to_string()),
                source_request: "Find the name field in the package file and output only the value"
                    .to_string(),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
            }),
            active_observed_facts: None,
        };
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "scripts/nl_tests/fixtures/device_local/package.json".to_string(),
            ..IntentOutputContract::default()
        };

        assert!(super::bare_path_only_input_can_fill_active_observable_task(
            Some(&snapshot),
            Some(TurnType::TaskRequest),
            Some(TargetTaskPolicy::Standalone),
            FirstLayerDecision::Clarify,
            &contract,
        ));
    }

    #[test]
    fn bare_filename_task_request_can_replace_active_existence_check() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("看看那个重启脚本在不在".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: "restart_clawd_latest.sh".to_string(),
            ..IntentOutputContract::default()
        };

        assert!(super::bare_path_only_input_can_fill_active_observable_task(
            Some(&snapshot),
            Some(TurnType::TaskRequest),
            Some(TargetTaskPolicy::ReplaceActive),
            FirstLayerDecision::PlannerExecute,
            &contract,
        ));
    }

    #[test]
    fn bare_path_with_executable_contract_can_fill_active_log_tail() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我看一下那个日志最近 20 行".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: "logs/clawd.log".to_string(),
            ..IntentOutputContract::default()
        };

        assert!(super::bare_path_only_input_can_fill_active_observable_task(
            Some(&snapshot),
            None,
            None,
            FirstLayerDecision::PlannerExecute,
            &contract,
        ));
    }

    #[test]
    fn bare_filename_can_replace_active_delivery_target() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("send the selected file".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                source_request: "send the selected file".to_string(),
                op_kind: crate::followup_frame::FollowupOpKind::Delivery,
                bound_target: Some("/tmp/old.md".to_string()),
                output_shape: Some(OutputResponseShape::FileToken.as_str().to_string()),
                source_task_id: "task-1".to_string(),
                updated_at_ts: 1,
                expires_at_ts: 2,
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        let contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            locator_hint: "README.md".to_string(),
            ..IntentOutputContract::default()
        };

        assert!(super::bare_path_only_input_can_fill_active_observable_task(
            Some(&snapshot),
            Some(TurnType::TaskRequest),
            None,
            FirstLayerDecision::PlannerExecute,
            &contract,
        ));
    }

    #[test]
    fn bare_path_without_observable_contract_still_needs_action_clarify() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我检查这个文件".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };

        assert!(
            !super::bare_path_only_input_can_fill_active_observable_task(
                Some(&snapshot),
                Some(TurnType::TaskAppend),
                Some(TargetTaskPolicy::ReuseActive),
                FirstLayerDecision::PlannerExecute,
                &IntentOutputContract::default(),
            )
        );
    }

    #[test]
    fn workspace_scope_listing_shape_is_not_treated_as_fileish_cue() {
        let surface =
            crate::intent::surface_signals::analyze_prompt_surface("看看当前目录有哪些顶层文件夹");
        assert!(!super::prompt_has_concrete_fileish_cue(&surface));
    }

    #[test]
    fn simple_command_shape_is_not_treated_as_fileish_cue() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface("执行 pwd");
        assert!(!super::prompt_has_concrete_fileish_cue(&surface));
    }

    #[test]
    fn locator_target_pair_still_counts_as_fileish_cue() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "比较 README.md 和 AGENTS.md 哪个更大",
        );
        assert!(super::prompt_has_concrete_fileish_cue(&surface));
    }

    /// §3.5c-小切口：intent_normalizer schema 与 Rust parser 漂移检查。
    ///
    /// 校验内容：
    /// 1. `prompts/schemas/intent_normalizer.schema.json` 是合法 JSON 且为 object schema；
    /// 2. `IntentNormalizerOut` 里所有 `#[serde(default)]` 字段都在 schema `properties` 里；
    /// 3. 每个 enum-bearing 字段的 schema 枚举值，喂给对应 `parse_*` 函数都能落到非默认 variant
    ///    （空字符串和 `"none"`/`"unknown"` 这种"显式无"语义值排除）。
    ///
    /// 任何一项不满足都说明 prompt / schema / parser 三者已漂移，应在本测试里同步更新。
    #[test]
    fn intent_normalizer_schema_drift() {
        const SCHEMA_RAW: &str =
            include_str!("../../../prompts/schemas/intent_normalizer.schema.json");
        let schema: serde_json::Value = serde_json::from_str(SCHEMA_RAW)
            .expect("intent_normalizer.schema.json must be valid JSON");
        assert_eq!(
            schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "schema root must be object"
        );

        // §3.5c-小切口 步骤 2：每个 IntentNormalizerOut 字段必须在 properties 里登记。
        const STRUCT_FIELDS: &[&str] = &[
            "resolved_user_intent",
            "answer_candidate",
            "resume_behavior",
            "schedule_kind",
            "wants_file_delivery",
            "should_refresh_long_term_memory",
            "agent_display_name_hint",
            "needs_clarify",
            "clarify_question",
            "reason",
            "confidence",
            "decision",
            "schedule_intent",
            "output_contract",
            "execution_recipe",
            "turn_type",
            "target_task_policy",
            "should_interrupt_active_run",
            "state_patch",
            "attachment_processing_required",
        ];
        let properties = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("schema must have `properties` object");
        for field in STRUCT_FIELDS {
            assert!(
                properties.contains_key(*field),
                "schema missing parser field `{}` under properties — sync prompts/schemas/intent_normalizer.schema.json with IntentNormalizerOut",
                field
            );
        }

        // §3.5c-小切口 步骤 3：枚举值 → parse_* 函数必须落到非默认 variant
        // （除非是显式的「无 / 未知」语义占位）。
        fn enum_strings<'a>(schema: &'a serde_json::Value, path: &[&str]) -> Vec<String> {
            let mut node = schema;
            for p in path {
                node = node
                    .get(*p)
                    .unwrap_or_else(|| panic!("schema path `{}` not found", path.join(".")));
            }
            node.get("enum")
                .and_then(|v| v.as_array())
                .unwrap_or_else(|| panic!("schema path `{}.enum` not found", path.join(".")))
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        }

        // resume_behavior：none / "" 是「无」语义，跳过。
        for token in enum_strings(&schema, &["properties", "resume_behavior"]) {
            if token.is_empty() || token == "none" {
                continue;
            }
            let parsed = super::parse_resume_behavior(&token);
            assert_ne!(
                parsed,
                super::ResumeBehavior::None,
                "resume_behavior token `{}` not recognized by parse_resume_behavior",
                token
            );
        }

        for token in enum_strings(&schema, &["properties", "schedule_kind"]) {
            if token.is_empty() || token == "none" {
                continue;
            }
            let parsed = super::parse_schedule_kind(&token);
            assert_ne!(
                parsed,
                super::ScheduleKind::None,
                "schedule_kind token `{}` not recognized by parse_schedule_kind",
                token
            );
        }

        for token in enum_strings(&schema, &["properties", "turn_type"]) {
            if token.is_empty() {
                continue;
            }
            let parsed = super::parse_turn_type(&token);
            assert!(
                parsed.is_some(),
                "turn_type token `{}` not recognized by parse_turn_type",
                token
            );
        }

        for token in enum_strings(&schema, &["properties", "target_task_policy"]) {
            if token.is_empty() {
                continue;
            }
            let parsed = super::parse_target_task_policy(&token);
            assert!(
                parsed.is_some(),
                "target_task_policy token `{}` not recognized by parse_target_task_policy",
                token
            );
        }

        // decision: the only first-layer semantic gate.
        for token in enum_strings(&schema, &["properties", "decision"]) {
            if token.is_empty() {
                continue;
            }
            assert!(
                super::parse_first_layer_decision_text(&token).is_some(),
                "decision token `{}` not recognized by parse_first_layer_decision_text",
                token
            );
        }

        for token in enum_strings(
            &schema,
            &[
                "properties",
                "output_contract",
                "properties",
                "response_shape",
            ],
        ) {
            if token.is_empty() || token == "free" {
                continue;
            }
            assert_ne!(
                super::parse_output_response_shape(&token),
                OutputResponseShape::Free,
                "response_shape `{}` not recognized",
                token
            );
        }
        for token in enum_strings(
            &schema,
            &[
                "properties",
                "output_contract",
                "properties",
                "locator_kind",
            ],
        ) {
            if token.is_empty() || token == "none" {
                continue;
            }
            assert_ne!(
                super::parse_output_locator_kind(&token),
                OutputLocatorKind::None,
                "locator_kind `{}` not recognized",
                token
            );
        }
        for token in enum_strings(
            &schema,
            &[
                "properties",
                "output_contract",
                "properties",
                "delivery_intent",
            ],
        ) {
            if token.is_empty() || token == "none" {
                continue;
            }
            assert_ne!(
                super::parse_output_delivery_intent(&token),
                OutputDeliveryIntent::None,
                "delivery_intent `{}` not recognized",
                token
            );
        }
        for token in enum_strings(
            &schema,
            &[
                "properties",
                "output_contract",
                "properties",
                "semantic_kind",
            ],
        ) {
            if token.is_empty() || token == "none" {
                continue;
            }
            if token == "scalar" {
                assert_eq!(
                    super::parse_output_semantic_kind(&token),
                    OutputSemanticKind::None,
                    "semantic_kind `scalar` is a legacy LLM alias and should normalize to none"
                );
                continue;
            }
            assert_ne!(
                super::parse_output_semantic_kind(&token),
                OutputSemanticKind::None,
                "semantic_kind `{}` not recognized",
                token
            );
        }
        for token in enum_strings(
            &schema,
            &[
                "properties",
                "output_contract",
                "properties",
                "self_extension",
                "properties",
                "mode",
            ],
        ) {
            if token.is_empty() || token == "none" {
                continue;
            }
            assert_ne!(
                super::parse_self_extension_mode(&token),
                crate::SelfExtensionMode::None,
                "self_extension.mode `{}` not recognized",
                token
            );
        }
        for token in enum_strings(
            &schema,
            &[
                "properties",
                "output_contract",
                "properties",
                "self_extension",
                "properties",
                "trigger",
            ],
        ) {
            if token.is_empty() || token == "none" {
                continue;
            }
            assert_ne!(
                super::parse_self_extension_trigger(&token),
                crate::SelfExtensionTrigger::None,
                "self_extension.trigger `{}` not recognized",
                token
            );
        }

        for token in enum_strings(
            &schema,
            &["properties", "execution_recipe", "properties", "kind"],
        ) {
            if token.is_empty() || token == "none" {
                continue;
            }
            assert_ne!(
                crate::execution_recipe::parse_execution_recipe_kind_text(&token),
                ExecutionRecipeKind::None,
                "execution_recipe.kind `{}` not recognized",
                token
            );
        }
        for token in enum_strings(
            &schema,
            &["properties", "execution_recipe", "properties", "profile"],
        ) {
            if token.is_empty() || token == "none" {
                continue;
            }
            assert_ne!(
                crate::execution_recipe::parse_execution_recipe_profile_text(&token),
                ExecutionRecipeProfile::None,
                "execution_recipe.profile `{}` not recognized",
                token
            );
        }
        for token in enum_strings(
            &schema,
            &[
                "properties",
                "execution_recipe",
                "properties",
                "target_scope",
            ],
        ) {
            if token.is_empty() || token == "none" || token == "unknown" {
                continue;
            }
            assert_ne!(
                crate::execution_recipe::parse_execution_recipe_target_scope_text(&token),
                ExecutionRecipeTargetScope::Unknown,
                "execution_recipe.target_scope `{}` not recognized",
                token
            );
        }
    }

    #[test]
    fn parse_output_semantic_kind_prefers_last_recognized_token_in_multi_value_output() {
        assert_eq!(
            super::parse_output_semantic_kind("sqlite_table_listing|sqlite_database_kind_judgment"),
            OutputSemanticKind::SqliteDatabaseKindJudgment
        );
    }
}
