use serde_json::{json, Value};

use super::{
    status_answer::agent_loop_rich_content_should_defer_status, AgentRunContext, AppState,
    ClaimedTask, LoopState,
};

pub(crate) fn deterministic_observed_execution_status_answer(
    _state: &AppState,
    _task: &ClaimedTask,
    _user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if agent_loop_rich_content_should_defer_status(agent_run_context, loop_state) {
        return None;
    }
    let observed_steps = loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        })
        .collect::<Vec<_>>();
    if observed_steps.last().is_some_and(|step| step.is_ok()) {
        return None;
    }
    if observed_steps.len() < 2 || !observed_steps.iter().any(|step| !step.is_ok()) {
        return None;
    }

    let mut steps = Vec::new();
    let mut succeeded_count = 0usize;
    let mut failed_count = 0usize;
    for (idx, step) in observed_steps.iter().enumerate() {
        let step_no = idx + 1;
        let skill = step.skill.trim();
        let mut payload = serde_json::Map::new();
        payload.insert("step_no".to_string(), json!(step_no));
        payload.insert("step_id".to_string(), json!(step.step_id.trim()));
        payload.insert("skill".to_string(), json!(skill));
        if step.is_ok() {
            succeeded_count += 1;
            payload.insert("status".to_string(), json!("ok"));
            let output = step
                .output
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(|text| {
                    crate::truncate_for_agent_trace(
                        &crate::visible_text::sanitize_user_visible_text(text).replace('\n', " "),
                    )
                });
            if let Some(output) = output {
                payload.insert("output_excerpt".to_string(), json!(output));
            }
            steps.push(Value::Object(payload));
            continue;
        }
        failed_count += 1;
        payload.insert("status".to_string(), json!("error"));
        match step
            .error
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            Some(error) => {
                if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
                    payload.insert("error_kind".to_string(), json!(structured.error_kind));
                    payload.insert("error_skill".to_string(), json!(structured.skill));
                    payload.insert("error_platform".to_string(), json!(structured.platform));
                    let error_excerpt = crate::truncate_for_agent_trace(
                        &crate::visible_text::sanitize_user_visible_text(&structured.error_text)
                            .replace('\n', " "),
                    );
                    payload.insert("error_excerpt".to_string(), json!(error_excerpt));
                    if let Some(extra) = structured.extra {
                        payload.insert("error_extra".to_string(), extra);
                    }
                } else {
                    let error_excerpt = crate::truncate_for_agent_trace(
                        &crate::visible_text::sanitize_user_visible_text(error).replace('\n', " "),
                    );
                    payload.insert("error_excerpt".to_string(), json!(error_excerpt));
                }
            }
            None => {
                payload.insert(
                    "message_key".to_string(),
                    json!("clawd.msg.execution.step_error_missing"),
                );
                payload.insert(
                    "reason_code".to_string(),
                    json!("execution_step_error_missing"),
                );
            }
        }
        steps.push(Value::Object(payload));
    }
    Some(
        json!({
            "message_key": "clawd.msg.execution.step_status_summary",
            "reason_code": "observed_execution_status",
            "status": "error",
            "succeeded_count": succeeded_count,
            "failed_count": failed_count,
            "steps": steps,
            "text": Value::Null,
        })
        .to_string(),
    )
}
