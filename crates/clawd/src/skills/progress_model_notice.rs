use serde::Deserialize;
use serde_json::{json, Value};

const START_PROMPT: &str = "prompts/layers/overlays/runtime_progress_start.md";
const LOGIN_PROMPT: &str = "prompts/layers/overlays/runtime_progress_login.md";

#[derive(Debug, Clone)]
pub(crate) struct ModelProgressNotice {
    pub(crate) evidence: Value,
    pub(crate) prompt_path: &'static str,
    pub(crate) suffix: String,
    pub(crate) persist_in_ui: bool,
}

fn machine_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-')
        })
}

fn start_notice(frame: &skill_sdk::SkillProgressFrame) -> Option<ModelProgressNotice> {
    let params = &frame.params;
    if !matches!(frame.kind, skill_sdk::SkillProgressKind::Progress)
        || params.get("notification_delivery")?.as_str()? != "runtime"
        || params.get("notification_renderer")?.as_str()? != "model"
        || params.get("notification_event")?.as_str()? != "started"
    {
        return None;
    }
    let continuous = params.get("continuous")?.as_bool()?;
    let capability = params.get("stop_capability")?.as_str()?;
    if !machine_token(capability) {
        return None;
    }
    // Project only this version's public machine contract, never arbitrary prose/paths.
    Some(ModelProgressNotice {
        evidence: json!({
            "event": "started", "continuous": continuous,
            "requested_items": params.get("requested_items")?.as_u64()?,
            "max_run_minutes": params.get("max_run_minutes")?.as_u64()?,
            "stop_capability": capability,
            "stop_after_current_item": params.get("stop_after_current_item")?.as_bool()?,
        }),
        prompt_path: START_PROMPT,
        suffix: "progress-started".to_string(),
        persist_in_ui: false,
    })
}

fn login_notice(frame: &skill_sdk::SkillProgressFrame) -> Option<ModelProgressNotice> {
    let params = &frame.params;
    if !matches!(frame.kind, skill_sdk::SkillProgressKind::Progress)
        || params.get("notification_delivery")?.as_str()? != "runtime"
        || params.get("notification_renderer")?.as_str()? != "model"
        || params.get("notification_event")?.as_str()? != "login_required"
    {
        return None;
    }
    let platform = params.get("platform")?.as_str()?;
    let reason_code = params.get("reason_code")?.as_str()?;
    if !machine_token(platform)
        || !matches!(reason_code, "login_required" | "challenge_required")
        || params.get("interaction")?.as_str()? != "browser_window"
    {
        return None;
    }
    Some(ModelProgressNotice {
        evidence: json!({
            "event": "login_required",
            "platform": platform,
            "reason_code": reason_code,
            "interaction": "browser_window",
            "task_waiting": true,
        }),
        prompt_path: LOGIN_PROMPT,
        suffix: format!("progress-login-required-{platform}"),
        persist_in_ui: true,
    })
}

pub(super) fn notice(frame: &skill_sdk::SkillProgressFrame) -> Option<ModelProgressNotice> {
    start_notice(frame).or_else(|| login_notice(frame))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelNotice {
    text: String,
}

fn notice_text(raw: &str) -> Result<String, String> {
    let notice = crate::prompt_utils::parse_llm_json_raw_or_any::<ModelNotice>(raw.trim())
        .ok_or_else(|| "progress_notice_model_response_invalid".to_string())?;
    if notice.text.trim().is_empty() {
        return Err("progress_notice_model_response_empty".to_string());
    }
    Ok(notice.text.trim().to_string())
}

pub(super) async fn deliver(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    notice: &ModelProgressNotice,
) -> Result<(), String> {
    if !task.channel.trim().eq_ignore_ascii_case("ui") {
        let already_attempted: bool = state
            .core
            .db
            .get()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM channel_delivery_receipts WHERE delivery_id = ?1)",
                rusqlite::params![format!("delivery:{}:{}", task.task_id, notice.suffix)],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if already_attempted {
            return Ok(());
        }
    }
    let request = crate::language_policy::task_original_user_text(task).unwrap_or_default();
    let language = crate::language_policy::task_response_language_hint(state, task, &request);
    let context = json!({
        "user_request": request,
        "language": language,
        "progress": notice.evidence,
    });
    let (template, source) =
        crate::bootstrap::load_required_prompt_template_for_state(state, notice.prompt_path)
            .map_err(|_| "progress_notice_prompt_unavailable".to_string())?;
    let prompt =
        crate::render_prompt_template(&template, &[("__CONTEXT_JSON__", &context.to_string())]);
    crate::log_prompt_render(
        state,
        &task.task_id,
        "progress_notice_prompt",
        &source,
        None,
    );
    let raw =
        crate::llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &source)
            .await
            .map_err(|_| "progress_notice_model_unavailable".to_string())?;
    let text = notice_text(&raw)?;
    if task.channel.trim().eq_ignore_ascii_case("ui") {
        let snapshot =
            crate::repo::conversation_inputs::conversation_presentation_snapshot_for_task(
                &state.core.db,
                &task.task_id,
            )
            .map_err(|error| error.to_string())?;
        let (instruction_revision, execution_epoch) = snapshot
            .map(|value| (value.instruction_revision, value.execution_epoch))
            .unwrap_or((0, 0));
        crate::repo::conversation_reply_items::persist_nonterminal_reply_item(
            state,
            task,
            "side_reply",
            "accepted",
            &text,
            instruction_revision,
            execution_epoch,
        )
        .map_err(|error| error.to_string())?;
        return Ok(());
    }
    let envelope = crate::delivery_service::build_proactive_text_envelope(
        state,
        task,
        payload,
        &notice.suffix,
        &text,
    )
    .map_err(|e| e.to_string())?;
    let result = crate::delivery_service::deliver_task_envelope(state, task, payload, &envelope)
        .await
        .map_err(|e| e.to_string())?;
    if result.accepted() {
        Ok(())
    } else {
        Err(result
            .error_code
            .unwrap_or_else(|| "progress_notice_delivery_not_accepted".to_string()))
    }
}

#[cfg(test)]
#[path = "progress_model_notice_tests.rs"]
mod tests;
