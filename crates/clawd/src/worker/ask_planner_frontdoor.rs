use anyhow::Result;
use serde_json::Value;
use tracing::info;

use crate::{AppState, ClaimedTask};

pub(super) struct PreparedAskRouting {
    pub(super) turn_boundary_envelope: crate::turn_boundary_envelope::TurnBoundaryEnvelope,
    pub(super) planner_user_request: String,
}

/// Builds only machine-owned context before the first planner round.
pub(super) async fn prepare_planner_owned_ask_routing(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    prompt: &str,
    _source: &str,
) -> Result<PreparedAskRouting> {
    let transcribed_prompt =
        crate::transcribe_attached_audio_for_ask(state, task, payload, prompt).await?;
    let attachment_count = payload
        .get("attachments")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let input_materialization = crate::turn_boundary_envelope::TurnInputMaterialization::classify(
        transcribed_prompt.is_some(),
        !prompt.trim().is_empty(),
        attachment_count,
    );
    let planner_user_request = transcribed_prompt.unwrap_or_else(|| prompt.to_string());
    let planner_user_request =
        crate::ui_attachments::prompt_with_ui_attachment_context(&planner_user_request, payload);
    let turn_boundary_envelope =
        crate::turn_boundary_envelope::TurnBoundaryEnvelope::from_claimed_task(
            task,
            payload,
            prompt,
            input_materialization,
            crate::agent_engine::explicit_machine_syntax_command_segment(prompt),
            crate::skills::task_allows_path_outside_workspace(state, Some(task)),
            crate::skills::task_allows_sudo(state, Some(task)),
        );
    info!(
        "{} planner_owned_frontdoor task_id={} attachment_count={} explicit_locator_count={} explicit_command={} raw_chars={}",
        crate::highlight_tag("routing"),
        task.task_id,
        turn_boundary_envelope.attachment_refs.len(),
        turn_boundary_envelope.structured_locator_facts.len(),
        turn_boundary_envelope.explicit_machine_command.is_some(),
        turn_boundary_envelope.raw_chars,
    );

    Ok(PreparedAskRouting {
        turn_boundary_envelope,
        planner_user_request,
    })
}
