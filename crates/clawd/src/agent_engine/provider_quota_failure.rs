use claw_core::provider_failure_policy::{
    ProviderFailureClass, PROVIDER_QUOTA_TERMINATED, PROVIDER_WAIT_RESUME_REASON,
};
use serde_json::json;

use super::{AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

pub(super) fn mark_if_exhausted(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
) -> bool {
    let Some(provider) = state.task_provider_quota_exhausted(&task.task_id) else {
        return false;
    };
    loop_state.last_stop_signal = Some(PROVIDER_QUOTA_TERMINATED.to_string());
    loop_state.task_checkpoint = None;
    loop_state.output_vars.remove("agent_loop.resume_reason");
    loop_state.task_lifecycle = Some(json!({
        "schema_version": 1, "state": "failed", "source": "llm_gateway",
        "reason_code": PROVIDER_QUOTA_TERMINATED, "message_key": "provider.quota_exhausted",
        "can_poll": true, "can_cancel": false, "retryable": false,
        "provider_status": {"provider": provider, "status_code": "quota_exhausted",
            "provider_error_class": "quota_exhausted", "external_provider_blocked": true,
            "retry_policy": "none", "retryable": false}
    }));
    true
}

pub(super) fn is_quota_stop(loop_state: &LoopState) -> bool {
    loop_state.last_stop_signal.as_deref() == Some(PROVIDER_QUOTA_TERMINATED)
}

pub(super) fn reply(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    context: Option<&AgentRunContext>,
) -> AskReply {
    let text = ProviderFailureClass::QuotaExhausted
        .terminal_notice_text()
        .expect("quota_terminal_notice_missing")
        .to_string();
    let journal = crate::finalize::build_from_loop_state(
        task,
        user_text,
        loop_state,
        context,
        None,
        true,
        &text,
        crate::task_journal::TaskJournalFinalStatus::Failure,
    );
    AskReply::non_llm(text)
        .with_failure(PROVIDER_QUOTA_TERMINATED)
        .with_task_journal(journal)
}

pub(crate) fn checkpoint_reply(
    state: &AppState,
    task: &ClaimedTask,
    checkpoint: &crate::task_lifecycle::TaskCheckpoint,
) -> Option<Result<AskReply, String>> {
    let provider = checkpoint_provider(checkpoint)?;
    if let Err(error) = super::ensure_task_running(state, task) {
        return Some(Err(error));
    }
    if let Some(metrics) = checkpoint.boundary_context.get("task_llm_metrics") {
        state.restore_task_llm_metrics_from_checkpoint(&task.task_id, metrics);
    }
    state.restore_task_llm_call_count_from_cost_ledger(&task.task_id);
    state.note_task_provider_quota_exhausted(&task.task_id, provider);
    let mut loop_state = LoopState::new();
    super::loop_state_seed::seed_loop_state_from_task_checkpoint(&mut loop_state, checkpoint);
    mark_if_exhausted(state, task, &mut loop_state);
    Some(Ok(reply(task, "", &loop_state, None)))
}

pub(super) fn checkpoint_provider(
    checkpoint: &crate::task_lifecycle::TaskCheckpoint,
) -> Option<String> {
    if checkpoint.schema_version != 1 {
        return None;
    }
    let boundary = &checkpoint.boundary_context;
    if boundary
        .get("resume_reason")
        .and_then(serde_json::Value::as_str)
        != Some(PROVIDER_WAIT_RESUME_REASON)
    {
        return None;
    }
    let status = boundary.get("provider_status")?;
    if ProviderFailureClass::from_str(status.get("status_code")?.as_str()?)
        != Some(ProviderFailureClass::QuotaExhausted)
        || status
            .get("external_provider_blocked")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return None;
    }
    status.get("provider")?.as_str().map(str::to_string)
}
