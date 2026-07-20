use serde_json::Value;

use crate::delivery_utils::trim_path_token;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadRangeEvidence {
    content: String,
    delivery_token: Option<String>,
}

pub(super) fn has_any_delivery_file_token(text: &str, messages: &[String]) -> bool {
    !crate::extract_delivery_file_tokens(text).is_empty()
        || messages
            .iter()
            .any(|message| !crate::extract_delivery_file_tokens(message).is_empty())
}

pub(super) fn route_has_file_delivery_contract(route_result: &crate::IntentOutputContract) -> bool {
    route_result.delivery_required
        || route_result.delivery_required
        || matches!(
            route_result.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

fn normalized_delivery_token_from_path(value: &Value) -> Option<String> {
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("output_path"))
        .or_else(|| value.get("path"))
        .and_then(Value::as_str)
        .map(trim_path_token)
        .filter(|path| !path.is_empty())?;
    Some(format!("FILE:{path}"))
}

fn file_artifact_delivery_token_from_value(value: &Value) -> Option<String> {
    if matches!(
        value.get("action").and_then(Value::as_str),
        Some("read_range" | "read_text_range" | "write_text" | "append_text")
    ) {
        return normalized_delivery_token_from_path(value);
    }
    value
        .get("extra")
        .and_then(file_artifact_delivery_token_from_value)
}

fn read_range_evidence_from_value(value: &Value) -> Option<ReadRangeEvidence> {
    if matches!(
        value.get("action").and_then(Value::as_str),
        Some("read_range" | "read_text_range")
    ) {
        let content = value
            .get("excerpt")
            .and_then(Value::as_str)
            .and_then(crate::agent_engine::observed_output::normalize_read_range_excerpt)
            .map(|content| content.trim().to_string())
            .filter(|content| !content.is_empty())?;
        return Some(ReadRangeEvidence {
            content,
            delivery_token: normalized_delivery_token_from_path(value),
        });
    }
    value.get("extra").and_then(read_range_evidence_from_value)
}

fn latest_read_range_evidence_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Option<ReadRangeEvidence> {
    journal.step_results.iter().rev().find_map(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            return None;
        }
        let value = serde_json::from_str::<Value>(step.output_excerpt.as_deref()?.trim()).ok()?;
        read_range_evidence_from_value(&value)
    })
}

fn answer_contains_content(answer_text: &str, answer_messages: &[String], content: &str) -> bool {
    let content = content.trim();
    !content.is_empty()
        && (answer_text.contains(content)
            || answer_messages
                .iter()
                .any(|message| message.contains(content)))
}

fn answer_contains_token(answer_text: &str, answer_messages: &[String], token: &str) -> bool {
    let token = token.trim();
    !token.is_empty()
        && (crate::extract_delivery_file_tokens(answer_text)
            .iter()
            .any(|candidate| candidate.trim() == token)
            || answer_messages.iter().any(|message| {
                crate::extract_delivery_file_tokens(message)
                    .iter()
                    .any(|candidate| candidate.trim() == token)
            }))
}

fn rebuild_answer_text_from_messages(messages: &[String]) -> String {
    messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn latest_file_delivery_token_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    journal.step_results.iter().rev().find_map(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            return None;
        }
        let output = step.output_excerpt.as_deref()?;
        crate::extract_delivery_file_tokens(output)
            .into_iter()
            .next()
            .or_else(|| {
                serde_json::from_str::<Value>(output.trim())
                    .ok()
                    .and_then(|value| file_artifact_delivery_token_from_value(&value))
            })
    })
}

pub(super) fn backfill_file_delivery_token_from_journal(
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    if !route_has_file_delivery_contract(route_result)
        || has_any_delivery_file_token(answer_text, answer_messages)
    {
        return false;
    }
    let Some(token) = latest_file_delivery_token_from_journal(journal) else {
        return false;
    };
    if answer_messages.is_empty() && !answer_text.trim().is_empty() {
        answer_messages.push(answer_text.trim().to_string());
    }
    answer_messages.retain(|message| message.trim() != token);
    answer_messages.insert(0, token);
    *answer_text = rebuild_answer_text_from_messages(answer_messages);
    true
}

pub(super) fn backfill_content_evidence_file_delivery_from_journal(
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    let contract = route_result.clone();
    if !contract.requires_content_evidence || !route_has_file_delivery_contract(route_result) {
        return false;
    }
    let Some(evidence) = latest_read_range_evidence_from_journal(journal) else {
        return false;
    };
    let delivery_token = evidence
        .delivery_token
        .clone()
        .or_else(|| latest_file_delivery_token_from_journal(journal));
    let content_present = answer_contains_content(answer_text, answer_messages, &evidence.content);
    let token_present = delivery_token
        .as_deref()
        .is_some_and(|token| answer_contains_token(answer_text, answer_messages, token))
        || (delivery_token.is_none() && has_any_delivery_file_token(answer_text, answer_messages));
    if content_present && token_present {
        return false;
    }
    if answer_messages.is_empty() && !answer_text.trim().is_empty() {
        answer_messages.push(answer_text.trim().to_string());
    }
    if let Some(token) = delivery_token.as_deref().filter(|_| !token_present) {
        answer_messages.retain(|message| message.trim() != token);
        answer_messages.insert(0, token.to_string());
    }
    if !content_present {
        answer_messages.retain(|message| message.trim() != evidence.content.as_str());
        let insert_at = delivery_token
            .as_deref()
            .filter(|token| {
                answer_messages
                    .first()
                    .is_some_and(|message| message.trim() == *token)
            })
            .map(|_| 1)
            .unwrap_or(0);
        answer_messages.insert(insert_at, evidence.content);
    }
    *answer_text = rebuild_answer_text_from_messages(answer_messages);
    true
}

pub(super) fn backfill_file_delivery_contract_from_journal(
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    let content_changed = backfill_content_evidence_file_delivery_from_journal(
        route_result,
        journal,
        answer_text,
        answer_messages,
    );
    let token_changed = backfill_file_delivery_token_from_journal(
        route_result,
        journal,
        answer_text,
        answer_messages,
    );
    content_changed || token_changed
}
