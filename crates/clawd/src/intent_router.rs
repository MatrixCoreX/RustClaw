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
    llm_gateway, memory, routing_context, schedule_service, AppState, ClaimedTask, RoutedMode,
};

// --- Fallback router only (not used when normalizer provides mode) ---
const INTENT_ROUTER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/intent_router_prompt.md");
const INTENT_ROUTER_PROMPT_PATH: &str = "prompts/intent_router_prompt.md";
const INTENT_ROUTER_RULES_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/intent_router_rules.md");
const INTENT_ROUTER_RULES_PATH: &str = "prompts/intent_router_rules.md";
const CLARIFY_QUESTION_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/clarify_question_prompt.md");
const CLARIFY_QUESTION_PROMPT_PATH: &str = "prompts/clarify_question_prompt.md";
const INTENT_NORMALIZER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/intent_normalizer_prompt.md");
const INTENT_NORMALIZER_PROMPT_PATH: &str = "prompts/intent_normalizer_prompt.md";
const ROUTING_POLICY_PERSONA_PROMPT: &str =
    "Neutral routing policy classifier. Ignore style/persona preferences and optimize for correct intent resolution, clarification, and guard decisions.";
// --- End fallback-only constants ---

#[derive(Debug)]
struct RouteDecision {
    mode: RoutedMode,
    reason: String,
    confidence: Option<f64>,
    evidence_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RouteDecisionOut {
    mode: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    evidence_refs: Vec<String>,
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
    pub(crate) needs_clarify: bool,
    pub(crate) reason: String,
    pub(crate) confidence: f64,
    /// Terminal mode: chat / act / ask_clarify / chat_act. Used to skip the separate router LLM.
    pub(crate) routed_mode: RoutedMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeBehavior {
    None,
    ResumeExecute,
    ResumeDiscuss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduleKind {
    None,
    Create,
    Update,
    Delete,
    Query,
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
    needs_clarify: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    mode: String,
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
    let resume_context_str = resume_context
        .map(|v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
        .filter(|s| !s.is_empty() && s != "{}")
        .unwrap_or_else(|| "<none>".to_string());
    let binding_context_str = binding_context
        .map(|v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
        .filter(|s| !s.is_empty() && s != "{}")
        .unwrap_or_else(|| "<none>".to_string());
    let recent_execution_context = routing_context::build_recent_execution_context(state, task, 8);
    let capability_map = crate::capability_map::build_capability_map_for_task(state, task);
    let memory_context = if state.memory.route_memory_enabled {
        let structured = memory::service::recall_structured_memory_context(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            user_request,
            state.memory.prompt_recall_limit.max(1),
            true,
            true,
        );
        memory::service::structured_memory_context_block(
            &structured,
            memory::retrieval::MemoryContextMode::Route,
            state.memory
                .route_trigger_budget_chars
                .max(384)
                .min(state.memory.route_memory_max_chars.max(384)),
        )
    } else {
        "<none>".to_string()
    };
    let (prompt_template, prompt_file) = crate::load_prompt_template_for_state(
        state,
        INTENT_NORMALIZER_PROMPT_PATH,
        INTENT_NORMALIZER_PROMPT_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__CAPABILITY_MAP__", &capability_map),
            ("__RESUME_CONTEXT__", &resume_context_str),
            ("__BINDING_CONTEXT__", &binding_context_str),
            ("__RECENT_EXECUTION_CONTEXT__", &recent_execution_context),
            ("__MEMORY_CONTEXT__", &memory_context),
            ("__NOW__", now_iso),
            ("__TIMEZONE__", timezone),
            ("__SCHEDULE_RULES__", schedule_rules),
            ("__REQUEST__", req),
        ],
    );
    crate::log_prompt_render(
        &task.task_id,
        "intent_normalizer_prompt",
        &prompt_file,
        None,
    );
    let llm_out =
        match llm_gateway::run_with_fallback_with_prompt_file(state, task, &prompt, &prompt_file)
            .await
        {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "intent_normalizer llm failed, fallback pass-through: task_id={} err={}",
                    task.task_id, err
                );
                return IntentNormalizerOutput {
                    resolved_user_intent: req.to_string(),
                    resume_behavior: ResumeBehavior::None,
                    schedule_kind: ScheduleKind::None,
                    needs_clarify: false,
                    reason: "llm_failed".to_string(),
                    confidence: 0.0,
                    routed_mode: RoutedMode::AskClarify,
                };
            }
        };
    let trimmed = llm_out.trim();
    let parsed_raw = serde_json::from_str::<IntentNormalizerOut>(trimmed).ok();
    let raw_parse_ok = parsed_raw.is_some();
    let parsed = parsed_raw.or_else(|| {
        crate::extract_first_json_object_any(&llm_out)
            .and_then(|json| serde_json::from_str::<IntentNormalizerOut>(&json).ok())
    });
    if !raw_parse_ok && parsed.is_some() {
        info!(
            "{} intent_normalizer task_id={} parse_recovery=fenced_json input={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req)
        );
    }
    if let Some(out) = parsed {
        let resolved = out.resolved_user_intent.trim();
        let resume_behavior = parse_resume_behavior(&out.resume_behavior);
        let schedule_kind = parse_schedule_kind(&out.schedule_kind);
        let confidence = out.confidence.clamp(0.0, 1.0);
        let routed_mode = parse_mode_text(&out.mode).unwrap_or(RoutedMode::AskClarify);
        info!(
            "{} intent_normalizer task_id={} input={} resolved_user_intent={} resume_behavior={:?} schedule_kind={:?} mode={:?} needs_clarify={} reason={} confidence={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req),
            crate::truncate_for_log(resolved),
            resume_behavior,
            schedule_kind,
            routed_mode,
            out.needs_clarify,
            crate::truncate_for_log(&out.reason),
            confidence
        );
        return IntentNormalizerOutput {
            resolved_user_intent: if resolved.is_empty() {
                req.to_string()
            } else {
                resolved.to_string()
            },
            resume_behavior,
            schedule_kind,
            needs_clarify: out.needs_clarify,
            reason: out.reason,
            confidence,
            routed_mode,
        };
    }
    warn!(
        "intent_normalizer parse failed, fallback pass-through: task_id={} raw={}",
        task.task_id,
        crate::truncate_for_log(&llm_out)
    );
    IntentNormalizerOutput {
        resolved_user_intent: req.to_string(),
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        needs_clarify: false,
        reason: "parse_failed".to_string(),
        confidence: 0.0,
        routed_mode: RoutedMode::AskClarify,
    }
}

pub(crate) async fn generate_clarify_question(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resolver_reason: &str,
) -> String {
    let (prompt_template, prompt_file) = crate::load_prompt_template_for_state(
        state,
        CLARIFY_QUESTION_PROMPT_PATH,
        CLARIFY_QUESTION_PROMPT_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__REQUEST__", user_request.trim()),
            ("__RESOLVER_REASON__", resolver_reason.trim()),
        ],
    );
    crate::log_prompt_render(&task.task_id, "clarify_question_prompt", &prompt_file, None);
    match llm_gateway::run_with_fallback_with_prompt_file(state, task, &prompt, &prompt_file).await
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
pub(crate) async fn route_request_mode(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
) -> RoutedMode {
    info!(
        "route_request_mode fallback path: normalizer did not provide mode, using legacy router LLM task_id={}",
        task.task_id
    );
    let recent_execution_context = routing_context::build_recent_execution_context(state, task, 5);
    let (memory_context, _log_long_term, _log_prefs, _log_recalled) =
        if state.memory.route_memory_enabled {
            let structured = memory::service::recall_structured_memory_context(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                user_request,
                state.memory.prompt_recall_limit.max(1),
                true,
                true,
            );
            let memory_context = memory::service::structured_memory_context_block(
                &structured,
                memory::retrieval::MemoryContextMode::Route,
                state
                    .memory
                    .route_trigger_budget_chars
                    .max(384)
                    .min(state.memory.route_memory_max_chars.max(384)),
            );
            let lt = structured
                .long_term_summary
                .as_deref()
                .map(crate::truncate_for_log)
                .unwrap_or_else(|| "<none>".to_string());
            let pref = if structured.preferences.is_empty() {
                "<none>".to_string()
            } else {
                crate::truncate_for_log(
                    &structured
                        .preferences
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(" | "),
                )
            };
            let recalled = crate::memory::retrieval::legacy_pairs_from_structured(&structured);
            let rec = if recalled.is_empty() {
                "<none>".to_string()
            } else {
                crate::truncate_for_log(
                    &recalled
                        .iter()
                        .map(|(role, content)| format!("{role}:{content}"))
                        .collect::<Vec<_>>()
                        .join(" | "),
                )
            };
            (memory_context, lt, pref, rec)
        } else {
            (
                "<none>".to_string(),
                "<none>".to_string(),
                "<none>".to_string(),
                "<none>".to_string(),
            )
        };
    let (prompt_template, prompt_file) = crate::load_prompt_template_for_state(
        state,
        INTENT_ROUTER_PROMPT_PATH,
        INTENT_ROUTER_PROMPT_TEMPLATE,
    );
    let (rules_template, _) = crate::load_prompt_template_for_state(
        state,
        INTENT_ROUTER_RULES_PATH,
        INTENT_ROUTER_RULES_TEMPLATE,
    );
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__ROUTING_RULES__", &rules_template),
            ("__RECENT_EXECUTION_CONTEXT__", &recent_execution_context),
            ("__MEMORY_CONTEXT__", &memory_context),
            ("__REQUEST__", user_request.trim()),
        ],
    );
    crate::log_prompt_render(&task.task_id, "intent_router_prompt", &prompt_file, None);
    let llm_out =
        match llm_gateway::run_with_fallback_with_prompt_file(state, task, &prompt, &prompt_file)
            .await
        {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "route_request_mode llm failed, fallback to ask_clarify: task_id={} err={}",
                    task.task_id, err
                );
                return RoutedMode::AskClarify;
            }
        };

    if let Some(decision) = parse_route_decision(&llm_out) {
        info!(
            "{} route_request_mode llm task_id={} mode={:?} confidence={} reason={} evidence_refs={:?} llm_out={}",
            crate::highlight_tag("routing"),
            task.task_id,
            decision.mode,
            decision.confidence.unwrap_or(-1.0),
            crate::truncate_for_log(&decision.reason),
            decision.evidence_refs,
            crate::truncate_for_log(&llm_out)
        );
        return decision.mode;
    }
    warn!(
        "route_request_mode parse failed, fallback to ask_clarify: task_id={} llm_out={}",
        task.task_id,
        crate::truncate_for_log(&llm_out)
    );
    RoutedMode::AskClarify
}

/// Used only by fallback `route_request_mode` to parse legacy router LLM output.
fn parse_route_decision(raw: &str) -> Option<RouteDecision> {
    let value = crate::parse_llm_json_extract_then_raw::<Value>(raw);
    if let Some(v) = value {
        if let Ok(out) = serde_json::from_value::<RouteDecisionOut>(v.clone()) {
            return Some(RouteDecision {
                mode: parse_mode_text(&out.mode)?,
                reason: out.reason,
                confidence: out.confidence.map(|c| c.clamp(0.0, 1.0)),
                evidence_refs: out.evidence_refs.into_iter().take(8).collect(),
            });
        }
        if let Some(mode_text) = v.get("mode").and_then(|m| m.as_str()) {
            return Some(RouteDecision {
                mode: parse_mode_text(mode_text)?,
                reason: String::new(),
                confidence: None,
                evidence_refs: Vec::new(),
            });
        }
    }

    parse_mode_text(raw).map(|mode| RouteDecision {
        mode,
        reason: String::new(),
        confidence: None,
        evidence_refs: Vec::new(),
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
