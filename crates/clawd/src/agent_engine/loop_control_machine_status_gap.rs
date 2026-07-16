use serde_json::Value;

use crate::answer_verifier::AnswerContract;

pub(super) fn answer_verifier_output_format_machine_payload_gap(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
    reply_text: &str,
) -> bool {
    if !verifier.high_confidence_retry_gap()
        || !verifier
            .missing_evidence_fields
            .iter()
            .any(|field| field == "output_format")
        || verifier
            .missing_evidence_fields
            .iter()
            .any(|field| field != "output_format")
    {
        return false;
    }
    if verifier.answer_incomplete_reason == "machine_status_token_visible" {
        return true;
    }
    if visible_answer_is_machine_field_projection(reply_text) {
        return true;
    }
    serde_json::from_str::<Value>(reply_text.trim())
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
                    .and_then(Value::as_str)
                    .is_some_and(|format| format == "machine_json")
                || (object.contains_key("status") && object.contains_key("steps"))
        })
}

fn visible_answer_is_machine_field_projection(reply_text: &str) -> bool {
    let mut field_count = 0usize;
    for token in reply_text.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if machine_projection_field_key(key) && !value.is_empty() {
            field_count += 1;
            if field_count >= 2 {
                return true;
            }
        }
    }
    false
}

fn machine_projection_field_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '.' | '-')
        })
        && key.chars().any(|ch| ch.is_ascii_lowercase())
}

fn visible_answer_is_observed_machine_status_token(
    answer_contract: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    reply_text: &str,
) -> bool {
    if matches!(
        answer_contract.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        return false;
    }
    if !answer_contract
        .output_contract
        .semantic_kind_is(crate::OutputSemanticKind::ServiceStatus)
        || !crate::evidence_policy::final_answer_shape_for_output_contract(
            &answer_contract.output_contract,
        )
        .is_some_and(|shape| shape.allows_model_language())
    {
        return false;
    }
    let token = reply_text.trim();
    if !single_machine_token_answer(token) {
        return false;
    }
    journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .filter_map(|output| serde_json::from_str::<Value>(output.trim()).ok())
        .any(|value| {
            observed_machine_status_tokens(&value)
                .iter()
                .any(|candidate| candidate == token)
        })
}

fn single_machine_token_answer(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 80
        && !token.starts_with('{')
        && !token.starts_with('[')
        && !token.chars().any(char::is_whitespace)
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '/'))
}

fn observed_machine_status_tokens(value: &Value) -> Vec<String> {
    let mut tokens = Vec::new();
    collect_observed_machine_status_tokens(value, &mut tokens);
    tokens
}

fn collect_observed_machine_status_tokens(value: &Value, tokens: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if matches!(key.as_str(), "text" | "error_text") {
                    continue;
                }
                if machine_status_field_key(key) {
                    if let Some(token) = child
                        .as_str()
                        .map(str::trim)
                        .filter(|token| !token.is_empty() && single_machine_token_answer(token))
                    {
                        tokens.push(token.to_string());
                    }
                }
                collect_observed_machine_status_tokens(child, tokens);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_observed_machine_status_tokens(item, tokens);
            }
        }
        _ => {}
    }
}

fn machine_status_field_key(key: &str) -> bool {
    matches!(
        key,
        "status" | "status_code" | "state" | "reason_code" | "message_key"
    )
}

pub(super) fn machine_status_visible_output_format_gap(
    answer_contract: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    reply_text: &str,
) -> Option<crate::answer_verifier::AnswerVerifierOut> {
    visible_answer_is_observed_machine_status_token(answer_contract, journal, reply_text).then(
        || {
            crate::answer_verifier::AnswerVerifierOut {
                pass: false,
                missing_evidence_fields: vec!["output_format".to_string()],
                answer_incomplete_reason: "machine_status_token_visible".to_string(),
                should_retry: true,
                retry_instruction: "render_observed_machine_status_as_user_visible_answer"
                    .to_string(),
                confidence: 0.9,
            }
            .normalized()
        },
    )
}
