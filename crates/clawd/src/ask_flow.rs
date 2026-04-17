use serde_json::{json, Value};

use crate::{AppState, AskReply, ClaimedTask, RoutedMode};

fn text_contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        )
    })
}

fn text_contains_ascii_alpha(text: &str) -> bool {
    text.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn chat_request_language_hint(user_text: &str) -> &'static str {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return "config_default";
    }
    match (
        text_contains_cjk(trimmed),
        text_contains_ascii_alpha(trimmed),
    ) {
        (true, false) => "zh-CN",
        (false, true) => "en",
        (true, true) => "mixed",
        (false, false) => "config_default",
    }
}

fn canonicalize_recent_scalar_reply(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return String::new();
    }
    collapsed.to_ascii_lowercase()
}

fn direct_same_or_different_answer_from_recent_replies(
    prefer_english: bool,
    recent_assistant_replies: &[String],
) -> Option<String> {
    if recent_assistant_replies.len() < 2 {
        return None;
    }
    let latest = canonicalize_recent_scalar_reply(&recent_assistant_replies[0]);
    let previous = canonicalize_recent_scalar_reply(&recent_assistant_replies[1]);
    if latest.is_empty() || previous.is_empty() {
        return None;
    }
    (latest == previous).then(|| if prefer_english { "same" } else { "一样" }.to_string())
}

fn direct_chat_answer_from_recent_replies(
    state: &AppState,
    task: &ClaimedTask,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.needs_clarify
        || route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind
            != crate::OutputSemanticKind::RecentScalarEqualityCheck
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
    let prefer_english = state
        .command_intent
        .default_locale
        .to_ascii_lowercase()
        .starts_with("en");
    direct_same_or_different_answer_from_recent_replies(prefer_english, &recent_assistant_replies)
}

/// Phase 1.5: normalizer 顺手给出的 chat 模式直接回复候选。命中 4 条护栏
/// 时直接复用，跳过第二次 `chat_response_prompt` LLM 调用。
/// 护栏：
///   G1 `routed_mode == Chat && !needs_clarify`
///   G2 `direct_reply_confidence >= 0.75`
///   G3 `!requires_content_evidence`（内容依赖任务走完整链路，不能 one-shot）
///   G4 候选文本非空、<= 800 字符、无 skill/tool/file_token 字面标记
///      （避免 LLM 把"需要调用 xxx"这种元指令当成成品回复复用出去）
fn direct_chat_reply_from_normalizer(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    const MIN_CONFIDENCE: f64 = 0.75;
    const MAX_LEN: usize = 800;
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    // G1: 只在 chat 主路径启用；act / chat_act / ask_clarify 不走这里。
    if route.routed_mode != RoutedMode::Chat || route.needs_clarify {
        return None;
    }
    // G2
    if route.direct_reply_confidence < MIN_CONFIDENCE {
        return None;
    }
    // G3
    if route.output_contract.requires_content_evidence {
        return None;
    }
    // G4
    let candidate = route.direct_reply_candidate.trim();
    if candidate.is_empty() || candidate.chars().count() > MAX_LEN {
        return None;
    }
    let lower = candidate.to_ascii_lowercase();
    // 任何一个迹象就判定该候选是 "计划类/元指令" 而不是成品回复。
    const META_MARKERS: &[&str] = &[
        "file:",
        "call_skill",
        "call skill",
        "execute_skill",
        "run_skill",
        "```tool",
        "```skill",
        "<tool_call>",
        "function_call",
        "tool_call",
    ];
    if META_MARKERS.iter().any(|m| lower.contains(m)) {
        return None;
    }
    // 文件发送 / file_token / scalar 等强约束 shape 同样不复用，保持原链路。
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) {
        return None;
    }
    Some(candidate.to_string())
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
    // Phase 0.3: 单入口复用 normalizer 的 clarify_question。
    //
    // 原先这里先 filter `route.needs_clarify=true`，导致 `post_route_policy`
    // 将 routed_mode 强制覆写为 `AskClarify`（例如缺少 locator）但 normalizer
    // 自己没把 `needs_clarify` 设为 true 的场景下，`clarify_question` 被丢弃，
    // 后续 `generate_or_reuse_clarify_question` 会带 `AllowModel` 策略再次触发
    // 一次 LLM 调用。只要 normalizer 已经给出 clarify_question，就直接复用，
    // 把"这一轮澄清问题由谁出"收敛到单一入口。
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
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

fn chat_user_request<'a>(resolved_prompt: &'a str, execution_user_request: &'a str) -> &'a str {
    if execution_user_request.trim() != resolved_prompt.trim() {
        execution_user_request
    } else {
        resolved_prompt
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
    if let Some(reply) = crate::self_extension::maybe_handle_ask_self_extension(
        state,
        task,
        resolved_prompt,
        execution_user_request,
        agent_run_context.as_ref(),
    )
    .await?
    {
        return Ok(reply);
    }
    match routed_mode {
        RoutedMode::Chat => {
            if let Some(direct_answer) =
                direct_chat_answer_from_recent_replies(state, task, agent_run_context.as_ref())
            {
                return Ok(AskReply::non_llm(direct_answer));
            }
            // Phase 1.5: 如果 normalizer 已经在第一轮 LLM 里给出可直接发给
            // 用户的回复候选，并且通过 `direct_chat_reply_from_normalizer` 里
            // 的 4 条护栏，就直接复用，跳过第二次 chat LLM。
            if let Some(direct_answer) = direct_chat_reply_from_normalizer(agent_run_context.as_ref())
            {
                tracing::info!(
                    "chat_direct_reply_from_normalizer_hit task_id={} len={}",
                    task.task_id,
                    direct_answer.chars().count()
                );
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
            let chat_user_request = chat_user_request(resolved_prompt, execution_user_request);
            let chat_prompt = crate::render_prompt_template(
                &chat_prompt_template,
                &[
                    ("__PERSONA_PROMPT__", &task_persona_prompt),
                    ("__CONTEXT__", &chat_prompt_context),
                    (
                        "__CONFIG_RESPONSE_LANGUAGE__",
                        &state.command_intent.default_locale,
                    ),
                    (
                        "__REQUEST_LANGUAGE_HINT__",
                        chat_request_language_hint(chat_user_request),
                    ),
                    ("__REQUEST__", chat_user_request),
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
            let clarify_policy = if preferred_clarify.is_none()
                && agent_run_context
                    .as_ref()
                    .and_then(|ctx| ctx.route_result.as_ref())
                    .is_some_and(|route| route.clarify_question.trim().is_empty())
            {
                crate::intent_router::ClarifyQuestionPolicy::SafeFallback
            } else {
                crate::intent_router::ClarifyQuestionPolicy::AllowModel
            };
            let clarify = crate::intent_router::generate_or_reuse_clarify_question(
                state,
                task,
                resolved_prompt,
                clarify_reason,
                None,
                preferred_clarify.as_deref(),
                clarify_policy,
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
        chat_prompt_context_with_route_resolution, chat_request_language_hint, chat_user_request,
        direct_same_or_different_answer_from_recent_replies,
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
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                locator_hint: "scripts".to_string(),
                ..Default::default()
            },
                    direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
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
    fn same_or_different_direct_answer_normalizes_exact_scalar_matches() {
        let replies = vec!["  Value \n".to_string(), "value".to_string()];
        let answer = direct_same_or_different_answer_from_recent_replies(false, &replies);
        assert_eq!(answer.as_deref(), Some("一样"));
    }

    #[test]
    fn chat_user_request_preserves_inline_structured_prompt_when_resolution_dropped_payload() {
        let prompt = r#"sort this JSON array by score descending and render it as a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
        let resolved = "Sort the provided JSON array by score in descending order and output as a markdown table";
        assert_eq!(chat_user_request(resolved, prompt), prompt);
    }

    #[test]
    fn chat_request_language_hint_prefers_current_request_language() {
        assert_eq!(chat_request_language_hint("写个两句短诗"), "zh-CN");
        assert_eq!(
            chat_request_language_hint("do not run anything, just tell me a very short joke"),
            "en"
        );
        assert_eq!(
            chat_request_language_hint("用 English 解释 README"),
            "mixed"
        );
        assert_eq!(chat_request_language_hint("12345"), "config_default");
    }
}
