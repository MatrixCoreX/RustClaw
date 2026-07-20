use super::*;

pub(super) fn route_requires_single_file_delivery(route: &AnswerContract) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) || matches!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    ) || (route.output_contract.delivery_required
        && !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryBatchFiles
        ))
}

pub(super) fn candidate_answer_has_grounded_existing_file_token(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let Some((_kind, raw_path)) =
        crate::finalize::parse_delivery_file_token(candidate_answer.trim())
    else {
        return false;
    };
    let token_path = std::path::Path::new(raw_path.trim());
    let Ok(canonical_token_path) = token_path.canonicalize() else {
        return false;
    };
    file_token_path_is_grounded_in_observations(journal, &canonical_token_path)
}

pub(super) fn file_token_path_is_grounded_in_observations(
    journal: &crate::task_journal::TaskJournal,
    canonical_token_path: &std::path::Path,
) -> bool {
    let current_dir = std::env::current_dir().ok();
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation(step)
            && step.output_excerpt.as_deref().is_some_and(|output| {
                observed_output_contains_path(output, canonical_token_path, current_dir.as_deref())
            })
    })
}

pub(super) fn observed_output_contains_path(
    output: &str,
    canonical_token_path: &std::path::Path,
    current_dir: Option<&std::path::Path>,
) -> bool {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        return json_value_contains_path(&value, canonical_token_path, current_dir);
    }
    candidate_path_matches(output.trim(), canonical_token_path, current_dir)
        || output.split_whitespace().any(|token| {
            candidate_path_matches(
                token.trim_matches(|ch: char| {
                    matches!(ch, '"' | '\'' | '`' | ',' | ';' | ':' | ')' | ']' | '}')
                }),
                canonical_token_path,
                current_dir,
            )
        })
}

pub(super) fn json_value_contains_path(
    value: &serde_json::Value,
    canonical_token_path: &std::path::Path,
    current_dir: Option<&std::path::Path>,
) -> bool {
    match value {
        serde_json::Value::String(candidate) => {
            candidate_path_matches(candidate, canonical_token_path, current_dir)
        }
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_value_contains_path(item, canonical_token_path, current_dir)),
        serde_json::Value::Object(map) => {
            if resolved_dir_names_contain_path(map, canonical_token_path) {
                return true;
            }
            map.values()
                .any(|item| json_value_contains_path(item, canonical_token_path, current_dir))
        }
        _ => false,
    }
}

pub(super) fn resolved_dir_names_contain_path(
    map: &serde_json::Map<String, serde_json::Value>,
    canonical_token_path: &std::path::Path,
) -> bool {
    let Some(resolved_dir) = map
        .get("resolved_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::path::Path::new)
    else {
        return false;
    };
    let Some(names) = map.get("names").and_then(|value| value.as_array()) else {
        return false;
    };
    names.iter().filter_map(|value| value.as_str()).any(|name| {
        let candidate = resolved_dir.join(name.trim());
        candidate
            .canonicalize()
            .is_ok_and(|path| path == canonical_token_path)
    })
}

pub(super) fn candidate_path_matches(
    candidate: &str,
    canonical_token_path: &std::path::Path,
    current_dir: Option<&std::path::Path>,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    let candidate_path = std::path::Path::new(candidate);
    if candidate_path
        .canonicalize()
        .is_ok_and(|path| path == canonical_token_path)
    {
        return true;
    }
    current_dir.is_some_and(|dir| {
        dir.join(candidate_path)
            .canonicalize()
            .is_ok_and(|path| path == canonical_token_path)
    })
}

pub(super) fn exact_observation_answer_is_exact_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract.requests_exact_command_output()
        || route.output_contract.delivery_required
    {
        return false;
    }
    let candidate = candidate_answer.trim();
    let external_steps = journal
        .step_results
        .iter()
        .filter(|step| is_external_execution_step(step))
        .collect::<Vec<_>>();
    if external_steps.is_empty() {
        return false;
    }
    external_steps.iter().all(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && step
                .output_excerpt
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| {
                    !output.is_empty() && !output.ends_with("...(truncated)") && output == candidate
                })
    })
}

pub(super) fn exact_bounded_read_answer_is_exact_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route.output_contract.requests_exact_command_output()
    {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    journal
        .step_results
        .iter()
        .filter(|step| step_can_supply_verifier_observation(step))
        .filter_map(|step| step.output_excerpt.as_deref())
        .flat_map(structured_json_values_from_step_output)
        .filter(|value| exact_bounded_read_value_matches_route(route, value))
        .flat_map(|value| exact_bounded_read_answer_variants_from_value(&value))
        .any(|observed| observed == candidate)
}

pub(super) fn exact_bounded_read_value_matches_route(
    route: &AnswerContract,
    value: &serde_json::Value,
) -> bool {
    if route.output_contract.locator_kind != crate::OutputLocatorKind::Path {
        return true;
    }
    let expected = route.output_contract.locator_hint.trim();
    if expected.is_empty() {
        return true;
    }
    exact_bounded_read_value_paths(value)
        .iter()
        .any(|actual| verifier_paths_equivalent(actual, expected))
}

pub(super) fn exact_bounded_read_value_paths(value: &serde_json::Value) -> Vec<String> {
    [
        value.get("path").and_then(|item| item.as_str()),
        value.get("resolved_path").and_then(|item| item.as_str()),
        value.pointer("/extra/path").and_then(|item| item.as_str()),
        value
            .pointer("/extra/resolved_path")
            .and_then(|item| item.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(ToString::to_string)
    .collect()
}

pub(super) fn verifier_paths_equivalent(actual: &str, expected: &str) -> bool {
    let actual = actual.trim();
    let expected = expected.trim();
    if actual == expected {
        return true;
    }
    let actual_path = std::path::Path::new(actual);
    let expected_path = std::path::Path::new(expected);
    actual_path
        .canonicalize()
        .ok()
        .zip(expected_path.canonicalize().ok())
        .is_some_and(|(actual, expected)| actual == expected)
}

pub(super) fn exact_bounded_read_answer_variants_from_value(
    value: &serde_json::Value,
) -> Vec<String> {
    let action = value
        .get("action")
        .or_else(|| value.pointer("/extra/action"))
        .and_then(|item| item.as_str())
        .map(str::trim);
    if !matches!(action, Some("read_range" | "read_text_range")) {
        return Vec::new();
    }
    [
        value.get("excerpt").and_then(|item| item.as_str()),
        value.get("content").and_then(|item| item.as_str()),
        value
            .pointer("/extra/excerpt")
            .and_then(|item| item.as_str()),
        value
            .pointer("/extra/content")
            .and_then(|item| item.as_str()),
    ]
    .into_iter()
    .flatten()
    .flat_map(exact_bounded_read_answer_variants_from_excerpt)
    .collect()
}

pub(super) fn exact_bounded_read_answer_variants_from_excerpt(excerpt: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let raw = excerpt.trim();
    if !raw.is_empty() {
        variants.push(raw.to_string());
    }
    let without_prefixes = read_range_excerpt_without_line_prefixes(excerpt)
        .trim()
        .to_string();
    if !without_prefixes.is_empty() && !variants.iter().any(|item| item == &without_prefixes) {
        variants.push(without_prefixes);
    }
    variants
}

pub(super) fn is_external_execution_step(step: &crate::task_journal::TaskJournalStepTrace) -> bool {
    !is_synthesis_or_verifier_step(step)
}

pub(super) fn step_can_supply_verifier_observation(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    step.status == crate::executor::StepExecutionStatus::Ok && !is_synthesis_or_verifier_step(step)
}

pub(super) fn step_can_supply_verifier_prompt_observation(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    is_external_execution_step(step)
        && crate::task_journal::observed_evidence_for_step_trace(step).is_some()
}

pub(super) fn step_can_supply_verifier_observation_for_route(
    route: &AnswerContract,
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    if !step_can_supply_verifier_observation(step) {
        return false;
    }
    if !route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && crate::task_journal::step_reads_text_content(step)
    {
        return false;
    }
    true
}

pub(super) fn is_synthesis_or_verifier_step(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    matches!(
        step.skill.as_str(),
        "synthesize_answer" | "respond" | "think" | "answer_verifier"
    )
}

pub(super) fn scalar_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    ) {
        return false;
    }
    scalar_answer_value_is_grounded_in_successful_observation(route, journal, candidate_answer)
}

pub(super) fn scalar_answer_value_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() || candidate_answer.lines().count() > 1 {
        return false;
    }
    if observed_scalar_values_from_evidence_map_for_route(route, journal)
        .iter()
        .any(|observed| observed == candidate_answer)
    {
        return true;
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && route.output_contract_is_unclassified()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && journal.step_results.iter().any(|step| {
            step_can_supply_verifier_observation_for_route(route, step)
                && step_can_supply_strict_evidence_for_route(route, step)
                && step.output_excerpt.as_deref().is_some_and(|output| {
                    structured_read_output_contains_scalar_answer(output, candidate_answer)
                })
        })
    {
        return true;
    }
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation_for_route(route, step)
            && step_can_supply_strict_evidence_for_route(route, step)
            && step.output_excerpt.as_deref().is_some_and(|output| {
                observed_output_contains_scalar_answer(output, candidate_answer)
            })
    })
}

pub(super) fn observed_output_contains_scalar_answer(output: &str, candidate_answer: &str) -> bool {
    let output = output.trim();
    if output == candidate_answer {
        return true;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        return json_value_contains_scalar_answer(&value, candidate_answer);
    }
    output
        .lines()
        .map(str::trim)
        .any(|line| line == candidate_answer)
}

pub(super) fn json_value_contains_scalar_answer(
    value: &serde_json::Value,
    candidate_answer: &str,
) -> bool {
    match value {
        serde_json::Value::String(value) => value.trim() == candidate_answer,
        serde_json::Value::Number(value) => value.to_string() == candidate_answer,
        serde_json::Value::Bool(value) => value.to_string() == candidate_answer,
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_value_contains_scalar_answer(item, candidate_answer)),
        serde_json::Value::Object(map) => map
            .values()
            .any(|item| json_value_contains_scalar_answer(item, candidate_answer)),
        serde_json::Value::Null => false,
    }
}

#[cfg(test)]
pub(super) fn successful_observed_evidence_items(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<serde_json::Value> {
    let mut items = Vec::new();
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation(step) {
            continue;
        }
        let Some(evidence) = crate::task_journal::observed_evidence_for_step_trace(step) else {
            continue;
        };
        if let Some(evidence_items) = evidence.get("items").and_then(|value| value.as_array()) {
            items.extend(evidence_items.iter().cloned());
        }
    }
    items
}

pub(super) fn successful_observed_evidence_items_for_route(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> Vec<serde_json::Value> {
    let mut items = Vec::new();
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
            continue;
        }
        let Some(evidence) = crate::task_journal::observed_evidence_for_step_trace(step) else {
            continue;
        };
        if route_requires_strict_extractor_eligibility(route)
            && !observed_evidence_is_strict_shape_eligible(&evidence)
        {
            continue;
        }
        if let Some(evidence_items) = evidence.get("items").and_then(|value| value.as_array()) {
            items.extend(evidence_items.iter().cloned());
        }
    }
    items
}

pub(super) fn route_requires_strict_extractor_eligibility(route: &AnswerContract) -> bool {
    crate::evidence_policy::final_answer_shape_for_output_contract(&route.output_contract)
        .is_some_and(|shape| {
            matches!(
                shape.class(),
                crate::evidence_policy::FinalAnswerShapeClass::ScalarValue
                    | crate::evidence_policy::FinalAnswerShapeClass::StrictList
                    | crate::evidence_policy::FinalAnswerShapeClass::SinglePath
                    | crate::evidence_policy::FinalAnswerShapeClass::DeliveryArtifact
            )
        })
}

pub(super) fn observed_evidence_is_strict_shape_eligible(evidence: &serde_json::Value) -> bool {
    evidence
        .pointer("/extractor/strict_shape_eligible")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

pub(super) fn step_can_supply_strict_evidence_for_route(
    route: &AnswerContract,
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    if !route_requires_strict_extractor_eligibility(route) {
        return true;
    }
    crate::task_journal::observed_evidence_for_step_trace(step)
        .as_ref()
        .is_some_and(observed_evidence_is_strict_shape_eligible)
}

#[cfg(test)]
pub(super) fn observed_scalar_values_from_evidence_map(
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut values = BTreeSet::new();
    for item in successful_observed_evidence_items(journal) {
        if observed_evidence_item_supports_scalar(&item) {
            push_observed_evidence_excerpt(&item, &mut values);
        }
    }
    values
}

pub(super) fn observed_scalar_values_from_evidence_map_for_route(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut values = BTreeSet::new();
    for item in successful_observed_evidence_items_for_route(route, journal) {
        if observed_evidence_item_supports_scalar(&item) {
            push_observed_evidence_excerpt(&item, &mut values);
        }
    }
    values
}

#[cfg(test)]
pub(super) fn observed_single_path_values_from_evidence_map(
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for item in successful_observed_evidence_items(journal) {
        if observed_evidence_item_supports_single_path(&item) {
            push_observed_evidence_excerpt(&item, &mut paths);
        }
    }
    paths
}

pub(super) fn observed_single_path_values_from_evidence_map_for_route(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for item in successful_observed_evidence_items_for_route(route, journal) {
        if observed_evidence_item_supports_single_path(&item) {
            push_observed_evidence_excerpt(&item, &mut paths);
        }
    }
    paths
}

#[cfg(test)]
pub(super) fn observed_strict_list_items_from_evidence_map(
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut items = BTreeSet::new();
    for item in successful_observed_evidence_items(journal) {
        if observed_evidence_item_supports_strict_list(&item) {
            if let Some(excerpt) = observed_evidence_excerpt(&item) {
                push_observed_list_item(&excerpt, &mut items);
            }
        }
    }
    items
}

pub(super) fn observed_strict_list_items_from_evidence_map_for_route(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut items = BTreeSet::new();
    for item in successful_observed_evidence_items_for_route(route, journal) {
        if observed_evidence_item_supports_strict_list(&item) {
            if let Some(excerpt) = observed_evidence_excerpt(&item) {
                push_observed_list_item(&excerpt, &mut items);
            }
        }
    }
    items
}

pub(super) fn push_observed_evidence_excerpt(
    item: &serde_json::Value,
    values: &mut BTreeSet<String>,
) {
    if let Some(excerpt) = observed_evidence_excerpt(item) {
        values.insert(excerpt);
    }
}

pub(super) fn observed_evidence_excerpt(item: &serde_json::Value) -> Option<String> {
    if item.get("redacted").and_then(|value| value.as_bool()) == Some(true) {
        return None;
    }
    item.get("excerpt")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            item.get("count")
                .and_then(|value| value.as_u64().map(|value| value.to_string()))
        })
}

pub(super) fn observed_evidence_field(item: &serde_json::Value) -> Option<&str> {
    item.get("field")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(super) fn observed_evidence_kind(item: &serde_json::Value) -> Option<&str> {
    item.get("kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(super) fn observed_evidence_item_supports_scalar(item: &serde_json::Value) -> bool {
    let kind = observed_evidence_kind(item);
    if !matches!(
        kind,
        Some("string" | "number" | "bool" | "text" | "null" | "array")
    ) {
        return false;
    }
    let Some(field) = observed_evidence_field(item) else {
        return false;
    };
    let leaf = observed_evidence_field_leaf(field);
    if kind == Some("array") {
        return matches!(
            leaf.as_str(),
            "entries"
                | "facts"
                | "files"
                | "items"
                | "matches"
                | "names"
                | "paths"
                | "results"
                | "rows"
                | "tables"
        ) && item.get("count").and_then(|value| value.as_u64()).is_some();
    }
    matches!(
        leaf.as_str(),
        "bytes"
            | "count"
            | "file_size"
            | "file_type"
            | "found"
            | "hidden"
            | "hidden_count"
            | "kind"
            | "length"
            | "manager"
            | "package_manager"
            | "present"
            | "schema_version"
            | "size"
            | "size_bytes"
            | "state"
            | "status"
            | "subject"
            | "total"
            | "type"
            | "value"
            | "version"
    )
}

pub(super) fn observed_evidence_item_supports_single_path(item: &serde_json::Value) -> bool {
    if !matches!(observed_evidence_kind(item), Some("string" | "text")) {
        return false;
    }
    let Some(field) = observed_evidence_field(item) else {
        return false;
    };
    single_path_evidence_key(observed_evidence_field_leaf(field).as_str())
}

pub(super) fn observed_evidence_item_supports_strict_list(item: &serde_json::Value) -> bool {
    if !matches!(
        observed_evidence_kind(item),
        Some("string" | "number" | "bool" | "text")
    ) {
        return false;
    }
    let Some(field) = observed_evidence_field(item) else {
        return false;
    };
    let normalized = field.to_ascii_lowercase();
    let leaf = observed_evidence_field_leaf(&normalized);
    if field_has_array_index(&normalized)
        && matches!(
            leaf.as_str(),
            "identity_value" | "name" | "path" | "resolved_path" | "table" | "table_name"
        )
    {
        return true;
    }
    [
        "directories",
        "dirs",
        "files",
        "identity_values",
        "keys",
        "names",
        "paths",
        "results",
        "tables",
    ]
    .iter()
    .any(|prefix| array_item_field_matches(&normalized, prefix))
}

pub(super) fn observed_evidence_field_leaf(field: &str) -> String {
    let leaf = field.rsplit('.').next().unwrap_or(field);
    let leaf = leaf.split_once('[').map_or(leaf, |(prefix, _)| prefix);
    leaf.trim().to_ascii_lowercase()
}

pub(super) fn field_has_array_index(field: &str) -> bool {
    field.contains('[') && field.contains(']')
}

pub(super) fn array_item_field_matches(field: &str, prefix: &str) -> bool {
    field == prefix
        || field.starts_with(&format!("{prefix}["))
        || field.contains(&format!(".{prefix}["))
}
