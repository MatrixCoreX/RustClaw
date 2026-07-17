use serde_json::{json, Value};

use super::AnswerVerifierOut;

const MIN_BARE_MACHINE_REF_VALUE_CHARS: usize = 4;

pub(super) fn local_compacted_machine_ref_answer_verifier_gap(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> Option<AnswerVerifierOut> {
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return None;
    }

    let continuity_refs = compacted_continuity_refs(journal);
    if continuity_refs.is_empty() {
        return None;
    }

    let mut exact_ref_count = 0;
    let mut missing_selected_refs = Vec::new();
    for machine_ref in continuity_refs {
        if contains_machine_token(candidate, &machine_ref) {
            exact_ref_count += 1;
            continue;
        }
        let Some((_, value)) = machine_ref.split_once(':') else {
            continue;
        };
        if value.chars().count() < MIN_BARE_MACHINE_REF_VALUE_CHARS
            || !contains_machine_token(candidate, value)
        {
            continue;
        }
        missing_selected_refs.push(machine_ref);
    }

    if missing_selected_refs.is_empty() || exact_ref_count + missing_selected_refs.len() < 2 {
        return None;
    }

    let retry_instruction = json!({
        "retry_policy": "preserve_selected_compacted_machine_refs_exactly",
        "required_machine_refs": missing_selected_refs,
    })
    .to_string();
    let missing_evidence_fields = missing_selected_refs
        .iter()
        .map(|machine_ref| format!("machine_ref:{machine_ref}"))
        .collect();
    Some(AnswerVerifierOut {
        pass: false,
        missing_evidence_fields,
        answer_incomplete_reason: "compacted_machine_reference_namespace_omitted".to_string(),
        should_retry: true,
        retry_instruction,
        confidence: 1.0,
    })
}

fn compacted_continuity_refs(journal: &crate::task_journal::TaskJournal) -> Vec<String> {
    let mut refs = Vec::new();
    for observation in &journal.task_observations {
        if observation.get("observation_kind").and_then(Value::as_str)
            != Some("context_compaction_record")
        {
            continue;
        }
        let Some(continuity_refs) = observation
            .pointer("/record/continuity_refs")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for item in continuity_refs {
            let machine_ref = item
                .get("ref")
                .and_then(Value::as_str)
                .or_else(|| item.as_str())
                .map(str::trim)
                .filter(|value| value.split_once(':').is_some());
            let Some(machine_ref) = machine_ref else {
                continue;
            };
            if refs.iter().any(|existing| existing == machine_ref) {
                continue;
            }
            refs.push(machine_ref.to_string());
        }
    }
    refs
}

fn contains_machine_token(text: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    text.match_indices(token).any(|(start, _)| {
        let end = start + token.len();
        token_boundary_before(text, start) && token_boundary_after(text, end)
    })
}

fn token_boundary_before(text: &str, start: usize) -> bool {
    start == 0
        || text[..start]
            .chars()
            .next_back()
            .is_none_or(|value| !is_machine_token_char(value))
}

fn token_boundary_after(text: &str, end: usize) -> bool {
    end == text.len()
        || text[end..]
            .chars()
            .next()
            .is_none_or(|value| !is_machine_token_char(value))
}

fn is_machine_token_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '.' | '/' | ':' | '-')
}
