//! Intent routing and unified normalizer for ask tasks.
//!
//! **Ask main path:** Only `run_intent_normalizer` is used (resolved intent, resume_behavior,
//! schedule_kind, needs_clarify, routed_mode in one LLM call).
//!
//! **Fallback only (do not wire to main path):** When normalizer did not provide a mode (e.g. parse
//! failure), `route_request_mode` runs a legacy router LLM. Assets used solely by that path:
//! `INTENT_ROUTER_*` / `INTENT_ROUTER_RULES_*`, `ROUTING_POLICY_*`, `RouteDecision`/`RouteDecisionOut`,
//! `parse_route_decision`, and `route_request_mode` itself.

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

// --- Fallback router only (not used when normalizer provides mode) ---
const INTENT_ROUTER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/intent_router_prompt.md");
const INTENT_ROUTER_PROMPT_LOGICAL_PATH: &str = "prompts/intent_router_prompt.md";
const INTENT_ROUTER_RULES_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/intent_router_rules.md");
const INTENT_ROUTER_RULES_LOGICAL_PATH: &str = "prompts/intent_router_rules.md";
const CLARIFY_QUESTION_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/clarify_question_prompt.md");
const CLARIFY_QUESTION_PROMPT_LOGICAL_PATH: &str = "prompts/clarify_question_prompt.md";
const INTENT_NORMALIZER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/intent_normalizer_prompt.md");
const INTENT_NORMALIZER_PROMPT_LOGICAL_PATH: &str = "prompts/intent_normalizer_prompt.md";
const ROUTING_POLICY_PERSONA_PROMPT: &str = "Neutral routing policy classifier. Ignore style/persona preferences and optimize for correct intent resolution, clarification, and guard decisions.";
// --- End fallback-only constants ---

#[derive(Debug)]
struct RouteDecision {
    mode: RoutedMode,
    resolved_user_intent: String,
    needs_clarify: bool,
    clarify_question: String,
    reason: String,
    confidence: Option<f64>,
    evidence_refs: Vec<String>,
    schedule_kind: ScheduleKind,
    schedule_intent: Option<crate::ScheduleIntentOutput>,
    wants_file_delivery: bool,
    should_refresh_long_term_memory: bool,
    agent_display_name_hint: String,
    output_contract: IntentOutputContract,
}

#[derive(Debug, Deserialize)]
struct RouteDecisionOut {
    mode: String,
    #[serde(default)]
    resolved_user_intent: String,
    #[serde(default)]
    needs_clarify: bool,
    #[serde(default)]
    clarify_question: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    evidence_refs: Vec<String>,
    #[serde(default)]
    schedule_kind: String,
    #[serde(default)]
    schedule_intent: Option<crate::ScheduleIntentOutput>,
    #[serde(default)]
    wants_file_delivery: bool,
    #[serde(default)]
    should_refresh_long_term_memory: bool,
    #[serde(default)]
    agent_display_name_hint: String,
    #[serde(default)]
    output_contract: Option<IntentOutputContractOut>,
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
    /// Terminal mode: chat / act / ask_clarify / chat_act. Used to skip the separate router LLM.
    pub(crate) routed_mode: RoutedMode,
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

fn render_self_extension_runtime(state: &AppState) -> String {
    serde_json::to_string_pretty(&json!({
        "enabled": state.self_extension.enabled,
        "auto_on_capability_gap": state.self_extension.auto_on_capability_gap,
        "allow_execute": state.self_extension.allow_execute,
        "allow_package_install": state.self_extension.allow_package_install,
        "allow_permanent_extension": state.self_extension.allow_permanent_extension,
        "allow_runtime_enable": state.self_extension.allow_runtime_enable,
        "supported_modes": ["temporary_fix", "permanent_extension"],
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn wants_file_delivery_from_contract(
    contract: &IntentOutputContract,
    explicit_wants_file_delivery: bool,
) -> bool {
    explicit_wants_file_delivery
        || contract.delivery_required
        || matches!(contract.response_shape, OutputResponseShape::FileToken)
        || matches!(
            contract.delivery_intent,
            OutputDeliveryIntent::FileSingle | OutputDeliveryIntent::DirectoryBatchFiles
        )
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
        routed_mode,
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
    let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
        state,
        INTENT_NORMALIZER_PROMPT_LOGICAL_PATH,
        INTENT_NORMALIZER_PROMPT_TEMPLATE,
    );
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
    crate::log_prompt_render(
        state,
        &task.task_id,
        "intent_normalizer_prompt",
        &prompt_source,
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
            let fallback =
                route_request_fallback(state, task, req, resume_context, binding_context).await;
            warn!(
                    "intent_normalizer llm failed, fallback to legacy router: task_id={} err={} mode={:?} locator_kind={:?} shape={:?}",
                    task.task_id,
                    err,
                    fallback.mode,
                    fallback.output_contract.locator_kind,
                    fallback.output_contract.response_shape
                );
            return normalizer_output_from_fallback(req, "llm_failed_fallback_router", fallback);
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
            "{} intent_normalizer task_id={} input={} resolved_user_intent={} resume_behavior={:?} schedule_kind={:?} mode={:?} wants_file_delivery={} needs_clarify={} reason={} confidence={} output_contract.shape={:?} output_contract.delivery_required={} output_contract.requires_content_evidence={} output_contract.locator_kind={:?}",
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
            output_contract.locator_kind
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
            routed_mode,
        };
    }
    warn!(
        "intent_normalizer parse failed, fallback pass-through: task_id={} raw={}",
        task.task_id,
        crate::truncate_for_log(&llm_out)
    );
    let fallback = route_request_fallback(state, task, req, resume_context, binding_context).await;
    normalizer_output_from_fallback(req, "parse_failed_fallback_router", fallback)
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
                &state.command_intent.default_locale,
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

/// **[FALLBACK]** Used only when normalizer did not provide a mode (e.g. JSON parse failure or legacy entry).
/// Ask main path always passes `Some(normalizer_out.routed_mode)`; this must not be called when normalizer
/// has already run. Do not expand usage; do not wire as primary path.
async fn route_request_fallback(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resume_context: Option<&Value>,
    binding_context: Option<&Value>,
) -> RouteDecision {
    info!(
        "route_request_mode fallback path: normalizer did not provide mode, using legacy router LLM task_id={}",
        task.task_id
    );
    let context_bundle = crate::task_context_builder::build_route_task_context_bundle(
        state,
        task,
        user_request,
        resume_context,
        binding_context,
        "",
        "",
        "",
    );
    let route_view = context_bundle
        .route_view
        .as_ref()
        .expect("route context bundle should include route_view");
    let (prompt_template, prompt_source) = crate::load_prompt_template_for_state(
        state,
        INTENT_ROUTER_PROMPT_LOGICAL_PATH,
        INTENT_ROUTER_PROMPT_TEMPLATE,
    );
    let (rules_template, _) = crate::load_prompt_template_for_state(
        state,
        INTENT_ROUTER_RULES_LOGICAL_PATH,
        INTENT_ROUTER_RULES_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__ROUTING_RULES__", &rules_template),
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
            ("__RECENT_TURNS_FULL__", &route_view.recent_turns_full),
            ("__LAST_TURN_FULL__", &route_view.last_turn_full),
            (
                "__RECENT_ASSISTANT_REPLIES__",
                &route_view.recent_assistant_replies,
            ),
            (
                "__RECENT_EXECUTION_CONTEXT__",
                &route_view.recent_execution_context,
            ),
            ("__MEMORY_CONTEXT__", &route_view.memory_context),
            ("__REQUEST__", user_request.trim()),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "intent_router_prompt",
        &prompt_source,
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
                "route_request_mode llm failed, fallback to ask_clarify: task_id={} err={}",
                task.task_id, err
            );
            return RouteDecision {
                mode: RoutedMode::AskClarify,
                resolved_user_intent: user_request.trim().to_string(),
                needs_clarify: true,
                clarify_question: String::new(),
                reason: "fallback_router_llm_failed".to_string(),
                confidence: None,
                evidence_refs: Vec::new(),
                schedule_kind: ScheduleKind::None,
                schedule_intent: None,
                wants_file_delivery: false,
                should_refresh_long_term_memory: false,
                agent_display_name_hint: String::new(),
                output_contract: IntentOutputContract::default(),
            };
        }
    };

    if let Some(decision) = parse_route_decision(&llm_out) {
        info!(
            "{} route_request_mode llm task_id={} mode={:?} needs_clarify={} confidence={} reason={} locator_kind={:?} shape={:?} delivery_intent={:?} evidence_refs={:?} llm_out={}",
            crate::highlight_tag("routing"),
            task.task_id,
            decision.mode,
            decision.needs_clarify,
            decision.confidence.unwrap_or(-1.0),
            crate::truncate_for_log(&decision.reason),
            decision.output_contract.locator_kind,
            decision.output_contract.response_shape,
            decision.output_contract.delivery_intent,
            decision.evidence_refs,
            crate::truncate_for_log(&llm_out)
        );
        return decision;
    }
    warn!(
        "route_request_mode parse failed, fallback to ask_clarify: task_id={} llm_out={}",
        task.task_id,
        crate::truncate_for_log(&llm_out)
    );
    RouteDecision {
        mode: RoutedMode::AskClarify,
        resolved_user_intent: user_request.trim().to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        reason: "fallback_router_parse_failed".to_string(),
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

pub(crate) async fn route_request_mode(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
) -> RoutedMode {
    route_request_fallback(state, task, user_request, None, None)
        .await
        .mode
}

/// Used only by fallback `route_request_mode` to parse legacy router LLM output.
fn parse_route_decision(raw: &str) -> Option<RouteDecision> {
    let value = crate::parse_llm_json_extract_then_raw::<Value>(raw);
    if let Some(v) = value {
        if let Ok(out) = serde_json::from_value::<RouteDecisionOut>(v.clone()) {
            let mode = parse_mode_text(&out.mode)?;
            let output_contract =
                parse_output_contract(out.output_contract.clone(), out.wants_file_delivery);
            let schedule_kind = parse_schedule_kind(&out.schedule_kind);
            let resolved_user_intent = out.resolved_user_intent.trim().to_string();
            let clarify_question = out.clarify_question.trim().to_string();
            let confidence = out.confidence.map(|c| c.clamp(0.0, 1.0));
            let schedule_intent = normalize_schedule_intent_from_normalizer(
                schedule_kind,
                out.schedule_intent.clone(),
                &resolved_user_intent,
                &out.reason,
                out.needs_clarify || matches!(mode, RoutedMode::AskClarify),
                &clarify_question,
                confidence.unwrap_or(0.0),
            );
            return Some(RouteDecision {
                mode,
                resolved_user_intent,
                needs_clarify: out.needs_clarify || matches!(mode, RoutedMode::AskClarify),
                clarify_question,
                reason: out.reason,
                confidence,
                evidence_refs: out.evidence_refs.into_iter().take(8).collect(),
                schedule_kind,
                schedule_intent,
                wants_file_delivery: wants_file_delivery_from_contract(
                    &output_contract,
                    out.wants_file_delivery,
                ),
                should_refresh_long_term_memory: out.should_refresh_long_term_memory,
                agent_display_name_hint: out.agent_display_name_hint.trim().to_string(),
                output_contract,
            });
        }
        if let Some(mode_text) = v.get("mode").and_then(|m| m.as_str()) {
            let mode = parse_mode_text(mode_text)?;
            return Some(RouteDecision {
                mode,
                resolved_user_intent: String::new(),
                needs_clarify: matches!(mode, RoutedMode::AskClarify),
                clarify_question: String::new(),
                reason: String::new(),
                confidence: None,
                evidence_refs: Vec::new(),
                schedule_kind: ScheduleKind::None,
                schedule_intent: None,
                wants_file_delivery: false,
                should_refresh_long_term_memory: false,
                agent_display_name_hint: String::new(),
                output_contract: IntentOutputContract::default(),
            });
        }
    }

    parse_mode_text(raw).map(|mode| RouteDecision {
        mode,
        resolved_user_intent: String::new(),
        needs_clarify: matches!(mode, RoutedMode::AskClarify),
        clarify_question: String::new(),
        reason: String::new(),
        confidence: None,
        evidence_refs: Vec::new(),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    })
}

/// Parses normalizer/legacy router mode string. chat_act is secondary: only when user explicitly asked for action + narrated summary.
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
        normalizer_output_from_fallback, parse_route_decision, ClarifyQuestionPolicy,
        IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
        OutputSemanticKind, RouteDecision,
    };
    use crate::{RoutedMode, SelfExtensionMode, SelfExtensionTrigger};

    #[test]
    fn fallback_route_parser_keeps_current_workspace_contract() {
        let raw = r#"{
            "mode":"chat_act",
            "resolved_user_intent":"把当前仓库顶层目录和文件列出来，简单分组就行",
            "needs_clarify":false,
            "reason":"self-contained current workspace inspection with grouped narration",
            "confidence":0.82,
            "output_contract":{
                "response_shape":"free",
                "requires_content_evidence":true,
                "delivery_required":false,
                "locator_kind":"current_workspace",
                "delivery_intent":"directory_lookup",
                "locator_hint":""
            }
        }"#;
        let parsed = parse_route_decision(raw).expect("fallback route decision");
        assert_eq!(parsed.mode, RoutedMode::ChatAct);
        assert!(!parsed.needs_clarify);
        assert_eq!(
            parsed.output_contract.locator_kind,
            OutputLocatorKind::CurrentWorkspace
        );
        assert_eq!(
            parsed.output_contract.delivery_intent,
            OutputDeliveryIntent::DirectoryLookup
        );
        assert_eq!(
            parsed.output_contract.response_shape,
            OutputResponseShape::Free
        );
    }

    #[test]
    fn fallback_route_parser_derives_file_delivery_from_contract() {
        let raw = r#"{
            "mode":"act",
            "output_contract":{
                "response_shape":"file_token",
                "requires_content_evidence":false,
                "delivery_required":true,
                "locator_kind":"filename",
                "delivery_intent":"file_single",
                "locator_hint":"README.md"
            }
        }"#;
        let parsed = parse_route_decision(raw).expect("fallback route delivery decision");
        assert_eq!(parsed.mode, RoutedMode::Act);
        assert!(parsed.wants_file_delivery);
        assert_eq!(
            parsed.output_contract.locator_kind,
            OutputLocatorKind::Filename
        );
        assert_eq!(
            parsed.output_contract.delivery_intent,
            OutputDeliveryIntent::FileSingle
        );
        assert_eq!(
            parsed.output_contract.response_shape,
            OutputResponseShape::FileToken
        );
    }

    #[test]
    fn fallback_route_parser_keeps_structured_semantic_hints() {
        let raw = r#"{
            "mode":"chat",
            "resolved_user_intent":"记住以后默认中文，并比较前两个结果是否一样",
            "should_refresh_long_term_memory":true,
            "agent_display_name_hint":"小爪",
            "output_contract":{
                "response_shape":"scalar",
                "requires_content_evidence":false,
                "delivery_required":false,
                "locator_kind":"none",
                "delivery_intent":"none",
                "semantic_kind":"recent_scalar_equality_check",
                "locator_hint":""
            }
        }"#;
        let parsed = parse_route_decision(raw).expect("fallback route semantic decision");
        assert!(parsed.should_refresh_long_term_memory);
        assert_eq!(parsed.agent_display_name_hint, "小爪");
        assert_eq!(
            parsed.output_contract.semantic_kind,
            OutputSemanticKind::RecentScalarEqualityCheck
        );
    }

    #[test]
    fn fallback_route_parser_parses_recent_artifacts_judgment_semantic_hint() {
        let raw = r#"{
            "mode":"chat_act",
            "resolved_user_intent":"列出 logs 目录最近修改的 3 个文件，并判断这些文件更像日志还是正式产物",
            "output_contract":{
                "response_shape":"free",
                "requires_content_evidence":true,
                "delivery_required":false,
                "locator_kind":"path",
                "delivery_intent":"none",
                "semantic_kind":"recent_artifacts_judgment",
                "locator_hint":"logs"
            }
        }"#;
        let parsed = parse_route_decision(raw).expect("fallback route recent artifacts decision");
        assert_eq!(parsed.mode, RoutedMode::ChatAct);
        assert_eq!(
            parsed.output_contract.semantic_kind,
            OutputSemanticKind::RecentArtifactsJudgment
        );
    }

    #[test]
    fn fallback_route_parser_parses_directory_purpose_summary_semantic_hint() {
        let raw = r#"{
            "mode":"chat_act",
            "resolved_user_intent":"列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的",
            "output_contract":{
                "response_shape":"free",
                "requires_content_evidence":true,
                "delivery_required":false,
                "locator_kind":"path",
                "delivery_intent":"none",
                "semantic_kind":"directory_purpose_summary",
                "locator_hint":"docs"
            }
        }"#;
        let parsed =
            parse_route_decision(raw).expect("fallback route directory purpose summary decision");
        assert_eq!(parsed.mode, RoutedMode::ChatAct);
        assert_eq!(
            parsed.output_contract.semantic_kind,
            OutputSemanticKind::DirectoryPurposeSummary
        );
    }

    #[test]
    fn fallback_route_parser_parses_content_excerpt_summary_semantic_hint() {
        let raw = r#"{
            "mode":"chat_act",
            "resolved_user_intent":"读一下 README.md 开头，然后用一句话总结",
            "output_contract":{
                "response_shape":"one_sentence",
                "requires_content_evidence":true,
                "delivery_required":false,
                "locator_kind":"filename",
                "delivery_intent":"none",
                "semantic_kind":"content_excerpt_summary",
                "locator_hint":"README.md"
            }
        }"#;
        let parsed =
            parse_route_decision(raw).expect("fallback route content excerpt summary decision");
        assert_eq!(parsed.mode, RoutedMode::ChatAct);
        assert_eq!(
            parsed.output_contract.semantic_kind,
            OutputSemanticKind::ContentExcerptSummary
        );
    }

    #[test]
    fn fallback_route_parser_parses_workspace_project_summary_semantic_hint() {
        let raw = r#"{
            "mode":"chat_act",
            "resolved_user_intent":"用非技术用户能听懂的话，简短解释当前仓库主要是干什么的",
            "output_contract":{
                "response_shape":"one_sentence",
                "requires_content_evidence":true,
                "delivery_required":false,
                "locator_kind":"current_workspace",
                "delivery_intent":"none",
                "semantic_kind":"workspace_project_summary",
                "locator_hint":""
            }
        }"#;
        let parsed =
            parse_route_decision(raw).expect("fallback route workspace project summary decision");
        assert_eq!(parsed.mode, RoutedMode::ChatAct);
        assert_eq!(
            parsed.output_contract.semantic_kind,
            OutputSemanticKind::WorkspaceProjectSummary
        );
    }

    #[test]
    fn fallback_route_parser_keeps_self_extension_contract() {
        let raw = r#"{
            "mode":"chat",
            "resolved_user_intent":"不要用现有技能，直接写个临时脚本把这个 json 排序后转成 markdown 表格",
            "output_contract":{
                "response_shape":"free",
                "requires_content_evidence":false,
                "delivery_required":false,
                "locator_kind":"none",
                "delivery_intent":"none",
                "semantic_kind":"none",
                "locator_hint":"",
                "self_extension":{
                    "mode":"temporary_fix",
                    "trigger":"explicit_user_request",
                    "execute_now":true
                }
            }
        }"#;
        let parsed = parse_route_decision(raw).expect("fallback route self extension decision");
        assert_eq!(
            parsed.output_contract.self_extension.mode,
            SelfExtensionMode::TemporaryFix
        );
        assert_eq!(
            parsed.output_contract.self_extension.trigger,
            SelfExtensionTrigger::ExplicitUserRequest
        );
        assert!(parsed.output_contract.self_extension.execute_now);
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
    fn fallback_route_parser_keeps_clarify_question_and_schedule_intent() {
        let raw = r#"{
            "mode":"ask_clarify",
            "resolved_user_intent":"每天早上提醒我看邮件",
            "needs_clarify":true,
            "clarify_question":"你希望每天几点提醒？",
            "reason":"missing schedule time",
            "confidence":0.64,
            "schedule_kind":"create",
            "schedule_intent":{
                "kind":"create",
                "timezone":"Asia/Shanghai",
                "schedule":{"type":"","run_at":"","time":"","weekday":0,"every_minutes":0,"cron":""},
                "task":{"kind":"ask","payload":{"prompt":"提醒我看邮件"}},
                "target_job_id":"",
                "raw":"每天早上提醒我看邮件",
                "reason":"missing schedule time",
                "needs_clarify":true,
                "clarify_question":"你希望每天几点提醒？",
                "confidence":0.64
            },
            "output_contract":{
                "response_shape":"free",
                "requires_content_evidence":false,
                "delivery_required":false,
                "locator_kind":"none",
                "delivery_intent":"none",
                "locator_hint":""
            }
        }"#;
        let parsed = parse_route_decision(raw).expect("fallback route clarify+schedule decision");
        assert_eq!(parsed.mode, RoutedMode::AskClarify);
        assert_eq!(parsed.clarify_question, "你希望每天几点提醒？");
        assert_eq!(parsed.schedule_kind, super::ScheduleKind::Create);
        let intent = parsed.schedule_intent.expect("schedule intent");
        assert_eq!(intent.kind, "create");
        assert!(intent.needs_clarify);
        assert_eq!(intent.clarify_question, "你希望每天几点提醒？");
    }

    #[test]
    fn clarify_question_policy_defaults_to_allow_model() {
        assert_eq!(
            ClarifyQuestionPolicy::default(),
            ClarifyQuestionPolicy::AllowModel
        );
    }
}
