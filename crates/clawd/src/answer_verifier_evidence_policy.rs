use super::*;

pub(super) fn evidence_policy_strict_list_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate_items = strict_list_answer_items(candidate_answer);
    if candidate_items.is_empty() {
        return false;
    }
    if let Some(limit) = strict_list_selector_limit(route) {
        if candidate_items.len() > limit {
            return false;
        }
    }
    let observed_items = observed_strict_list_items(route, journal);
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

fn strict_list_selector_limit(route: &AnswerContract) -> Option<usize> {
    route
        .output_contract
        .selection
        .list_selector
        .limit
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

pub(super) fn strict_list_route_allows_observed_subset(route: &AnswerContract) -> bool {
    route.output_contract.requests_exact_list()
}

pub(super) fn evidence_policy_single_path_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if let Some(candidate_path) = strict_single_path_answer(candidate_answer) {
        return observed_single_path_values(route, journal)
            .iter()
            .any(|observed_path| single_path_matches_observed(&candidate_path, observed_path));
    }
    false
}

pub(super) fn evidence_policy_delivery_artifact_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    route_requires_single_file_delivery(route)
        && (candidate_answer_has_grounded_existing_file_token(journal, candidate_answer)
            || candidate_answer_has_grounded_existing_plain_path(journal, candidate_answer))
}

pub(super) fn candidate_answer_has_grounded_existing_plain_path(
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

pub(super) fn strict_single_path_answer(answer: &str) -> Option<String> {
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

pub(super) fn observed_single_path_values(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut paths = observed_single_path_values_from_evidence_map_for_route(route, journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
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

pub(super) fn collect_single_path_values_from_json(
    value: &serde_json::Value,
    paths: &mut BTreeSet<String>,
) {
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

pub(super) fn collect_joined_path_values_from_json_object(
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

pub(super) fn joined_result_already_contains_root(root: &str, child: &str) -> bool {
    let root = root.trim().trim_matches('/');
    if root.is_empty() || root == "." {
        return true;
    }
    let child = child.trim().trim_start_matches("./");
    child == root || child.starts_with(&format!("{root}/"))
}

pub(super) fn single_path_evidence_key(key: &str) -> bool {
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

pub(super) fn single_path_matches_observed(candidate_path: &str, observed_path: &str) -> bool {
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

pub(super) fn strict_list_answer_items(answer: &str) -> Vec<String> {
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

pub(super) fn strip_list_marker(raw: &str) -> String {
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

pub(super) fn strict_list_item_variants_for_route(
    _route: &AnswerContract,
    item: &str,
    _observed_item: bool,
) -> Vec<String> {
    let mut variants = strict_list_item_variants(item);
    variants.sort();
    variants.dedup();
    variants
}

pub(super) fn strict_list_item_variants(item: &str) -> Vec<String> {
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

pub(super) fn strict_list_candidate_annotates_observed_item(
    candidate: &str,
    observed: &str,
) -> bool {
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

pub(super) fn normalize_strict_list_item(item: &str) -> String {
    item.trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_ascii_lowercase()
}

pub(super) fn observed_strict_list_items(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    let mut items = observed_strict_list_items_from_evidence_map_for_route(route, journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
            continue;
        }
        if !step_can_supply_strict_evidence_for_route(route, step) {
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

pub(super) fn collect_observed_strict_list_items_from_value(
    value: &serde_json::Value,
    items: &mut BTreeSet<String>,
) {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        collect_observed_strict_list_items_from_value(extra, items);
    }
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

pub(super) fn push_string_array_values(
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

pub(super) fn push_array_strings(value: &serde_json::Value, items: &mut BTreeSet<String>) {
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

pub(super) fn collect_observed_list_item_object_fields(
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

pub(super) fn observed_name_size_item(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
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

pub(super) fn push_observed_list_item(text: &str, items: &mut BTreeSet<String>) {
    let item = normalize_strict_list_item(text);
    if !item.is_empty() {
        items.insert(item);
    }
}
