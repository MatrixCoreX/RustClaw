use serde::Deserialize;
use serde_json::{json, Value};

const SUFFIX: &str = "progress-started";
// Optional opt-in path: loaded on demand, not required for hosts without progress notices.
const PROMPT: &str = "prompts/layers/overlays/runtime_progress_start.md";

pub(super) fn start_evidence(frame: &skill_sdk::SkillProgressFrame) -> Option<Value> {
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
    if capability.is_empty()
        || !capability.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-')
        })
    {
        return None;
    }
    // Project only this version's public machine contract, never arbitrary prose/paths.
    Some(json!({
        "event": "started", "continuous": continuous,
        "requested_items": params.get("requested_items")?.as_u64()?,
        "max_run_minutes": params.get("max_run_minutes")?.as_u64()?,
        "stop_capability": capability,
        "stop_after_current_item": params.get("stop_after_current_item")?.as_bool()?,
    }))
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
    evidence: &Value,
) -> Result<(), String> {
    let already_attempted: bool = state
        .core
        .db
        .get()
        .map_err(|e| e.to_string())?
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM channel_delivery_receipts WHERE delivery_id = ?1)",
            rusqlite::params![format!("delivery:{}:{SUFFIX}", task.task_id)],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if already_attempted {
        return Ok(());
    }
    let request = crate::language_policy::task_original_user_text(task).unwrap_or_default();
    let language = crate::language_policy::task_response_language_hint(state, task, &request);
    let context = json!({ "user_request": request, "language": language, "progress": evidence });
    let (template, source) =
        crate::bootstrap::load_required_prompt_template_for_state(state, PROMPT)
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
    let envelope =
        crate::delivery_service::build_proactive_text_envelope(state, task, payload, SUFFIX, &text)
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
