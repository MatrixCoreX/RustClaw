use serde_json::{json, Value};
use std::path::Path;

use crate::{AppState, AskReply, ClaimedTask, RoutedMode};

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

fn normalizer_answer_candidate_from_resolved_prompt(resolved_prompt: &str) -> Option<String> {
    let (_intent, candidate) = resolved_prompt.rsplit_once("\nanswer_candidate:")?;
    let candidate = candidate.trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

fn paths_refer_to_same_existing_location(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn normalizer_answer_candidate_matches_runtime_fact(state: &AppState, candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() || candidate.contains('\n') {
        return false;
    }
    let candidate_path = Path::new(candidate);
    if !candidate_path.is_absolute() {
        return false;
    }
    if paths_refer_to_same_existing_location(candidate_path, &state.skill_rt.workspace_root) {
        return true;
    }
    std::env::current_dir()
        .ok()
        .is_some_and(|cwd| paths_refer_to_same_existing_location(candidate_path, &cwd))
}

fn normalizer_chat_direct_answer_candidate(
    state: &AppState,
    resolved_prompt: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
    allow_budget_fallback: bool,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    if route.needs_clarify || route.is_execute_gate() {
        return None;
    }
    let contract = &route.output_contract;
    if contract.requires_content_evidence
        || contract.delivery_required
        || !matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
    {
        return None;
    }
    let candidate = normalizer_answer_candidate_from_resolved_prompt(resolved_prompt)?;
    if normalizer_answer_candidate_matches_runtime_fact(state, &candidate) {
        return Some(candidate);
    }
    allow_budget_fallback.then_some(candidate)
}

fn runtime_scalar_path_direct_answer_candidate(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    if route.needs_clarify || !route.is_execute_gate() {
        return None;
    }
    let contract = &route.output_contract;
    if !matches!(contract.response_shape, crate::OutputResponseShape::Scalar)
        || !matches!(
            contract.semantic_kind,
            crate::OutputSemanticKind::ScalarPathOnly
        )
        || !matches!(
            contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        )
        || contract.delivery_required
        || !matches!(contract.delivery_intent, crate::OutputDeliveryIntent::None)
    {
        return None;
    }
    let candidate = contract.locator_hint.trim();
    normalizer_answer_candidate_matches_runtime_fact(state, candidate)
        .then(|| candidate.to_string())
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

fn fuzzy_locator_clarify_context(candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let candidate_block = candidates
        .iter()
        .enumerate()
        .map(|(idx, value)| format!("candidate_{}: {}", idx + 1, value))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "clarify_case: fuzzy_locator_candidates\nexact_target_found: false\ncandidate_count: {}\n{}",
        candidates.len(),
        candidate_block
    ))
}

fn preferred_route_clarify_question(
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
    None
}

fn route_structured_clarify_context(
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if let Some(context) = fuzzy_locator_clarify_context(&ctx.fuzzy_locator_suggestions) {
        return Some(context);
    }
    if !route.needs_clarify || !route.output_contract.locator_hint.trim().is_empty() {
        return None;
    }
    let clarify_case = if route.output_contract.delivery_required {
        Some("missing_file_locator")
    } else if route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        )
    {
        Some("missing_read_target")
    } else {
        None
    }?;
    Some(
        [
            format!("clarify_case: {clarify_case}"),
            format!(
                "locator_kind: {}",
                route.output_contract.locator_kind.as_str()
            ),
            format!(
                "response_shape: {}",
                route.output_contract.response_shape.as_str()
            ),
            format!(
                "requires_content_evidence: {}",
                route.output_contract.requires_content_evidence
            ),
            format!(
                "delivery_required: {}",
                route.output_contract.delivery_required
            ),
        ]
        .join("\n"),
    )
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
    format!(
        "Original user request:\n{original}\n\nResolved semantic intent / answer candidate:\n{semantic}\n\nUse the resolved semantic intent to answer the original request. If the original request asks for only a value, ID, path, name, or one short answer, output only the resolved value with no preamble."
    )
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
    if let Some(candidate) =
        runtime_scalar_path_direct_answer_candidate(state, agent_run_context.as_ref())
    {
        tracing::info!(
            "{} worker_once: ask runtime_scalar_path_direct_answer task_id={} len={}",
            crate::highlight_tag("routing"),
            task.task_id,
            candidate.len()
        );
        return Ok(AskReply::llm(candidate));
    }
    match &ask_mode {
        crate::AskMode::ClarifyOrChat {
            entry: crate::ChatEntryStrategy::NormalizerThenChat,
        } => {
            let allow_candidate_budget_fallback =
                state.task_llm_budget_exceeded(&task.task_id).is_some();
            if let Some(candidate) = normalizer_chat_direct_answer_candidate(
                state,
                resolved_prompt,
                agent_run_context.as_ref(),
                allow_candidate_budget_fallback,
            ) {
                tracing::info!(
                    "{} worker_once: ask normalizer_answer_candidate_budget_fallback task_id={} len={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    candidate.len()
                );
                return Ok(AskReply::llm(candidate));
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
            let preferred_clarify = preferred_route_clarify_question(agent_run_context.as_ref());
            let structured_clarify_context =
                route_structured_clarify_context(agent_run_context.as_ref());
            let clarify_policy = if structured_clarify_context.is_some()
                || (preferred_clarify.is_none()
                    && agent_run_context
                        .as_ref()
                        .and_then(|ctx| ctx.route_result.as_ref())
                        .is_some_and(|route| route.clarify_question.trim().is_empty()))
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
                structured_clarify_context.as_deref(),
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
        normalizer_chat_direct_answer_candidate, preferred_route_clarify_question,
        route_structured_clarify_context, runtime_scalar_path_direct_answer_candidate,
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
        assert!(request.contains("Resolved semantic intent / answer candidate:"));
        assert!(request.contains("client-like-continuous-20260428_144029"));
        assert!(request.contains("output only the resolved value"));
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
    fn normalizer_chat_direct_answer_uses_candidate_only_without_evidence_contract() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent:
                "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer supplied candidate".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜",
                Some(&ctx),
                true,
            )
            .as_deref(),
            Some("早出晚归血汗钱\n苦中作乐笑开颜")
        );

        assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜",
                Some(&ctx),
                false,
            ),
            None
        );
    }

    #[test]
    fn normalizer_chat_direct_answer_does_not_bypass_evidence_contract() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent:
                "检查当前目录是否有隐藏文件\nanswer_candidate: 有，例如 .git、.gitignore、.pids"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "needs local evidence".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Medium,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Strict,
                requires_content_evidence: true,
                semantic_kind: crate::OutputSemanticKind::HiddenEntriesCheck,
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                "检查当前目录是否有隐藏文件\nanswer_candidate: 有，例如 .git、.gitignore、.pids",
                Some(&ctx),
                true,
            ),
            None
        );
    }

    #[test]
    fn normalizer_chat_direct_answer_uses_runtime_fact_candidate_without_budget_fallback() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
            resolved_intent: format!(
                "User request: output absolute path of current working directory\nanswer_candidate: {runtime_path}"
            ),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer supplied runtime fact".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                &format!(
                    "User request: output absolute path of current working directory\nanswer_candidate: {runtime_path}"
                ),
                Some(&ctx),
                false,
            )
            .as_deref(),
            Some(runtime_path.as_str())
        );
    }

    #[test]
    fn runtime_scalar_path_direct_answer_uses_verified_contract_locator() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "Output the current workspace path".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "runtime scalar path".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Scalar,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                locator_hint: runtime_path.clone(),
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
            Some(runtime_path.as_str())
        );
    }

    #[test]
    fn runtime_scalar_path_direct_answer_rejects_unverified_locator() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "Output the current workspace path".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "runtime scalar path".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Scalar,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                locator_hint: "/tmp/not-the-rustclaw-workspace".to_string(),
                ..Default::default()
            },
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };

        assert_eq!(
            runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)),
            None
        );
    }

    #[test]
    fn preferred_route_clarify_question_respects_explicit_route_question_before_generic_fallback() {
        let mut route = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "看看那个目录下面都有什么".to_string(),
            needs_clarify: true,
            clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
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
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route.clone()),
            ..Default::default()
        };
        assert_eq!(
            preferred_route_clarify_question(Some(&ctx)).as_deref(),
            Some("LOCATOR_CLARIFY_PROMPT")
        );

        route.clarify_question.clear();
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(preferred_route_clarify_question(Some(&ctx)), None);
        let context = route_structured_clarify_context(Some(&ctx)).expect("structured context");
        assert!(context.contains("clarify_case: missing_read_target"));
        assert!(context.contains("locator_kind: path"));
    }

    #[test]
    fn fuzzy_locator_candidates_are_structured_context_not_hard_question() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::AskClarify,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::AskClarify),
            resolved_intent: "读取 Cargo.toml 的 package.name，只输出值".to_string(),
            needs_clarify: true,
            clarify_question: String::new(),
            route_reason: "llm_contract:generic_filename_scalar_extract".to_string(),
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
        };
        let ctx = crate::agent_engine::AgentRunContext {
            route_result: Some(route),
            fuzzy_locator_suggestions: vec![
                "/tmp/a/Cargo.toml".to_string(),
                "/tmp/b/Cargo.toml".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(preferred_route_clarify_question(Some(&ctx)), None);
        let context = route_structured_clarify_context(Some(&ctx)).expect("structured context");
        assert!(context.contains("clarify_case: fuzzy_locator_candidates"));
        assert!(context.contains("candidate_1: /tmp/a/Cargo.toml"));
        assert!(context.contains("candidate_2: /tmp/b/Cargo.toml"));
    }
}
