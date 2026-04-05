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
use serde_json::Value;
use tracing::{info, warn};

use crate::{
    llm_gateway, schedule_service, AppState, ClaimedTask, RiskCeiling, RouteResult, RoutedMode,
};

pub(crate) use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    ResumeBehavior, ScheduleKind,
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
    reason: String,
    confidence: Option<f64>,
    evidence_refs: Vec<String>,
    wants_file_delivery: bool,
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
    reason: String,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    evidence_refs: Vec<String>,
    #[serde(default)]
    wants_file_delivery: bool,
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
    pub(crate) wants_file_delivery: bool,
    pub(crate) needs_clarify: bool,
    pub(crate) reason: String,
    pub(crate) confidence: f64,
    pub(crate) output_contract: IntentOutputContract,
    /// Terminal mode: chat / act / ask_clarify / chat_act. Used to skip the separate router LLM.
    pub(crate) routed_mode: RoutedMode,
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
        route_reason: normalizer_out.reason.clone(),
        route_confidence: Some(normalizer_out.confidence),
        visible_skill_candidates: state.planner_visible_skills_for_task(task),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: normalizer_out.resume_behavior,
        schedule_kind: normalizer_out.schedule_kind,
        wants_file_delivery: normalizer_out.wants_file_delivery,
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
    needs_clarify: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    mode: String,
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
    locator_hint: String,
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
        contract.locator_hint = raw.locator_hint.trim().to_string();
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

fn normalizer_output_from_fallback(
    user_request: &str,
    fallback_reason_prefix: &str,
    decision: RouteDecision,
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
        schedule_kind: ScheduleKind::None,
        wants_file_delivery: decision.wants_file_delivery,
        needs_clarify: decision.needs_clarify,
        reason,
        confidence: decision.confidence.unwrap_or(0.0),
        output_contract: decision.output_contract,
        routed_mode,
    }
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
        let routed_mode = crate::post_route_policy::enforce_content_evidence_execution_mode(
            routed_mode_raw,
            &output_contract,
            out.needs_clarify,
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
            wants_file_delivery: out.wants_file_delivery,
            needs_clarify: out.needs_clarify,
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
                reason: "fallback_router_llm_failed".to_string(),
                confidence: None,
                evidence_refs: Vec::new(),
                wants_file_delivery: false,
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
        reason: "fallback_router_parse_failed".to_string(),
        confidence: None,
        evidence_refs: Vec::new(),
        wants_file_delivery: false,
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
            return Some(RouteDecision {
                mode,
                resolved_user_intent: out.resolved_user_intent.trim().to_string(),
                needs_clarify: out.needs_clarify || matches!(mode, RoutedMode::AskClarify),
                reason: out.reason,
                confidence: out.confidence.map(|c| c.clamp(0.0, 1.0)),
                evidence_refs: out.evidence_refs.into_iter().take(8).collect(),
                wants_file_delivery: wants_file_delivery_from_contract(
                    &output_contract,
                    out.wants_file_delivery,
                ),
                output_contract,
            });
        }
        if let Some(mode_text) = v.get("mode").and_then(|m| m.as_str()) {
            let mode = parse_mode_text(mode_text)?;
            return Some(RouteDecision {
                mode,
                resolved_user_intent: String::new(),
                needs_clarify: matches!(mode, RoutedMode::AskClarify),
                reason: String::new(),
                confidence: None,
                evidence_refs: Vec::new(),
                wants_file_delivery: false,
                output_contract: IntentOutputContract::default(),
            });
        }
    }

    parse_mode_text(raw).map(|mode| RouteDecision {
        mode,
        resolved_user_intent: String::new(),
        needs_clarify: matches!(mode, RoutedMode::AskClarify),
        reason: String::new(),
        confidence: None,
        evidence_refs: Vec::new(),
        wants_file_delivery: false,
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
) -> Result<Option<String>, String> {
    schedule_service::try_handle_schedule_request(state, task, prompt).await
}

#[cfg(test)]
mod tests {
    use super::{
        normalizer_output_from_fallback, parse_route_decision, IntentOutputContract,
        OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, RouteDecision,
    };
    use crate::RoutedMode;

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
    fn fallback_normalizer_output_still_enforces_content_evidence_execution_mode() {
        let out = normalizer_output_from_fallback(
            "把当前目录有没有隐藏文件看一下",
            "parse_failed_fallback_router",
            RouteDecision {
                mode: RoutedMode::Chat,
                resolved_user_intent: "看一下当前目录有没有隐藏文件".to_string(),
                needs_clarify: false,
                reason: "current workspace executable request".to_string(),
                confidence: Some(0.72),
                evidence_refs: Vec::new(),
                wants_file_delivery: false,
                output_contract: IntentOutputContract {
                    response_shape: OutputResponseShape::Scalar,
                    requires_content_evidence: true,
                    delivery_required: false,
                    locator_kind: OutputLocatorKind::CurrentWorkspace,
                    delivery_intent: OutputDeliveryIntent::None,
                    locator_hint: String::new(),
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
}
