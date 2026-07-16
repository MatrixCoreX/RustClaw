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
            crate::OutputSemanticKind::ExcerptKindJudgment,
            crate::OutputSemanticKind::ContentExcerptSummary,
            crate::OutputSemanticKind::ContentExcerptWithSummary,
            crate::OutputSemanticKind::DirectoryPurposeSummary,
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
        .self_extension
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

pub(in crate::answer_verifier) fn observed_names_all_mentioned(
    candidate_answer: &str,
    names: &[String],
) -> bool {
    names
        .iter()
        .all(|name| observed_name_is_mentioned(candidate_answer, name))
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

pub(in crate::answer_verifier) fn observed_content_excerpt_path_names(
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation(step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        collect_content_excerpt_path_names_from_structured_output(output, &mut names);
        collect_content_excerpt_path_names_from_truncated_json(output, &mut names);
    }
    names
}

pub(in crate::answer_verifier) fn collect_content_excerpt_path_names_from_structured_output(
    output: &str,
    names: &mut BTreeSet<String>,
) {
    for value in structured_json_values_from_step_output(output) {
        if !value_is_read_content_observation(&value) {
            continue;
        }
        for path in json_path_string_values(&value) {
            collect_path_name_variants(&path, names);
        }
    }
}

pub(in crate::answer_verifier) fn value_is_read_content_observation(
    value: &serde_json::Value,
) -> bool {
    matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("read_range" | "read_text_range")
    ) && value
        .get("excerpt")
        .or_else(|| value.get("content"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|text| !text.is_empty())
}

pub(in crate::answer_verifier) fn json_path_string_values(
    value: &serde_json::Value,
) -> Vec<String> {
    [
        value.get("path").and_then(|value| value.as_str()),
        value.get("resolved_path").and_then(|value| value.as_str()),
        value
            .pointer("/extra/path")
            .and_then(|value| value.as_str()),
        value
            .pointer("/extra/resolved_path")
            .and_then(|value| value.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(ToString::to_string)
    .collect()
}

pub(in crate::answer_verifier) fn collect_content_excerpt_path_names_from_truncated_json(
    output: &str,
    names: &mut BTreeSet<String>,
) {
    if !output_has_read_content_machine_tokens(output) {
        return;
    }
    for key in ["path", "resolved_path"] {
        for value in json_string_field_values_from_text(output, key) {
            collect_path_name_variants(&value, names);
        }
    }
}

pub(in crate::answer_verifier) fn output_has_read_content_machine_tokens(output: &str) -> bool {
    (output.contains("\"read_range\"") || output.contains("\"read_text_range\""))
        && (output.contains("\"excerpt\"") || output.contains("\"content\""))
}

pub(in crate::answer_verifier) fn json_string_field_values_from_text(
    text: &str,
    field: &str,
) -> Vec<String> {
    let needle = format!("\"{field}\":\"");
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find(&needle) {
        let after = &rest[idx + needle.len()..];
        let mut value = String::new();
        let mut escaped = false;
        for ch in after.chars() {
            if escaped {
                value.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                break;
            }
            value.push(ch);
        }
        if !value.trim().is_empty() {
            values.push(value);
        }
        rest = after;
    }
    values
}

pub(in crate::answer_verifier) fn collect_path_name_variants(
    path: &str,
    names: &mut BTreeSet<String>,
) {
    for variant in path_variants(path) {
        if !variant.trim().is_empty() {
            names.insert(variant);
        }
    }
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
