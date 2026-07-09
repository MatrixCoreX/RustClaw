use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    final_answer_text_from_delivery, log_deterministic_delivery_record,
    successful_delivery_final_status,
};

pub(super) fn visible_answer_is_machine_payload(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text.trim())
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|object| {
            object.contains_key("message_key")
                || object.contains_key("reason_code")
                || object.contains_key("candidates")
                || object.contains_key("risks")
                || object.contains_key("contract_marker")
                || object
                    .get("output_format")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|format| format == "machine_json")
                || (object.contains_key("status") && object.contains_key("steps"))
        })
}

pub(super) fn visible_machine_payload_should_remain_structured(text: &str) -> bool {
    let Ok(serde_json::Value::Object(object)) =
        serde_json::from_str::<serde_json::Value>(text.trim())
    else {
        return false;
    };
    if object
        .get("output_format")
        .and_then(serde_json::Value::as_str)
        == Some("machine_json")
        && object
            .get("owner_layer")
            .and_then(serde_json::Value::as_str)
            == Some("subagent_runtime")
    {
        return true;
    }
    let Some(message_key) = object
        .get("message_key")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    matches!(
        message_key,
        "clawd.msg.config_edit.guard" | "clawd.msg.config_risk.summary"
    ) && object.contains_key("path")
        && (object.contains_key("risk_count")
            || object.contains_key("count")
            || object.contains_key("candidates")
            || object.contains_key("risks"))
}

fn route_allows_machine_payload_visible_render(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    !route.output_contract.delivery_required
        && !route.wants_file_delivery
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
}

pub(super) async fn render_machine_payload_delivery_if_needed(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) {
    if !route_allows_machine_payload_visible_render(agent_run_context) {
        return;
    }
    let current = final_answer_text_from_delivery(delivery_messages);
    if !visible_answer_is_machine_payload(&current) {
        return;
    }
    if visible_machine_payload_should_remain_structured(&current) {
        log_deterministic_delivery_record(
            &task.task_id,
            "render_machine_payload_delivery",
            "preserved_structured_payload",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&current, delivery_messages);
    let provisional_journal = crate::finalize::build_from_loop_state(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_consistent,
        &current,
        successful_delivery_final_status(loop_state, None),
    );
    let verifier = crate::answer_verifier::AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["output_format".to_string()],
        answer_incomplete_reason: "machine_payload_visible".to_string(),
        should_retry: true,
        retry_instruction: "output_format".to_string(),
        confidence: 0.9,
    }
    .normalized();
    let Some(rendered) = crate::finalize::retry_loop_answer_after_verifier(
        state,
        task,
        user_text,
        &provisional_journal,
        &current,
        &verifier,
    )
    .await
    else {
        return;
    };
    if rendered.trim().is_empty() || visible_answer_is_machine_payload(&rendered) {
        return;
    }
    delivery_messages.clear();
    delivery_messages.push(rendered.clone());
    loop_state.delivery_messages = delivery_messages.clone();
    loop_state.last_user_visible_respond = Some(rendered);
    log_deterministic_delivery_record(
        &task.task_id,
        "render_machine_payload_delivery",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
}
