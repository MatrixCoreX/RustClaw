use serde_json::{json, Value};

use crate::{AppState, AskReply, ClaimedTask, RoutedMode};

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
        .policy
        .command_intent
        .default_locale
        .to_ascii_lowercase()
        .starts_with("en");
    direct_same_or_different_answer_from_recent_replies(prefer_english, &recent_assistant_replies)
}

fn build_resume_continue_execute_prompt_from_parts(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
    resume_instruction: &str,
    resume_steps: Option<&Value>,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
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

    let (prompt_template, _) = crate::bootstrap::load_required_prompt_template_for_state(
        state,
        "prompts/resume_continue_execute_prompt.md",
    )?;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    Ok(crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text),
            ("__RESUME_CONTEXT__", &resume_context_json),
            ("__RESUME_STEPS__", &resume_steps_json),
            ("__RESUME_INSTRUCTION__", resume_instruction),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
        ],
    ))
}

pub(crate) fn build_resume_continue_execute_prompt(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    fallback_user_text: &str,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
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
        task,
        user_text,
        &resume_context,
        resume_instruction,
        resume_steps,
    )
}

pub(crate) fn build_resume_continue_execute_prompt_from_context(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    build_resume_continue_execute_prompt_from_parts(
        state,
        task,
        user_text,
        resume_context,
        "",
        None,
    )
}

fn build_resume_followup_discussion_prompt_from_parts(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let resume_context_json =
        serde_json::to_string_pretty(resume_context).unwrap_or_else(|_| resume_context.to_string());
    let (prompt_template, _) = crate::bootstrap::load_required_prompt_template_for_state(
        state,
        crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_LOGICAL_PATH,
    )?;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    Ok(crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text.trim()),
            ("__RESUME_CONTEXT__", &resume_context_json),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
        ],
    ))
}

pub(crate) fn build_resume_followup_discussion_prompt(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    fallback_user_text: &str,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    let user_text = payload
        .get("resume_user_text")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_user_text)
        .trim();
    let resume_context = payload
        .get("resume_context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    build_resume_followup_discussion_prompt_from_parts(state, task, user_text, &resume_context)
}

pub(crate) fn build_resume_followup_discussion_prompt_from_context(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    resume_context: &Value,
) -> Result<String, crate::bootstrap::RequiredPromptLoadError> {
    build_resume_followup_discussion_prompt_from_parts(state, task, user_text, resume_context)
}

fn chat_act_goal_from_prompt(prompt_with_memory: &str) -> String {
    format!(
        "{}\n\nMode hint: chat_act. Complete required actions first, then return a concise user-facing reply that confirms results naturally.",
        prompt_with_memory
    )
}

fn fuzzy_locator_clarify_question(
    state: &crate::AppState,
    route: &crate::RouteResult,
) -> Option<String> {
    let candidates =
        crate::post_route_policy::fuzzy_locator_candidates_from_route_reason(&route.route_reason);
    if candidates.is_empty() {
        return None;
    }
    let candidate_block = candidates
        .iter()
        .enumerate()
        .map(|(idx, value)| format!("{}. {}", idx + 1, value))
        .collect::<Vec<_>>()
        .join("\n");
    let header = if state
        .policy
        .command_intent
        .default_locale
        .trim()
        .to_ascii_lowercase()
        .starts_with("en")
    {
        "I found multiple matching candidates. Reply with the number or the full path:"
    } else {
        "我找到了多个匹配候选，请直接回复序号或完整路径："
    };
    Some(format!("{header}\n{candidate_block}"))
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
    let question = route.clarify_question.trim();
    if !question.is_empty() {
        return Some(question.to_string());
    }
    if let Some(question) = fuzzy_locator_clarify_question(state, route) {
        return Some(question);
    }
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
    None
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
        "### ROUTE_RESOLUTION\nTreat the following route resolution as authoritative for this turn. It is resolved context, not missing context. If older memory or unrelated assistant history conflicts with it, prefer this resolution unless the user explicitly asks about older history.\n{}\n",
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
    let trimmed_context = chat_prompt_context.trim();
    if trimmed_context.is_empty() || trimmed_context == "<none>" {
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

fn chat_request_for_prompt(original_user_request: &str, semantic_request: &str) -> String {
    let original = original_user_request.trim();
    let semantic = semantic_request.trim();
    if original.is_empty() || original == semantic {
        return semantic.to_string();
    }
    format!("Original user request:\n{original}\n\nResolved semantic intent:\n{semantic}")
}

fn task_payload_text(task: &ClaimedTask) -> Option<String> {
    crate::task_payload_value(task)?
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn execute_via_planner_loop(
    state: &AppState,
    task: &ClaimedTask,
    prompt_with_memory: &str,
    execution_user_request: &str,
    ask_mode: &crate::AskMode,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<AskReply, String> {
    let planner_goal = if ask_mode.finalize_chat_wrapped() {
        chat_act_goal_from_prompt(prompt_with_memory)
    } else {
        prompt_with_memory.to_string()
    };
    crate::agent_engine::run_agent_with_tools(
        state,
        task,
        &planner_goal,
        execution_user_request,
        agent_run_context,
    )
    .await
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
    route_ask_mode: Option<crate::AskMode>,
    agent_run_context: Option<crate::agent_engine::AgentRunContext>,
) -> Result<AskReply, String> {
    // Phase 2.7: legacy `route_request_mode` (second-LLM router) was removed. Callers now
    // pass the folded route ask_mode directly; if for some reason a caller drops it, default
    // to AskClarify rather than burning another LLM round-trip.
    let route_ask_mode_for_log = route_ask_mode.clone();
    let (ask_mode, override_reason) = if resume_force_chat {
        (
            crate::AskMode::from_routed_mode(RoutedMode::Chat),
            Some("resume_force_chat"),
        )
    } else if let Some(mode) = route_ask_mode {
        (mode, None)
    } else if agent_mode {
        (
            crate::AskMode::from_routed_mode(RoutedMode::AskClarify),
            Some("route_ask_mode=None and agent_mode=true"),
        )
    } else {
        (
            crate::AskMode::from_routed_mode(RoutedMode::Chat),
            Some("route_ask_mode=None and agent_mode=false"),
        )
    };
    let routed_mode = ask_mode.to_routed_mode();
    tracing::info!(
        "{} worker_once: ask task_id={} normalizer_mode={:?} routed_mode={:?} agent_mode={} override={}",
        crate::highlight_tag("routing"),
        task.task_id,
        route_ask_mode_for_log,
        routed_mode,
        agent_mode,
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
    match &ask_mode {
        crate::AskMode::ClarifyOrChat {
            entry: crate::ChatEntryStrategy::NormalizerThenChat,
        } => {
            if let Some(direct_answer) =
                direct_chat_answer_from_recent_replies(state, task, agent_run_context.as_ref())
            {
                return Ok(AskReply::non_llm(direct_answer));
            }
            let chat_prompt_context = chat_prompt_context_with_route_resolution(
                chat_prompt_context,
                agent_run_context.as_ref(),
            );
            let resolved_chat_prompt =
                crate::bootstrap::load_required_prompt_template_for_state_with_meta(
                    state,
                    crate::CHAT_RESPONSE_PROMPT_LOGICAL_PATH,
                )
                .map_err(|e| e.to_string())?;
            let chat_prompt_template = resolved_chat_prompt.template;
            let chat_prompt_source = resolved_chat_prompt.source;
            let chat_prompt_version = resolved_chat_prompt.version;
            crate::log_prompt_render_with_version(
                state,
                &task.task_id,
                "chat_response_prompt",
                &chat_prompt_source,
                chat_prompt_version.as_deref(),
                None,
            );
            let task_persona_prompt = state.task_persona_prompt(task);
            let chat_user_request = chat_user_request(resolved_prompt, execution_user_request);
            let current_turn_user_request =
                task_payload_text(task).unwrap_or_else(|| chat_user_request.to_string());
            let request_language_hint = crate::language_policy::task_response_language_hint(
                state,
                task,
                &current_turn_user_request,
            );
            let request_for_chat_prompt =
                chat_request_for_prompt(&current_turn_user_request, chat_user_request);
            let chat_prompt = crate::render_prompt_template(
                &chat_prompt_template,
                &[
                    ("__PERSONA_PROMPT__", &task_persona_prompt),
                    ("__CONTEXT__", &chat_prompt_context),
                    (
                        "__CONFIG_RESPONSE_LANGUAGE__",
                        &state.policy.command_intent.default_locale,
                    ),
                    ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
                    ("__REQUEST__", &request_for_chat_prompt),
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
        crate::AskMode::Act { .. } => {
            execute_via_planner_loop(
                state,
                task,
                prompt_with_memory,
                execution_user_request,
                &ask_mode,
                agent_run_context.clone(),
            )
            .await
        }
        crate::AskMode::ClarifyOrChat {
            entry: crate::ChatEntryStrategy::NormalizerThenClarify,
        } => {
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
                // §7.2: ask_flow 路由到 AskClarify 但 route_result 也没给 clarify_question
                // → IntentUnresolved（与 ask_pipeline 同语义）。
                crate::fallback::ClarifyFallbackSource::IntentUnresolved,
            )
            .await;
            Ok(AskReply::non_llm(clarify))
        }
        // Phase 3.2 Stage C5：execute_ask_routed 入参 normalizer_mode 是 RoutedMode
        // （4 个变体），经 from_routed_mode 派生只会得到上面 4 个 entry，
        // ResumeFollowupDiscussion / ResumeContinue 不在此入口。
        // 防御性地兜底回 chat 路径（与历史 fallback 一致：normalizer_mode 缺失也会
        // 走 Chat），同时打 warn 便于发现误用。
        other => {
            tracing::warn!(
                "{} worker_once: ask execute_ask_routed unexpected_ask_mode task_id={} ask_mode={}",
                crate::highlight_tag("routing"),
                task.task_id,
                other.as_str()
            );
            Err(format!(
                "execute_ask_routed unexpected ask_mode {}",
                other.as_str()
            ))
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
        chat_prompt_context_with_route_resolution, chat_request_for_prompt, chat_user_request,
        direct_same_or_different_answer_from_recent_replies, preferred_route_clarify_question,
        task_payload_text,
    };

    #[test]
    fn chat_prompt_context_appends_authoritative_route_resolution() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
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
    fn chat_prompt_context_replaces_empty_placeholder_with_route_resolution() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent: "client-like-continuous-20260428_144029".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.94),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let rendered = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));
        assert!(!rendered.contains("<none>"));
        assert!(rendered.contains("### ROUTE_RESOLUTION"));
        assert!(rendered.contains("client-like-continuous-20260428_144029"));
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
    fn chat_request_for_prompt_keeps_original_constraints_and_semantic_anchor() {
        let request = chat_request_for_prompt(
            "刚才我让你记住的测试编号是什么？只回答编号。",
            "client-like-continuous-20260428_144029",
        );
        assert!(request.contains("Original user request:"));
        assert!(request.contains("只回答编号"));
        assert!(request.contains("Resolved semantic intent:"));
        assert!(request.contains("client-like-continuous-20260428_144029"));
    }

    #[test]
    fn task_payload_text_preserves_raw_current_turn_for_chat_language_hint() {
        let task = crate::ClaimedTask {
            task_id: "task".to_string(),
            user_id: 1,
            chat_id: 1,
            user_key: None,
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: serde_json::json!({"text":"先只看登录模块"}).to_string(),
        };
        assert_eq!(task_payload_text(&task).as_deref(), Some("先只看登录模块"));
    }

    #[test]
    fn response_language_hint_prefers_current_request_language() {
        assert_eq!(
            crate::language_policy::preferred_response_language_hint("写个两句短诗", None),
            "zh-CN"
        );
        assert_eq!(
            crate::language_policy::preferred_response_language_hint(
                "do not run anything, just tell me a very short joke",
                None
            ),
            "en"
        );
        assert_eq!(
            crate::language_policy::preferred_response_language_hint(
                "用 English 解释 README",
                None
            ),
            "mixed"
        );
        assert_eq!(
            crate::language_policy::preferred_response_language_hint("12345", None),
            "config_default"
        );
    }

    #[test]
    fn preferred_route_clarify_question_respects_explicit_route_question_before_generic_fallback() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let mut route = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "看看那个目录下面都有什么".to_string(),
            needs_clarify: true,
            clarify_question: "请提供具体要查看的目录名或路径。".to_string(),
            route_reason: "fresh_deictic_missing_locator:directory_lookup".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                locator_kind: crate::OutputLocatorKind::Path,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route.clone()),
            ..Default::default()
        };
        assert_eq!(
            preferred_route_clarify_question(&state, Some(&ctx)).as_deref(),
            Some("请提供具体要查看的目录名或路径。")
        );

        route.clarify_question.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(
            preferred_route_clarify_question(&state, Some(&ctx)).as_deref(),
            Some("Please provide the specific file name or path to read.")
        );
    }

    #[test]
    fn preferred_route_clarify_question_uses_fuzzy_locator_candidates_without_llm() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "读取 Cargo.toml 的 package.name，只输出值".to_string(),
            needs_clarify: true,
            clarify_question: String::new(),
            route_reason: "route_contract:generic_filename_scalar_extract; fuzzy_locator_candidates=/tmp/a/Cargo.toml | /tmp/b/Cargo.toml".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: true,
                locator_kind: crate::OutputLocatorKind::Filename,
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let clarify = preferred_route_clarify_question(&state, Some(&ctx))
            .expect("fuzzy locator clarify should be synthesized");
        assert!(clarify.contains("/tmp/a/Cargo.toml"));
        assert!(clarify.contains("/tmp/b/Cargo.toml"));
        assert!(clarify.contains("1."));
        assert!(clarify.contains("2."));
    }
}
