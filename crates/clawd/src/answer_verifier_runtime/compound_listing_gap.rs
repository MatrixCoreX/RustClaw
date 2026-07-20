use super::*;

pub(crate) fn local_compound_listing_answer_verifier_gap(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> Option<AnswerVerifierOut> {
    let contract = route_result.effective_output_contract();
    if !contract.requires_content_evidence
        || contract.delivery_required
        || !route_result.output_contract_marker_is_any(&[
            crate::OutputSemanticKind::ContentExcerptSummary,
            crate::OutputSemanticKind::ContentExcerptWithSummary,
        ])
    {
        return None;
    }
    let Some(_limit) = requested_listing_name_limit(route_result) else {
        return None;
    };
    let names = observed_inventory_names_for_contract(route_result, journal)?;
    if names.len() < 2 || !journal_has_content_excerpt_observation(journal) {
        return None;
    }
    let missing = names
        .iter()
        .filter(|name| !observed_name_is_mentioned(candidate_answer, name))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return None;
    }
    Some(AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: "observed_listing_candidates_omitted".to_string(),
        should_retry: true,
        retry_instruction: "retry_policy=use_observed_listing_candidates_and_content_excerpt;repeat_rejected_answer=false".to_string(),
        confidence: 0.92,
    })
}

pub(in crate::answer_verifier) fn latest_observed_inventory_names(
    journal: &crate::task_journal::TaskJournal,
) -> Option<Vec<String>> {
    latest_observed_directory_structure_names(journal).and_then(|names| {
        (!names.is_empty()).then_some(
            names
                .into_iter()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>(),
        )
    })
}

pub(in crate::answer_verifier) fn latest_observed_directory_structure_names(
    journal: &crate::task_journal::TaskJournal,
) -> Option<Vec<String>> {
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .flat_map(structured_json_values_from_step_output)
        .find_map(|value| {
            if !value_is_directory_structure_observation(&value) {
                return None;
            }
            let mut names = BTreeSet::new();
            collect_observed_strict_list_items_from_value(&value, &mut names);
            let names = names.into_iter().collect::<Vec<_>>();
            (!names.is_empty()).then_some(names)
        })
}

pub(in crate::answer_verifier) fn value_is_directory_structure_observation(
    value: &serde_json::Value,
) -> bool {
    matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("inventory_dir" | "list_dir" | "tree_summary")
    ) || value
        .get("names")
        .and_then(|value| value.as_array())
        .is_some()
        || value
            .get("names_by_kind")
            .and_then(|value| value.as_object())
            .is_some()
        || value
            .get("entries")
            .and_then(|value| value.as_array())
            .is_some()
        || value
            .get("candidates")
            .and_then(|value| value.as_array())
            .is_some()
}

pub(in crate::answer_verifier) fn observed_inventory_names_for_contract(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> Option<Vec<String>> {
    let mut names = latest_observed_inventory_names(journal)?;
    if let Some(limit) = requested_listing_name_limit(route_result) {
        names.truncate(limit.min(names.len()));
    }
    Some(names)
}

pub(in crate::answer_verifier) fn requested_listing_name_limit(
    route_result: &AnswerContract,
) -> Option<usize> {
    route_result
        .output_contract
        .selection
        .list_selector
        .limit
        .and_then(|limit| usize::try_from(limit).ok())
        .filter(|limit| *limit > 0)
        .or_else(|| {
            contract_hint_selector_limit(&route_result.request_text)
                .and_then(|limit| usize::try_from(limit).ok())
                .filter(|limit| *limit > 0)
        })
}

pub(in crate::answer_verifier) fn contract_hint_selector_limit(text: &str) -> Option<u64> {
    text.split(|ch: char| ch == '\n' || ch == ';' || ch == ',' || ch.is_whitespace())
        .filter_map(|part| part.split_once('='))
        .find_map(|(key, value)| {
            (key.trim() == "selector_limit")
                .then(|| value.trim().parse::<u64>().ok())
                .flatten()
        })
}

pub(in crate::answer_verifier) fn observed_name_is_mentioned(
    candidate_answer: &str,
    name: &str,
) -> bool {
    let answer = candidate_answer.replace('\\', "/").to_ascii_lowercase();
    let normalized = name.replace('\\', "/").to_ascii_lowercase();
    !normalized.is_empty() && answer.contains(&normalized)
}

pub(in crate::answer_verifier) fn journal_has_content_excerpt_observation(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .flat_map(structured_json_values_from_step_output)
        .any(|value| {
            matches!(
                value.get("action").and_then(|value| value.as_str()),
                Some("read_range" | "read_text_range")
            ) && value
                .get("excerpt")
                .or_else(|| value.get("content"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|text| !text.is_empty())
        })
}

pub(in crate::answer_verifier) fn structured_json_values_from_step_output(
    output: &str,
) -> Vec<serde_json::Value> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return Vec::new();
    };
    let mut values = vec![value.clone()];
    if let Some(extra) = value.get("extra") {
        values.push(extra.clone());
    }
    values
}
