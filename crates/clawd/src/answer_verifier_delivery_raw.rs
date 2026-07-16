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

pub(super) fn raw_command_answer_is_exact_successful_observation(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
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
            && step.skill == "run_cmd"
            && step
                .output_excerpt
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| {
                    !output.is_empty() && !output.ends_with("...(truncated)") && output == candidate
                })
    })
}

pub(super) fn raw_bounded_read_answer_is_exact_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
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
        .filter(|value| raw_bounded_read_value_matches_route(route, value))
        .flat_map(|value| raw_bounded_read_answer_variants_from_value(&value))
        .any(|observed| observed == candidate)
}

pub(super) fn raw_bounded_read_value_matches_route(
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
    raw_bounded_read_value_paths(value)
        .iter()
        .any(|actual| verifier_paths_equivalent(actual, expected))
}

pub(super) fn raw_bounded_read_value_paths(value: &serde_json::Value) -> Vec<String> {
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

pub(super) fn raw_bounded_read_answer_variants_from_value(
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
    .flat_map(raw_bounded_read_answer_variants_from_excerpt)
    .collect()
}

pub(super) fn raw_bounded_read_answer_variants_from_excerpt(excerpt: &str) -> Vec<String> {
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

pub(super) fn existence_with_path_answer_is_grounded_in_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::ExistenceWithPath) {
        return false;
    }
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation_for_route(route, step)
            && step.output_excerpt.as_deref().is_some_and(|output| {
                path_batch_facts_contain_answer_path(output, candidate_answer)
            })
    })
}

pub(super) fn path_batch_facts_contain_answer_path(output: &str, candidate_answer: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    let has_path_batch_shape = value.get("action").and_then(|item| item.as_str())
        == Some("path_batch_facts")
        || value
            .get("facts")
            .and_then(|item| item.as_array())
            .is_some();
    if !has_path_batch_shape {
        return false;
    }
    value
        .get("facts")
        .and_then(|item| item.as_array())
        .is_some_and(|facts| {
            facts.iter().any(|fact| {
                fact.get("exists").and_then(|item| item.as_bool()).is_some()
                    && path_fact_candidates(fact)
                        .into_iter()
                        .any(|path| candidate_answer.contains(path.as_str()))
            })
        })
}

pub(super) fn path_fact_candidates(fact: &serde_json::Value) -> Vec<String> {
    let mut paths = Vec::new();
    let mut push_path = |value: Option<&serde_json::Value>| {
        if let Some(path) = value
            .and_then(|item| item.as_str())
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            paths.push(path.to_string());
        }
    };
    push_path(fact.get("resolved_path"));
    push_path(fact.get("path"));
    push_path(fact.get("name"));
    if let Some(inner) = fact.get("fact").and_then(|item| item.as_object()) {
        push_path(inner.get("resolved_path"));
        push_path(inner.get("path"));
        push_path(inner.get("name"));
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn structured_keys_answer_is_grounded_in_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract.requires_content_evidence && journal.step_results.is_empty() {
        return false;
    }
    let candidate_tokens = key_answer_tokens(candidate_answer);
    if candidate_tokens.is_empty() {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation_for_route(route, step)
            && step.output_excerpt.as_deref().is_some_and(|output| {
                structured_keys_from_output(output).is_some_and(|keys| {
                    !keys.is_empty()
                        && keys.iter().all(|key| {
                            normalized_key_answer_units(key).into_iter().all(|unit| {
                                candidate_tokens
                                    .iter()
                                    .any(|token| token.eq_ignore_ascii_case(&unit))
                            })
                        })
                })
            })
    })
}

pub(super) fn structured_keys_from_output(output: &str) -> Option<Vec<String>> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    if value.get("action").and_then(|item| item.as_str()) != Some("structured_keys")
        || !value
            .get("exists")
            .and_then(|item| item.as_bool())
            .unwrap_or(false)
    {
        return None;
    }
    let values = value
        .get("keys")
        .or_else(|| value.get("identity_values"))
        .and_then(|item| item.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })?;
    Some(values)
}

pub(super) fn execution_failed_step_answer_is_grounded_in_failed_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::ExecutionFailedStep) {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() || answer_text_is_machine_json_payload(candidate) {
        return false;
    }
    if successful_external_step_output_matches_candidate(journal, candidate) {
        return false;
    }
    let failed_tokens = execution_failed_step_machine_tokens(journal);
    if failed_tokens.is_empty() {
        return false;
    }
    let candidate_lower = candidate.to_ascii_lowercase();
    failed_tokens.iter().any(|token| {
        let token = token.trim();
        !token.is_empty() && candidate_lower.contains(&token.to_ascii_lowercase())
    })
}

pub(super) fn answer_text_is_machine_json_payload(answer_text: &str) -> bool {
    let Ok(serde_json::Value::Object(obj)) =
        serde_json::from_str::<serde_json::Value>(answer_text.trim())
    else {
        return false;
    };
    [
        "message_key",
        "reason_code",
        "error_code",
        "missing_evidence_fields",
        "answer_incomplete_reason",
    ]
    .iter()
    .any(|key| obj.contains_key(*key))
}

pub(super) fn successful_external_step_output_matches_candidate(
    journal: &crate::task_journal::TaskJournal,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    !candidate.is_empty()
        && journal.step_results.iter().any(|step| {
            step_can_supply_verifier_observation(step)
                && step.output_excerpt.as_deref().is_some_and(|output| {
                    let output = output.trim();
                    !output.is_empty() && output == candidate
                })
        })
}

pub(super) fn execution_failed_step_machine_tokens(
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    for step in &journal.step_results {
        if !is_external_execution_step(step)
            || step.status != crate::executor::StepExecutionStatus::Error
        {
            continue;
        }
        push_machine_token(&step.step_id, &mut tokens);
        let Some(error) = step.error_excerpt.as_deref() else {
            continue;
        };
        let Some(structured) = crate::skills::parse_structured_skill_error(error) else {
            continue;
        };
        push_machine_token(&structured.error_kind, &mut tokens);
        if let Some(extra) = structured.extra.as_ref() {
            push_json_string_machine_token(extra.get("command"), &mut tokens);
            push_json_string_machine_token(extra.get("exit_category"), &mut tokens);
            push_json_string_machine_token(extra.get("exit_classification_source"), &mut tokens);
            if let Some(exit_code) = extra.get("exit_code").and_then(|value| value.as_i64()) {
                push_machine_token(&exit_code.to_string(), &mut tokens);
            }
        }
    }
    tokens
}

pub(super) fn push_json_string_machine_token(
    value: Option<&serde_json::Value>,
    tokens: &mut BTreeSet<String>,
) {
    if let Some(value) = value.and_then(|value| value.as_str()) {
        push_machine_token(value, tokens);
    }
}

pub(super) fn push_machine_token(value: &str, tokens: &mut BTreeSet<String>) {
    let value = value.trim();
    if value.len() >= 3
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/'))
    {
        tokens.insert(value.to_ascii_lowercase());
    }
}

pub(super) fn normalized_key_answer_units(key: &str) -> Vec<String> {
    let tokens = key_answer_tokens(key);
    if tokens.len() == 1 {
        tokens
    } else {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![trimmed.to_ascii_lowercase()]
        }
    }
}

pub(super) fn key_answer_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens.sort();
    tokens.dedup();
    tokens
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
    if route.output_contract_marker_is(crate::OutputSemanticKind::FileBasename)
        && observed_single_path_values(route, journal)
            .iter()
            .filter_map(|path| {
                std::path::Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
            })
            .any(|basename| basename == candidate_answer)
    {
        return true;
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
                    | crate::evidence_policy::FinalAnswerShapeClass::Table
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

#[cfg(test)]
pub(super) fn observed_table_cells_from_evidence_map(
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut cells = BTreeSet::new();
    for item in successful_observed_evidence_items(journal) {
        if observed_evidence_item_supports_table_cell(&item) {
            if let Some(excerpt) = observed_evidence_excerpt(&item) {
                let normalized = normalize_strict_list_item(&excerpt);
                if !normalized.is_empty() {
                    cells.insert(normalized);
                }
            }
        }
    }
    cells
}

pub(super) fn observed_table_cells_from_evidence_map_for_route(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut cells = BTreeSet::new();
    for item in successful_observed_evidence_items_for_route(route, journal) {
        if observed_evidence_item_supports_table_cell(&item) {
            if let Some(excerpt) = observed_evidence_excerpt(&item) {
                let normalized = normalize_strict_list_item(&excerpt);
                if !normalized.is_empty() {
                    cells.insert(normalized);
                }
            }
        }
    }
    cells
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

pub(super) fn observed_evidence_item_supports_table_cell(item: &serde_json::Value) -> bool {
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
    normalized.contains("rows[") || array_item_field_matches(&normalized, "results")
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
