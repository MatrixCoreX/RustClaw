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
    let audio_materialization =
        crate::transcribe_attached_audio_for_ask(state, task, payload, "").await?;
    let video_materialization =
        crate::ask_flow::transcribe_attached_videos_for_ask(state, task, payload, prompt).await?;
    let attachment_count = payload
        .get("attachments")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let input_materialization = crate::turn_boundary_envelope::TurnInputMaterialization::classify(
        audio_materialization
            .as_ref()
            .is_some_and(|materialization| materialization.transcript_available)
            || video_materialization
                .as_ref()
                .is_some_and(|materialization| materialization.transcript_available),
        !prompt.trim().is_empty(),
        attachment_count,
    );
    let mut materialized_inputs = Vec::new();
    if let Some(materialization) = audio_materialization {
        materialized_inputs.push(materialization.planner_text);
    }
    if let Some(materialization) = video_materialization {
        materialized_inputs.push(materialization.planner_text);
    }
    let planner_user_request = if materialized_inputs.is_empty() {
        prompt.to_string()
    } else {
        let mut request = materialized_inputs.join("\n\n");
        if !prompt.trim().is_empty() {
            request.push_str("\n\n[AGENT_TYPED_TEXT]\n");
            request.push_str(prompt.trim());
            request.push_str("\n[/AGENT_TYPED_TEXT]");
        }
        request
    };
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
