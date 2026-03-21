use serde_json::{json, Value};

use crate::{AppState, AskReply, ClaimedTask, RoutedMode};

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
    let resume_steps = payload
        .get("resume_steps")
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
    let resume_context_json = serde_json::to_string_pretty(&resume_context)
        .unwrap_or_else(|_| resume_context.to_string());
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
    let resume_context_json = serde_json::to_string_pretty(&resume_context)
        .unwrap_or_else(|_| resume_context.to_string());
    let (prompt_template, _) = crate::bootstrap::load_prompt_template_for_state(
        state,
        crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_PATH,
        crate::RESUME_FOLLOWUP_DISCUSSION_PROMPT_TEMPLATE,
    );
    crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text),
            ("__RESUME_CONTEXT__", &resume_context_json),
        ],
    )
}

fn chat_act_goal_from_prompt(prompt_with_memory: &str) -> String {
    format!(
        "{}\n\nMode hint: chat_act. Complete required actions first, then return a concise user-facing reply that confirms results naturally.",
        prompt_with_memory
    )
}

pub(crate) async fn execute_ask_routed(
    state: &AppState,
    task: &ClaimedTask,
    chat_prompt_context: &str,
    prompt_with_memory: &str,
    resolved_prompt: &str,
    agent_mode: bool,
    resume_force_chat: bool,
    normalizer_mode: Option<RoutedMode>,
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
            let (chat_prompt_template, chat_prompt_file) =
                crate::bootstrap::load_prompt_template_for_state(
                    state,
                    crate::CHAT_RESPONSE_PROMPT_PATH,
                    crate::CHAT_RESPONSE_PROMPT_TEMPLATE,
                );
            crate::log_prompt_render(
                &task.task_id,
                "chat_response_prompt",
                &chat_prompt_file,
                None,
            );
            let task_persona_prompt = state.task_persona_prompt(task);
            let chat_prompt = crate::render_prompt_template(
                &chat_prompt_template,
                &[
                    ("__PERSONA_PROMPT__", &task_persona_prompt),
                    ("__CONTEXT__", chat_prompt_context),
                    ("__REQUEST__", resolved_prompt),
                ],
            );
            crate::llm_gateway::run_with_fallback_with_prompt_file(
                state,
                task,
                &chat_prompt,
                &chat_prompt_file,
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
                resolved_prompt,
            )
            .await
        }
        RoutedMode::ChatAct => {
            crate::agent_engine::run_agent_with_tools(
                state,
                task,
                &chat_act_goal_from_prompt(prompt_with_memory),
                resolved_prompt,
            )
            .await
        }
        RoutedMode::AskClarify => {
            let clarify = crate::intent_router::generate_clarify_question(
                state,
                task,
                resolved_prompt,
                "router_selected_ask_clarify",
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
