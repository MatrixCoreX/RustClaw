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
fn is_explicit_schedule_direct_text(payload: &Value, prompt: &str) -> bool {
    let is_schedule_triggered = payload
        .get("schedule_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let schedule_task_mode = payload
        .get("schedule_task_mode")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let schedule_force_agent = payload
        .get("schedule_force_agent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    is_schedule_triggered
        && !schedule_force_agent
        && schedule_task_mode
            .eq_ignore_ascii_case(crate::schedule_service::SCHEDULE_TASK_MODE_DIRECT_TEXT)
        && !prompt.trim().is_empty()
}

pub(super) async fn maybe_finalize_schedule_direct_text_success(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    prompt: &str,
) -> anyhow::Result<bool> {
    if !is_explicit_schedule_direct_text(payload, prompt) {
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

#[cfg(test)]
mod tests {
    use super::is_explicit_schedule_direct_text;
    use serde_json::json;

    #[test]
    fn schedule_without_explicit_direct_text_runs_through_agent() {
        for payload in [
            json!({"schedule_triggered": true}),
            json!({"schedule_triggered": true, "schedule_task_mode": "agent"}),
            json!({"schedule_triggered": true, "schedule_task_mode": "unknown"}),
        ] {
            assert!(!is_explicit_schedule_direct_text(
                &payload,
                "scheduled-work-fixture"
            ));
        }
    }

    #[test]
    fn only_explicit_direct_text_can_finalize_before_agent() {
        assert!(is_explicit_schedule_direct_text(
            &json!({
                "schedule_triggered": true,
                "schedule_task_mode": "direct_text"
            }),
            "literal-reminder-fixture"
        ));
        assert!(!is_explicit_schedule_direct_text(
            &json!({
                "schedule_triggered": true,
                "schedule_task_mode": "direct_text",
                "schedule_force_agent": true
            }),
            "scheduled-work-fixture"
        ));
        assert!(!is_explicit_schedule_direct_text(
            &json!({
                "schedule_triggered": true,
                "schedule_task_mode": "direct_text"
            }),
            "  "
        ));
    }
}
