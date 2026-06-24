use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::ClaimedTask;

use super::log_deterministic_delivery_record;

pub(super) fn attach_machine_envelope_delivery_from_loop(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(message) = latest_machine_envelope_message(loop_state) else {
        return false;
    };
    if !loop_state
        .delivery_messages
        .iter()
        .any(|existing| existing.trim() == message)
    {
        append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, message);
    }
    mark_machine_envelope_loop_complete(task, loop_state, finalizer_summary, agent_run_context)
}

pub(super) fn mark_machine_envelope_delivery_complete(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    delivery_messages: &[String],
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    if !delivery_messages
        .iter()
        .any(|message| machine_envelope_payload(message).is_some())
    {
        return false;
    }
    mark_machine_envelope_loop_complete(task, loop_state, finalizer_summary, agent_run_context)
}

fn mark_machine_envelope_loop_complete(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(message) = latest_machine_envelope_message(loop_state) else {
        return false;
    };
    loop_state.last_user_visible_respond = Some(message);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len().max(1),
        ..Default::default()
    });
    log_deterministic_delivery_record(
        &task.task_id,
        "machine_envelope_terminal_delivery",
        "kept",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn latest_machine_envelope_message(loop_state: &LoopState) -> Option<String> {
    loop_state
        .delivery_messages
        .iter()
        .rev()
        .find_map(|message| machine_envelope_payload(message).map(|_| message.trim().to_string()))
        .or_else(|| {
            loop_state
                .last_user_visible_respond
                .as_deref()
                .and_then(|message| {
                    machine_envelope_payload(message).map(|_| message.trim().to_string())
                })
        })
}

fn machine_envelope_payload(message: &str) -> Option<serde_json::Value> {
    let payload: serde_json::Value = serde_json::from_str(message.trim()).ok()?;
    if !payload.is_object() {
        return None;
    }
    if payload
        .get("output_format")
        .and_then(serde_json::Value::as_str)
        != Some("machine_json")
    {
        return None;
    }
    let owner_layer = payload
        .get("owner_layer")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if owner_layer.is_empty() {
        return None;
    }
    Some(payload)
}
