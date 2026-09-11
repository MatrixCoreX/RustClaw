use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

use super::machine_envelope::{machine_envelope_reports_failure, raw_machine_envelope_payload};

pub(super) async fn finalize_unresolved_machine_failure(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<AskReply> {
    // A later successful action/response belongs to the normal recovery path.
    let step = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.skill != "think")?;
    let payload = raw_machine_envelope_payload(step.output.as_deref()?)?;
    if step.is_ok() && !machine_envelope_reports_failure(&payload) {
        return None;
    }
    let reason = payload
        .get("error_code")
        .and_then(serde_json::Value::as_str)
        .or(step.error.as_deref())
        .unwrap_or("machine_execution_failed");
    let language_hint = super::final_reply_language_hint(state, task, user_text, agent_run_context);
    let contract = crate::fallback::UserResponseContract::tool_failure(
        reason,
        user_text,
        &super::route_resolved_intent(agent_run_context),
        vec![
            format!("step_id: {}", step.step_id),
            format!("skill: {}", step.skill),
            format!(
                "execution_observation: {}",
                crate::truncate_for_agent_trace(&payload.to_string())
            ),
        ],
        vec![
            "success_allowed=false".to_string(),
            "raw_machine_payload_visible=false".to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    let message = crate::fallback::compose_user_response_from_contract(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
    )
    .await;
    loop_state.delivery_messages = vec![message.clone()];
    loop_state.last_user_visible_respond = Some(message.clone());
    let summary = crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        parsed: true,
        contract_ok: false,
        completion_ok: Some(false),
        needs_clarify: Some(false),
        ..Default::default()
    };
    let journal = crate::finalize::build_terminal_from_loop_state(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
        Some(summary),
        true,
        &message,
        crate::task_journal::TaskJournalFinalStatus::Failure,
    )
    .await;
    Some(
        AskReply::non_llm(message.clone())
            .with_messages(vec![message.clone()])
            .with_task_journal(journal)
            .with_failure(message),
    )
}
