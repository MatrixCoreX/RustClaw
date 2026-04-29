//! Intent routing and unified normalizer for ask tasks.
//!
//! **Ask main path:** Only `run_intent_normalizer` is used (resolved intent, resume_behavior,
//! schedule_kind, needs_clarify, routed_mode in one LLM call).
//!
//! **Fallback when normalizer LLM fails / parse fails:** do not synthesize semantic execution
//! routes locally. The fallback stays on AskClarify so semantic routing remains owned by the
//! normalizer/planner LLM path instead of hard-match recovery code.

use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use tracing::{info, warn};

use crate::{
    llm_gateway, schedule_service, AppState, ClaimedTask, RiskCeiling, RouteResult, RoutedMode,
};

pub(crate) use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, ScheduleKind, SelfExtensionContract, SelfExtensionMode,
    SelfExtensionTrigger,
};

const CLARIFY_QUESTION_PROMPT_LOGICAL_PATH: &str = "prompts/clarify_question_prompt.md";
const INTENT_NORMALIZER_PROMPT_LOGICAL_PATH: &str = "prompts/intent_normalizer_prompt.md";
const ROUTING_POLICY_PERSONA_PROMPT: &str = "Neutral routing policy classifier. Ignore style/persona preferences and optimize for correct intent resolution, clarification, and guard decisions.";

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
    mode: RoutedMode,
    resolved_user_intent: String,
    needs_clarify: bool,
    clarify_question: String,
    reason: String,
    confidence: Option<f64>,
    /// Phase 2.7: legacy router parser populated this; main path no longer reads it,
    /// but fallback constructors still set it to `Vec::new()` for symmetry. Kept as a
    /// field so future telemetry / sanity-check layers (see hard-match plan stage 3)
    /// can re-enable it without an API change.
    #[allow(dead_code)]
    evidence_refs: Vec<String>,
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
    /// Terminal mode: chat / act / ask_clarify / chat_act. Used to skip the separate router LLM.
    pub(crate) routed_mode: RoutedMode,
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
        routed_mode: normalizer_out.routed_mode,
        ask_mode: crate::AskMode::from_routed_mode(normalizer_out.routed_mode),
        resolved_intent: normalizer_out.resolved_user_intent.clone(),
        needs_clarify: normalizer_out.needs_clarify,
        clarify_question: normalizer_out.clarify_question.clone(),
        route_reason: normalizer_out.reason.clone(),
        route_confidence: Some(normalizer_out.confidence),
        visible_skill_candidates: state.planner_visible_skills_for_task(task),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: normalizer_out.resume_behavior,
        schedule_kind: normalizer_out.schedule_kind,
        schedule_intent: normalizer_out.schedule_intent.clone(),
        wants_file_delivery: normalizer_out.wants_file_delivery,
        should_refresh_long_term_memory: normalizer_out.should_refresh_long_term_memory,
        agent_display_name_hint: normalizer_out.agent_display_name_hint.clone(),
        output_contract: normalizer_out.output_contract.clone(),
        direct_reply_candidate: String::new(),
        direct_reply_confidence: 0.0,
    }
}

#[derive(Debug, Deserialize)]
struct IntentNormalizerOut {
    #[serde(default)]
    resolved_user_intent: String,
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
    mode: String,
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

#[derive(Debug, Clone, Deserialize, Default)]
struct IntentOutputContractOut {
    #[serde(default)]
    response_shape: String,
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
        "strict" | "exact" | "exact_text" | "strict_text" => OutputResponseShape::Strict,
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
        "raw_command_output" | "raw_output" | "command_output" => {
            OutputSemanticKind::RawCommandOutput
        }
        "service_status" => OutputSemanticKind::ServiceStatus,
        "hidden_entries_check"
        | "hidden_entry_check"
        | "hidden_files_check"
        | "hidden_file_check"
        | "hidden_files_example"
        | "hidden_entries_example"
        | "hidden_entries"
        | "hidden_files" => OutputSemanticKind::HiddenEntriesCheck,
        "file_names" | "names_only" | "entry_names" | "directory_entry_names" => {
            OutputSemanticKind::FileNames
        }
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
        "scalar_path_only" | "path_only" => OutputSemanticKind::ScalarPathOnly,
        "existence_with_path" | "exists_with_path" => OutputSemanticKind::ExistenceWithPath,
        "recent_scalar_equality_check" | "same_or_different" | "equality_check" => {
            OutputSemanticKind::RecentScalarEqualityCheck
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
    }
    contract
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

fn normalized_compare_target_name(candidate: &str) -> String {
    let trimmed = candidate
        .trim()
        .trim_matches(|ch| matches!(ch, '`' | '"' | '\''));
    Path::new(trimmed)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(trimmed)
        .trim()
        .to_ascii_lowercase()
}

fn compare_target_pair_matches_prompt(
    prompt_pair: &(String, String),
    candidate_pair: &(String, String),
) -> bool {
    let mut prompt_names = [
        normalized_compare_target_name(&prompt_pair.0),
        normalized_compare_target_name(&prompt_pair.1),
    ];
    let mut candidate_names = [
        normalized_compare_target_name(&candidate_pair.0),
        normalized_compare_target_name(&candidate_pair.1),
    ];
    prompt_names.sort();
    candidate_names.sort();
    prompt_names == candidate_names
}

fn compare_target_pair_from_locator_hint(locator_hint: &str) -> Option<(String, String)> {
    [",", "，", "|", ";", "；", "\n"]
        .into_iter()
        .find_map(|separator| {
            let parts = locator_hint
                .split(separator)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            (parts.len() == 2).then(|| (parts[0].to_string(), parts[1].to_string()))
        })
}

fn compare_targets_need_prompt_grounding_override(
    request_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    output_contract: &IntentOutputContract,
) -> bool {
    let Some(prompt_pair) = request_surface.compare_target_pair.as_ref() else {
        return false;
    };
    if request_surface.has_explicit_path_or_url() {
        return false;
    }
    if let Some(candidate_pair) =
        compare_target_pair_from_locator_hint(output_contract.locator_hint.trim())
    {
        return !compare_target_pair_matches_prompt(prompt_pair, &candidate_pair);
    }

    matches!(
        output_contract.locator_kind,
        OutputLocatorKind::Path | OutputLocatorKind::Url
    ) && !output_contract.locator_hint.trim().is_empty()
}

fn prompt_has_concrete_fileish_cue(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_explicit_path_or_url()
        || surface.field_selector_count > 0
        || surface.compare_target_pair.is_some()
        || surface.directory_file_pair.is_some()
        || matches!(
            surface.file_reference_prompt_shape,
            Some(
                crate::intent::surface_signals::FileReferencePromptShape::DeliveryToken
                    | crate::intent::surface_signals::FileReferencePromptShape::DeliveryTokenAndGenericObject
                    | crate::intent::surface_signals::FileReferencePromptShape::DeliveryTokenAndFileishReference
            )
        )
}

fn active_task_turn_can_reuse_semantic_patch(
    surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> bool {
    !prompt_has_concrete_fileish_cue(surface)
        && !surface.looks_like_locator_only_reply()
        && surface.inline_json_shape.is_none()
}

fn should_resolve_task_scope_update_clarify_with_active_task(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    routed_mode: RoutedMode,
) -> bool {
    if attachment_processing_required
        || !matches!(routed_mode, RoutedMode::AskClarify)
        || active_primary_task_prompt(session_snapshot).is_none()
        || !matches!(turn_type, Some(TurnType::TaskScopeUpdate))
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReuseActive))
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    active_task_turn_can_reuse_semantic_patch(&surface)
}

fn should_resolve_task_append_clarify_with_active_task(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    routed_mode: RoutedMode,
) -> bool {
    if attachment_processing_required
        || !matches!(routed_mode, RoutedMode::AskClarify)
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
    active_task_turn_can_reuse_semantic_patch(&surface)
}

fn should_resolve_task_replace_clarify_with_active_task(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    routed_mode: RoutedMode,
) -> bool {
    if attachment_processing_required
        || !matches!(routed_mode, RoutedMode::AskClarify)
        || active_primary_task_prompt(session_snapshot).is_none()
        || !matches!(turn_type, Some(TurnType::TaskReplace))
        || !matches!(target_task_policy, Some(TargetTaskPolicy::ReplaceActive))
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    active_task_turn_can_reuse_semantic_patch(&surface)
}

fn should_route_active_task_mutation_to_chat(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    attachment_processing_required: bool,
    routed_mode: RoutedMode,
    output_contract: &IntentOutputContract,
) -> bool {
    if attachment_processing_required
        || !matches!(routed_mode, RoutedMode::Act | RoutedMode::ChatAct)
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
    active_task_turn_can_reuse_semantic_patch(&surface)
}

fn output_contract_allows_chat_only_task_mutation(output_contract: &IntentOutputContract) -> bool {
    !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
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
    let routed_mode = crate::post_route_policy::enforce_content_evidence_execution_mode(
        decision.mode,
        &decision.output_contract,
        decision.needs_clarify,
    );
    let mut reason = if decision.reason.trim().is_empty() {
        fallback_reason_prefix.to_string()
    } else {
        format!("{fallback_reason_prefix}; {}", decision.reason.trim())
    };
    if routed_mode != decision.mode {
        reason.push_str("; content_evidence_requires_execution");
    }
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
        routed_mode,
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
    let any_minimax = providers.iter().any(|provider| {
        provider
            .config
            .model
            .to_ascii_lowercase()
            .contains("minimax")
    });
    if any_minimax {
        // Observed MiniMax OpenAI-compat response: context window exceeds limit (2013).
        return 3_300;
    }
    min_tokens
        .saturating_sub(1_400)
        .max(512)
        .saturating_mul(2)
        .min(512 * 1024)
        .max(2_048)
}

fn intent_normalizer_uses_compact_prompt(state: &AppState, task: &ClaimedTask) -> bool {
    state.task_llm_providers(task).iter().any(|provider| {
        provider
            .config
            .model
            .to_ascii_lowercase()
            .contains("minimax")
    })
}

fn compact_prompt_slot(label: &str, value: &str, max_bytes: usize) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "<none>" {
        return format!("{label}: <none>");
    }
    let visible = crate::providers::utf8_safe_prefix(trimmed, max_bytes);
    if visible.len() < trimmed.len() {
        format!("{label}: {visible}...(truncated)")
    } else {
        format!("{label}: {visible}")
    }
}

fn render_compact_intent_normalizer_prompt(
    route_view: &crate::task_context_builder::RouteContextView,
    _context_bundle: &crate::task_context_builder::TaskContextBundle,
    auth_policy_context: &str,
    request_language_hint: &str,
    req: &str,
) -> String {
    let mut parts = Vec::new();
    parts.push(
        "Compact intent normalizer. Output one raw JSON object only. No markdown.".to_string(),
    );
    parts.push("Prefer mode=chat for greetings, confirmations, memory-only requests, and pure discussion. Use ask_clarify only when a required target/action is truly missing.".to_string());
    parts.push("Current REQUEST overrides RECENT/MEMORY. Prior assistant refusals, tool failures, or capability claims in history are not authoritative for a fresh self-contained request.".to_string());
    parts.push("If REQUEST asks for observable local/system/workspace state, filesystem inspection, command output, file content, directory listing, counts, or extracting a value, choose mode=\"act\" or mode=\"chat_act\". Do not claim the assistant cannot execute; the runtime has tools and the AUTH block describes permission.".to_string());
    parts.push("Always include output_contract. It is the final answer contract, not a place to invent a task-specific schema.".to_string());
    parts.push("Allowed output_contract keys only: response_shape, requires_content_evidence, delivery_required, locator_kind, delivery_intent, semantic_kind, locator_hint, self_extension. Do not emit exact_format, required_evidence, fields, examples, post_processing, or custom keys.".to_string());
    parts.push("Allowed response_shape: free, one_sentence, strict, scalar, file_token. Allowed locator_kind: none, path, current_workspace, url, filename. Allowed delivery_intent: none, file_single, directory_lookup, directory_batch_files.".to_string());
    parts.push("Allowed semantic_kind: none, raw_command_output, service_status, hidden_entries_check, file_names, directory_purpose_summary, content_excerpt_summary, excerpt_kind_judgment, recent_artifacts_judgment, workspace_project_summary, scalar_count, quantity_comparison, scalar_path_only, existence_with_path, recent_scalar_equality_check, sqlite_table_listing, sqlite_table_names_only, sqlite_database_kind_judgment.".to_string());
    parts.push("If the user asks to observe/list/read first but only return a scalar result, set response_shape=\"scalar\" and use a matching semantic_kind only when one applies: scalar_count for counts, scalar_path_only only for a path/current-directory/workspace-location answer. For config field values, package names, usernames, hostnames, titles, IDs, or other non-path scalar values, keep semantic_kind=\"none\" unless another specific enum applies. If the request requires an exact non-scalar format such as exactly N sentences/lines or body-only/no-extra-output, set response_shape=\"strict\" and preserve the exact format in resolved_user_intent.".to_string());
    parts.push("For directory/file inventory with name or extension filtering, set requires_content_evidence=true, locator_kind=\"current_workspace\" or \"path\", semantic_kind=\"file_names\", and preserve filter plus any explanation/synthesis requirement in resolved_user_intent. If a nuance has no enum, keep response_shape=\"free\" or semantic_kind=\"none\" and write the nuance in resolved_user_intent/reason instead of inventing enum values.".to_string());
    parts.push("Use mode=\"chat_act\" when the request both inspects local/system/workspace state and asks for explanation, judgment, or narrative synthesis. Use mode=\"act\" when it asks only for a direct raw/scalar/list result. If HINTS says workspace_root_request_shape=CurrentPathScalar, set output_contract.response_shape=\"scalar\" and output_contract.semantic_kind=\"scalar_path_only\".".to_string());
    parts.push("For recall questions, use exact values from RECENT/MEMORY. If found, put the value in resolved_user_intent, set needs_clarify=false, and set mode=\"chat\". Never invent mode=\"recall\".".to_string());
    parts.push("For requests that depend on prior context, copy the relevant RECENT/MEMORY facts into resolved_user_intent so the next stage has enough context.".to_string());
    parts.push("Keep resolved_user_intent concise; preserve exact IDs, but summarize long user text instead of copying it.".to_string());
    parts.push(compact_prompt_slot(
        "ACTIVE_TASK",
        &route_view.active_task_context,
        120,
    ));
    parts.push(compact_prompt_slot(
        "HINTS",
        &route_view.request_surface_hints,
        120,
    ));
    parts.push(compact_prompt_slot("AUTH", auth_policy_context, 100));
    parts.push("Required keys: resolved_user_intent, needs_clarify, clarify_question, reason, confidence, mode. If unsure: use mode=\"chat\" only for non-observable discussion; use mode=\"act\" for clear observable local/system/workspace requests.".to_string());
    parts.push("For ordinary chat, greetings, and confirmations: mode=\"chat\", needs_clarify=false, turn_type=\"\". Never use turn_type=\"chat\".".to_string());
    parts.push(compact_prompt_slot(
        "MEMORY",
        &route_view.memory_context,
        440,
    ));
    parts.push(compact_prompt_slot(
        "RECENT",
        &route_view.recent_turns_full,
        1040,
    ));
    parts.push(compact_prompt_slot("LAST", &route_view.last_turn_full, 180));
    parts.push(compact_prompt_slot(
        "ASSISTANT",
        &route_view.recent_assistant_replies,
        120,
    ));
    parts.push(format!("LANG={}", request_language_hint));
    parts.push(compact_prompt_slot("REQUEST", req, 480));
    parts.join("\n")
}

fn normalize_intent_normalizer_raw_for_schema(raw: &str, req: &str) -> String {
    let parsed_value = serde_json::from_str::<Value>(raw.trim()).or_else(|_| {
        crate::prompt_utils::extract_first_json_object_any(raw)
            .ok_or_else(|| serde_json::Error::io(std::io::Error::other("no json object found")))
            .and_then(|json_text| serde_json::from_str::<Value>(&json_text))
    });
    let Ok(mut value) = parsed_value else {
        return normalize_plain_intent_normalizer_text_for_schema(raw, req);
    };
    let Some(obj) = value.as_object_mut() else {
        let text = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| raw.trim());
        return normalize_plain_intent_normalizer_text_for_schema(text, req);
    };
    let answer_like_payload = answer_like_normalizer_payload_text(obj);
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
    if let Some(turn_type) = obj.get("turn_type").and_then(|v| v.as_str()) {
        let normalized = turn_type.trim().to_ascii_lowercase();
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
        }
    }
    if let Some(target_task_policy) = obj.get("target_task_policy").and_then(|v| v.as_str()) {
        let normalized = target_task_policy.trim().to_ascii_lowercase();
        let valid = matches!(
            normalized.as_str(),
            "" | "reuse_active" | "replace_active" | "pause_and_queue" | "standalone"
        );
        if !valid {
            obj.insert(
                "target_task_policy".to_string(),
                Value::String(String::new()),
            );
        }
    }
    normalize_output_contract_for_schema(obj);
    serde_json::to_string(&value).unwrap_or_else(|_| raw.to_string())
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
        "mode",
        "output_contract",
        "execution_recipe",
        "direct_reply_candidate",
        "direct_reply_confidence",
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
    normalize_output_contract_for_schema(&mut obj);
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| raw.to_string())
}

fn normalize_intent_normalizer_scalar_types_for_schema(obj: &mut serde_json::Map<String, Value>) {
    if obj
        .get("clarify_question")
        .is_some_and(|value| value.is_null())
    {
        obj.insert("clarify_question".to_string(), Value::String(String::new()));
    }
    normalize_confidence_field(obj, "confidence");
}

fn normalize_schema_token(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .trim_matches('_')
        .to_string()
}

fn normalize_output_response_shape_for_schema(raw: &str) -> &'static str {
    match normalize_schema_token(raw).as_str() {
        "one_sentence" | "single_sentence" | "sentence" | "short_sentence" => "one_sentence",
        "strict" | "exact" | "exact_text" | "strict_text" | "exact_format" => "strict",
        "scalar" | "value" | "value_only" | "single_value" | "field_value" => "scalar",
        "file_token" | "file" | "delivery_token" => "file_token",
        // Model-side shape descriptions such as object/list/inline are not runtime
        // answer contracts. Preserve the request as executable and let the planner
        // produce the requested final form instead of failing schema validation.
        _ => "free",
    }
}

fn normalize_output_locator_kind_for_schema(raw: &str) -> &'static str {
    match normalize_schema_token(raw).as_str() {
        "path" | "file_path" | "directory" | "dir" => "path",
        "current_workspace" | "workspace" | "repo" | "repository" => "current_workspace",
        "url" | "uri" | "link" => "url",
        "filename" | "file_name" | "basename" => "filename",
        _ => "none",
    }
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
        "file_names" | "names_only" | "entry_names" | "directory_entry_names" => {
            OutputSemanticKind::FileNames.as_str()
        }
        "value_only" | "file_field_value" | "field_value" => OutputSemanticKind::None.as_str(),
        normalized => parse_output_semantic_kind(normalized).as_str(),
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
    obj.entry("resume_behavior".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    obj.entry("schedule_kind".to_string())
        .or_insert_with(|| Value::String("none".to_string()));
    obj.entry("schedule_intent".to_string())
        .or_insert(Value::Null);
    obj.entry("wants_file_delivery".to_string())
        .or_insert(Value::Bool(false));
    obj.entry("should_refresh_long_term_memory".to_string())
        .or_insert(Value::Bool(false));
    obj.entry("agent_display_name_hint".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("needs_clarify".to_string())
        .or_insert(Value::Bool(false));
    obj.entry("clarify_question".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("reason".to_string())
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("confidence".to_string())
        .or_insert_with(|| Value::from(0.8));
    obj.entry("mode".to_string())
        .or_insert_with(|| Value::String("chat".to_string()));
    if let Some(mode) = obj.get("mode").and_then(|v| v.as_str()) {
        let normalized = mode.trim().to_ascii_lowercase();
        let canonical = match normalized.as_str() {
            "chat" | "act" | "chat_act" | "ask_clarify" => None,
            "clarify" | "ask" => Some("ask_clarify"),
            "recall" | "memory_recall" | "memory" | "memory_storage" | "save_context"
            | "confirmation" | "respond" | "response" => Some("chat"),
            _ => Some("chat"),
        };
        if let Some(canonical) = canonical {
            obj.insert("mode".to_string(), Value::String(canonical.to_string()));
        }
    }
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
    obj.entry("state_patch".to_string()).or_insert(Value::Null);
    obj.entry("attachment_processing_required".to_string())
        .or_insert(Value::Bool(false));
}

fn normalize_output_contract_for_schema(obj: &mut serde_json::Map<String, Value>) {
    let Some(contract) = obj
        .get_mut("output_contract")
        .and_then(|v| v.as_object_mut())
    else {
        return;
    };
    contract.retain(|key, _| {
        matches!(
            key.as_str(),
            "response_shape"
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
    contract
        .entry("requires_content_evidence".to_string())
        .or_insert(Value::Bool(false));
    contract
        .entry("delivery_required".to_string())
        .or_insert(Value::Bool(false));
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
    contract.insert(
        "semantic_kind".to_string(),
        Value::String(semantic_kind.to_string()),
    );
    contract
        .entry("locator_hint".to_string())
        .or_insert_with(|| Value::String(String::new()));
    let self_extension = contract
        .entry("self_extension".to_string())
        .or_insert_with(|| {
            serde_json::json!({
                "mode": "none",
                "trigger": "none",
                "execute_now": false
            })
        });
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
    let head_take = (max_bytes * 45) / 100;
    let tail_take = (max_bytes * 45) / 100;
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
    format!("{head}{note}{tail}")
}

/// Unified intent normalizer: one LLM call for resume decision + intent completion + schedule classification + needs_clarify + routed_mode.
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
            let fallback = empty_ask_clarify_decision(req, "normalizer_prompt_missing");
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
    let prompt = if intent_normalizer_uses_compact_prompt(state, task) {
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
            let fallback = empty_ask_clarify_decision(req, "normalizer_llm_failed");
            return normalizer_output_from_fallback(
                req,
                "llm_failed_safe_clarify",
                fallback,
                Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable),
            );
        }
    };
    let llm_out_for_parse = normalize_intent_normalizer_raw_for_schema(&llm_out, req);
    let parsed = crate::prompt_utils::validate_against_schema::<IntentNormalizerOut>(
        &llm_out_for_parse,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    );
    if let Ok(validated) = &parsed {
        if !validated.raw_parse_ok {
            info!(
                "{} intent_normalizer task_id={} parse_recovery=schema_repair input={}",
                crate::highlight_tag("routing"),
                task.task_id,
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
    if let Some(out) = parsed {
        let resolved = out.resolved_user_intent.trim();
        let mut resume_behavior = parse_resume_behavior(&out.resume_behavior);
        if resume_context.is_none() && resume_behavior != ResumeBehavior::None {
            warn!(
                "intent_normalizer override resume_behavior to none: task_id={} raw_resume_behavior={}",
                task.task_id, out.resume_behavior
            );
            resume_behavior = ResumeBehavior::None;
        }
        let schedule_kind = parse_schedule_kind(&out.schedule_kind);
        let confidence = out.confidence.clamp(0.0, 1.0);
        let routed_mode_raw = parse_mode_text(&out.mode).unwrap_or(RoutedMode::AskClarify);
        let mut output_contract =
            parse_output_contract(out.output_contract.clone(), out.wants_file_delivery);
        let mut clarify_question = out.clarify_question.trim().to_string();
        let execution_recipe_hint = parse_execution_recipe_hint(out.execution_recipe.clone());
        let mut routed_mode = crate::post_route_policy::enforce_content_evidence_execution_mode(
            routed_mode_raw,
            &output_contract,
            out.needs_clarify,
        );
        let mut needs_clarify = out.needs_clarify;
        let schedule_intent = normalize_schedule_intent_from_normalizer(
            schedule_kind,
            out.schedule_intent.clone(),
            if resolved.is_empty() { req } else { resolved },
            &out.reason,
            out.needs_clarify,
            &clarify_question,
            confidence,
        );
        let turn_type = parse_turn_type(&out.turn_type);
        let target_task_policy = parse_target_task_policy(&out.target_task_policy);
        let state_patch = out.state_patch.clone().filter(|value| !value.is_null());
        let mut reason = out.reason;
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
        let mut resolved_user_intent = if resolved.is_empty() {
            req.to_string()
        } else {
            resolved.to_string()
        };
        if compare_targets_need_prompt_grounding_override(&req_surface, &output_contract) {
            if let Some((left, right)) = req_surface.compare_target_pair.as_ref() {
                resolved_user_intent = req.to_string();
                output_contract.locator_kind = OutputLocatorKind::Filename;
                output_contract.locator_hint = format!("{left}, {right}");
                if reason.trim().is_empty() {
                    reason = "current_turn_compare_targets_override".to_string();
                } else if !reason.contains("current_turn_compare_targets_override") {
                    reason.push_str("; current_turn_compare_targets_override");
                }
                info!(
                    "{} intent_normalizer task_id={} compare_target_override resolved_user_intent={} locator_hint={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    crate::truncate_for_log(&resolved_user_intent),
                    crate::truncate_for_log(&output_contract.locator_hint),
                );
            }
        }
        if should_resolve_task_scope_update_clarify_with_active_task(
            req,
            session_snapshot,
            turn_type,
            target_task_policy,
            out.attachment_processing_required,
            routed_mode,
        ) {
            needs_clarify = false;
            clarify_question.clear();
            routed_mode = RoutedMode::Chat;
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
        if should_route_active_task_mutation_to_chat(
            req,
            session_snapshot,
            turn_type,
            target_task_policy,
            out.attachment_processing_required,
            routed_mode,
            &output_contract,
        ) {
            routed_mode = RoutedMode::Chat;
            if reason.trim().is_empty() {
                reason = "active_task_mutation_to_chat".to_string();
            } else if !reason.contains("active_task_mutation_to_chat") {
                reason.push_str("; active_task_mutation_to_chat");
            }
            info!(
                "{} intent_normalizer task_id={} turn_analysis_override=active_task_mutation_to_chat input={}",
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
            routed_mode,
        ) {
            needs_clarify = false;
            clarify_question.clear();
            routed_mode = RoutedMode::Chat;
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
            routed_mode,
        ) {
            needs_clarify = false;
            clarify_question.clear();
            routed_mode = RoutedMode::Chat;
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
        if routed_mode != routed_mode_raw {
            info!(
                "{} intent_normalizer task_id={} mode_override={:?} -> {:?} reason=content_evidence_requires_execution locator_kind={:?} shape={:?}",
                crate::highlight_tag("routing"),
                task.task_id,
                routed_mode_raw,
                routed_mode,
                output_contract.locator_kind,
                output_contract.response_shape
            );
        }
        info!(
            "{} intent_normalizer task_id={} input={} resolved_user_intent={} resume_behavior={:?} schedule_kind={:?} mode={:?} wants_file_delivery={} needs_clarify={} reason={} confidence={} output_contract.shape={:?} output_contract.delivery_required={} output_contract.requires_content_evidence={} output_contract.locator_kind={:?} execution_recipe_hint={} turn_analysis={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req),
            crate::truncate_for_log(&resolved_user_intent),
            resume_behavior,
            schedule_kind,
            routed_mode,
            out.wants_file_delivery,
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
            turn_analysis_log,
        );
        // Structural safety guard only: a single path/file token has no action verb, so
        // ask for the missing operation instead of letting the planner repair a non-actionable
        // instruction. This does not classify natural-language intent.
        let bare_path_only = is_bare_path_only_input_for_clarify(req, &req_surface);
        let (needs_clarify_eff, clarify_question_eff) = if !needs_clarify && bare_path_only {
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
        let routed_mode_eff = if needs_clarify_eff && !needs_clarify {
            RoutedMode::AskClarify
        } else {
            routed_mode
        };
        return IntentNormalizerOutput {
            resolved_user_intent,
            resume_behavior,
            schedule_kind,
            schedule_intent,
            wants_file_delivery: out.wants_file_delivery,
            should_refresh_long_term_memory: out.should_refresh_long_term_memory,
            agent_display_name_hint: out.agent_display_name_hint.trim().to_string(),
            needs_clarify: needs_clarify_eff,
            clarify_question: clarify_question_eff,
            reason,
            confidence,
            output_contract,
            execution_recipe_hint,
            routed_mode: routed_mode_eff,
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
    let fallback = empty_ask_clarify_decision(req, "normalizer_parse_failed");
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
        || surface.has_workspace_single_token_hint()
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
fn empty_ask_clarify_decision(user_request: &str, reason: &str) -> RouteDecision {
    RouteDecision {
        mode: RoutedMode::AskClarify,
        resolved_user_intent: user_request.trim().to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        reason: reason.to_string(),
        confidence: None,
        evidence_refs: Vec::new(),
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
                return crate::fallback::render_clarify_fallback(
                    state,
                    &task.task_id,
                    crate::fallback::ClarifyFallbackSource::LlmUnavailable,
                    Some(&err.to_string()),
                );
            }
        };
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
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
                crate::fallback::render_clarify_fallback(
                    state,
                    &task.task_id,
                    crate::fallback::ClarifyFallbackSource::EmptyResponse,
                    None,
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
            crate::fallback::render_clarify_fallback(
                state,
                &task.task_id,
                crate::fallback::ClarifyFallbackSource::LlmUnavailable,
                Some(&hint),
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
    let preferred = preferred_question
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);
    if let Some(question) = preferred {
        return question;
    }
    if matches!(policy, ClarifyQuestionPolicy::SafeFallback)
        && !safe_fallback_source_should_try_llm(default_source)
    {
        return crate::fallback::render_clarify_fallback(
            state,
            &task.task_id,
            default_source,
            None,
        );
    }
    if matches!(policy, ClarifyQuestionPolicy::SafeFallback) {
        let request_language_hint =
            crate::language_policy::task_response_language_hint(state, task, user_request);
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
        return crate::fallback::compose_user_response_from_contract(
            state,
            task,
            &contract,
            default_source,
        )
        .await;
    }
    generate_clarify_question(
        state,
        task,
        user_request,
        resolver_reason,
        candidate_context,
    )
    .await
}

fn safe_fallback_source_should_try_llm(source: crate::fallback::ClarifyFallbackSource) -> bool {
    !matches!(
        source,
        crate::fallback::ClarifyFallbackSource::LlmUnavailable
    )
}

/// Parses normalizer mode string. chat_act is secondary: only when user explicitly asked for action + narrated summary.
fn parse_mode_text(raw: &str) -> Option<RoutedMode> {
    let mode_text = raw.trim().to_ascii_lowercase();
    if mode_text.contains("ask_clarify") {
        return Some(RoutedMode::AskClarify);
    }
    if mode_text.contains("chat_act") || mode_text.contains("chat+act") {
        return Some(RoutedMode::ChatAct);
    }
    if mode_text.contains("\"act\"") || mode_text == "act" {
        return Some(RoutedMode::Act);
    }
    if mode_text.contains("\"chat\"") || mode_text == "chat" {
        return Some(RoutedMode::Chat);
    }
    None
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
        normalizer_output_from_fallback, parse_execution_recipe_hint, ClarifyQuestionPolicy,
        IntentExecutionRecipeOut, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
        OutputResponseShape, OutputSemanticKind, RouteDecision, TargetTaskPolicy, TurnType,
    };
    use crate::{
        execution_recipe::{
            ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeTargetScope,
        },
        RoutedMode,
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
    fn normalizer_schema_normalization_maps_recall_mode_to_chat() {
        let raw = r#"{"resolved_user_intent":"client-like-continuous-123","needs_clarify":false,"clarify_question":"","reason":"recent memory recall","confidence":1.0,"mode":"recall"}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("chat"));
        assert_eq!(
            value.get("resolved_user_intent").and_then(|v| v.as_str()),
            Some("client-like-continuous-123")
        );
    }

    #[test]
    fn normalizer_schema_normalization_preserves_object_resolved_intent() {
        let raw = r#"{"resolved_user_intent":{"test_id":"client-like-continuous-123"},"needs_clarify":false,"mode":"chat"}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(
            value.get("resolved_user_intent").and_then(|v| v.as_str()),
            Some(r#"{"test_id":"client-like-continuous-123"}"#)
        );
    }

    #[test]
    fn normalizer_schema_normalization_accepts_percent_confidence() {
        let raw = r#"{"resolved_user_intent":"检查当前目录隐藏文件","needs_clarify":false,"clarify_question":"","reason":"local inspection","confidence":100,"mode":"act"}"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(value.get("confidence").and_then(|v| v.as_f64()), Some(1.0));
        assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("act"));
    }

    #[test]
    fn normalizer_schema_normalization_coerces_model_contract_synonyms() {
        let raw = r#"{
          "resolved_user_intent": {"action":"find_file","target":"rustclaw.service","scope":"repository"},
          "needs_clarify": false,
          "reason": "repo inspection",
          "confidence": 0.9,
          "mode": "chat_act",
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
            Some("free")
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
    fn compact_normalizer_prompt_pins_output_contract_schema() {
        let route_view = crate::task_context_builder::RouteContextView {
            request_surface_hints: "workspace_root_request_shape=CurrentPathScalar".to_string(),
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
        assert!(prompt
            .contains("Allowed response_shape: free, one_sentence, strict, scalar, file_token"));
        assert!(prompt.contains("Allowed semantic_kind: none, raw_command_output"));
        assert!(prompt.contains("Do not emit exact_format, required_evidence, fields"));
        assert!(prompt.contains("instead of inventing enum values"));
        assert!(prompt.contains("Use mode=\"chat_act\" when the request both inspects"));
    }

    #[test]
    fn normalizer_schema_normalization_coerces_hidden_files_check_synonym() {
        let raw = r#"{
          "resolved_user_intent": "检查当前目录是否存在隐藏文件并提供3个示例",
          "needs_clarify": false,
          "reason": "local hidden entries check",
          "confidence": 1.0,
          "mode": "act",
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
        assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("act"));
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
            Some("free")
        );
        crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
            &normalized,
            crate::prompt_utils::PromptSchemaId::IntentNormalizer,
        )
        .expect("schema validation");
    }

    #[test]
    fn normalizer_schema_normalization_does_not_invent_contract_from_surface() {
        let raw = r#"{
          "resolved_user_intent": "检查当前目录是否有隐藏文件，如有则列出3个例子",
          "needs_clarify": false,
          "reason": "local hidden entries check",
          "confidence": 0.98,
          "mode": "act"
        }"#;
        let normalized = super::normalize_intent_normalizer_raw_for_schema(
            raw,
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
        );
        let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
        assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("act"));
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
  "mode": "act",
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
        assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("act"));
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
          "mode": "act",
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
        assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("act"));
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
            Some("free")
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
        // 反例：历史版本里返回 None 会让下游 fallback 到 keyword 启发式；
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
    fn fallback_normalizer_output_still_enforces_content_evidence_execution_mode() {
        let out = normalizer_output_from_fallback(
            "把当前目录有没有隐藏文件看一下",
            "parse_failed_fallback_router",
            RouteDecision {
                mode: RoutedMode::Chat,
                resolved_user_intent: "看一下当前目录有没有隐藏文件".to_string(),
                needs_clarify: false,
                clarify_question: String::new(),
                reason: "current workspace executable request".to_string(),
                confidence: Some(0.72),
                evidence_refs: Vec::new(),
                schedule_kind: super::ScheduleKind::None,
                schedule_intent: None,
                wants_file_delivery: false,
                should_refresh_long_term_memory: false,
                agent_display_name_hint: String::new(),
                output_contract: IntentOutputContract {
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
        assert_eq!(out.routed_mode, RoutedMode::Act);
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
                mode: RoutedMode::AskClarify,
                resolved_user_intent: String::new(),
                needs_clarify: true,
                clarify_question: String::new(),
                reason: "fallback_router_llm_failed".to_string(),
                confidence: None,
                evidence_refs: Vec::new(),
                schedule_kind: super::ScheduleKind::None,
                schedule_intent: None,
                wants_file_delivery: false,
                should_refresh_long_term_memory: false,
                agent_display_name_hint: String::new(),
                output_contract: IntentOutputContract::default(),
            },
            Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable),
        );
        assert_eq!(out.routed_mode, RoutedMode::AskClarify);
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
                RoutedMode::AskClarify,
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
                RoutedMode::AskClarify,
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
            RoutedMode::AskClarify,
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
            RoutedMode::AskClarify,
        ));
    }

    #[test]
    fn active_task_scope_update_is_routed_back_to_chat() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_route_active_task_mutation_to_chat(
            "先只看登录模块",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            RoutedMode::Act,
            &IntentOutputContract::default(),
        ));
    }

    #[test]
    fn active_task_scope_update_en_is_routed_back_to_chat_from_chat_act() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                last_primary_task_prompt: Some("Help me create a test plan".to_string()),
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(super::should_route_active_task_mutation_to_chat(
            "Only focus on the login module first",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            RoutedMode::ChatAct,
            &IntentOutputContract::default(),
        ));
    }

    #[test]
    fn active_task_output_table_refinement_is_routed_back_to_chat() {
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
        assert!(super::should_route_active_task_mutation_to_chat(
            "把结果改成 markdown table 输出",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            RoutedMode::Act,
            &IntentOutputContract::default(),
        ));
    }

    #[test]
    fn active_task_correct_is_routed_back_to_chat() {
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
        assert!(super::should_route_active_task_mutation_to_chat(
            "Correction: not Python 3.10, use Python 3.11",
            Some(&snapshot),
            Some(TurnType::TaskCorrect),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            RoutedMode::Act,
            &IntentOutputContract::default(),
        ));
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
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            ..IntentOutputContract::default()
        };
        assert!(!super::should_route_active_task_mutation_to_chat(
            "Focus only on the UI part",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            RoutedMode::ChatAct,
            &contract,
        ));
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
            RoutedMode::AskClarify,
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
            RoutedMode::AskClarify,
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
            RoutedMode::AskClarify,
        ));
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
    fn compare_target_pair_still_counts_as_fileish_cue() {
        let surface = crate::intent::surface_signals::analyze_prompt_surface(
            "比较 README.md 和 AGENTS.md 哪个更大",
        );
        assert!(super::prompt_has_concrete_fileish_cue(&surface));
    }

    #[test]
    fn compare_target_override_detects_path_drift_from_memory() {
        let request_surface = crate::intent::surface_signals::analyze_prompt_surface(
            "比较 README.md 和 AGENTS.md 哪个更大，再用一句通俗话解释原因",
        );
        let output_contract = IntentOutputContract {
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md"
                .to_string(),
            ..IntentOutputContract::default()
        };
        assert!(super::compare_targets_need_prompt_grounding_override(
            &request_surface,
            &output_contract,
        ));
    }

    #[test]
    fn compare_target_override_skips_prompt_grounded_filename_pair() {
        let request_surface = crate::intent::surface_signals::analyze_prompt_surface(
            "比较 README.md 和 AGENTS.md 哪个更大，再用一句通俗话解释原因",
        );
        let output_contract = IntentOutputContract {
            locator_kind: OutputLocatorKind::Filename,
            locator_hint: "AGENTS.md, README.md".to_string(),
            ..IntentOutputContract::default()
        };
        assert!(!super::compare_targets_need_prompt_grounding_override(
            &request_surface,
            &output_contract,
        ));
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
            "resume_behavior",
            "schedule_kind",
            "wants_file_delivery",
            "should_refresh_long_term_memory",
            "agent_display_name_hint",
            "needs_clarify",
            "clarify_question",
            "reason",
            "confidence",
            "mode",
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

        // mode：parse_mode_text 是 substring 匹配，没匹配返回 None。
        for token in enum_strings(&schema, &["properties", "mode"]) {
            if token.is_empty() {
                continue;
            }
            assert!(
                super::parse_mode_text(&token).is_some(),
                "mode token `{}` not recognized by parse_mode_text",
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
