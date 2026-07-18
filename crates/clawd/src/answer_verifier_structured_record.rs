use std::collections::{BTreeMap, BTreeSet};

use super::{
    observed_evidence_excerpt, successful_observed_evidence_items_for_route, AnswerContract,
    AnswerVerifierOut,
};

const SCHEDULE_PREVIEW_FIELDS: [&str; 3] = ["datetime", "timezone", "title"];

pub(super) fn schedule_preview_answer_is_grounded_in_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route
        .output_contract
        .semantic_kind_is(crate::OutputSemanticKind::SchedulePreview)
    {
        return false;
    }
    let Some(candidate) = parse_exact_machine_record(candidate_answer, &SCHEDULE_PREVIEW_FIELDS)
    else {
        return false;
    };
    let observed = observed_machine_record_values(route, journal, &SCHEDULE_PREVIEW_FIELDS);
    SCHEDULE_PREVIEW_FIELDS.iter().all(|field| {
        candidate.get(*field).is_some_and(|candidate_value| {
            observed
                .get(*field)
                .is_some_and(|values| values.contains(candidate_value))
        })
    })
}

pub(super) fn local_structured_record_answer_verifier_gap(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> Option<AnswerVerifierOut> {
    if !route
        .output_contract
        .semantic_kind_is(crate::OutputSemanticKind::SchedulePreview)
        || schedule_preview_answer_is_grounded_in_observation(route, journal, candidate_answer)
    {
        return None;
    }
    let candidate =
        parse_exact_machine_record(candidate_answer, &SCHEDULE_PREVIEW_FIELDS).unwrap_or_default();
    let observed = observed_machine_record_values(route, journal, &SCHEDULE_PREVIEW_FIELDS);
    let missing_fields = SCHEDULE_PREVIEW_FIELDS
        .iter()
        .filter(|field| {
            candidate.get(**field).is_none_or(|candidate_value| {
                observed
                    .get(**field)
                    .is_none_or(|values| !values.contains(candidate_value))
            })
        })
        .map(|field| (*field).to_string())
        .collect::<Vec<_>>();
    let field_list = if missing_fields.is_empty() {
        SCHEDULE_PREVIEW_FIELDS.join(",")
    } else {
        missing_fields.join(",")
    };
    Some(AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: if missing_fields.is_empty() {
            SCHEDULE_PREVIEW_FIELDS
                .iter()
                .map(|field| (*field).to_string())
                .collect()
        } else {
            missing_fields
        },
        answer_incomplete_reason: format!("ungrounded_structured_record_fields:{field_list}"),
        should_retry: true,
        retry_instruction: format!("project_observed_machine_fields_only:{field_list}"),
        confidence: 1.0,
    })
}

fn parse_exact_machine_record(
    text: &str,
    allowed_fields: &[&str],
) -> Option<BTreeMap<String, String>> {
    let mut record = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let pair = line.split_once('=').or_else(|| line.split_once(':'))?;
        let key = pair.0.trim();
        let value = pair.1.trim();
        if value.is_empty()
            || !allowed_fields.contains(&key)
            || record.insert(key.to_string(), value.to_string()).is_some()
        {
            return None;
        }
    }
    (record.len() == allowed_fields.len()).then_some(record)
}

fn observed_machine_record_values(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    fields: &[&str],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut observed = BTreeMap::<String, BTreeSet<String>>::new();
    for item in successful_observed_evidence_items_for_route(route, journal) {
        let Some(field) = item.get("field").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !fields.contains(&field) {
            continue;
        }
        let Some(value) = observed_evidence_excerpt(&item) else {
            continue;
        };
        if !value.trim().is_empty() {
            observed
                .entry(field.to_string())
                .or_default()
                .insert(value.trim().to_string());
        }
    }
    observed
}

#[cfg(test)]
#[path = "answer_verifier_structured_record_tests.rs"]
mod tests;
