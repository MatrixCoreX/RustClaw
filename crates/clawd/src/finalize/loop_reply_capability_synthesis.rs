use tracing::info;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

pub(super) async fn finalize_capability_synthesis(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<AskReply> {
    let answer = loop_state
        .last_capability_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|answer| !answer.is_empty())?
        .to_string();
    if loop_state.capability_results.is_empty()
        || loop_state.capability_results.iter().any(|result| {
            result.delivery.intent
                != claw_core::capability_result::CapabilityDeliveryIntent::ModelSynthesis
                || result.status != claw_core::capability_result::CapabilityResultStatus::Ok
        })
    {
        return None;
    }

    loop_state.delivery_messages.clear();
    loop_state.delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer.clone());
    let evidence_count = loop_state
        .capability_results
        .iter()
        .map(|result| result.evidence.len())
        .sum();
    let confidence = loop_state
        .output_vars
        .get("agent_loop.capability_synthesis_confidence")
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value.clamp(0.0, 1.0));
    let summary = crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        confidence,
        used_evidence_ids_count: evidence_count,
        ..Default::default()
    };
    let delivery_messages = vec![answer.clone()];
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&answer, &delivery_messages);
    let mut journal = crate::finalize::build_terminal_from_loop_state(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
        Some(summary),
        delivery_consistent,
        &answer,
        crate::task_journal::TaskJournalFinalStatus::Success,
    )
    .await;
    if let Some(output_contract) = agent_run_context.and_then(AgentRunContext::output_contract) {
        let answer_contract =
            crate::answer_verifier::AnswerContract::new(user_text, output_contract.clone());
        if let Some(verifier) = crate::answer_verifier::verify_answer_observe_only(
            state,
            task,
            user_text,
            &answer_contract,
            &journal,
            &answer,
        )
        .await
        {
            journal.record_answer_verifier_summary(verifier);
        }
    }
    info!(
        "final_result_source=capability_result_synthesis task_id={} result_count={} evidence_count={}",
        task.task_id,
        loop_state.capability_results.len(),
        evidence_count
    );
    Some(
        AskReply::llm(answer)
            .with_messages(delivery_messages)
            .with_task_journal(journal),
    )
}
