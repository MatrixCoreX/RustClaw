use super::*;

pub(super) fn promote_publishable_strict_json_projection_for_verifier_candidate(
    reply: &mut AskReply,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    if loop_state
        .output_vars
        .get("agent_loop.strict_json_projection_publishable")
        .map(String::as_str)
        != Some("true")
    {
        return false;
    }
    let Some(answer) = loop_state
        .output_vars
        .get("agent_loop.strict_json_projection_output")
        .map(String::as_str)
        .map(str::trim)
        .filter(|answer| !answer.is_empty())
    else {
        return false;
    };
    if serde_json::from_str::<Value>(answer)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_none_or(|object| object.is_empty())
    {
        return false;
    }
    let should_promote = {
        let Some(journal) = reply.task_journal.as_ref() else {
            return false;
        };
        !crate::answer_verifier::should_verify_answer(route_result, journal, answer)
            || crate::answer_verifier::structurally_satisfies_answer_contract(
                route_result,
                journal,
                answer,
            )
    };
    if !should_promote {
        return false;
    }

    let answer = answer.to_string();
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(answer.clone());
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    tracing::info!("answer_verifier_candidate_promoted_strict_json_projection");
    true
}

pub(super) fn promote_local_code_projection_from_machine_evidence_for_verifier_candidate(
    reply: &mut AskReply,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(answer) = crate::agent_engine::local_code_strict_json_projection_from_machine_evidence(
        user_text,
        loop_state,
        agent_run_context,
    ) else {
        return false;
    };
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(answer.trim()) else {
        return false;
    };
    if object.is_empty() {
        return false;
    }

    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(answer.clone());
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: journal.step_results.len(),
            ..Default::default()
        });
        journal.push_task_observation(json!({
            "kind": "agent_loop_strict_json_projection",
            "owner_layer": "agent_loop",
            "schema_version": 1,
            "publishable": true,
            "output": answer,
        }));
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    tracing::info!("answer_verifier_candidate_promoted_local_code_machine_projection");
    true
}

pub(super) fn answer_verifier_retry_budget_available(
    policy: &AgentLoopGuardPolicy,
    answer_verifier_retry_count: usize,
) -> bool {
    answer_verifier_retry_count < policy.answer_verifier_retry_limit
}

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
    true
}

pub(super) async fn attach_answer_verifier_if_missing(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    route_result: Option<&RouteResult>,
    reply: &mut AskReply,
) {
    if reply.should_fail_task || reply_final_status_is_clarify(reply) {
        return;
    }
    let Some(route_result) = route_result else {
        return;
    };
    let Some(journal) = reply.task_journal.as_mut() else {
        return;
    };
    if journal.answer_verifier_summary.is_some() {
        return;
    }
    if let Some(answer_verifier) =
        machine_status_visible_output_format_gap(route_result, journal, &reply.text)
    {
        journal.record_answer_verifier_summary(answer_verifier);
        return;
    }
    if let Some(answer_verifier) = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        user_text,
        route_result,
        journal,
        &reply.text,
    )
    .await
    {
        journal.record_answer_verifier_summary(answer_verifier);
    }
}

pub(super) fn answer_contract_route_result_for_reply(
    agent_run_context: Option<&AgentRunContext>,
    reply: &AskReply,
) -> Option<RouteResult> {
    reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.route_result.clone())
        .or_else(|| agent_run_context.and_then(|ctx| ctx.route_result.clone()))
}
