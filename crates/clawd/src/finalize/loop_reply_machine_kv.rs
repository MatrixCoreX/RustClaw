use serde_json::Value;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::ClaimedTask;

use super::{final_answer_text_from_delivery, log_deterministic_delivery_record};

pub(super) fn replace_delivery_with_requested_machine_kv_summary(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    if current_delivery_contains_full_structured_contract(loop_state, delivery_messages) {
        return false;
    }
    let mut observed_texts = Vec::new();
    for step in &loop_state.executed_step_results {
        if !step.is_ok() {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            output,
            &mut observed_texts,
        );
    }
    for message in delivery_messages.iter() {
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            message,
            &mut observed_texts,
        );
    }
    for message in &loop_state.delivery_messages {
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            message,
            &mut observed_texts,
        );
    }
    for message in [
        loop_state.last_user_visible_respond.as_deref(),
        loop_state.last_publishable_synthesis_output.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            message,
            &mut observed_texts,
        );
    }
    observed_texts.sort();
    observed_texts.dedup();
    let request_surfaces = requested_machine_kv_request_surfaces(user_text, agent_run_context);
    let Some(answer) =
        crate::machine_kv_projection::requested_machine_kv_summary_from_observation_inputs(
            request_surfaces.iter().map(String::as_str),
            &observed_texts,
        )
    else {
        return false;
    };
    let current = final_answer_text_from_delivery(delivery_messages);
    if current.trim() == answer {
        loop_state.last_user_visible_respond = Some(answer);
        return true;
    }
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
    log_deterministic_delivery_record(
        &task.task_id,
        "requested_machine_kv_summary",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn current_delivery_contains_full_structured_contract(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    delivery_messages
        .iter()
        .chain(loop_state.delivery_messages.iter())
        .chain(loop_state.last_user_visible_respond.iter())
        .any(|message| structured_contract_json_should_remain_full(message))
}

fn structured_contract_json_should_remain_full(text: &str) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    object.contains_key("contract_marker")
        && (object.contains_key("async_timeout_policy")
            || object.contains_key("adapter_result")
            || object.contains_key("pending_async_job_contract")
            || object.contains_key("task_lifecycle")
            || object.contains_key("execution_policy"))
}

fn requested_machine_kv_request_surfaces(
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Vec<String> {
    let mut surfaces = vec![user_text.to_string()];
    let Some(ctx) = agent_run_context else {
        return surfaces;
    };
    for value in [
        ctx.original_user_request.as_deref(),
        ctx.user_request.as_deref(),
        ctx.route_result
            .as_ref()
            .map(|route| route.resolved_intent.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, value);
    }
    if let Some(state_patch) = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
    {
        crate::machine_kv_projection::collect_requested_machine_kv_surfaces_from_state_patch(
            state_patch,
            &mut surfaces,
        );
    }
    surfaces
}
