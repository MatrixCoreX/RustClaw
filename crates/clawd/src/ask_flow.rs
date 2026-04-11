use serde_json::{json, Value};

use crate::{AppState, AskReply, ClaimedTask, RoutedMode};

fn looks_like_same_or_different_request(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    trimmed.contains("一样")
        || trimmed.contains("相同")
        || trimmed.contains("不同")
        || lower.contains("same")
        || lower.contains("different")
        || lower.contains("equal")
        || lower.contains("match")
}

fn prefers_chinese_reply(text: &str) -> bool {
    text.chars().any(|ch| {
        ('\u{4e00}'..='\u{9fff}').contains(&ch) || ('\u{3400}'..='\u{4dbf}').contains(&ch)
    })
}

fn canonicalize_recent_scalar_reply(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return String::new();
    }
    let lower = collapsed.to_ascii_lowercase();
    if collapsed.ends_with("字段不存在")
        || lower.contains("missing field")
        || lower.contains("field not found")
        || lower.contains("field does not exist")
    {
        return "__field_missing__".to_string();
    }
    if collapsed.contains("未找到该文件")
        || collapsed.contains("文件不存在")
        || collapsed.contains("没有找到该文件")
        || lower.contains("file not found")
        || lower.contains("not found")
    {
        return "__not_found__".to_string();
    }
    lower
}

fn direct_same_or_different_answer_from_recent_replies(
    user_text: &str,
    resolved_intent: &str,
    recent_assistant_replies: &[String],
) -> Option<String> {
    if recent_assistant_replies.len() < 2 {
        return None;
    }
    let semantic_request = if resolved_intent.trim().is_empty() {
        user_text
    } else {
        resolved_intent
    };
    if !looks_like_same_or_different_request(semantic_request) {
        return None;
    }
    let latest = canonicalize_recent_scalar_reply(&recent_assistant_replies[0]);
    let previous = canonicalize_recent_scalar_reply(&recent_assistant_replies[1]);
    if latest.is_empty() || previous.is_empty() {
        return None;
    }
    let same = latest == previous;
    Some(
        if prefers_chinese_reply(semantic_request) {
            if same { "一样" } else { "不一样" }
        } else if same {
            "same"
        } else {
            "different"
        }
        .to_string(),
    )
}

fn direct_chat_answer_from_recent_replies(
    state: &AppState,
    task: &ClaimedTask,
    resolved_prompt: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.needs_clarify
        || route.output_contract.requires_content_evidence
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
        )
    {
        return None;
    }
    let recent_assistant_replies = crate::memory::read_recent_assistant_reply_texts(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        2,
    );
    direct_same_or_different_answer_from_recent_replies(
        resolved_prompt,
        &route.resolved_intent,
        &recent_assistant_replies,
    )
}

fn build_resume_continue_execute_prompt_from_parts(
    state: &AppState,
    user_text: &str,
    resume_context: &Value,
    resume_instruction: &str,
    resume_steps: Option<&Value>,
) -> String {
    let resume_steps = resume_steps
        .cloned()
        .filter(|v| v.as_array().map(|arr| !arr.is_empty()).unwrap_or(false))
        .unwrap_or_else(|| {
            resume_context
                .get("remaining_actions")
                .cloned()
                .filter(|v| v.as_array().map(|arr| !arr.is_empty()).unwrap_or(false))
                .unwrap_or_else(|| {
                    resume_context
                        .get("remaining_steps")
                        .cloned()
                        .unwrap_or_else(|| json!([]))
                })
        });
    let resume_context_json =
        serde_json::to_string_pretty(resume_context).unwrap_or_else(|_| resume_context.to_string());
    let resume_steps_json =
        serde_json::to_string_pretty(&resume_steps).unwrap_or_else(|_| resume_steps.to_string());

    let (prompt_template, _) = crate::bootstrap::load_prompt_template_for_state(
        state,
        "prompts/resume_continue_execute_prompt.md",
        crate::RESUME_CONTINUE_EXECUTE_PROMPT_TEMPLATE,
    );
    crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text),
            ("__RESUME_CONTEXT__", &resume_context_json),
            ("__RESUME_STEPS__", &resume_steps_json),
            ("__RESUME_INSTRUCTION__", resume_instruction),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.command_intent.default_locale,
            ),
        ],
    )
}

pub(crate) fn build_resume_continue_execute_prompt(
    state: &AppState,
    payload: &Value,
    fallback_user_text: &str,
) -> String {
    let user_text = payload
        .get("resume_user_text")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_user_text);
    let resume_context = payload
        .get("resume_context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let resume_instruction = payload
        .get("resume_instruction")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let resume_steps = payload.get("resume_steps");
    build_resume_continue_execute_prompt_from_parts(
        state,
        user_text,
        &resume_context,
        resume_instruction,
        resume_steps,
    )
}

pub(crate) fn build_resume_continue_execute_prompt_from_context(
    state: &AppState,
    user_text: &str,
    resume_context: &Value,
) -> String {
    build_resume_continue_execute_prompt_from_parts(state, user_text, resume_context, "", None)
}

fn build_resume_followup_discussion_prompt_from_parts(
    state: &AppState,
    user_text: &str,
    resume_context: &Value,
) -> String {
    let resume_context_json =
        serde_json::to_string_pretty(resume_context).unwrap_or_else(|_| resume_context.to_string());
    let (prompt_template, _) = crate::bootstrap::load_prompt_template_for_state(
        state,
        crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_LOGICAL_PATH,
        crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_TEMPLATE,
    );
    crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text.trim()),
            ("__RESUME_CONTEXT__", &resume_context_json),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.command_intent.default_locale,
            ),
        ],
    )
}

pub(crate) fn build_resume_followup_discussion_prompt(
    state: &AppState,
    payload: &Value,
    fallback_user_text: &str,
) -> String {
    let user_text = payload
        .get("resume_user_text")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_user_text)
        .trim();
    let resume_context = payload
        .get("resume_context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    build_resume_followup_discussion_prompt_from_parts(state, user_text, &resume_context)
}

pub(crate) fn build_resume_followup_discussion_prompt_from_context(
    state: &AppState,
    user_text: &str,
    resume_context: &Value,
) -> String {
    build_resume_followup_discussion_prompt_from_parts(state, user_text, resume_context)
}

fn chat_act_goal_from_prompt(prompt_with_memory: &str) -> String {
    format!(
        "{}\n\nMode hint: chat_act. Complete required actions first, then return a concise user-facing reply that confirms results naturally.",
        prompt_with_memory
    )
}

fn preferred_route_clarify_question(
    state: &crate::AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .filter(|route| route.needs_clarify)?;
    let missing_locator = route.output_contract.locator_hint.trim().is_empty();
    if route.output_contract.delivery_required && missing_locator {
        return Some(crate::i18n_t_with_default(
            state,
            "clawd.msg.clarify_missing_file_locator",
            "Please provide the specific file name or path.",
        ));
    }
    if route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
        && missing_locator
    {
        return Some(crate::i18n_t_with_default(
            state,
            "clawd.msg.clarify_missing_read_target",
            "Please provide the specific file name or path to read.",
        ));
    }
    let question = route.clarify_question.trim();
    (!question.is_empty()).then(|| question.to_string())
}

fn chat_route_resolution_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    let mut lines = Vec::new();
    let resolved_intent = route.resolved_intent.trim();
    if !resolved_intent.is_empty() {
        lines.push(format!("resolved_user_intent: {resolved_intent}"));
    }
    let locator_hint = route.output_contract.locator_hint.trim();
    if !locator_hint.is_empty() {
        lines.push(format!("locator_hint: {locator_hint}"));
    }
    let route_reason = route.route_reason.trim();
    if !route_reason.is_empty() {
        lines.push(format!("route_reason: {route_reason}"));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "### ROUTE_RESOLUTION\nTreat the following route resolution as authoritative for this turn. If older memory or unrelated assistant history conflicts with it, prefer this resolution unless the user explicitly asks about older history.\n{}\n",
        lines.join("\n")
    ))
}

fn chat_prompt_context_with_route_resolution(
    chat_prompt_context: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> String {
    let Some(route_context) = chat_route_resolution_context(agent_run_context) else {
        return chat_prompt_context.to_string();
    };
    if chat_prompt_context.trim().is_empty() {
        route_context
    } else {
        format!("{chat_prompt_context}\n\n{route_context}")
    }
}

pub(crate) async fn execute_ask_routed(
    state: &AppState,
    task: &ClaimedTask,
    chat_prompt_context: &str,
    prompt_with_memory: &str,
    resolved_prompt: &str,
    execution_user_request: &str,
    agent_mode: bool,
    resume_force_chat: bool,
    normalizer_mode: Option<RoutedMode>,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<AskReply, String> {
    let (routed_mode, used_fallback_router, override_reason) = if resume_force_chat {
        (RoutedMode::Chat, false, Some("resume_force_chat"))
    } else if let Some(m) = normalizer_mode {
        (m, false, None)
    } else if agent_mode {
        let mode = crate::intent_router::route_request_mode(state, task, resolved_prompt).await;
        (mode, true, None)
    } else {
        (
            RoutedMode::Chat,
            false,
            Some("normalizer_mode=None and agent_mode=false"),
        )
    };
    tracing::info!(
        "{} worker_once: ask task_id={} normalizer_mode={:?} routed_mode={:?} agent_mode={} used_fallback_router={} override={}",
        crate::highlight_tag("routing"),
        task.task_id,
        normalizer_mode,
        routed_mode,
        agent_mode,
        used_fallback_router,
        override_reason.unwrap_or("")
    );
    match routed_mode {
        RoutedMode::Chat => {
            if let Some(direct_answer) = direct_chat_answer_from_recent_replies(
                state,
                task,
                resolved_prompt,
                agent_run_context.as_ref(),
            ) {
                return Ok(AskReply::non_llm(direct_answer));
            }
            let chat_prompt_context = chat_prompt_context_with_route_resolution(
                chat_prompt_context,
                agent_run_context.as_ref(),
            );
            let (chat_prompt_template, chat_prompt_source) =
                crate::bootstrap::load_prompt_template_for_state(
                    state,
                    crate::CHAT_RESPONSE_PROMPT_LOGICAL_PATH,
                    crate::CHAT_RESPONSE_PROMPT_TEMPLATE,
                );
            crate::log_prompt_render(
                state,
                &task.task_id,
                "chat_response_prompt",
                &chat_prompt_source,
                None,
            );
            let task_persona_prompt = state.task_persona_prompt(task);
            let chat_prompt = crate::render_prompt_template(
                &chat_prompt_template,
                &[
                    ("__PERSONA_PROMPT__", &task_persona_prompt),
                    ("__CONTEXT__", &chat_prompt_context),
                    (
                        "__CONFIG_RESPONSE_LANGUAGE__",
                        &state.command_intent.default_locale,
                    ),
                    ("__REQUEST__", resolved_prompt),
                ],
            );
            crate::llm_gateway::run_with_fallback_with_prompt_source(
                state,
                task,
                &chat_prompt,
                &chat_prompt_source,
            )
            .await
            .map(crate::AskReply::llm)
            .map_err(|e| e.to_string())
        }
        RoutedMode::Act => {
            crate::agent_engine::run_agent_with_tools(
                state,
                task,
                prompt_with_memory,
                execution_user_request,
                agent_run_context.clone(),
            )
            .await
        }
        RoutedMode::ChatAct => {
            crate::agent_engine::run_agent_with_tools(
                state,
                task,
                &chat_act_goal_from_prompt(prompt_with_memory),
                execution_user_request,
                agent_run_context.clone(),
            )
            .await
        }
        RoutedMode::AskClarify => {
            let clarify_reason = agent_run_context
                .as_ref()
                .and_then(|ctx| ctx.route_result.as_ref())
                .map(|route| route.route_reason.as_str())
                .unwrap_or("router_selected_ask_clarify");
            let preferred_clarify =
                preferred_route_clarify_question(state, agent_run_context.as_ref());
            let clarify = crate::intent_router::generate_or_reuse_clarify_question(
                state,
                task,
                resolved_prompt,
                clarify_reason,
                None,
                preferred_clarify.as_deref(),
            )
            .await;
            Ok(AskReply::non_llm(clarify))
        }
    }
}

pub(crate) async fn analyze_attached_images_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    resolved_prompt: &str,
) -> anyhow::Result<Option<String>> {
    let Some(images) = payload.get("images").and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    if images.is_empty() {
        return Ok(None);
    }
    let mut args = json!({
        "action": "describe",
        "images": images,
    });
    let instruction = resolved_prompt.trim();
    if let Some(obj) = args.as_object_mut() {
        if !instruction.is_empty() {
            obj.insert(
                "instruction".to_string(),
                Value::String(instruction.to_string()),
            );
        }
        if let Some(language) = payload
            .get("response_language")
            .or_else(|| payload.get("language"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            obj.insert(
                "response_language".to_string(),
                Value::String(language.to_string()),
            );
        }
    }
    crate::skills::run_skill_with_runner(state, task, "image_vision", args)
        .await
        .map_err(anyhow::Error::msg)
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::{
        chat_prompt_context_with_route_resolution, direct_same_or_different_answer_from_recent_replies,
    };

    #[test]
    fn chat_prompt_context_appends_authoritative_route_resolution() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            resolved_intent: "上一个和上上个哪个更多，只回答目录名".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "'上一个'=assistant[-1](document,17), '上上个'=assistant[-2](scripts,48); scripts 更多".to_string(),
            route_confidence: Some(0.94),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            output_contract: crate::IntentOutputContract {
                locator_hint: "scripts".to_string(),
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let rendered = chat_prompt_context_with_route_resolution(
            "### MEMORY_CONTEXT\nRECENT_ASSISTANT_RESULTS\n- old summary",
            Some(&ctx),
        );
        assert!(rendered.contains("### ROUTE_RESOLUTION"));
        assert!(rendered.contains("resolved_user_intent: 上一个和上上个哪个更多，只回答目录名"));
        assert!(rendered.contains("locator_hint: scripts"));
        assert!(rendered.contains("scripts 更多"));
    }

    #[test]
    fn same_or_different_direct_answer_normalizes_missing_field_outcomes() {
        let replies = vec![
            "package.name 字段不存在".to_string(),
            "name 字段不存在".to_string(),
        ];
        let answer = direct_same_or_different_answer_from_recent_replies(
            "这两个一样吗，只回答一样或不一样",
            "比较前两个助手回复是否相同，只回答一样或不一样",
            &replies,
        );
        assert_eq!(answer.as_deref(), Some("一样"));
    }
}
