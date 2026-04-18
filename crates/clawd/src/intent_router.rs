//! Intent routing and unified normalizer for ask tasks.
//!
//! **Ask main path:** Only `run_intent_normalizer` is used (resolved intent, resume_behavior,
//! schedule_kind, needs_clarify, routed_mode in one LLM call).
//!
//! **Fallback when normalizer LLM fails / parse fails:** Build an empty `RouteDecision`
//! (mode = AskClarify, empty contract) and feed it to `normalizer_output_from_fallback`,
//! which internally consults `deterministic_fallback_route_decision` to recover a routed mode
//! from the user request without making another LLM call. The legacy `intent_router_prompt`
//! second-LLM path was removed in Phase 2.7 (Phase 1.5 telemetry: legacy router rescued only
//! ~3% of normalizer failures while doubling latency on the unhappy path).

use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::{
    llm_gateway, schedule_service, AppState, ClaimedTask, RiskCeiling, RouteResult, RoutedMode,
};

pub(crate) use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, ScheduleKind, SelfExtensionContract, SelfExtensionMode,
    SelfExtensionTrigger,
};

const CLARIFY_QUESTION_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/clarify_question_prompt.md");
const CLARIFY_QUESTION_PROMPT_LOGICAL_PATH: &str = "prompts/clarify_question_prompt.md";
const INTENT_NORMALIZER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/intent_normalizer_prompt.md");
const INTENT_NORMALIZER_PROMPT_LOGICAL_PATH: &str = "prompts/intent_normalizer_prompt.md";
const ROUTING_POLICY_PERSONA_PROMPT: &str = "Neutral routing policy classifier. Ignore style/persona preferences and optimize for correct intent resolution, clarification, and guard decisions.";

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
    /// Phase 1.5: `chat` 模式下 normalizer 可以顺手给出直接回复候选，
    /// 命中 4 条护栏就可以跳过第二次 LLM（`chat_response_prompt`）。
    /// 未命中或为空时走原链路，无损回退。
    pub(crate) direct_reply_candidate: String,
    pub(crate) direct_reply_confidence: f64,
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
        direct_reply_candidate: normalizer_out.direct_reply_candidate.clone(),
        direct_reply_confidence: normalizer_out.direct_reply_confidence,
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
    direct_reply_candidate: String,
    #[serde(default)]
    direct_reply_confidence: f64,
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

fn parse_output_semantic_kind(s: &str) -> OutputSemanticKind {
    match s.trim().to_ascii_lowercase().as_str() {
        "raw_command_output" | "raw_output" | "command_output" => {
            OutputSemanticKind::RawCommandOutput
        }
        "service_status" => OutputSemanticKind::ServiceStatus,
        "hidden_entries_check" => OutputSemanticKind::HiddenEntriesCheck,
        "directory_purpose_summary" | "listing_purpose_summary" | "directory_listing_summary" => {
            OutputSemanticKind::DirectoryPurposeSummary
        }
        "content_excerpt_summary" | "document_excerpt_summary" | "file_excerpt_summary" => {
            OutputSemanticKind::ContentExcerptSummary
        }
        "recent_artifacts_judgment" | "artifact_style_classification" => {
            OutputSemanticKind::RecentArtifactsJudgment
        }
        "workspace_project_summary" | "project_overview" | "workspace_overview_summary" => {
            OutputSemanticKind::WorkspaceProjectSummary
        }
        "scalar_count" | "count" => OutputSemanticKind::ScalarCount,
        "quantity_comparison" | "comparison" => OutputSemanticKind::QuantityComparison,
        "scalar_path_only" | "path_only" => OutputSemanticKind::ScalarPathOnly,
        "existence_with_path" | "exists_with_path" => OutputSemanticKind::ExistenceWithPath,
        "recent_scalar_equality_check" | "same_or_different" | "equality_check" => {
            OutputSemanticKind::RecentScalarEqualityCheck
        }
        _ => OutputSemanticKind::None,
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

fn parse_execution_recipe_hint(
    out: Option<IntentExecutionRecipeOut>,
) -> Option<crate::execution_recipe::ExecutionRecipeSpec> {
    // 关键语义（B1 修复）：
    //   - `out == None`           → normalizer 没在响应里给出 execution_recipe 字段，
    //                               说明 LLM 没决断；下游应 fallback 到 keyword detect。
    //   - `out == Some` 且 kind != none → normalizer 显式给出 ops loop spec，照用。
    //   - `out == Some` 且 kind == none → normalizer 显式说"这不是 ops loop"，
    //                               同样应被信任。返回 Some(default spec)（kind=None,
    //                               runtime.is_active()=false），让下游知道 normalizer
    //                               已分类过，不要再用 keyword detect 误升级。
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

fn trim_fallback_locator_token(token: &str) -> String {
    token
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
        .to_string()
}

fn extract_explicit_path_token_for_fallback(user_request: &str) -> Option<String> {
    user_request
        .split_whitespace()
        .filter_map(|token| {
            let trimmed = trim_fallback_locator_token(token);
            let candidate = trimmed
                .split(|ch: char| {
                    matches!(
                        ch,
                        ',' | '，'
                            | '。'
                            | ':'
                            | '：'
                            | ';'
                            | '；'
                            | ')'
                            | '）'
                            | ']'
                            | '}'
                            | '>'
                            | '》'
                    )
                })
                .next()
                .unwrap_or_default()
                .trim();
            (!candidate.is_empty()).then(|| candidate.to_string())
        })
        .find(|token| crate::worker::has_explicit_path_or_url_locator_hint(token))
}

fn request_contains_inline_json_payload(user_request: &str) -> bool {
    crate::extract_first_json_value_any(user_request).is_some()
}

fn request_looks_like_inline_structured_transform(user_request: &str) -> bool {
    let lower = user_request.trim().to_ascii_lowercase();
    request_contains_inline_json_payload(user_request)
        && ["json", "sort", "markdown", "table", "render", "convert"]
            .iter()
            .any(|needle| lower.contains(needle))
}

fn request_wants_file_delivery(user_request: &str) -> bool {
    let lower = user_request.trim().to_ascii_lowercase();
    let zh = user_request.trim();
    [
        "send me",
        "send it",
        "deliver",
        "attach",
        "upload",
        "as a file",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || ["发给我", "发我", "直接发文件", "别贴正文", "作为文件"]
            .iter()
            .any(|needle| zh.contains(needle))
}

fn request_prefers_one_sentence(user_request: &str) -> bool {
    let lower = user_request.trim().to_ascii_lowercase();
    let raw = user_request.trim();
    [
        "one sentence",
        "single sentence",
        "keep it brief",
        "briefly",
        "brief ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || ["一句话", "一句大白话", "简短", "简要", "简洁", "brief"]
            .iter()
            .any(|needle| raw.contains(needle))
}

fn request_prefers_scalar(user_request: &str) -> bool {
    let lower = user_request.trim().to_ascii_lowercase();
    let raw = user_request.trim();
    let list_or_table = ["markdown table", "table", "list", "列表", "表格"]
        .iter()
        .any(|needle| lower.contains(needle) || raw.contains(needle));
    if list_or_table {
        return false;
    }
    [
        "output only",
        "return only",
        "only the name field",
        "only the branch",
        "only the result",
        "just the value",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || ["只输出", "只返回", "只给结果", "只回答", "只回", "只输出值"]
            .iter()
            .any(|needle| raw.contains(needle))
}

fn fallback_response_shape(user_request: &str, delivery_required: bool) -> OutputResponseShape {
    if delivery_required {
        OutputResponseShape::FileToken
    } else if request_prefers_scalar(user_request) {
        OutputResponseShape::Scalar
    } else if request_prefers_one_sentence(user_request) {
        OutputResponseShape::OneSentence
    } else {
        OutputResponseShape::Free
    }
}

fn deterministic_fallback_route_decision(user_request: &str) -> Option<RouteDecision> {
    let trimmed = user_request.trim();
    if trimmed.is_empty() {
        return None;
    }

    if request_looks_like_inline_structured_transform(trimmed) {
        return Some(RouteDecision {
            mode: RoutedMode::Act,
            resolved_user_intent: trimmed.to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: "deterministic_inline_structured_transform".to_string(),
            confidence: Some(0.4),
            evidence_refs: Vec::new(),
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: SelfExtensionContract::default(),
            },
        });
    }

    let explicit_path = extract_explicit_path_token_for_fallback(trimmed)?;
    let delivery_required = request_wants_file_delivery(trimmed);
    let response_shape = fallback_response_shape(trimmed, delivery_required);
    let requires_content_evidence = !delivery_required;
    let semantic_kind = if requires_content_evidence
        && matches!(
            response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        ) {
        OutputSemanticKind::ContentExcerptSummary
    } else {
        OutputSemanticKind::None
    };
    let routed_mode = if delivery_required
        || matches!(
            response_shape,
            OutputResponseShape::Scalar | OutputResponseShape::FileToken
        ) {
        RoutedMode::Act
    } else {
        RoutedMode::ChatAct
    };
    Some(RouteDecision {
        mode: routed_mode,
        resolved_user_intent: trimmed.to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "deterministic_explicit_locator_fallback".to_string(),
        confidence: Some(0.45),
        evidence_refs: Vec::new(),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: delivery_required,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape,
            requires_content_evidence,
            delivery_required,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: if delivery_required {
                OutputDeliveryIntent::FileSingle
            } else {
                OutputDeliveryIntent::None
            },
            semantic_kind,
            locator_hint: explicit_path,
            self_extension: SelfExtensionContract::default(),
        },
    })
}

fn normalizer_output_from_fallback(
    user_request: &str,
    fallback_reason_prefix: &str,
    decision: RouteDecision,
) -> IntentNormalizerOutput {
    let fallback_contract_is_empty = matches!(
        decision.output_contract.response_shape,
        OutputResponseShape::Free
    ) && !decision.output_contract.requires_content_evidence
        && !decision.output_contract.delivery_required
        && matches!(
            decision.output_contract.locator_kind,
            OutputLocatorKind::None
        )
        && matches!(
            decision.output_contract.delivery_intent,
            OutputDeliveryIntent::None
        )
        && matches!(
            decision.output_contract.semantic_kind,
            OutputSemanticKind::None
        )
        && decision.output_contract.locator_hint.trim().is_empty();
    let decision = if decision.needs_clarify
        && matches!(decision.mode, RoutedMode::AskClarify)
        && fallback_contract_is_empty
    {
        deterministic_fallback_route_decision(user_request).unwrap_or(decision)
    } else {
        decision
    };
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
        direct_reply_candidate: String::new(),
        direct_reply_confidence: 0.0,
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

/// Unified intent normalizer: one LLM call for resume decision + intent completion + schedule classification + needs_clarify + routed_mode.
pub(crate) async fn run_intent_normalizer(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resume_context: Option<&Value>,
    binding_context: Option<&Value>,
    now_iso: &str,
    timezone: &str,
    schedule_rules: &str,
) -> IntentNormalizerOutput {
    let req = user_request.trim();
    let context_bundle = crate::task_context_builder::build_route_task_context_bundle(
        state,
        task,
        user_request,
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
    let resolved_prompt = crate::load_prompt_template_for_state_with_meta(
        state,
        INTENT_NORMALIZER_PROMPT_LOGICAL_PATH,
        INTENT_NORMALIZER_PROMPT_TEMPLATE,
    );
    let prompt_template = resolved_prompt.template;
    let prompt_source = resolved_prompt.source;
    let prompt_version = resolved_prompt.version;
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
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
            ("__REQUEST__", req),
        ],
    );
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
            // Phase 2.7: legacy router second-LLM path removed. Build an empty AskClarify
            // RouteDecision and let `normalizer_output_from_fallback` consult
            // `deterministic_fallback_route_decision` for a no-LLM recovery path.
            warn!(
                "intent_normalizer llm failed, falling back to deterministic recovery: task_id={} err={}",
                task.task_id, err
            );
            let fallback = empty_ask_clarify_decision(req, "normalizer_llm_failed");
            return normalizer_output_from_fallback(req, "llm_failed_deterministic", fallback);
        }
    };
    let trimmed = llm_out.trim();
    let raw_parse_ok = serde_json::from_str::<IntentNormalizerOut>(trimmed).is_ok();
    let parsed =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<IntentNormalizerOut>(&llm_out);
    if !raw_parse_ok && parsed.is_some() {
        info!(
            "{} intent_normalizer task_id={} parse_recovery=extract_or_repair input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req)
        );
    }
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
        let output_contract =
            parse_output_contract(out.output_contract.clone(), out.wants_file_delivery);
        let clarify_question = out.clarify_question.trim().to_string();
        let execution_recipe_hint = parse_execution_recipe_hint(out.execution_recipe.clone());
        let routed_mode = crate::post_route_policy::enforce_content_evidence_execution_mode(
            routed_mode_raw,
            &output_contract,
            out.needs_clarify,
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
            "{} intent_normalizer task_id={} input={} resolved_user_intent={} resume_behavior={:?} schedule_kind={:?} mode={:?} wants_file_delivery={} needs_clarify={} reason={} confidence={} output_contract.shape={:?} output_contract.delivery_required={} output_contract.requires_content_evidence={} output_contract.locator_kind={:?} execution_recipe_hint={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req),
            crate::truncate_for_log(resolved),
            resume_behavior,
            schedule_kind,
            routed_mode,
            out.wants_file_delivery,
            out.needs_clarify,
            crate::truncate_for_log(&out.reason),
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
                .unwrap_or_else(|| "none".to_string())
        );
        return IntentNormalizerOutput {
            resolved_user_intent: if resolved.is_empty() {
                req.to_string()
            } else {
                resolved.to_string()
            },
            resume_behavior,
            schedule_kind,
            schedule_intent,
            wants_file_delivery: out.wants_file_delivery,
            should_refresh_long_term_memory: out.should_refresh_long_term_memory,
            agent_display_name_hint: out.agent_display_name_hint.trim().to_string(),
            needs_clarify: out.needs_clarify,
            clarify_question,
            reason: out.reason,
            confidence,
            output_contract,
            execution_recipe_hint,
            routed_mode,
            direct_reply_candidate: out.direct_reply_candidate.trim().to_string(),
            direct_reply_confidence: out.direct_reply_confidence,
        };
    }
    warn!(
        "intent_normalizer parse failed, falling back to deterministic recovery: task_id={} raw={}",
        task.task_id,
        crate::truncate_for_log(&llm_out)
    );
    // Phase 2.7: legacy router second-LLM removed; rely on deterministic fallback inside
    // `normalizer_output_from_fallback`.
    let _ = (resume_context, binding_context);
    let fallback = empty_ask_clarify_decision(req, "normalizer_parse_failed");
    normalizer_output_from_fallback(req, "parse_failed_deterministic", fallback)
}

/// Fallback `RouteDecision` used when normalizer LLM fails or its output cannot be parsed.
/// Marked `AskClarify` + empty contract so that `normalizer_output_from_fallback` will
/// invoke `deterministic_fallback_route_decision(user_request)` to recover a usable
/// routed mode without making a second LLM call.
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
    let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
        state,
        CLARIFY_QUESTION_PROMPT_LOGICAL_PATH,
        CLARIFY_QUESTION_PROMPT_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__REQUEST__", user_request.trim()),
            ("__RESOLVER_REASON__", resolver_reason.trim()),
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
                crate::i18n_t_with_default(
                    state,
                    "clawd.msg.clarify_question_fallback",
                    "I need to clarify: what task is this message about? Please provide the target or context.",
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
            crate::i18n_t_with_default(
                state,
                "clawd.msg.clarify_question_fallback",
                "I need to clarify: what task is this message about? Please provide the target or context.",
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
) -> String {
    let preferred = preferred_question
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);
    if let Some(question) = preferred {
        return question;
    }
    if matches!(policy, ClarifyQuestionPolicy::SafeFallback) {
        return crate::i18n_t_with_default(
            state,
            "clawd.msg.clarify_question_fallback",
            "I need to clarify: what task is this message about? Please provide the target or context.",
        );
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
        OutputResponseShape, OutputSemanticKind, RouteDecision,
    };
    use crate::{
        execution_recipe::{ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeTargetScope},
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
    fn parse_execution_recipe_hint_missing_profile_falls_back_to_default_spec() {
        // 历史语义：profile 缺失 → None（让下游 fallback 到 keyword detect）
        // B1 修复后：normalizer 显式回了 execution_recipe 字段（即使 profile 缺）就视为
        // 已分类，返回 default spec（kind=None, inactive），不再 fallback 到 keyword。
        // 这样可以避免 keyword detect 因 STABLE_FACTS 污染而误升级 read-only 任务。
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
        // 反例：返回 None → 下游 fallback 到 detect_execution_recipe（keyword 启发式）
        // → 长期记忆里残留的 "configs/" "verify" 关键字会把任务误升级为
        // OpsClosedLoop config_change，让 read-only 的 `pwd` 任务跑挂。
        let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
        }))
        .expect("explicit kind=none should still be Some so detect_execution_recipe is bypassed");
        assert_eq!(spec.kind, ExecutionRecipeKind::None);
        assert!(
            !crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(spec).is_active(),
            "default spec must produce an inactive runtime state"
        );
    }

    #[test]
    fn parse_execution_recipe_hint_missing_field_falls_back_to_keyword_detect() {
        // 当 normalizer 完全没在响应里给出 execution_recipe 字段时（None），
        // 下游应该 fallback 到 keyword detect。这是历史行为，需要保留。
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
        );
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert!(!out.needs_clarify);
        assert_eq!(
            out.output_contract.locator_kind,
            OutputLocatorKind::CurrentWorkspace
        );
    }

    #[test]
    fn fallback_normalizer_repairs_explicit_relative_path_scalar_read_after_router_failure() {
        let out = normalizer_output_from_fallback(
            "read scripts/nl_tests/fixtures/device_local/package.json and output only the name field",
            "llm_failed_fallback_router",
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
        );
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert!(!out.needs_clarify);
        assert_eq!(
            out.output_contract.response_shape,
            OutputResponseShape::Scalar
        );
        assert_eq!(out.output_contract.locator_kind, OutputLocatorKind::Path);
        assert_eq!(
            out.output_contract.locator_hint,
            "scripts/nl_tests/fixtures/device_local/package.json"
        );
    }

    #[test]
    fn fallback_normalizer_repairs_explicit_relative_path_summary_after_router_failure() {
        let out = normalizer_output_from_fallback(
            "看一下 scripts/nl_tests/fixtures/device_local/configs/app_config.toml，然后用一句大白话说它主要配置了什么",
            "llm_failed_fallback_router",
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
        );
        assert_eq!(out.routed_mode, RoutedMode::ChatAct);
        assert!(!out.needs_clarify);
        assert_eq!(
            out.output_contract.response_shape,
            OutputResponseShape::OneSentence
        );
        assert_eq!(
            out.output_contract.semantic_kind,
            OutputSemanticKind::ContentExcerptSummary
        );
        assert_eq!(out.output_contract.locator_kind, OutputLocatorKind::Path);
    }

    #[test]
    fn fallback_normalizer_repairs_inline_json_transform_after_router_failure() {
        let out = normalizer_output_from_fallback(
            r#"sort this JSON array by score descending and render it as a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#,
            "llm_failed_fallback_router",
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
        );
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert!(!out.needs_clarify);
        assert_eq!(out.output_contract.locator_kind, OutputLocatorKind::None);
        assert_eq!(
            out.output_contract.response_shape,
            OutputResponseShape::Free
        );
    }

    #[test]
    fn clarify_question_policy_defaults_to_allow_model() {
        assert_eq!(
            ClarifyQuestionPolicy::default(),
            ClarifyQuestionPolicy::AllowModel
        );
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
        let schema: serde_json::Value =
            serde_json::from_str(SCHEMA_RAW).expect("intent_normalizer.schema.json must be valid JSON");
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
            "direct_reply_candidate",
            "direct_reply_confidence",
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
                node = node.get(*p).unwrap_or_else(|| {
                    panic!("schema path `{}` not found", path.join("."))
                });
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

        for token in enum_strings(&schema, &["properties", "output_contract", "properties", "response_shape"]) {
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
        for token in enum_strings(&schema, &["properties", "output_contract", "properties", "locator_kind"]) {
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
        for token in enum_strings(&schema, &["properties", "output_contract", "properties", "delivery_intent"]) {
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
        for token in enum_strings(&schema, &["properties", "output_contract", "properties", "semantic_kind"]) {
            if token.is_empty() || token == "none" {
                continue;
            }
            assert_ne!(
                super::parse_output_semantic_kind(&token),
                OutputSemanticKind::None,
                "semantic_kind `{}` not recognized",
                token
            );
        }
        for token in enum_strings(&schema, &["properties", "output_contract", "properties", "self_extension", "properties", "mode"]) {
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
        for token in enum_strings(&schema, &["properties", "output_contract", "properties", "self_extension", "properties", "trigger"]) {
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

        for token in enum_strings(&schema, &["properties", "execution_recipe", "properties", "kind"]) {
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
        for token in enum_strings(&schema, &["properties", "execution_recipe", "properties", "profile"]) {
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
        for token in enum_strings(&schema, &["properties", "execution_recipe", "properties", "target_scope"]) {
            if token.is_empty() || token == "unknown" {
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
}
