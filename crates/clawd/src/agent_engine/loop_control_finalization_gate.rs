use super::*;

pub(super) fn retry_verifier_accepts_rewritten_answer(
    verifier: &crate::answer_verifier::AnswerVerifierOut,
    retried_answer: &str,
) -> bool {
    verifier.pass
        && !verifier.high_confidence_gap()
        && retry_rewritten_answer_is_publishable(retried_answer)
}

pub(super) fn retry_rewritten_answer_is_publishable(retried_answer: &str) -> bool {
    if local_code_json_answer_has_unresolved_publication(retried_answer) {
        return false;
    }
    if serde_json::from_str::<Value>(retried_answer.trim())
        .ok()
        .is_some_and(|value| json_value_contains_unresolved_machine_token(&value))
    {
        return false;
    }
    true
}

pub(super) async fn attach_answer_verifier_if_missing(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    answer_contract: Option<&crate::answer_verifier::AnswerContract>,
    reply: &mut AskReply,
) {
    if reply.should_fail_task || reply_final_status_is_clarify(reply) {
        return;
    }
    let Some(answer_contract) = answer_contract else {
        return;
    };
    let Some(journal) = reply.task_journal.as_mut() else {
        return;
    };
    if journal.answer_verifier_summary.is_some() || journal.has_answer_verification_for(&reply.text)
    {
        return;
    }
    let outcome = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        user_text,
        answer_contract,
        journal,
        &reply.text,
    )
    .await;
    journal.record_answer_verification(outcome, &reply.text);
}

pub(super) fn answer_contract_for_reply(
    user_text: &str,
    reply: &AskReply,
) -> Option<crate::answer_verifier::AnswerContract> {
    // Planner-owned turns still need in-loop verification without a format contract.
    reply.task_journal.as_ref().map(|journal| {
        crate::answer_verifier::AnswerContract::new(
            user_text,
            journal.output_contract.clone().unwrap_or_default(),
        )
    })
}

pub(super) fn checkpoint_handoff_reply_with_verification(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    prior_reply: &AskReply,
) -> AskReply {
    let mut handoff =
        super::checkpoint_handoff_reply(task, user_text, loop_state, agent_run_context);
    if let (Some(source), Some(target)) = (
        prior_reply.task_journal.as_ref(),
        handoff.task_journal.as_mut(),
    ) {
        target.answer_verification = source.answer_verification.clone();
        target.answer_verifier_summary = source.answer_verifier_summary.clone();
    }
    handoff
}

#[cfg(test)]
#[path = "loop_control_finalization_gate_tests.rs"]
mod tests;
