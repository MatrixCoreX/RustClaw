use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

pub(super) struct PreparedAskInput {
    pub(super) prompt: String,
    pub(super) source: String,
}

pub(super) struct PreparedRunSkillInput {
    pub(super) skill_name: String,
    pub(super) args: Value,
}

pub(super) fn opaque_user_prompt(payload: &Value) -> &str {
    payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

pub(super) async fn prepare_ask_input(
    _state: &AppState,
    _task: &ClaimedTask,
    payload: &mut Value,
) -> PreparedAskInput {
    PreparedAskInput {
        prompt: opaque_user_prompt(payload).to_string(),
        source: payload
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

pub(super) fn prepare_run_skill_input(payload: &Value) -> PreparedRunSkillInput {
    PreparedRunSkillInput {
        skill_name: payload
            .get("skill_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        args: payload.get("args").cloned().unwrap_or_else(|| json!("")),
    }
}

/// Scheduled direct-text delivery is an explicit protocol mode, not an
/// ordinary semantic routing decision.
pub(super) async fn maybe_finalize_schedule_direct_text_success(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    prompt: &str,
) -> anyhow::Result<bool> {
    let is_schedule_triggered = payload
        .get("schedule_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let schedule_task_mode = payload
        .get("schedule_task_mode")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let schedule_force_agent = payload
        .get("schedule_force_agent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !is_schedule_triggered
        || schedule_force_agent
        || (!schedule_task_mode.is_empty() && schedule_task_mode != "direct_text")
        || prompt.trim().is_empty()
    {
        return Ok(false);
    }

    let answer_text = crate::intercept_response_text_for_delivery(prompt.trim());
    crate::finalize::finalize_ask_direct_success(
        state,
        task,
        payload,
        prompt,
        &answer_text,
        "schedule_direct_text",
        false,
        "",
    )
    .await?;
    Ok(true)
}
