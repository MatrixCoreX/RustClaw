use serde::Deserialize;
use serde_json::Value;
use tracing::{info, warn};

use crate::{llm_gateway, memory, routing_context, schedule_service, AppState, ClaimedTask, RoutedMode};

const INTENT_ROUTER_PROMPT_TEMPLATE: &str = include_str!("../../../prompts/intent_router_prompt.md");
const INTENT_ROUTER_RULES_TEMPLATE: &str = include_str!("../../../prompts/intent_router_rules.md");
const CONTEXT_RESOLVER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/context_resolver_prompt.md");
const CLARIFY_QUESTION_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/clarify_question_prompt.md");
const RESUME_FOLLOWUP_INTENT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/resume_followup_intent_prompt.md");
const ROUTING_POLICY_PERSONA_PROMPT: &str =
    "Neutral routing policy classifier. Ignore style/persona preferences and optimize for correct intent resolution, clarification, and guard decisions.";

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

#[derive(Debug, Deserialize)]
struct ContextResolverOut {
    #[serde(default)]
    resolved_user_intent: String,
    #[serde(default)]
    needs_clarify: bool,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Deserialize)]
struct ResumeFollowupIntentOut {
    #[serde(default)]
    decision: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    bind_resume_context: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ResumeFollowupDecision {
    pub(crate) decision: String,
    pub(crate) reason: String,
    pub(crate) confidence: Option<f64>,
    pub(crate) bind_resume_context: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ContextResolution {
    pub(crate) resolved_user_intent: String,
    pub(crate) needs_clarify: bool,
    pub(crate) confidence: Option<f64>,
    pub(crate) reason: String,
}

pub(crate) async fn resolve_user_request_with_context(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
) -> ContextResolution {
    let req = user_request.trim();
    if req.is_empty() {
        return ContextResolution {
            resolved_user_intent: String::new(),
            needs_clarify: false,
            confidence: None,
            reason: String::new(),
        };
    }
    let recent_execution_context = routing_context::build_recent_execution_context(state, task, 8);
    let memory_context = if state.memory.route_memory_enabled {
        let (long_term_summary, preferences, recalled) = memory::service::recall_memory_context_parts(
            state,
            task.user_key.as_deref(),
            task.user_id,
            task.chat_id,
            user_request,
            state.memory.prompt_recall_limit.max(1),
            true,
            true,
        );
        memory::service::memory_context_block(
            long_term_summary.as_deref(),
            &preferences,
            &recalled,
            state.memory.route_memory_max_chars.max(384),
        )
    } else {
        "<none>".to_string()
    };
    let prompt = crate::render_prompt_template(
        CONTEXT_RESOLVER_PROMPT_TEMPLATE,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__RECENT_EXECUTION_CONTEXT__", &recent_execution_context),
            ("__MEMORY_CONTEXT__", &memory_context),
            ("__REQUEST__", req),
        ],
    );
    crate::log_prompt_render(
        &task.task_id,
        "context_resolver_prompt",
        "prompts/context_resolver_prompt.md",
        None,
    );
    let llm_out = match llm_gateway::run_with_fallback_with_prompt_file(
        state,
        task,
        &prompt,
        "prompts/context_resolver_prompt.md",
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "resolve_user_request_with_context llm failed, fallback original: task_id={} err={}",
                task.task_id, err
            );
            return ContextResolution {
                resolved_user_intent: req.to_string(),
                needs_clarify: false,
                confidence: None,
                reason: "llm_failed".to_string(),
            };
        }
    };
    let parsed = crate::parse_llm_json_extract_then_raw::<ContextResolverOut>(&llm_out);
    if let Some(out) = parsed {
        let resolved = out.resolved_user_intent.trim();
        let confidence = out.confidence.unwrap_or(-1.0);
        info!(
            "{} resolve_user_request_with_context task_id={} needs_clarify={} confidence={} reason={} resolved={}",
            crate::highlight_tag("routing"),
            task.task_id,
            out.needs_clarify,
            confidence,
            crate::truncate_for_log(&out.reason),
            crate::truncate_for_log(resolved)
        );
        if !resolved.is_empty() {
            return ContextResolution {
                resolved_user_intent: resolved.to_string(),
                needs_clarify: out.needs_clarify,
                confidence: out.confidence.map(|c| c.clamp(0.0, 1.0)),
                reason: out.reason,
            };
        }
    } else {
        warn!(
            "resolve_user_request_with_context parse failed, fallback original: task_id={} llm_out={}",
            task.task_id,
            crate::truncate_for_log(&llm_out)
        );
    }
    ContextResolution {
        resolved_user_intent: req.to_string(),
        needs_clarify: false,
        confidence: None,
        reason: "parse_failed".to_string(),
    }
}

pub(crate) async fn classify_resume_followup_intent(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resume_context: &Value,
) -> ResumeFollowupDecision {
    let resume_context_json = serde_json::to_string_pretty(resume_context)
        .unwrap_or_else(|_| resume_context.to_string());
    let prompt = crate::render_prompt_template(
        RESUME_FOLLOWUP_INTENT_PROMPT_TEMPLATE,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__RESUME_CONTEXT__", &resume_context_json),
            ("__REQUEST__", user_request.trim()),
        ],
    );
    crate::log_prompt_render(
        &task.task_id,
        "resume_followup_intent_prompt",
        "prompts/resume_followup_intent_prompt.md",
        None,
    );
    let llm_out = match llm_gateway::run_with_fallback_with_prompt_file(
        state,
        task,
        &prompt,
        "prompts/resume_followup_intent_prompt.md",
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "classify_resume_followup_intent llm failed, fallback defer: task_id={} err={}",
                task.task_id, err
            );
            return ResumeFollowupDecision {
                decision: "defer".to_string(),
                reason: "llm_failed".to_string(),
                confidence: None,
                bind_resume_context: false,
            };
        }
    };
    let parsed = crate::parse_llm_json_extract_then_raw::<ResumeFollowupIntentOut>(&llm_out);
    if let Some(out) = parsed {
        let decision = match out.decision.trim().to_ascii_lowercase().as_str() {
            "resume" | "abandon" | "defer" => out.decision.trim().to_ascii_lowercase(),
            _ => "defer".to_string(),
        };
        info!(
            "{} classify_resume_followup_intent task_id={} decision={} confidence={} reason={}",
            crate::highlight_tag("routing"),
            task.task_id,
            decision,
            out.confidence.unwrap_or(-1.0),
            crate::truncate_for_log(&out.reason)
        );
        return ResumeFollowupDecision {
            decision,
            reason: out.reason,
            confidence: out.confidence.map(|c| c.clamp(0.0, 1.0)),
            bind_resume_context: out.bind_resume_context,
        };
    }
    warn!(
        "classify_resume_followup_intent parse failed, fallback defer: task_id={} raw={}",
        task.task_id,
        crate::truncate_for_log(&llm_out)
    );
    ResumeFollowupDecision {
        decision: "defer".to_string(),
        reason: "parse_failed".to_string(),
        confidence: None,
        bind_resume_context: false,
    }
}

pub(crate) async fn generate_clarify_question(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resolver_reason: &str,
) -> String {
    let prompt = crate::render_prompt_template(
        CLARIFY_QUESTION_PROMPT_TEMPLATE,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__REQUEST__", user_request.trim()),
            ("__RESOLVER_REASON__", resolver_reason.trim()),
        ],
    );
    crate::log_prompt_render(
        &task.task_id,
        "clarify_question_prompt",
        "prompts/clarify_question_prompt.md",
        None,
    );
    match llm_gateway::run_with_fallback_with_prompt_file(
        state,
        task,
        &prompt,
        "prompts/clarify_question_prompt.md",
    )
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

pub(crate) async fn route_request_mode(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
) -> RoutedMode {
    let recent_execution_context = routing_context::build_recent_execution_context(state, task, 5);
    let (memory_context, _log_long_term, _log_prefs, _log_recalled) =
        if state.memory.route_memory_enabled {
            let (long_term_summary, preferences, recalled) = memory::service::recall_memory_context_parts(
                state,
                task.user_key.as_deref(),
                task.user_id,
                task.chat_id,
                user_request,
                state.memory.prompt_recall_limit.max(1),
                true,
                true,
            );
            let memory_context = memory::service::memory_context_block(
                long_term_summary.as_deref(),
                &preferences,
                &recalled,
                state.memory.route_memory_max_chars.max(384),
            );
            let lt = long_term_summary
                .as_deref()
                .map(crate::truncate_for_log)
                .unwrap_or_else(|| "<none>".to_string());
            let pref = if preferences.is_empty() {
                "<none>".to_string()
            } else {
                crate::truncate_for_log(
                    &preferences
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(" | "),
                )
            };
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
    let prompt = crate::render_prompt_template(
        INTENT_ROUTER_PROMPT_TEMPLATE,
        &[
            ("__PERSONA_PROMPT__", ROUTING_POLICY_PERSONA_PROMPT),
            ("__ROUTING_RULES__", INTENT_ROUTER_RULES_TEMPLATE),
            ("__RECENT_EXECUTION_CONTEXT__", &recent_execution_context),
            ("__MEMORY_CONTEXT__", &memory_context),
            ("__REQUEST__", user_request.trim()),
        ],
    );
    crate::log_prompt_render(
        &task.task_id,
        "intent_router_prompt",
        "prompts/intent_router_prompt.md",
        None,
    );
    let llm_out = match llm_gateway::run_with_fallback_with_prompt_file(
        state,
        task,
        &prompt,
        "prompts/intent_router_prompt.md",
    )
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
