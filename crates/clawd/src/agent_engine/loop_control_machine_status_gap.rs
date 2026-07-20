use serde_json::Value;

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
