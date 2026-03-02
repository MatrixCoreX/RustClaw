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
const IMAGE_TAIL_ROUTING_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/image_tail_routing_prompt.md");

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
    let (log_long_term, log_prefs, log_recalled) = if state.memory.route_memory_enabled {
        let (long_term_summary, preferences, recalled) = memory::service::recall_memory_context_parts(
            state,
            task.user_id,
            task.chat_id,
            user_request,
            state.memory.prompt_recall_limit.max(1),
            true,
            true,
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
        (lt, pref, rec)
    } else {
        ("<none>".to_string(), "<none>".to_string(), "<none>".to_string())
    };
    let prompt = CONTEXT_RESOLVER_PROMPT_TEMPLATE
        .replace("__PERSONA_PROMPT__", &state.persona_prompt)
        .replace("__RECENT_EXECUTION_CONTEXT__", &recent_execution_context)
        .replace("__MEMORY_CONTEXT__", &memory_context)
        .replace("__REQUEST__", req);
    info!(
        "prompt_invocation task_id={} prompt_name=context_resolver_prompt memory.long_term_summary={} memory.preferences={} memory.recalled_recent={}",
        task.task_id,
        log_long_term,
        log_prefs,
        log_recalled
    );
    let llm_out = match llm_gateway::run_with_fallback(state, task, &prompt).await {
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
    let parsed = crate::extract_json_object(&llm_out)
        .and_then(|s| serde_json::from_str::<ContextResolverOut>(&s).ok())
        .or_else(|| serde_json::from_str::<ContextResolverOut>(llm_out.trim()).ok());
    if let Some(out) = parsed {
        let resolved = out.resolved_user_intent.trim();
        let confidence = out.confidence.unwrap_or(-1.0);
        info!(
            "resolve_user_request_with_context task_id={} needs_clarify={} confidence={} reason={} resolved={}",
            task.task_id,
            out.needs_clarify,
            confidence,
            crate::truncate_for_log(&out.reason),
            crate::truncate_for_log(resolved)
        );
        if !resolved.is_empty() {
            let mut final_resolved = resolved.to_string();
            if resolved != req {
                final_resolved = format!(
                    "{}\n\n[Original user message]\n{}",
                    resolved,
                    req
                );
            }
            return ContextResolution {
                resolved_user_intent: final_resolved,
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

pub(crate) async fn generate_clarify_question(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    resolver_reason: &str,
) -> String {
    let prompt = CLARIFY_QUESTION_PROMPT_TEMPLATE
        .replace("__PERSONA_PROMPT__", &state.persona_prompt)
        .replace("__REQUEST__", user_request.trim())
        .replace("__RESOLVER_REASON__", resolver_reason.trim());
    info!(
        "prompt_invocation task_id={} prompt_name=clarify_question_prompt memory.long_term_summary=<none> memory.preferences=<none> memory.recalled_recent=<none>",
        task.task_id
    );
    match llm_gateway::run_with_fallback(state, task, &prompt).await {
        Ok(v) => {
            let out = v.trim();
            if out.is_empty() {
                "我需要确认一下：你这条消息是针对哪件事情？请补充目标或上下文。".to_string()
            } else {
                out.to_string()
            }
        }
        Err(err) => {
            warn!(
                "generate_clarify_question llm failed, fallback default: task_id={} err={}",
                task.task_id, err
            );
            "我需要确认一下：你这条消息是针对哪件事情？请补充目标或上下文。".to_string()
        }
    }
}

pub(crate) async fn route_request_mode(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
) -> RoutedMode {
    let recent_execution_context = routing_context::build_recent_execution_context(state, task, 5);
    let memory_context = if state.memory.route_memory_enabled {
        let (long_term_summary, preferences, recalled) = memory::service::recall_memory_context_parts(
            state,
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
    let (log_long_term, log_prefs, log_recalled) = if state.memory.route_memory_enabled {
        let (long_term_summary, preferences, recalled) = memory::service::recall_memory_context_parts(
            state,
            task.user_id,
            task.chat_id,
            user_request,
            state.memory.prompt_recall_limit.max(1),
            true,
            true,
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
        (lt, pref, rec)
    } else {
        ("<none>".to_string(), "<none>".to_string(), "<none>".to_string())
    };
    let prompt = INTENT_ROUTER_PROMPT_TEMPLATE
        .replace("__PERSONA_PROMPT__", &state.persona_prompt)
        .replace("__ROUTING_RULES__", INTENT_ROUTER_RULES_TEMPLATE)
        .replace("__RECENT_EXECUTION_CONTEXT__", &recent_execution_context)
        .replace("__MEMORY_CONTEXT__", &memory_context)
        .replace("__REQUEST__", user_request.trim());
    info!(
        "prompt_invocation task_id={} prompt_name=intent_router_prompt memory.long_term_summary={} memory.preferences={} memory.recalled_recent={}",
        task.task_id,
        log_long_term,
        log_prefs,
        log_recalled
    );
    if state.routing.debug_log_prompt {
        info!(
            "route_request_mode prompt task_id={} prompt={}",
            task.task_id,
            crate::truncate_for_log(&prompt)
        );
    }
    let llm_out = match llm_gateway::run_with_fallback(state, task, &prompt).await {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "route_request_mode llm failed, fallback to chat: task_id={} err={}",
                task.task_id, err
            );
            return RoutedMode::Chat;
        }
    };

    if let Some(decision) = parse_route_decision(&llm_out) {
        info!(
            "route_request_mode llm task_id={} mode={:?} confidence={} reason={} evidence_refs={:?} llm_out={}",
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
        "route_request_mode parse failed, fallback to chat: task_id={} llm_out={}",
        task.task_id,
        crate::truncate_for_log(&llm_out)
    );
    RoutedMode::Chat
}

fn parse_route_decision(raw: &str) -> Option<RouteDecision> {
    let value = crate::extract_json_object(raw)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .or_else(|| serde_json::from_str::<Value>(raw.trim()).ok());
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
        return Some(RoutedMode::Chat);
    }
    if mode_text.contains("chat_act") || mode_text.contains("chat+act") {
        return Some(RoutedMode::ChatAct);
    }
    if mode_text.contains("\"act\"") || mode_text == "act" {
        return Some(RoutedMode::Act);
    }
    if mode_text.contains("\"chat\"") || mode_text == "chat" || mode_text.contains("clarify") {
        return Some(RoutedMode::Chat);
    }
    None
}

pub(crate) async fn should_apply_image_tail_handling_with_llm(
    state: &AppState,
    task: &ClaimedTask,
    request: &str,
) -> bool {
    let req = request.trim();
    if req.is_empty() {
        return false;
    }
    let prompt = IMAGE_TAIL_ROUTING_PROMPT_TEMPLATE.replace("__REQUEST__", req);
    info!(
        "prompt_invocation task_id={} prompt_name=image_tail_routing_prompt memory.long_term_summary=<none> memory.preferences=<none> memory.recalled_recent=<none>",
        task.task_id
    );
    let out = match llm_gateway::run_with_fallback(state, task, &prompt).await {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "image tail routing llm failed: task_id={} err={}",
                task.task_id, err
            );
            return false;
        }
    };
    serde_json::from_str::<Value>(out.trim())
        .ok()
        .or_else(|| crate::extract_first_json_object_any(&out).and_then(|s| serde_json::from_str::<Value>(&s).ok()))
        .and_then(|v| v.get("image_goal").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

pub(crate) async fn try_handle_schedule_request(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
) -> Result<Option<String>, String> {
    schedule_service::try_handle_schedule_request(state, task, prompt).await
}
