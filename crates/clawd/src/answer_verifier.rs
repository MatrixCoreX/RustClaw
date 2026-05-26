use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::json;

use crate::{AppState, ClaimedTask, RouteResult, TaskContract};

const ANSWER_VERIFIER_PROMPT_LOGICAL_PATH: &str = "prompts/answer_verifier_prompt.md";
const MAX_VERIFIER_STEPS: usize = 8;
const DEFAULT_RETRY_INSTRUCTION: &str = "Re-answer using the observed execution evidence and the original user request/output contract. Do not repeat the rejected answer.";

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub(crate) struct AnswerVerifierOut {
    #[serde(default)]
    pub(crate) pass: bool,
    #[serde(default)]
    pub(crate) missing_evidence_fields: Vec<String>,
    #[serde(default)]
    pub(crate) answer_incomplete_reason: String,
    #[serde(default)]
    pub(crate) should_retry: bool,
    #[serde(default)]
    pub(crate) retry_instruction: String,
    #[serde(default)]
    pub(crate) confidence: f64,
}

impl AnswerVerifierOut {
    pub(crate) fn normalized(mut self) -> Self {
        self.confidence = self.confidence.clamp(0.0, 1.0);
        self.missing_evidence_fields = self
            .missing_evidence_fields
            .into_iter()
            .map(|field| field.trim().to_string())
            .filter(|field| !field.is_empty())
            .collect();
        self.retry_instruction = self.retry_instruction.trim().to_string();
        self.answer_incomplete_reason = self.answer_incomplete_reason.trim().to_string();
        if self.high_confidence_gap() {
            self.should_retry = true;
            if self.retry_instruction.is_empty() {
                self.retry_instruction = DEFAULT_RETRY_INSTRUCTION.to_string();
            }
        }
        self
    }

    pub(crate) fn high_confidence_gap(&self) -> bool {
        self.confidence >= 0.55 && !self.pass
    }
}

pub(crate) fn should_verify_answer(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
) -> bool {
    let candidate = answer_text.trim();
    if candidate.is_empty() || route_result.needs_clarify || route_result.is_clarify_gate() {
        return false;
    }
    if matches!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Clarify)
    ) {
        return false;
    }
    let task_contract = TaskContract::from_route_result(route_result);
    if task_contract.intent_kind.as_str() != "planner_execute" {
        return false;
    }
    task_contract.evidence_required
        || !journal.step_results.is_empty()
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
}

pub(crate) fn structurally_satisfies_answer_contract(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if let Some(shape) = crate::contract_matrix::final_answer_shape_for_output_contract(
        &route_result.output_contract,
    ) {
        if shape.class() == crate::contract_matrix::FinalAnswerShapeClass::ScalarValue {
            return matrix_scalar_answer_is_grounded_in_successful_observation(
                route_result,
                journal,
                candidate_answer,
            );
        }
        if shape.class() == crate::contract_matrix::FinalAnswerShapeClass::StrictList {
            return matrix_strict_list_answer_is_grounded_in_successful_observation(
                route_result,
                journal,
                candidate_answer,
            );
        }
        if shape.class() == crate::contract_matrix::FinalAnswerShapeClass::Table {
            return matrix_table_answer_is_grounded_in_successful_observation(
                journal,
                candidate_answer,
            );
        }
        if shape.class() == crate::contract_matrix::FinalAnswerShapeClass::SinglePath {
            return matrix_single_path_answer_is_grounded_in_successful_observation(
                route_result,
                journal,
                candidate_answer,
            );
        }
        if shape.class() == crate::contract_matrix::FinalAnswerShapeClass::DeliveryArtifact {
            return matrix_delivery_artifact_answer_is_grounded_in_successful_observation(
                route_result,
                journal,
                candidate_answer,
            );
        }
    }
    if route_requires_single_file_delivery(route_result)
        && candidate_answer_has_grounded_existing_file_token(journal, candidate_answer)
    {
        return true;
    }
    if raw_command_answer_is_exact_single_successful_observation(journal, candidate_answer) {
        return true;
    }
    if markdown_heading_answer_is_grounded_in_read_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if existence_with_path_answer_is_grounded_in_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if structured_keys_answer_is_grounded_in_observation(route_result, journal, candidate_answer) {
        return true;
    }
    scalar_answer_is_grounded_in_successful_observation(route_result, journal, candidate_answer)
}

fn matrix_strict_list_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate_items = strict_list_answer_items(candidate_answer);
    if candidate_items.is_empty() {
        return false;
    }
    let observed_items = observed_strict_list_items(journal);
    if observed_items.is_empty() {
        return false;
    }
    let observed_variants = observed_items
        .iter()
        .flat_map(|item| strict_list_item_variants_for_route(route, item, true))
        .collect::<BTreeSet<_>>();
    let candidate_variants = candidate_items
        .iter()
        .flat_map(|item| strict_list_item_variants_for_route(route, item, false))
        .collect::<BTreeSet<_>>();
    let candidate_is_observed = candidate_items.iter().all(|item| {
        strict_list_item_variants_for_route(route, item, false)
            .iter()
            .any(|item| observed_variants.contains(item))
    });
    if !candidate_is_observed {
        return false;
    }
    if strict_list_route_allows_observed_subset(route) {
        return true;
    }
    observed_items.iter().all(|item| {
        strict_list_item_variants_for_route(route, item, true)
            .iter()
            .any(|item| candidate_variants.contains(item))
            || candidate_items
                .iter()
                .any(|candidate| strict_list_candidate_annotates_observed_item(candidate, item))
    })
}

fn strict_list_route_allows_observed_subset(route: &RouteResult) -> bool {
    matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FilePaths | crate::OutputSemanticKind::DirectoryNames
    )
}

fn matrix_table_answer_is_grounded_in_successful_observation(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate_cells = markdown_table_data_cells(candidate_answer);
    if candidate_cells.is_empty() {
        return false;
    }
    let observed_cells = observed_table_cells(journal);
    if observed_cells.is_empty() {
        return false;
    }
    candidate_cells.is_subset(&observed_cells) && observed_cells.is_subset(&candidate_cells)
}

fn matrix_single_path_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if let Some(candidate_path) = strict_single_path_answer(candidate_answer) {
        return observed_single_path_values(journal)
            .iter()
            .any(|observed_path| single_path_matches_observed(&candidate_path, observed_path));
    }
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarPathOnly {
        return false;
    }
    let candidate_items = strict_list_answer_items(candidate_answer);
    if candidate_items.len() <= 1 {
        return false;
    }
    let observed_variants = observed_strict_list_items(journal)
        .iter()
        .flat_map(|item| strict_list_item_variants(item))
        .collect::<BTreeSet<_>>();
    !observed_variants.is_empty()
        && candidate_items.iter().all(|item| {
            strict_list_item_variants(item)
                .iter()
                .any(|item| observed_variants.contains(item))
        })
}

fn matrix_delivery_artifact_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    route_requires_single_file_delivery(route)
        && (candidate_answer_has_grounded_existing_file_token(journal, candidate_answer)
            || candidate_answer_has_grounded_existing_plain_path(journal, candidate_answer))
}

fn candidate_answer_has_grounded_existing_plain_path(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let Some(candidate_path) = strict_single_path_answer(candidate_answer) else {
        return false;
    };
    let Ok(canonical_candidate_path) = std::path::Path::new(&candidate_path).canonicalize() else {
        return false;
    };
    file_token_path_is_grounded_in_observations(journal, &canonical_candidate_path)
}

fn strict_single_path_answer(answer: &str) -> Option<String> {
    let answer = answer.trim();
    if answer.is_empty() || answer.lines().count() > 1 {
        return None;
    }
    let lower = answer.to_ascii_lowercase();
    if lower.starts_with("file:")
        || answer.contains(':')
        || answer.contains('：')
        || answer.ends_with('.')
        || answer.ends_with('。')
    {
        return None;
    }
    Some(answer.to_string())
}

fn observed_single_path_values(journal: &crate::task_journal::TaskJournal) -> BTreeSet<String> {
    let mut paths = observed_single_path_values_from_evidence_map(journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation(step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
            collect_single_path_values_from_json(&value, &mut paths);
        } else if let Some(path) = strict_single_path_answer(output) {
            paths.insert(path);
        }
    }
    paths
}

fn collect_single_path_values_from_json(value: &serde_json::Value, paths: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            collect_joined_path_values_from_json_object(map, paths);
            for (key, child) in map {
                if single_path_evidence_key(key) {
                    if let Some(path) = child
                        .as_str()
                        .map(str::trim)
                        .filter(|path| !path.is_empty())
                    {
                        paths.insert(path.to_string());
                    }
                }
                collect_single_path_values_from_json(child, paths);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_single_path_values_from_json(item, paths);
            }
        }
        _ => {}
    }
}

fn collect_joined_path_values_from_json_object(
    map: &serde_json::Map<String, serde_json::Value>,
    paths: &mut BTreeSet<String>,
) {
    let Some(root) = map
        .get("resolved_path")
        .or_else(|| map.get("root"))
        .or_else(|| map.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    for key in ["results", "names", "paths", "files"] {
        let Some(items) = map.get(key).and_then(|value| value.as_array()) else {
            continue;
        };
        for item in items {
            let Some(child) = item
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let child_path = std::path::Path::new(child);
            if child_path.is_absolute() || joined_result_already_contains_root(root, child) {
                paths.insert(child.to_string());
            } else {
                paths.insert(
                    std::path::Path::new(root)
                        .join(child_path)
                        .display()
                        .to_string(),
                );
            }
        }
    }
}

fn joined_result_already_contains_root(root: &str, child: &str) -> bool {
    let root = root.trim().trim_matches('/');
    if root.is_empty() || root == "." {
        return true;
    }
    let child = child.trim().trim_start_matches("./");
    child == root || child.starts_with(&format!("{root}/"))
}

fn single_path_evidence_key(key: &str) -> bool {
    matches!(
        key,
        "path"
            | "resolved_path"
            | "cwd"
            | "current_dir"
            | "working_directory"
            | "workspace_root"
            | "root"
            | "archive_path"
            | "output_path"
            | "created_path"
            | "destination_path"
            | "target_path"
            | "file_path"
            | "result_path"
    )
}

fn single_path_matches_observed(candidate_path: &str, observed_path: &str) -> bool {
    let candidate_path = candidate_path.trim();
    let observed_path = observed_path.trim();
    if candidate_path.is_empty() || observed_path.is_empty() {
        return false;
    }
    if candidate_path == observed_path {
        return true;
    }
    let candidate = std::path::Path::new(candidate_path);
    let observed = std::path::Path::new(observed_path);
    if candidate.canonicalize().is_ok_and(|candidate| {
        observed
            .canonicalize()
            .is_ok_and(|observed| candidate == observed)
    }) {
        return true;
    }
    std::env::current_dir().ok().is_some_and(|dir| {
        dir.join(candidate).canonicalize().is_ok_and(|candidate| {
            observed
                .canonicalize()
                .is_ok_and(|observed| candidate == observed)
        })
    })
}

fn markdown_table_data_cells(answer: &str) -> BTreeSet<String> {
    let rows = answer
        .lines()
        .map(markdown_table_row_cells)
        .filter(|cells| !cells.is_empty())
        .collect::<Vec<_>>();
    if rows.len() < 3 || !markdown_table_separator_row(&rows[1]) {
        return BTreeSet::new();
    }
    let mut cells = BTreeSet::new();
    for row in rows.iter().skip(2) {
        for cell in row {
            let normalized = normalize_strict_list_item(cell);
            if !normalized.is_empty() {
                cells.insert(normalized);
            }
        }
    }
    cells
}

fn markdown_table_row_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return Vec::new();
    }
    trimmed
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .map(str::to_string)
        .collect()
}

fn markdown_table_separator_row(cells: &[String]) -> bool {
    cells.iter().all(|cell| {
        let value = cell.trim();
        value.len() >= 3
            && value.chars().all(|ch| matches!(ch, '-' | ':' | ' ' | '\t'))
            && value.chars().any(|ch| ch == '-')
    })
}

fn observed_table_cells(journal: &crate::task_journal::TaskJournal) -> BTreeSet<String> {
    let mut cells = observed_table_cells_from_evidence_map(journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation(step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
            continue;
        };
        collect_observed_table_cells_from_value(&value, &mut cells);
    }
    cells
}

fn collect_observed_table_cells_from_value(
    value: &serde_json::Value,
    cells: &mut BTreeSet<String>,
) {
    if let Some(rows) = value.get("rows").and_then(|value| value.as_array()) {
        collect_observed_table_cells_from_rows(rows, cells);
    }
    if let Some(rows) = value
        .pointer("/result/rows")
        .and_then(|value| value.as_array())
    {
        collect_observed_table_cells_from_rows(rows, cells);
    }
}

fn collect_observed_table_cells_from_rows(
    rows: &[serde_json::Value],
    cells: &mut BTreeSet<String>,
) {
    for row in rows {
        match row {
            serde_json::Value::Object(map) => {
                for value in map.values() {
                    push_observed_table_cell(value, cells);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    push_observed_table_cell(value, cells);
                }
            }
            value => push_observed_table_cell(value, cells),
        }
    }
}

fn push_observed_table_cell(value: &serde_json::Value, cells: &mut BTreeSet<String>) {
    let text = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        _ => String::new(),
    };
    let normalized = normalize_strict_list_item(&text);
    if !normalized.is_empty() {
        cells.insert(normalized);
    }
}

fn strict_list_answer_items(answer: &str) -> Vec<String> {
    let mut items = Vec::new();
    for line in answer.lines() {
        let line = strip_list_marker(line);
        if line.is_empty() || line.ends_with(':') || line.ends_with('：') {
            continue;
        }
        let segments = line
            .split([',', '，'])
            .map(strip_list_marker)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        items.extend(segments);
    }
    items.sort_by_key(|item| item.to_ascii_lowercase());
    items.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    items
}

fn strip_list_marker(raw: &str) -> String {
    let mut value = raw
        .trim()
        .trim_matches('`')
        .trim()
        .trim_start_matches(['-', '*', '•'])
        .trim()
        .to_string();
    if let Some((prefix, rest)) = value.split_once('.') {
        if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
            value = rest.trim().to_string();
        }
    }
    value.trim_matches('`').trim().to_string()
}

fn strict_list_item_variants_for_route(
    route: &RouteResult,
    item: &str,
    observed_item: bool,
) -> Vec<String> {
    let mut variants = strict_list_item_variants(item);
    if observed_item
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::DirectoryNames
    {
        variants.extend(strict_list_parent_directory_variants(item));
    }
    variants.sort();
    variants.dedup();
    variants
}

fn strict_list_item_variants(item: &str) -> Vec<String> {
    let normalized = normalize_strict_list_item(item);
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut variants = vec![normalized.clone()];
    if let Some(file_name) = std::path::Path::new(&normalized)
        .file_name()
        .and_then(|value| value.to_str())
        .map(normalize_strict_list_item)
        .filter(|value| !value.is_empty() && value != &normalized)
    {
        variants.push(file_name);
    }
    variants.sort();
    variants.dedup();
    variants
}

fn strict_list_parent_directory_variants(item: &str) -> Vec<String> {
    let normalized = normalize_strict_list_item(item);
    if normalized.is_empty() {
        return Vec::new();
    }
    let path = std::path::Path::new(&normalized);
    let parent = path
        .parent()
        .map(|value| {
            let text = value.to_string_lossy();
            if text.is_empty() {
                ".".to_string()
            } else {
                text.to_string()
            }
        })
        .unwrap_or_else(|| ".".to_string());
    vec![normalize_strict_list_item(&parent)]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect()
}

fn strict_list_candidate_annotates_observed_item(candidate: &str, observed: &str) -> bool {
    let candidate = normalize_strict_list_item(candidate);
    let observed = normalize_strict_list_item(observed);
    !candidate.is_empty()
        && !observed.is_empty()
        && candidate.len() > observed.len()
        && candidate.starts_with(&observed)
        && candidate[observed.len()..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
}

fn normalize_strict_list_item(item: &str) -> String {
    item.trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_lowercase()
}

fn observed_strict_list_items(journal: &crate::task_journal::TaskJournal) -> Vec<String> {
    let mut items = observed_strict_list_items_from_evidence_map(journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation(step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
            continue;
        };
        collect_observed_strict_list_items_from_value(&value, &mut items);
    }
    items.into_iter().collect()
}

fn collect_observed_strict_list_items_from_value(
    value: &serde_json::Value,
    items: &mut BTreeSet<String>,
) {
    push_string_array_values(
        value,
        items,
        &[
            "keys",
            "identity_values",
            "names",
            "paths",
            "files",
            "dirs",
            "directories",
            "results",
            "tables",
        ],
    );
    if let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(|value| value.as_object())
    {
        for child in names_by_kind.values() {
            push_array_strings(child, items);
        }
    }
    for key in ["entries", "items", "facts", "matches", "rows"] {
        if let Some(array) = value.get(key).and_then(|value| value.as_array()) {
            for item in array {
                collect_observed_list_item_object_fields(item, items);
            }
        }
    }
}

fn push_string_array_values(
    value: &serde_json::Value,
    items: &mut BTreeSet<String>,
    keys: &[&str],
) {
    for key in keys {
        if let Some(child) = value.get(*key) {
            push_array_strings(child, items);
        }
    }
}

fn push_array_strings(value: &serde_json::Value, items: &mut BTreeSet<String>) {
    let Some(array) = value.as_array() else {
        return;
    };
    for item in array {
        if let Some(text) = item.as_str() {
            push_observed_list_item(text, items);
        } else {
            collect_observed_list_item_object_fields(item, items);
        }
    }
}

fn collect_observed_list_item_object_fields(
    value: &serde_json::Value,
    items: &mut BTreeSet<String>,
) {
    let Some(map) = value.as_object() else {
        return;
    };
    if let Some(with_size) = observed_name_size_item(map) {
        push_observed_list_item(&with_size, items);
    }
    for key in [
        "name",
        "path",
        "resolved_path",
        "table",
        "table_name",
        "identity_value",
    ] {
        if let Some(text) = map.get(key).and_then(|value| value.as_str()) {
            push_observed_list_item(text, items);
        }
    }
}

fn observed_name_size_item(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let name = map
        .get("name")
        .or_else(|| map.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let size = map
        .get("size_bytes")
        .or_else(|| map.get("size"))
        .and_then(|value| match value {
            serde_json::Value::Number(value) => Some(value.to_string()),
            serde_json::Value::String(value) => Some(value.trim().to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())?;
    Some(format!("{name} {size}"))
}

fn push_observed_list_item(text: &str, items: &mut BTreeSet<String>) {
    let item = normalize_strict_list_item(text);
    if !item.is_empty() {
        items.insert(item);
    }
}

fn matrix_scalar_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let Some(shape) =
        crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
    else {
        return false;
    };
    if shape.class() != crate::contract_matrix::FinalAnswerShapeClass::ScalarValue {
        return false;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount
        && (!scalar_answer_is_strict_for_shape(shape, candidate_answer)
            || route.output_contract.response_shape != crate::OutputResponseShape::Scalar)
    {
        return count_summary_answer_is_grounded_in_successful_observation(
            journal,
            candidate_answer,
            route.output_contract.response_shape != crate::OutputResponseShape::Scalar,
        );
    }
    scalar_answer_is_strict_for_shape(shape, candidate_answer)
        && scalar_answer_value_is_grounded_in_successful_observation(journal, candidate_answer)
}

fn count_summary_answer_is_grounded_in_successful_observation(
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
    let mut observed_values = observed_scalar_values_from_evidence_map(journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation(step) {
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

fn observed_scalar_values_from_output(output: &str) -> BTreeSet<String> {
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

fn collect_scalar_values_from_json(value: &serde_json::Value, values: &mut BTreeSet<String>) {
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

fn scalar_token_occurs_in_text(text: &str, scalar: &str) -> bool {
    let scalar = scalar.trim();
    !scalar.is_empty()
        && text
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|token| token == scalar)
}

fn scalar_answer_is_strict_for_shape(
    shape: crate::contract_matrix::FinalAnswerShape,
    candidate_answer: &str,
) -> bool {
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() || candidate_answer.lines().count() > 1 {
        return false;
    }
    if shape == crate::contract_matrix::FinalAnswerShape::SingleCommitSubject {
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

fn markdown_heading_answer_is_grounded_in_read_observation(
    route: &RouteResult,
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
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::None
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

fn normalize_markdown_heading_answer(answer: &str) -> Option<String> {
    let answer = answer.trim();
    if answer.is_empty() || answer.lines().count() > 1 {
        return None;
    }
    let heading = answer.trim_start_matches('#').trim();
    (!heading.is_empty()).then(|| heading.to_string())
}

fn markdown_heading_from_read_observation(output: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    let object = value.as_object()?;
    let action = object.get("action").and_then(|value| value.as_str())?;
    if !matches!(action, "read_range" | "read_text_range") {
        return None;
    }
    let excerpt = object.get("excerpt").and_then(|value| value.as_str())?;
    excerpt
        .lines()
        .map(strip_read_range_line_prefix)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find_map(normalize_markdown_heading_answer)
}

fn strip_read_range_line_prefix(line: &str) -> &str {
    let Some((prefix, rest)) = line.split_once('|') else {
        return line;
    };
    if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
        rest
    } else {
        line
    }
}

fn route_requires_single_file_delivery(route: &RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) || matches!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    ) || (route.wants_file_delivery
        && !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryBatchFiles
        ))
}

fn candidate_answer_has_grounded_existing_file_token(
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

fn file_token_path_is_grounded_in_observations(
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

fn observed_output_contains_path(
    output: &str,
    canonical_token_path: &std::path::Path,
    current_dir: Option<&std::path::Path>,
) -> bool {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        return json_value_contains_path(&value, canonical_token_path, current_dir);
    }
    candidate_path_matches(output.trim(), canonical_token_path, current_dir)
}

fn json_value_contains_path(
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

fn resolved_dir_names_contain_path(
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

fn candidate_path_matches(
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

fn raw_command_answer_is_exact_single_successful_observation(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let mut external_steps = journal
        .step_results
        .iter()
        .filter(|step| is_external_execution_step(step));
    let Some(step) = external_steps.next() else {
        return false;
    };
    if external_steps.next().is_some() {
        return false;
    }
    if step.status != crate::executor::StepExecutionStatus::Ok || step.skill != "run_cmd" {
        return false;
    }
    let Some(output) = step.output_excerpt.as_deref().map(str::trim) else {
        return false;
    };
    !output.is_empty() && !output.ends_with("...(truncated)") && output == candidate_answer.trim()
}

fn is_external_execution_step(step: &crate::task_journal::TaskJournalStepTrace) -> bool {
    !is_synthesis_or_verifier_step(step)
}

fn step_can_supply_verifier_observation(step: &crate::task_journal::TaskJournalStepTrace) -> bool {
    step.status == crate::executor::StepExecutionStatus::Ok && !is_synthesis_or_verifier_step(step)
}

fn step_can_supply_verifier_observation_for_route(
    route: &RouteResult,
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    if !step_can_supply_verifier_observation(step) {
        return false;
    }
    if !route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && !route.wants_file_delivery
        && crate::task_journal::step_reads_text_content(step)
    {
        return false;
    }
    true
}

fn is_synthesis_or_verifier_step(step: &crate::task_journal::TaskJournalStepTrace) -> bool {
    matches!(
        step.skill.as_str(),
        "synthesize_answer" | "respond" | "think" | "answer_verifier"
    )
}

fn existence_with_path_answer_is_grounded_in_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPath {
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

fn path_batch_facts_contain_answer_path(output: &str, candidate_answer: &str) -> bool {
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

fn path_fact_candidates(fact: &serde_json::Value) -> Vec<String> {
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
    if let Some(inner) = fact.get("fact").and_then(|item| item.as_object()) {
        push_path(inner.get("resolved_path"));
        push_path(inner.get("path"));
    }
    paths.sort();
    paths.dedup();
    paths
}

fn structured_keys_answer_is_grounded_in_observation(
    route: &RouteResult,
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
        step_can_supply_verifier_observation(step)
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

fn structured_keys_from_output(output: &str) -> Option<Vec<String>> {
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

fn normalized_key_answer_units(key: &str) -> Vec<String> {
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

fn key_answer_tokens(text: &str) -> Vec<String> {
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

fn scalar_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    ) {
        return false;
    }
    scalar_answer_value_is_grounded_in_successful_observation(journal, candidate_answer)
}

fn scalar_answer_value_is_grounded_in_successful_observation(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() || candidate_answer.lines().count() > 1 {
        return false;
    }
    if observed_scalar_values_from_evidence_map(journal)
        .iter()
        .any(|observed| observed == candidate_answer)
    {
        return true;
    }
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation(step)
            && step.output_excerpt.as_deref().is_some_and(|output| {
                observed_output_contains_scalar_answer(output, candidate_answer)
            })
    })
}

fn observed_output_contains_scalar_answer(output: &str, candidate_answer: &str) -> bool {
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

fn json_value_contains_scalar_answer(value: &serde_json::Value, candidate_answer: &str) -> bool {
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

fn successful_observed_evidence_items(
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

fn observed_scalar_values_from_evidence_map(
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

fn observed_single_path_values_from_evidence_map(
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

fn observed_strict_list_items_from_evidence_map(
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

fn observed_table_cells_from_evidence_map(
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

fn push_observed_evidence_excerpt(item: &serde_json::Value, values: &mut BTreeSet<String>) {
    if let Some(excerpt) = observed_evidence_excerpt(item) {
        values.insert(excerpt);
    }
}

fn observed_evidence_excerpt(item: &serde_json::Value) -> Option<String> {
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

fn observed_evidence_field(item: &serde_json::Value) -> Option<&str> {
    item.get("field")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn observed_evidence_kind(item: &serde_json::Value) -> Option<&str> {
    item.get("kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn observed_evidence_item_supports_scalar(item: &serde_json::Value) -> bool {
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

fn observed_evidence_item_supports_single_path(item: &serde_json::Value) -> bool {
    if !matches!(observed_evidence_kind(item), Some("string" | "text")) {
        return false;
    }
    let Some(field) = observed_evidence_field(item) else {
        return false;
    };
    single_path_evidence_key(observed_evidence_field_leaf(field).as_str())
}

fn observed_evidence_item_supports_strict_list(item: &serde_json::Value) -> bool {
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

fn observed_evidence_item_supports_table_cell(item: &serde_json::Value) -> bool {
    if !matches!(
        observed_evidence_kind(item),
        Some("string" | "number" | "bool")
    ) {
        return false;
    }
    let Some(field) = observed_evidence_field(item) else {
        return false;
    };
    field.to_ascii_lowercase().contains("rows[")
}

fn observed_evidence_field_leaf(field: &str) -> String {
    let leaf = field.rsplit('.').next().unwrap_or(field);
    let leaf = leaf.split_once('[').map_or(leaf, |(prefix, _)| prefix);
    leaf.trim().to_ascii_lowercase()
}

fn field_has_array_index(field: &str) -> bool {
    field.contains('[') && field.contains(']')
}

fn array_item_field_matches(field: &str, prefix: &str) -> bool {
    field == prefix
        || field.starts_with(&format!("{prefix}["))
        || field.contains(&format!(".{prefix}["))
}

pub(crate) async fn verify_answer_observe_only(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> Option<AnswerVerifierOut> {
    if !should_verify_answer(route_result, journal, candidate_answer) {
        return None;
    }
    if let Some(local_gap) = local_missing_evidence_verifier_gap(route_result, journal) {
        tracing::warn!(
            task_id = %task.task_id,
            missing_evidence_fields = ?local_gap.missing_evidence_fields,
            answer_incomplete_reason = %local_gap.answer_incomplete_reason,
            retry_instruction = %local_gap.retry_instruction,
            "answer_verifier_local_missing_evidence_gap"
        );
        return Some(local_gap);
    }
    if structural_satisfaction_can_skip_verifier(route_result, journal, candidate_answer) {
        tracing::info!(
            task_id = %task.task_id,
            "answer_verifier_skipped_structural_satisfaction"
        );
        return None;
    }
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        ANSWER_VERIFIER_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            tracing::info!(
                "answer_verifier prompt_missing task_id={} err={}",
                task.task_id,
                err
            );
            return None;
        }
    };
    let task_contract = TaskContract::from_route_result(route_result);
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            ("__USER_REQUEST__", user_request.trim()),
            (
                "__TASK_CONTRACT__",
                &task_contract_prompt_block(&task_contract),
            ),
            (
                "__OUTPUT_CONTRACT__",
                &output_contract_prompt_block(route_result),
            ),
            (
                "__EXECUTION_EVIDENCE__",
                &execution_evidence_prompt_block(journal),
            ),
            ("__CANDIDATE_ANSWER__", candidate_answer.trim()),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "answer_verifier_prompt",
        &resolved.source,
        resolved.version.as_deref(),
        None,
    );
    let prompt_source = resolved.source;
    let llm_out = match crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::info!(
                "answer_verifier llm_failed task_id={} err={}",
                task.task_id,
                err
            );
            return None;
        }
    };
    let validation = match crate::prompt_utils::validate_against_schema::<AnswerVerifierOut>(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::AnswerVerifier,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok || validated.schema_normalized {
                tracing::info!(
                        "answer_verifier schema_parse_recovery task_id={} raw_parse_ok={} schema_normalized={}",
                        task.task_id,
                        validated.raw_parse_ok,
                        validated.schema_normalized
                    );
            }
            validated.value.normalized()
        }
        Err(err) => {
            tracing::info!(
                "answer_verifier schema_validation_failed task_id={} err={}",
                task.task_id,
                err
            );
            return None;
        }
    };
    if validation.high_confidence_gap() {
        tracing::warn!(
            task_id = %task.task_id,
            missing_evidence_fields = ?validation.missing_evidence_fields,
            answer_incomplete_reason = %validation.answer_incomplete_reason,
            should_retry = validation.should_retry,
            retry_instruction = %validation.retry_instruction,
            confidence = validation.confidence,
            "answer_verifier_observed_gap"
        );
    }
    Some(validation)
}

fn structural_satisfaction_can_skip_verifier(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    local_missing_evidence_verifier_gap(route_result, journal).is_none()
        && structurally_satisfies_answer_contract(route_result, journal, candidate_answer)
}

pub(crate) fn local_missing_evidence_verifier_gap(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> Option<AnswerVerifierOut> {
    let task_contract = TaskContract::from_route_result(route_result);
    if task_contract.intent_kind.as_str() != "planner_execute"
        || task_contract.required_evidence_fields.is_empty()
    {
        return None;
    }
    let coverage = crate::task_journal::evidence_coverage_for_route(route_result, journal);
    if coverage.is_complete() {
        return None;
    }
    let missing = coverage.missing_evidence;
    Some(AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: missing.clone(),
        answer_incomplete_reason: format!(
            "missing required execution evidence: {}",
            missing.join(",")
        ),
        should_retry: true,
        retry_instruction: format!(
            "Collect the missing required evidence fields before finalizing: {}.",
            missing.join(", ")
        ),
        confidence: 0.9,
    })
}

fn task_contract_prompt_block(task_contract: &TaskContract) -> String {
    task_contract.compact_prompt_line()
}

fn output_contract_prompt_block(route_result: &RouteResult) -> String {
    serde_json::to_string_pretty(&json!({
        "response_shape": route_result.output_contract.response_shape.as_str(),
        "requires_content_evidence": route_result.output_contract.requires_content_evidence,
        "delivery_required": route_result.output_contract.delivery_required,
        "locator_kind": route_result.output_contract.locator_kind.as_str(),
        "delivery_intent": route_result.output_contract.delivery_intent.as_str(),
        "semantic_kind": route_result.output_contract.semantic_kind.as_str(),
        "locator_hint": route_result.output_contract.locator_hint,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn provider_safe_excerpt_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64:{hash:016x}")
}

fn provider_safe_step_evidence(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> serde_json::Value {
    json!({
        "step_id": step.step_id,
        "skill": step.skill,
        "status": step.status.as_str(),
        "observed_evidence": crate::task_journal::observed_evidence_for_step_trace(step),
        "output_excerpt_present": step.output_excerpt.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "output_excerpt_hash": step.output_excerpt.as_deref().map(provider_safe_excerpt_hash),
        "error_excerpt_present": step.error_excerpt.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "error_excerpt_hash": step.error_excerpt.as_deref().map(provider_safe_excerpt_hash),
    })
}

fn execution_evidence_prompt_block(journal: &crate::task_journal::TaskJournal) -> String {
    let mut steps = journal
        .step_results
        .iter()
        .rev()
        .take(MAX_VERIFIER_STEPS)
        .map(provider_safe_step_evidence)
        .collect::<Vec<_>>();
    steps.reverse();
    serde_json::to_string_pretty(&steps).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::{
        execution_evidence_prompt_block, local_missing_evidence_verifier_gap,
        observed_scalar_values_from_evidence_map, observed_single_path_values_from_evidence_map,
        observed_strict_list_items_from_evidence_map, observed_table_cells_from_evidence_map,
        should_verify_answer, structural_satisfaction_can_skip_verifier,
        structurally_satisfies_answer_contract, AnswerVerifierOut,
    };

    fn route_with_mode(ask_mode: crate::AskMode) -> crate::RouteResult {
        crate::RouteResult {
            ask_mode,
            resolved_intent: "test intent".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        }
    }

    #[test]
    fn answer_verifier_schema_accepts_typed_output() {
        let raw = json!({
            "pass": false,
            "missing_evidence_fields": ["size_bytes"],
            "answer_incomplete_reason": "missing requested size evidence",
            "should_retry": true,
            "retry_instruction": "Collect file metadata and answer with path plus size.",
            "confidence": 0.86
        });
        let validated = crate::prompt_utils::validate_against_schema::<AnswerVerifierOut>(
            &raw.to_string(),
            crate::prompt_utils::PromptSchemaId::AnswerVerifier,
        )
        .expect("schema should validate answer verifier output");
        assert!(!validated.value.pass);
        assert!(validated.value.should_retry);
    }

    #[test]
    fn answer_verifier_schema_drift() {
        const SCHEMA_RAW: &str =
            include_str!("../../../prompts/schemas/answer_verifier.schema.json");
        let schema: serde_json::Value =
            serde_json::from_str(SCHEMA_RAW).expect("answer_verifier schema must be valid JSON");
        assert_eq!(
            schema.get("type").and_then(serde_json::Value::as_str),
            Some("object"),
            "answer_verifier schema root must be object"
        );
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&json!(false)),
            "answer_verifier schema must reject unknown fields after canonicalization"
        );

        let expected = [
            "pass",
            "missing_evidence_fields",
            "answer_incomplete_reason",
            "should_retry",
            "retry_instruction",
            "confidence",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema must have object properties");
        let actual = properties
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual, expected,
            "answer_verifier.schema.json properties drifted from AnswerVerifierOut"
        );

        let required = schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .expect("schema must have required fields")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            required, expected,
            "answer_verifier.schema.json required set drifted from AnswerVerifierOut"
        );

        let raw = json!({
            "pass": true,
            "missing_evidence_fields": [],
            "answer_incomplete_reason": "",
            "should_retry": false,
            "retry_instruction": "",
            "confidence": 1.0
        })
        .to_string();
        crate::prompt_utils::validate_against_schema::<AnswerVerifierOut>(
            &raw,
            crate::prompt_utils::PromptSchemaId::AnswerVerifier,
        )
        .expect("schema-conformant answer verifier payload must deserialize");
    }

    #[test]
    fn answer_verifier_gap_is_high_confidence_only() {
        let low = AnswerVerifierOut {
            pass: false,
            confidence: 0.2,
            ..AnswerVerifierOut::default()
        };
        let high = AnswerVerifierOut {
            pass: false,
            confidence: 0.8,
            ..AnswerVerifierOut::default()
        };
        assert!(!low.high_confidence_gap());
        assert!(high.high_confidence_gap());
    }

    #[test]
    fn answer_verifier_normalizes_high_confidence_gap_to_retry() {
        let normalized = AnswerVerifierOut {
            pass: false,
            confidence: 0.82,
            retry_instruction: "  ".to_string(),
            ..AnswerVerifierOut::default()
        }
        .normalized();
        assert!(normalized.should_retry);
        assert!(!normalized.retry_instruction.trim().is_empty());
    }

    #[test]
    fn execution_evidence_prompt_uses_provider_safe_redacted_view() {
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-provider-safe", "ask", "检查配置");
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "config_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "path": "/tmp/app.toml",
                    "token": "sk-test-secret-token-that-should-not-leak"
                })
                .to_string(),
            ),
            error: Some("password=secret-value-that-should-not-leak".to_string()),
            started_at: 1,
            finished_at: 2,
        });

        let block = execution_evidence_prompt_block(&journal);

        assert!(block.contains("\"observed_evidence\""));
        assert!(block.contains("\"output_excerpt_hash\""));
        assert!(block.contains("\"error_excerpt_hash\""));
        assert!(!block.contains("\"output_excerpt\""));
        assert!(!block.contains("\"error_excerpt\""));
        assert!(!block.contains("sk-test-secret-token-that-should-not-leak"));
        assert!(!block.contains("password=secret-value-that-should-not-leak"));
        assert!(block.contains("\"redacted\": true"));
    }

    #[test]
    fn direct_answer_route_skips_answer_verifier() {
        let route = route_with_mode(crate::AskMode::direct_answer());
        let journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
        assert!(!should_verify_answer(&route, &journal, "hi"));
    }

    #[test]
    fn clarify_final_status_skips_answer_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
        route.output_contract.requires_content_evidence = true;
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

        assert!(!should_verify_answer(
            &route,
            &journal,
            "please provide the missing path"
        ));
    }

    #[test]
    fn local_missing_evidence_gap_reports_required_fields() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-local-gap", "ask", "exists?");
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(json!({"path": "/tmp/a.txt", "exists": true}).to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        let gap =
            local_missing_evidence_verifier_gap(&route, &journal).expect("missing kind evidence");
        assert_eq!(gap.missing_evidence_fields, vec!["kind"]);
        assert!(gap.should_retry);
        assert!(gap.high_confidence_gap());
    }

    #[test]
    fn local_missing_evidence_gap_skips_when_required_fields_are_observed() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-local-gap-ok", "ask", "list names");
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(json!({"names": ["Cargo.toml"]}).to_string()),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
    }

    #[test]
    fn structural_satisfaction_does_not_skip_missing_contract_evidence() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-structural-gap", "ask", "exists?");
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                json!({
                    "action": "path_batch_facts",
                    "facts": [{
                        "path": "/tmp/a.txt",
                        "exists": true
                    }]
                })
                .to_string(),
            ),
            error: None,
            started_at: 1,
            finished_at: 2,
        });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "/tmp/a.txt exists"
        ));
        let gap =
            local_missing_evidence_verifier_gap(&route, &journal).expect("missing kind evidence");
        assert_eq!(gap.missing_evidence_fields, vec!["kind"]);
        assert!(!structural_satisfaction_can_skip_verifier(
            &route,
            &journal,
            "/tmp/a.txt exists"
        ));
    }

    #[test]
    fn grounded_file_token_satisfies_file_delivery_contract_before_llm_verifier() {
        let root = std::env::temp_dir().join(format!(
            "rustclaw-answer-verifier-file-token-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        let file = root.join("release_checklist.md");
        std::fs::write(&file, "ok").expect("write temp file");

        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.wants_file_delivery = true;
        route.output_contract.delivery_required = true;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-file-token", "ask", "send that file");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "path_batch_facts",
                        "facts": [{
                            "path": file.display().to_string(),
                            "fact": {
                                "kind": "file",
                                "resolved_path": file.display().to_string()
                            }
                        }]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            &format!("FILE:{}", file.display())
        ));

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn matrix_delivery_artifact_shape_rejects_raw_command_summary_answer() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.wants_file_delivery = true;
        route.output_contract.delivery_required = true;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-delivery-shape", "ask", "send file");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "run_cmd".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("done".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(!structurally_satisfies_answer_contract(
            &route, &journal, "done"
        ));
    }

    #[test]
    fn matrix_delivery_artifact_shape_accepts_grounded_plain_path() {
        let root = std::env::temp_dir().join(format!(
            "rustclaw-answer-verifier-plain-delivery-path-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        let file = root.join("report.md");
        std::fs::write(&file, "ok").expect("write temp file");

        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.wants_file_delivery = true;
        route.output_contract.delivery_required = true;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-delivery-path", "ask", "send file");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "path": file.display().to_string(),
                        "resolved_path": file.display().to_string()
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            &file.display().to_string()
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            &format!("File: {}", file.display())
        ));

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scalar_answer_grounded_in_plain_observation_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-scalar", "ask", "where am I");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "run_cmd".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("/home/guagua/rustclaw\n".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "/home/guagua/rustclaw"
        ));
    }

    #[test]
    fn scalar_answer_grounded_in_json_observation_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-json-scalar", "ask", "count them");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(json!({"count": 3, "items": ["a", "b", "c"]}).to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route, &journal, "3"
        ));
    }

    #[test]
    fn matrix_scalar_shape_requires_plain_scalar_answer() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-matrix-scalar", "ask", "count them");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(json!({"count": 3, "items": ["a", "b", "c"]}).to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route, &journal, "3"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "The count is 3."
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route, &journal, "count: 3"
        ));
    }

    #[test]
    fn matrix_scalar_count_shape_allows_observed_component_breakdown() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-component-count", "ask", "count dirs");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "system_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "count_inventory",
                        "counts": {
                            "total": 66,
                            "files": 40,
                            "dirs": 26
                        }
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "文件：40 个\n文件夹：26 个"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "总数：66 个"
        ));
    }

    #[test]
    fn matrix_single_path_shape_accepts_root_prefixed_results() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-root-prefixed-path", "ask", "find it");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_search".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "find_name",
                        "count": 1,
                        "root": "plan",
                        "results": ["plan/agent_intelligence_architecture_plan_20260511.md"]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "plan/agent_intelligence_architecture_plan_20260511.md"
        ));
    }

    #[test]
    fn structured_keys_answer_covering_all_keys_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::StructuredKeys;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-keys", "ask", "list keys");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "config_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "structured_keys",
                        "exists": true,
                        "container_type": "object",
                        "count": 3,
                        "keys": ["app", "features", "paths"]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "app, features, paths"
        ));
        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "app\nfeatures\npaths"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "app, features"
        ));
    }

    #[test]
    fn matrix_strict_list_shape_rejects_unobserved_items() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-matrix-list", "ask", "list files");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "inventory_dir",
                        "names_only": true,
                        "names": ["README.md", "Cargo.toml"],
                        "entries": [
                            {"name": "README.md", "kind": "file", "path": "/tmp/repo/README.md"},
                            {"name": "Cargo.toml", "kind": "file", "path": "/tmp/repo/Cargo.toml"}
                        ]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "- README.md\n- Cargo.toml"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "- README.md\n- missing.txt"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "The files are README.md and Cargo.toml."
        ));
    }

    #[test]
    fn matrix_table_shape_requires_markdown_table_answer() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;
        route.output_contract.locator_hint = "data/app.sqlite".to_string();
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-matrix-table", "ask", "list tables");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "db_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "columns": ["name"],
                        "rows": [
                            {"name": "orders"},
                            {"name": "users"}
                        ]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "| name |\n| --- |\n| orders |\n| users |"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "orders, users"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "| name |\n| --- |\n| orders |"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "| name |\n| --- |\n| orders |\n| payments |"
        ));
    }

    #[test]
    fn matrix_single_path_shape_requires_plain_grounded_path() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchivePack;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-matrix-path", "ask", "pack logs");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "archive_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "pack",
                        "archive_path": "/tmp/rustclaw/report.zip",
                        "source_paths": ["/tmp/rustclaw/report.md"]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "/tmp/rustclaw/report.zip"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "Archive: /tmp/rustclaw/report.zip"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "The archive is /tmp/rustclaw/report.zip"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "/tmp/rustclaw/missing.zip"
        ));
    }

    #[test]
    fn matrix_scalar_shape_uses_observed_evidence_map_values() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let mut journal = crate::task_journal::TaskJournal::for_task(
            "task-matrix-scalar-evidence",
            "ask",
            "count them",
        );
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "fs_basic",
                json!({"count": 3, "items": ["a", "b", "c"]}).to_string(),
            ));

        assert!(observed_scalar_values_from_evidence_map(&journal).contains("3"));
        assert!(structurally_satisfies_answer_contract(
            &route, &journal, "3"
        ));
    }

    #[test]
    fn matrix_scalar_shape_does_not_use_content_excerpt_as_field_value() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
        let mut journal = crate::task_journal::TaskJournal::for_task(
            "task-matrix-scalar-content-excerpt",
            "ask",
            "service status",
        );
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_read",
                "fs_basic",
                json!({
                    "action": "read_text_range",
                    "path": "/tmp/status-notes.md",
                    "excerpt": "1|running"
                })
                .to_string(),
            ));

        assert!(!observed_scalar_values_from_evidence_map(&journal).contains("1|running"));
        assert!(!structurally_satisfies_answer_contract(
            &route, &journal, "running"
        ));
    }

    #[test]
    fn matrix_strict_list_shape_uses_observed_evidence_map_values() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        let mut journal = crate::task_journal::TaskJournal::for_task(
            "task-matrix-list-evidence",
            "ask",
            "list files",
        );
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "fs_basic",
                json!({
                    "action": "inventory_dir",
                    "names": ["README.md", "Cargo.toml"]
                })
                .to_string(),
            ));

        let items = observed_strict_list_items_from_evidence_map(&journal);
        assert!(items.contains("readme.md"));
        assert!(items.contains("cargo.toml"));
        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "- README.md\n- Cargo.toml"
        ));
    }

    #[test]
    fn matrix_scalar_shape_accepts_count_from_array_evidence_for_non_scalar_route_shape() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-array-count", "ask", "count rows");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "db_basic",
                json!({"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}).to_string(),
            ));

        assert!(structurally_satisfies_answer_contract(
            &route, &journal, "2"
        ));
    }

    #[test]
    fn matrix_file_path_list_shape_allows_grounded_filtered_subset() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-path-subset", "ask", "find path");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "fs_basic",
                json!({
                    "action": "find_name",
                    "results": ["plan/a.md", "plan/b.md", "docs/c.md"]
                })
                .to_string(),
            ));

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "plan/b.md"
        ));
        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "plan/missing.md"
        ));
    }

    #[test]
    fn matrix_shape_grounding_ignores_synthesis_and_verifier_steps() {
        let mut list_route = route_with_mode(crate::AskMode::planner_execute_plain());
        list_route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        list_route.output_contract.requires_content_evidence = true;
        list_route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
        let mut list_journal =
            crate::task_journal::TaskJournal::for_task("task-synth-list", "ask", "list files");
        list_journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_synth",
                "synthesize_answer",
                json!({"names": ["README.md", "Cargo.toml"]}).to_string(),
            ));
        list_journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_verifier",
            "answer_verifier",
            json!({"observed_evidence": {"items": [{"kind": "filename", "excerpt": "README.md"}]}})
                .to_string(),
        ));
        assert!(!structurally_satisfies_answer_contract(
            &list_route,
            &list_journal,
            "- README.md\n- Cargo.toml"
        ));

        let mut table_route = route_with_mode(crate::AskMode::planner_execute_plain());
        table_route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        table_route.output_contract.requires_content_evidence = true;
        table_route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;
        let mut table_journal =
            crate::task_journal::TaskJournal::for_task("task-synth-table", "ask", "list tables");
        table_journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_respond",
                "respond",
                json!({"rows": [{"name": "orders"}, {"name": "users"}]}).to_string(),
            ));
        assert!(!structurally_satisfies_answer_contract(
            &table_route,
            &table_journal,
            "| name |\n| --- |\n| orders |\n| users |"
        ));

        let mut path_route = route_with_mode(crate::AskMode::planner_execute_plain());
        path_route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        path_route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchivePack;
        let mut path_journal =
            crate::task_journal::TaskJournal::for_task("task-synth-path", "ask", "pack logs");
        path_journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_think",
                "think",
                json!({"archive_path": "/tmp/rustclaw/report.zip"}).to_string(),
            ));
        assert!(!structurally_satisfies_answer_contract(
            &path_route,
            &path_journal,
            "/tmp/rustclaw/report.zip"
        ));

        let mut scalar_route = route_with_mode(crate::AskMode::planner_execute_plain());
        scalar_route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        scalar_route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let mut scalar_journal =
            crate::task_journal::TaskJournal::for_task("task-synth-scalar", "ask", "count files");
        scalar_journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_synth",
                "synthesize_answer",
                json!({"count": 3}).to_string(),
            ));
        assert!(!structurally_satisfies_answer_contract(
            &scalar_route,
            &scalar_journal,
            "3"
        ));
    }

    #[test]
    fn matrix_table_shape_uses_observed_evidence_map_cells() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;
        let mut journal = crate::task_journal::TaskJournal::for_task(
            "task-matrix-table-evidence",
            "ask",
            "list tables",
        );
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "db_basic",
                json!({
                    "columns": ["name"],
                    "rows": [
                        {"name": "orders"},
                        {"name": "users"}
                    ]
                })
                .to_string(),
            ));

        let cells = observed_table_cells_from_evidence_map(&journal);
        assert!(cells.contains("orders"));
        assert!(cells.contains("users"));
        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "| name |\n| --- |\n| orders |\n| users |"
        ));
    }

    #[test]
    fn matrix_single_path_shape_uses_observed_evidence_map_paths() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchivePack;
        let mut journal = crate::task_journal::TaskJournal::for_task(
            "task-matrix-path-evidence",
            "ask",
            "pack logs",
        );
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "archive_basic",
                json!({
                    "archive_path": "/tmp/rustclaw/report.zip",
                    "source_paths": ["/tmp/rustclaw/report.md"]
                })
                .to_string(),
            ));

        assert!(observed_single_path_values_from_evidence_map(&journal)
            .contains("/tmp/rustclaw/report.zip"));
        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "/tmp/rustclaw/report.zip"
        ));
    }

    #[test]
    fn structured_keys_answer_accepts_array_identity_values() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::StructuredKeys;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-array-keys", "ask", "list names");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "config_basic",
                json!({
                    "action": "structured_keys",
                    "exists": true,
                    "container_type": "array",
                    "count": 2,
                    "identity_values": ["fs_basic", "config-basic"]
                })
                .to_string(),
            ));

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "`fs_basic`, `config-basic`"
        ));
    }

    #[test]
    fn structured_keys_answer_uses_observed_action_when_semantic_label_missing() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-keys-missing-label", "ask", "keys");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "config_basic",
                json!({
                    "action": "structured_keys",
                    "exists": true,
                    "container_type": "object",
                    "count": 3,
                    "keys": ["app", "features", "paths"]
                })
                .to_string(),
            ));

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "app, features, paths"
        ));
    }

    #[test]
    fn markdown_heading_answer_grounded_in_read_range_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-read-heading", "ask", "read it");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "action": "read_range",
                "excerpt": "1|# RustClaw\n2|\n3|<img src=\"./RustClaw.png\" width=\"420\" />\n4|",
                "path": "README.md"
            })
            .to_string(),
        ));

        assert!(structurally_satisfies_answer_contract(
            &route, &journal, "RustClaw"
        ));
        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "# RustClaw"
        ));
    }

    #[test]
    fn existence_with_path_answer_grounded_by_existing_path_fact_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-exists", "ask", "check path");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "system_basic",
                json!({
                    "action": "path_batch_facts",
                    "facts": [{
                        "exists": true,
                        "path": "README.md",
                        "fact": {
                            "kind": "file",
                            "resolved_path": "/repo/README.md"
                        }
                    }]
                })
                .to_string(),
            ));

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "有，路径：/repo/README.md"
        ));
    }

    #[test]
    fn existence_with_path_answer_grounded_by_missing_path_fact_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-missing", "ask", "check path");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_1",
                "system_basic",
                json!({
                    "action": "path_batch_facts",
                    "facts": [{
                        "exists": false,
                        "path": "missing.txt",
                        "error": "not found"
                    }]
                })
                .to_string(),
            ));

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "未找到 `missing.txt`，请确认路径后再继续。"
        ));
    }

    #[test]
    fn existence_with_path_answer_ignores_doc_parse_path_facts() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        route.output_contract.requires_content_evidence = false;
        let mut journal = crate::task_journal::TaskJournal::for_task(
            "task-exists-doc-parse",
            "ask",
            "check path",
        );
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_parse",
                "doc_parse",
                json!({
                    "action": "parse_doc",
                    "path": "README.md",
                    "facts": [{
                        "exists": true,
                        "path": "README.md",
                        "fact": {
                            "kind": "file",
                            "resolved_path": "/repo/README.md"
                        }
                    }]
                })
                .to_string(),
            ));

        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "有，路径：/repo/README.md"
        ));
    }

    #[test]
    fn existence_with_path_answer_ignores_read_text_path_facts() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        route.output_contract.requires_content_evidence = false;
        let mut journal = crate::task_journal::TaskJournal::for_task(
            "task-exists-read-text",
            "ask",
            "check path",
        );
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace::ok(
                "step_read",
                "fs_basic",
                json!({
                    "action": "read_text_range",
                    "path": "README.md",
                    "facts": [{
                        "exists": true,
                        "path": "README.md",
                        "fact": {
                            "kind": "file",
                            "resolved_path": "/repo/README.md"
                        }
                    }]
                })
                .to_string(),
            ));

        assert!(!structurally_satisfies_answer_contract(
            &route,
            &journal,
            "有，路径：/repo/README.md"
        ));
    }

    #[test]
    fn exact_single_run_cmd_output_skips_llm_verifier_without_scalar_contract() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-run-cmd", "ask", "run it");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "run_cmd".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("line 1\nline 2\n".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_2".to_string(),
                skill: "synthesize_answer".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("line 1\nline 2".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "line 1\nline 2"
        ));
    }

    #[test]
    fn exact_run_cmd_output_skip_requires_single_external_step() {
        let route = route_with_mode(crate::AskMode::planner_execute_plain());
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-two-commands", "ask", "run both");
        for (idx, output) in ["first", "second"].into_iter().enumerate() {
            journal
                .step_results
                .push(crate::task_journal::TaskJournalStepTrace {
                    step_id: format!("step_{}", idx + 1),
                    skill: "run_cmd".to_string(),
                    status: crate::executor::StepExecutionStatus::Ok,
                    output_excerpt: Some(output.to_string()),
                    error_excerpt: None,
                    started_at: 0,
                    finished_at: 0,
                });
        }

        assert!(!structurally_satisfies_answer_contract(
            &route, &journal, "second"
        ));
    }

    #[test]
    fn free_shape_non_command_plain_observation_still_uses_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-free", "ask", "summarize output");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("ok".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(!structurally_satisfies_answer_contract(
            &route, &journal, "ok"
        ));
    }
}
