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
    let (payload, host_setup_failure) = unresolved_failure_payload(step)?;
    let reason = payload
        .get("error_code")
        .and_then(serde_json::Value::as_str)
        .or(step.error.as_deref())
        .unwrap_or("machine_execution_failed");
    let language_hint = super::final_reply_language_hint(state, task, user_text, agent_run_context);
    let mut observed_facts = vec![
        format!("step_id: {}", step.step_id),
        format!("skill: {}", step.skill),
        format!(
            "execution_observation: {}",
            crate::truncate_for_agent_trace(&payload.to_string())
        ),
    ];
    if let Some(statuses) = child_task_status_projection(&payload) {
        observed_facts.push(format!("child_task_statuses: {statuses}"));
    }
    if host_setup_failure {
        match crate::agent_engine::observed_output::finalization_observed_output_entries(
            state, task, loop_state,
        ) {
            Ok(entries) => observed_facts.extend(entries),
            Err(_) => {
                observed_facts.push("completed_observation_projection_available=false".into())
            }
        }
    }
    let mut policy = vec![
        "success_allowed=false".to_string(),
        "raw_machine_payload_visible=false".to_string(),
        "child_status_source=child_task_statuses".to_string(),
        "failed_parent_does_not_fail_successful_siblings=true".to_string(),
    ];
    if host_setup_failure {
        policy.push("failure_owner=runtime_setup".into());
        policy.push("missing_target_request_allowed=false".into());
    }
    let contract = crate::fallback::UserResponseContract::tool_failure(
        reason,
        user_text,
        &super::route_resolved_intent(agent_run_context),
        observed_facts,
        policy,
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
    loop_state
        .delivery_messages
        .retain(|existing| raw_machine_envelope_payload(existing).is_none());
    crate::agent_engine::append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        message.clone(),
    );
    let delivery = loop_state.delivery_messages.clone();
    let text = super::final_answer_text_from_delivery(&delivery);
    loop_state.last_user_visible_respond = Some(text.clone());
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
        crate::task_journal::delivery_payload_consistent(&text, &delivery),
        &text,
        crate::task_journal::TaskJournalFinalStatus::Failure,
    )
    .await;
    Some(
        AskReply::non_llm(text)
            .with_messages(delivery)
            .with_task_journal(journal)
            .with_failure(message),
    )
}

fn unresolved_failure_payload(
    step: &crate::executor::StepExecutionResult,
) -> Option<(serde_json::Value, bool)> {
    if let Some(payload) = step
        .output
        .as_deref()
        .and_then(raw_machine_envelope_payload)
    {
        return (!step.is_ok() || machine_envelope_reports_failure(&payload))
            .then_some((payload, false));
    }
    if step.is_ok() {
        return None;
    }
    // Runner admission failures live in the typed error channel, not stdout.
    // Decode once through the shared producer-aware contract; never infer the
    // failure owner from the diagnostic prose or the requested target's name.
    let error = crate::skills::parse_structured_skill_error(step.error.as_deref()?)?;
    let extra = error.extra?;
    if extra.get("message_key").and_then(serde_json::Value::as_str)
        != Some("clawd.skill.pre_dispatch_failed")
        || extra
            .get("failure_phase")
            .and_then(serde_json::Value::as_str)
            != Some("pre_dispatch")
        || extra
            .get("side_effect_applied")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
        || extra
            .get("recovery_action")
            .and_then(serde_json::Value::as_str)
            != Some("refresh_skill_admission")
    {
        return None;
    }
    Some((
        serde_json::json!({"schema_version":1,"status":"error",
        "source_skill":error.skill,"error_code":error.error_code,"extra":extra}),
        true,
    ))
}

#[cfg(test)]
#[path = "loop_reply_machine_failure_tests.rs"]
mod tests;

fn child_task_status_projection(payload: &serde_json::Value) -> Option<serde_json::Value> {
    let nodes = payload
        .pointer("/child_task_merge/child_task_graph/nodes")?
        .as_array()?;
    let statuses = nodes
        .iter()
        .take(16)
        .map(|node| {
            serde_json::json!({
                "child_task_id": node.get("child_task_id"),
                "role": node.get("role"),
                "status": node.get("task_status").or_else(|| node.get("execution_state")),
                "error_code": node.pointer("/runtime/child_task_result/error_code"),
                "evidence_refs": node.pointer("/runtime/child_task_result/evidence_refs"),
                "finding_refs": node.pointer("/runtime/child_task_result/finding_refs"),
                "structured_result": node.pointer("/runtime/child_task_result/structured_result")
                    .filter(|result| result.to_string().len() <= 4096),
                "structured_result_projection_limited": node.pointer("/runtime/child_task_result/structured_result")
                    .is_some_and(|result| result.to_string().len() > 4096),
            })
        })
        .collect::<Vec<_>>();
    (!statuses.is_empty()).then(|| serde_json::json!(statuses))
}

#[cfg(test)]
#[path = "loop_reply_child_status_tests.rs"]
mod child_status_tests;
