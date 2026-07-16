use serde_json::Value;

use crate::AppState;

use super::task_content_evidence_delivery::{
    has_any_delivery_file_token, route_has_file_delivery_contract,
};
use super::task_resume::text_looks_like_missing_file_target;

pub(super) fn should_reinsert_execution_summaries_for_delivery(
    route_result: &crate::IntentOutputContract,
    answer_text: &str,
) -> bool {
    let output_contract = route_result.clone();
    if (output_contract.response_shape == crate::OutputResponseShape::Scalar
        || route_requests_config_validation(route_result))
        && !answer_text.trim().is_empty()
        && !crate::finalize::is_execution_summary_message(answer_text)
    {
        return false;
    }
    if strict_structured_final_answer_suppresses_execution_summary(route_result, answer_text) {
        return false;
    }
    true
}

fn route_requests_config_validation(route_result: &crate::IntentOutputContract) -> bool {
    crate::finalize::route_matches_validation_verdict_output_contract(route_result)
}

pub(super) fn drop_execution_summaries_when_delivery_is_scalar(
    route_result: &crate::IntentOutputContract,
    answer_text: &str,
    answer_messages: &mut Vec<String>,
) {
    if should_reinsert_execution_summaries_for_delivery(route_result, answer_text) {
        return;
    }
    answer_messages.retain(|message| !crate::finalize::is_execution_summary_message(message));
}

fn strict_structured_final_answer_suppresses_execution_summary(
    route_result: &crate::IntentOutputContract,
    answer_text: &str,
) -> bool {
    route_result.response_shape == crate::OutputResponseShape::Strict
        && !route_has_file_delivery_contract(route_result)
        && structured_machine_final_answer(answer_text)
}

fn structured_machine_final_answer(answer_text: &str) -> bool {
    let trimmed = answer_text.trim();
    if trimmed.is_empty() || crate::finalize::is_execution_summary_message(trimmed) {
        return false;
    }
    if serde_json::from_str::<Value>(trimmed)
        .ok()
        .is_some_and(|value| value.is_object() || value.is_array())
    {
        return true;
    }
    let lines = trimmed
        .split_whitespace()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    !lines.is_empty()
        && lines.iter().all(|line| {
            let Some((key, value)) = line.split_once('=') else {
                return false;
            };
            !key.trim().is_empty()
                && !value.trim().is_empty()
                && key
                    .trim()
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        })
}

pub(super) fn journal_has_missing_file_search_evidence(
    journal: Option<&crate::task_journal::TaskJournal>,
) -> bool {
    journal
        .into_iter()
        .flat_map(|journal| journal.step_results.iter().rev())
        .any(|step| {
            step.output_excerpt
                .as_deref()
                .is_some_and(crate::finalize::loop_reply::output_excerpt_has_missing_file_evidence)
                || step
                    .error_excerpt
                    .as_deref()
                    .is_some_and(text_looks_like_missing_file_target)
        })
}

fn journal_has_non_control_step(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.step_results.iter().any(|step| {
        !matches!(
            step.skill.as_str(),
            "respond" | "synthesize_answer" | "think" | "answer_verifier"
        )
    })
}

pub(super) fn delivery_path_gap_should_finalize_as_clarify(
    route_result: &crate::IntentOutputContract,
    answer_text: &str,
    answer_messages: &[String],
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    route_has_file_delivery_contract(route_result)
        && !has_any_delivery_file_token(answer_text, answer_messages)
        && !journal_has_non_control_step(journal)
        && journal
            .answer_verifier_summary
            .as_ref()
            .is_some_and(|summary| {
                !summary.pass
                    && summary
                        .missing_evidence_fields
                        .iter()
                        .any(|field| field.trim() == "path")
            })
}

pub(super) fn should_use_missing_file_delivery_reply(
    route_result: &crate::IntentOutputContract,
    answer: &crate::AskReply,
) -> bool {
    route_has_file_delivery_contract(route_result)
        && !answer.should_fail_task
        && !has_any_delivery_file_token(&answer.text, &answer.messages)
        && journal_has_missing_file_search_evidence(answer.task_journal.as_ref())
}

pub(super) async fn missing_file_delivery_reply_text(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    route_result: &crate::IntentOutputContract,
    answer: &crate::AskReply,
) -> Option<String> {
    if !should_use_missing_file_delivery_reply(route_result, answer) {
        return None;
    }
    let language_hint = crate::language_policy::first_clear_request_language_hint([prompt, ""])
        .unwrap_or_else(|| {
            crate::language_policy::task_response_language_hint(state, task, prompt)
        });
    Some(
        crate::fallback::compose_missing_file_delivery_response(
            state,
            task,
            prompt,
            "",
            Some(route_result.locator_hint.as_str()),
            &language_hint,
        )
        .await,
    )
}
