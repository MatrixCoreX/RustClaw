use super::*;

pub(super) fn evidence_policy_scalar_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let Some(shape) =
        crate::evidence_policy::final_answer_shape_for_output_contract(&route.output_contract)
    else {
        return false;
    };
    if shape.class() != crate::evidence_policy::FinalAnswerShapeClass::ScalarValue {
        return false;
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
        && (!scalar_answer_is_strict_for_shape(shape, candidate_answer)
            || route.output_contract.response_shape != crate::OutputResponseShape::Scalar)
    {
        return count_summary_answer_is_grounded_in_successful_observation(
            route,
            journal,
            candidate_answer,
            route.output_contract.response_shape != crate::OutputResponseShape::Scalar,
        );
    }
    scalar_answer_is_strict_for_shape(shape, candidate_answer)
        && scalar_answer_value_is_grounded_in_successful_observation(
            route,
            journal,
            candidate_answer,
        )
}

pub(super) fn count_summary_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
    allow_single_observed_scalar: bool,
) -> bool {
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    if allow_single_observed_scalar && candidate.lines().count() > 1 {
        return false;
    }
    let mut observed_values = observed_scalar_values_from_evidence_map_for_route(route, journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
            continue;
        }
        if !step_can_supply_strict_evidence_for_route(route, step) {
            continue;
        }
        if let Some(output) = step.output_excerpt.as_deref() {
            observed_values.extend(observed_scalar_values_from_output(output));
        }
    }
    observed_values
        .iter()
        .filter(|observed| scalar_token_occurs_in_text(candidate, observed))
        .collect::<BTreeSet<_>>()
        .len()
        >= if allow_single_observed_scalar { 1 } else { 2 }
}

pub(super) fn quantity_comparison_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison) {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() || candidate.lines().count() > 2 {
        return false;
    }
    let mut sizes = observed_quantity_size_values(journal);
    if sizes.is_empty() {
        return false;
    }
    sizes.sort_unstable();
    sizes.dedup();
    if sizes
        .iter()
        .any(|size| scalar_token_occurs_in_text(candidate, &size.to_string()))
    {
        return true;
    }
    quantity_answer_mentions_human_size(candidate, &sizes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedScalarValue {
    pub(super) source_key: String,
    pub(super) text: String,
}

pub(super) fn recent_scalar_equality_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
        || route.output_contract.delivery_required
    {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() || candidate.lines().count() > 1 {
        return false;
    }
    let observations = recent_structured_scalar_values_from_journal(journal, 2);
    if observations.len() < 2 {
        return false;
    }
    let left = observations[0].text.trim();
    let right = observations[1].text.trim();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if !observations[0].source_key.is_empty()
        && observations[0].source_key == observations[1].source_key
    {
        return false;
    }
    observed_scalar_text_occurs_in_candidate(candidate, left)
        && observed_scalar_text_occurs_in_candidate(candidate, right)
        && candidate.contains(if left == right { "==" } else { "!=" })
}

pub(super) fn observed_scalar_text_occurs_in_candidate(candidate: &str, observed: &str) -> bool {
    let observed = observed.trim();
    if observed.is_empty() {
        return false;
    }
    if observed.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return scalar_token_occurs_in_text(candidate, observed);
    }
    candidate.contains(observed)
}

pub(super) fn recent_structured_scalar_values_from_journal(
    journal: &crate::task_journal::TaskJournal,
    limit: usize,
) -> Vec<ObservedScalarValue> {
    let mut recent = journal
        .step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .filter_map(observed_scalar_value_from_step_output)
        .take(limit.max(1))
        .collect::<Vec<_>>();
    recent.reverse();
    recent
}

pub(super) fn observed_scalar_value_from_step_output(output: &str) -> Option<ObservedScalarValue> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    observed_scalar_value_from_json(&value)
        .or_else(|| value.get("extra").and_then(observed_scalar_value_from_json))
}

pub(super) fn observed_scalar_value_from_json(
    value: &serde_json::Value,
) -> Option<ObservedScalarValue> {
    match value.get("action").and_then(|item| item.as_str()) {
        Some("extract_field" | "read_field") => {
            observed_scalar_value_from_extract_item(value, None)
        }
        Some("extract_fields" | "read_fields") => {
            let results = value.get("results")?.as_array()?;
            if results.len() != 1 {
                return None;
            }
            observed_scalar_value_from_extract_item(results.first()?, Some(value))
        }
        _ => None,
    }
}

pub(super) fn observed_scalar_value_from_extract_item(
    item: &serde_json::Value,
    parent: Option<&serde_json::Value>,
) -> Option<ObservedScalarValue> {
    if item.get("exists").and_then(|value| value.as_bool()) == Some(false) {
        return None;
    }
    let text = item
        .get("value_text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            item.get("value")
                .or_else(|| item.get("field_value"))
                .and_then(observed_scalar_json_value_text)
        })?;
    let path = item
        .get("resolved_path")
        .or_else(|| item.get("path"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            parent.and_then(|parent| {
                parent
                    .get("resolved_path")
                    .or_else(|| parent.get("path"))
                    .and_then(|value| value.as_str())
            })
        })
        .unwrap_or("")
        .trim();
    let field = item
        .get("resolved_field_path")
        .or_else(|| item.get("field_path"))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    let source_key = format!("{path}#{field}");
    Some(ObservedScalarValue { source_key, text })
}

pub(super) fn observed_scalar_json_value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

pub(super) fn observed_quantity_size_values(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<u64> {
    let mut sizes = Vec::new();
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
            collect_quantity_size_values_from_json(&value, &mut sizes);
        }
    }
    sizes
}

pub(super) fn collect_quantity_size_values_from_json(
    value: &serde_json::Value,
    sizes: &mut Vec<u64>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if matches!(key.as_str(), "size_bytes" | "total_size_bytes") {
                    if let Some(size) = child.as_u64() {
                        sizes.push(size);
                    }
                }
                collect_quantity_size_values_from_json(child, sizes);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_quantity_size_values_from_json(item, sizes);
            }
        }
        _ => {}
    }
}

pub(super) fn quantity_answer_mentions_human_size(candidate: &str, sizes: &[u64]) -> bool {
    let numbers = numeric_literals(candidate);
    if numbers.is_empty() {
        return false;
    }
    sizes.iter().any(|size| {
        let size = *size as f64;
        size >= 1024.0 * 1024.0
            && numbers
                .iter()
                .any(|number| human_size_number_matches_bytes(*number, size))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedPathSize {
    pub(super) path: String,
    pub(super) size_bytes: u64,
}

pub(super) fn directory_purpose_summary_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::DirectoryPurposeSummary) {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    if directory_purpose_summary_listing_content_answer_is_grounded(route, journal, candidate) {
        return true;
    }
    if directory_purpose_summary_find_ext_answer_is_grounded(journal, candidate) {
        return true;
    }
    let Some(largest) = observed_largest_path_batch_size(journal) else {
        return false;
    };
    path_size_answer_mentions_observed_largest(candidate, &largest)
}

pub(super) fn workspace_project_summary_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary) {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    let Some(names) = latest_observed_directory_structure_names(journal) else {
        return false;
    };
    let mentioned = names
        .iter()
        .filter(|name| observed_name_is_mentioned(candidate, name))
        .take(2)
        .count();
    mentioned >= 2
}

pub(super) fn recent_artifacts_judgment_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::RecentArtifactsJudgment)
        || route.output_contract.delivery_required
    {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    let content_names = observed_content_excerpt_path_names(journal);
    if content_names.is_empty() {
        return false;
    }
    let expected = observed_recent_artifact_answer_names(route, journal, &content_names);
    if expected.is_empty() {
        return false;
    }
    expected
        .iter()
        .all(|name| observed_name_is_mentioned(candidate, name))
}

pub(super) fn observed_recent_artifact_answer_names(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    content_names: &BTreeSet<String>,
) -> Vec<String> {
    let inventory_names = observed_inventory_names_for_contract(route, journal).unwrap_or_default();
    let mut names = content_artifact_file_names(content_names)
        .into_iter()
        .filter(|content_name| {
            inventory_names.is_empty()
                || inventory_names.iter().any(|inventory_name| {
                    recent_artifact_name_matches_content_name(inventory_name, content_name)
                })
        })
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    if names.len() > 4 && requested_listing_name_limit(route).is_none() {
        names.truncate(2);
    }
    names
}

pub(super) fn content_artifact_file_names(content_names: &BTreeSet<String>) -> Vec<String> {
    let mut names = content_names
        .iter()
        .filter_map(|name| {
            std::path::Path::new(name)
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .map(str::trim)
                .filter(|file_name| !file_name.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    names
}

pub(super) fn recent_artifact_name_matches_content_name(name: &str, content_name: &str) -> bool {
    let name = name.replace('\\', "/").to_ascii_lowercase();
    let content_name = content_name.replace('\\', "/").to_ascii_lowercase();
    !name.is_empty()
        && !content_name.is_empty()
        && (name == content_name
            || content_name.ends_with(&format!("/{name}"))
            || name.ends_with(&format!("/{content_name}")))
}

pub(super) fn directory_purpose_summary_listing_content_answer_is_grounded(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
        return false;
    }
    let Some(names) = observed_inventory_names_for_contract(route, journal) else {
        return false;
    };
    if names.len() < 2 || !journal_has_content_excerpt_observation(journal) {
        return false;
    }
    if candidate_answer.contains("largest.") {
        let Some(largest) = observed_largest_path_batch_size(journal) else {
            return false;
        };
        if !path_size_answer_mentions_observed_largest(candidate_answer, &largest) {
            return false;
        }
    }
    if requested_listing_name_limit(route).is_some() {
        observed_names_all_mentioned(candidate_answer, &names)
    } else {
        names
            .iter()
            .any(|name| observed_name_is_mentioned(candidate_answer, name))
    }
}

pub(super) fn observed_largest_path_batch_size(
    journal: &crate::task_journal::TaskJournal,
) -> Option<ObservedPathSize> {
    let mut largest: Option<ObservedPathSize> = None;
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation(step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        for value in structured_json_values_from_step_output(output) {
            collect_path_batch_sizes(&value, &mut largest);
        }
    }
    largest
}

pub(super) fn collect_path_batch_sizes(
    value: &serde_json::Value,
    largest: &mut Option<ObservedPathSize>,
) {
    if let Some(file) = value.pointer("/size_summary/largest_file") {
        collect_path_size_object(file, largest);
    }
    for key in ["entries", "facts"] {
        if let Some(items) = value.get(key).and_then(|item| item.as_array()) {
            for item in items {
                collect_path_size_object(item, largest);
            }
        }
    }
    collect_path_size_object(value, largest);
}

pub(super) fn collect_path_size_object(
    value: &serde_json::Value,
    largest: &mut Option<ObservedPathSize>,
) {
    if value.get("exists").and_then(|item| item.as_bool()) == Some(false) {
        return;
    }
    let Some(size_bytes) = value
        .get("size_bytes")
        .or_else(|| value.get("size"))
        .or_else(|| value.get("fact").and_then(|item| item.get("size_bytes")))
        .and_then(|item| item.as_u64())
    else {
        return;
    };
    let Some(path) = path_fact_candidates(value)
        .into_iter()
        .find(|path| !path.trim().is_empty())
    else {
        return;
    };
    let candidate = ObservedPathSize { path, size_bytes };
    if largest
        .as_ref()
        .is_none_or(|current| candidate.size_bytes > current.size_bytes)
    {
        *largest = Some(candidate);
    }
}

pub(super) fn directory_purpose_summary_find_ext_answer_is_grounded(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let Some((results, count)) = observed_find_ext_results(journal) else {
        return false;
    };
    if count == 0 || results.is_empty() || !answer_mentions_size(candidate_answer, count as u64) {
        return false;
    }
    if results.len() <= 80 {
        return results
            .iter()
            .all(|path| path_answer_mentions_any_variant(candidate_answer, path));
    }
    results
        .iter()
        .take(8)
        .all(|path| path_answer_mentions_any_variant(candidate_answer, path))
}

pub(super) fn observed_find_ext_results(
    journal: &crate::task_journal::TaskJournal,
) -> Option<(Vec<String>, usize)> {
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .flat_map(structured_json_values_from_step_output)
        .find_map(|value| find_ext_results_from_value(&value))
}

pub(super) fn find_ext_results_from_value(
    value: &serde_json::Value,
) -> Option<(Vec<String>, usize)> {
    if let Some(batch) = find_ext_results_from_object(value) {
        return Some(batch);
    }
    if let Some(extra) = value.get("extra") {
        if let Some(batch) = find_ext_results_from_object(extra) {
            return Some(batch);
        }
    }
    None
}

pub(super) fn find_ext_results_from_object(
    value: &serde_json::Value,
) -> Option<(Vec<String>, usize)> {
    if value.get("action").and_then(|item| item.as_str()) != Some("find_ext") {
        return None;
    }
    let results = value
        .get("results")
        .and_then(|item| item.as_array())?
        .iter()
        .filter_map(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if results.is_empty() {
        return None;
    }
    let count = value
        .get("count")
        .and_then(|item| item.as_u64())
        .and_then(|item| usize::try_from(item).ok())
        .unwrap_or(results.len());
    Some((results, count))
}

pub(super) fn path_size_answer_mentions_observed_largest(
    candidate_answer: &str,
    largest: &ObservedPathSize,
) -> bool {
    if !answer_mentions_size(candidate_answer, largest.size_bytes) {
        return false;
    }
    path_answer_mentions_any_variant(candidate_answer, &largest.path)
}

pub(super) fn answer_mentions_size(candidate_answer: &str, size_bytes: u64) -> bool {
    scalar_token_occurs_in_text(candidate_answer, &size_bytes.to_string())
        || numeric_literals(candidate_answer)
            .into_iter()
            .any(|number| (number - size_bytes as f64).abs() < f64::EPSILON)
}

pub(super) fn path_answer_mentions_any_variant(candidate_answer: &str, path: &str) -> bool {
    path_variants(path)
        .into_iter()
        .any(|variant| candidate_answer.contains(variant.as_str()))
}

pub(super) fn path_variants(path: &str) -> BTreeSet<String> {
    let mut variants = BTreeSet::new();
    let trimmed = path.trim();
    if !trimmed.is_empty() {
        variants.insert(trimmed.to_string());
    }
    let normalized = trimmed.replace('\\', "/");
    if !normalized.is_empty() {
        variants.insert(normalized.clone());
    }
    if let Some(file_name) = normalized
        .rsplit('/')
        .next()
        .filter(|item| !item.is_empty())
    {
        variants.insert(file_name.to_string());
    }
    variants
}

pub(super) fn human_size_number_matches_bytes(number: f64, bytes: f64) -> bool {
    if number <= 0.0 || !number.is_finite() {
        return false;
    }
    [
        1024.0_f64.powi(4),
        1024.0_f64.powi(3),
        1024.0_f64.powi(2),
        1000.0_f64.powi(4),
        1000.0_f64.powi(3),
        1000.0_f64.powi(2),
    ]
    .into_iter()
    .map(|unit| bytes / unit)
    .any(|scaled| {
        scaled >= 1.0
            && ((number - scaled).abs() <= 0.15
                || (number - scaled).abs() / scaled.max(1.0) <= 0.02)
    })
}

pub(super) fn numeric_literals(text: &str) -> Vec<f64> {
    let mut values = Vec::new();
    let mut token = String::new();
    let mut has_digit = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == ',' || ch == '.' {
            if ch.is_ascii_digit() {
                has_digit = true;
            }
            token.push(ch);
            continue;
        }
        if has_digit {
            push_numeric_literal(&mut values, &token);
        }
        token.clear();
        has_digit = false;
    }
    if has_digit {
        push_numeric_literal(&mut values, &token);
    }
    values
}

pub(super) fn push_numeric_literal(values: &mut Vec<f64>, token: &str) {
    let normalized = token.trim_matches('.').replace(',', "");
    if normalized.is_empty() || normalized == "." {
        return;
    }
    if let Ok(value) = normalized.parse::<f64>() {
        values.push(value);
    }
}

pub(super) fn observed_scalar_values_from_output(output: &str) -> BTreeSet<String> {
    let mut values = BTreeSet::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
        collect_scalar_values_from_json(&value, &mut values);
    } else {
        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if line.parse::<i64>().is_ok() {
                values.insert(line.to_string());
            }
        }
    }
    values
}

pub(super) fn collect_scalar_values_from_json(
    value: &serde_json::Value,
    values: &mut BTreeSet<String>,
) {
    match value {
        serde_json::Value::Number(value) => {
            values.insert(value.to_string());
        }
        serde_json::Value::Bool(value) => {
            values.insert(value.to_string());
        }
        serde_json::Value::String(value) => {
            let value = value.trim();
            if value.parse::<i64>().is_ok() {
                values.insert(value.to_string());
            }
        }
        serde_json::Value::Array(items) => {
            values.insert(items.len().to_string());
            for item in items {
                collect_scalar_values_from_json(item, values);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values() {
                collect_scalar_values_from_json(value, values);
            }
        }
        serde_json::Value::Null => {}
    }
}

pub(super) fn scalar_token_occurs_in_text(text: &str, scalar: &str) -> bool {
    let scalar = scalar.trim();
    !scalar.is_empty()
        && text
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|token| token == scalar)
}

pub(super) fn scalar_answer_is_strict_for_shape(
    shape: crate::evidence_policy::FinalAnswerShape,
    candidate_answer: &str,
) -> bool {
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() || candidate_answer.lines().count() > 1 {
        return false;
    }
    if shape == crate::evidence_policy::FinalAnswerShape::SingleCommitSubject {
        return !candidate_answer.ends_with('.') && !candidate_answer.ends_with('。');
    }
    let lower = candidate_answer.to_ascii_lowercase();
    if lower.contains(" is ") || lower.contains("：") || lower.contains(':') {
        return false;
    }
    if candidate_answer.ends_with('.') || candidate_answer.ends_with('。') {
        return false;
    }
    true
}

pub(super) fn markdown_heading_answer_is_grounded_in_read_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Strict
                | crate::OutputResponseShape::OneSentence
        )
        || !route.output_contract_is_unclassified()
        || matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
    {
        return false;
    }
    let Some(candidate_heading) = normalize_markdown_heading_answer(candidate_answer) else {
        return false;
    };
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation(step)
            && step.output_excerpt.as_deref().is_some_and(|output| {
                markdown_heading_from_read_observation(output)
                    .is_some_and(|heading| heading == candidate_heading)
            })
    })
}

pub(super) fn normalize_markdown_heading_answer(answer: &str) -> Option<String> {
    let answer = answer.trim();
    if answer.is_empty() || answer.lines().count() > 1 {
        return None;
    }
    let heading = answer.trim_start_matches('#').trim();
    (!heading.is_empty()).then(|| heading.to_string())
}

pub(super) fn markdown_heading_from_read_observation(output: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    markdown_heading_from_read_observation_value(&value, 0)
}

fn markdown_heading_from_read_observation_value(
    value: &serde_json::Value,
    depth: usize,
) -> Option<String> {
    if depth > 3 {
        return None;
    }
    let object = value.as_object()?;
    let action = object
        .get("action")
        .or_else(|| value.pointer("/extra/action"))
        .and_then(|value| value.as_str());
    if !matches!(action, Some("read_range" | "read_text_range")) {
        if let Some(answer) = object
            .get("extra")
            .and_then(|extra| markdown_heading_from_read_observation_value(extra, depth + 1))
        {
            return Some(answer);
        }
        return object
            .get("text")
            .and_then(|text| text.as_str())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text.trim()).ok())
            .and_then(|text_value| {
                markdown_heading_from_read_observation_value(&text_value, depth + 1)
            });
    }
    [
        object.get("excerpt").and_then(|value| value.as_str()),
        object.get("content").and_then(|value| value.as_str()),
        value
            .pointer("/extra/excerpt")
            .and_then(|value| value.as_str()),
        value
            .pointer("/extra/content")
            .and_then(|value| value.as_str()),
    ]
    .into_iter()
    .flatten()
    .find_map(markdown_heading_from_read_excerpt)
}

fn markdown_heading_from_read_excerpt(excerpt: &str) -> Option<String> {
    excerpt
        .lines()
        .map(strip_read_range_line_prefix)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find_map(normalize_markdown_heading_answer)
}

pub(super) fn strip_read_range_line_prefix(line: &str) -> &str {
    let Some((prefix, rest)) = line.split_once('|') else {
        return line;
    };
    if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
        rest
    } else {
        line
    }
}
