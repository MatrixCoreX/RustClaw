use super::*;

pub(in crate::agent_engine::loop_control) fn answer_verifier_gap_requests_observed_content_rewrite(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> bool {
    verifier.should_retry
        && verifier
            .missing_evidence_fields
            .iter()
            .any(|field| field == "unsupported_claims")
        && !verifier.missing_evidence_fields.iter().any(|field| {
            matches!(
                field.as_str(),
                "content_excerpt"
                    | "field_value"
                    | "any_of(command_output|content_excerpt|field_value)"
                    | "any_of(command_output|content_excerpt|count|field_value)"
            )
        })
}

pub(in crate::agent_engine::loop_control) fn answer_verifier_gap_has_observed_content_evidence(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal
        .step_results
        .iter()
        .any(step_has_observed_content_evidence)
}

fn step_has_observed_content_evidence(step: &crate::task_journal::TaskJournalStepTrace) -> bool {
    if step.status != crate::executor::StepExecutionStatus::Ok
        || matches!(
            step.skill.as_str(),
            "respond" | "synthesize_answer" | "think" | "answer_verifier"
        )
    {
        return false;
    }
    let Some(evidence) = crate::task_journal::observed_evidence_for_step_trace(step) else {
        return false;
    };
    observed_evidence_has_content_field(&evidence)
}

fn observed_evidence_has_content_field(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            let direct_content_field = object
                .get("field")
                .and_then(Value::as_str)
                .is_some_and(is_observed_content_field);
            direct_content_field || object.values().any(observed_evidence_has_content_field)
        }
        Value::Array(items) => items.iter().any(observed_evidence_has_content_field),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn is_observed_content_field(field: &str) -> bool {
    matches!(
        field,
        "content_excerpt" | "excerpt" | "extra.content_excerpt" | "extra.excerpt" | "field_value"
    )
}

pub(in crate::agent_engine::loop_control) async fn try_rewrite_answer_verifier_gap_with_observed_evidence(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    route_result: Option<&RouteResult>,
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract.delivery_required
        || route.wants_file_delivery
        || !verifier.high_confidence_retry_gap()
        || !verifier.should_retry
    {
        return false;
    }
    let Some(journal_snapshot) = reply.task_journal.clone() else {
        return false;
    };
    let coverage_complete =
        crate::task_journal::evidence_coverage_for_route(route, &journal_snapshot).is_complete();
    let observed_content_rewrite = answer_verifier_gap_requests_observed_content_rewrite(verifier)
        && answer_verifier_gap_has_observed_content_evidence(&journal_snapshot);
    if !coverage_complete && !observed_content_rewrite {
        return false;
    }
    let verifier_out = answer_verifier_summary_to_out(verifier);
    let rejected_answer = reply.text.clone();
    let Some(retried_answer) = crate::finalize::retry_loop_answer_after_verifier(
        state,
        task,
        user_text,
        &journal_snapshot,
        &rejected_answer,
        &verifier_out,
    )
    .await
    else {
        return false;
    };
    if let Some(retry_verifier) = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        user_text,
        route,
        &journal_snapshot,
        &retried_answer,
    )
    .await
    {
        if retry_verifier_accepts_rewritten_answer(&retry_verifier, &retried_answer) {
            if commit_answer_verifier_retry_answer(reply, retried_answer) {
                tracing::info!("answer_verifier_retry_rewritten_with_observed_evidence");
                return true;
            }
            return false;
        }
        if let Some(journal) = reply.task_journal.as_mut() {
            journal.record_answer_verifier_summary(retry_verifier);
        }
        return false;
    }
    if retry_rewritten_answer_is_publishable(&retried_answer) {
        if commit_answer_verifier_retry_answer(reply, retried_answer) {
            tracing::info!("answer_verifier_retry_rewritten_with_observed_evidence");
            true
        } else {
            false
        }
    } else {
        tracing::info!(
            "answer_verifier_retry_observed_rewrite_unpublishable_unresolved_machine_fields"
        );
        false
    }
}
