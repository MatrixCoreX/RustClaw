use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use super::*;

pub(super) fn route_requests_exact_name_list(route: &crate::IntentOutputContract) -> bool {
    route.requests_exact_name_list()
}

pub(super) fn route_requests_exact_list(route: &crate::IntentOutputContract) -> bool {
    route.requests_exact_list()
}

pub(super) fn selected_name_list_prefers_observed_projection(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    if !route_requests_exact_name_list(route)
        || route
            .selection
            .list_selector
            .sort_by
            .as_deref()
            .is_some_and(matrix_size_ranked_sort_token)
    {
        return false;
    }

    matrix_strict_list_observed_answer(route, loop_state).is_some()
}

fn matrix_size_ranked_sort_token(sort_by: &str) -> bool {
    matches!(sort_by.trim(), "size_desc" | "size_asc")
}

pub(super) fn matrix_observed_answer_candidate_for_shape(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    shape_class: crate::evidence_policy::FinalAnswerShapeClass,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract());
    match shape_class {
        crate::evidence_policy::FinalAnswerShapeClass::DeliveryArtifact => {
            direct_exact_scalar_path_from_dry_run_payload(loop_state, agent_run_context)
                .or_else(|| {
                    direct_file_token_from_observed_auto_locator_filename(
                        loop_state,
                        agent_run_context,
                    )
                })
                .or_else(|| {
                    direct_file_token_from_observed_find_entries(
                        state,
                        loop_state,
                        agent_run_context,
                    )
                })
                .or_else(|| {
                    direct_file_token_from_observed_inventory(loop_state, agent_run_context)
                })
        }
        crate::evidence_policy::FinalAnswerShapeClass::ScalarValue => {
            direct_scalar_observed_answer(Some(state), loop_state, agent_run_context)
                .or_else(|| {
                    direct_exact_scalar_path_from_dry_run_payload(loop_state, agent_run_context)
                })
                .or_else(|| {
                    direct_exact_scalar_path_from_written_path(loop_state, agent_run_context)
                })
                .or_else(|| {
                    direct_scalar_path_candidate_list_from_observed_outputs(
                        loop_state,
                        agent_run_context,
                    )
                })
        }
        crate::evidence_policy::FinalAnswerShapeClass::SinglePath => {
            direct_exact_scalar_path_from_dry_run_payload(loop_state, agent_run_context)
                .or_else(|| {
                    direct_exact_scalar_path_from_written_path(loop_state, agent_run_context)
                })
                .or_else(|| {
                    direct_scalar_path_candidate_list_from_observed_outputs(
                        loop_state,
                        agent_run_context,
                    )
                })
                .or_else(|| {
                    direct_scalar_observed_answer(Some(state), loop_state, agent_run_context)
                })
        }
        crate::evidence_policy::FinalAnswerShapeClass::StrictList => route
            .and_then(|route| matrix_strict_list_observed_answer(route, loop_state))
            .or_else(|| {
                direct_structured_observed_answer_allowing_implicit_metadata_path_facts(
                    Some(state),
                    loop_state,
                    agent_run_context,
                )
            }),
        crate::evidence_policy::FinalAnswerShapeClass::Freeform
        | crate::evidence_policy::FinalAnswerShapeClass::GroundedSummary
        | crate::evidence_policy::FinalAnswerShapeClass::Verdict => None,
    }
}

pub(in crate::finalize::loop_reply) fn matrix_strict_list_observed_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_supports_matrix_strict_list_observed_answer(route) {
        return None;
    }
    if let Some(answer) = matrix_ranked_size_list_observed_answer(route, loop_state) {
        return Some(answer);
    }
    if let Some(answer) = matrix_inventory_file_paths_observed_answer(route, loop_state) {
        return Some(answer);
    }
    let mut items = BTreeMap::<String, String>::new();
    for step in &loop_state.executed_step_results {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let output =
            crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                output,
            );
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&output) else {
            continue;
        };
        collect_matrix_strict_list_items(route, &value, &mut items);
    }
    if items.is_empty() {
        return None;
    }
    let mut values = items.into_values().collect::<Vec<_>>();
    if let Some(limit) = matrix_list_selector_limit(route) {
        values.truncate(limit.min(values.len()));
    }
    let answer = values.join("\n");
    Some((answer, matrix_observed_shape_summary(loop_state)))
}

pub(super) fn stale_file_token_delivery_listing_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_has_unresolved_file_token_delivery_contract(route)
        || !current_delivery_is_file_token(delivery_messages)
        || super::super::file_delivery::planned_file_delivery_uses_runtime_selection_template(
            loop_state,
        )
        || !latest_plan_requested_directory_inventory(loop_state)
    {
        return None;
    }
    observed_directory_listing_answer(loop_state)
        .map(|answer| (answer, matrix_observed_shape_summary(loop_state)))
}

pub(super) fn stale_file_token_delivery_bounded_read_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let contract = route.clone();
    if !route_has_file_token_delivery_signal(route)
        || !contract.requires_content_evidence
        || !current_delivery_is_file_token(delivery_messages)
        || super::super::file_delivery::planned_file_delivery_uses_runtime_selection_template(
            loop_state,
        )
        || latest_plan_has_direct_file_delivery_respond(loop_state)
    {
        return None;
    }
    latest_bounded_read_range_answer_from_loop(loop_state, false)
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
        .map(|answer| (answer, matrix_observed_shape_summary(loop_state)))
}

fn route_has_file_token_delivery_signal(route: &crate::IntentOutputContract) -> bool {
    let contract = route.clone();
    route.delivery_required
        || contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || matches!(
            contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
        )
}

fn route_has_unresolved_file_token_delivery_contract(route: &crate::IntentOutputContract) -> bool {
    let contract = route.clone();
    route_has_file_token_delivery_signal(route)
        && matches!(contract.locator_kind, crate::OutputLocatorKind::None)
        && contract.locator_hint.trim().is_empty()
}

fn current_delivery_is_file_token(delivery_messages: &[String]) -> bool {
    let answer = final_answer_text_from_delivery(delivery_messages);
    let first_line = answer.trim().lines().next().unwrap_or_default().trim();
    crate::finalize::parse_delivery_file_token(first_line).is_some()
}

fn latest_plan_has_direct_file_delivery_respond(loop_state: &LoopState) -> bool {
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|round| round.plan_result.as_ref())
        .any(|plan| {
            plan.steps.iter().any(|step| {
                if step.action_type != "respond" && step.skill != "respond" {
                    return false;
                }
                let Some(content) = step
                    .args
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|content| !content.is_empty())
                else {
                    return false;
                };
                content.lines().count() == 1
                    && crate::finalize::parse_delivery_file_token(content).is_some()
            })
        })
}

fn latest_plan_requested_directory_inventory(loop_state: &LoopState) -> bool {
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|round| round.plan_result.as_ref())
        .any(|plan| {
            plan.steps.iter().any(|step| {
                let action = step
                    .args
                    .get("action")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim);
                matches!(action, Some("list_dir" | "inventory_dir"))
                    || matches!(
                        step.skill.as_str(),
                        "filesystem.list_dir"
                            | "filesystem.list_entries"
                            | "fs_basic.list_dir"
                            | "system_basic.inventory_dir"
                    )
            })
        })
}

fn observed_directory_listing_answer(loop_state: &LoopState) -> Option<String> {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let output =
            crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                output,
            );
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&output) else {
            continue;
        };
        if let Some(answer) = directory_listing_answer_from_value(&value) {
            return Some(answer);
        }
    }
    None
}

fn directory_listing_answer_from_value(value: &serde_json::Value) -> Option<String> {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        if let Some(answer) = directory_listing_answer_from_value(extra) {
            return Some(answer);
        }
    }
    if !matches!(
        value.get("action").and_then(serde_json::Value::as_str),
        Some("inventory_dir" | "list_dir")
    ) {
        return None;
    }
    let mut items = observed_directory_listing_names(value);
    items.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    if items.len() < 2 {
        return None;
    }
    Some(items.join("\n"))
}

fn observed_directory_listing_names(value: &serde_json::Value) -> Vec<String> {
    if let Some(names) = value.get("names").and_then(serde_json::Value::as_array) {
        let items = names
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !items.is_empty() {
            return items;
        }
    }
    if let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
    {
        let mut items = Vec::new();
        for key in ["files", "dirs", "other"] {
            if let Some(array) = names_by_kind.get(key).and_then(serde_json::Value::as_array) {
                items.extend(
                    array
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(ToString::to_string),
                );
            }
        }
        if !items.is_empty() {
            return items;
        }
    }
    value
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            entry
                .get("name")
                .or_else(|| entry.get("path"))
                .or_else(|| entry.get("resolved_path"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| {
                    Path::new(name)
                        .file_name()
                        .and_then(|file_name| file_name.to_str())
                        .map(str::trim)
                        .filter(|file_name| !file_name.is_empty())
                        .unwrap_or(name)
                        .to_string()
                })
        })
        .collect()
}

pub(in crate::finalize::loop_reply) fn generic_observed_machine_projection_answer(
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let output =
            crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                output,
            );
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&output) else {
            continue;
        };
        let Some(answer) = generic_machine_projection_from_value(&value) else {
            continue;
        };
        let answer = answer.trim().to_string();
        if !answer.is_empty() {
            return Some((answer, matrix_observed_shape_summary(loop_state)));
        }
    }
    None
}

fn generic_machine_projection_from_value(value: &serde_json::Value) -> Option<String> {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        if let Some(answer) = generic_machine_projection_from_value(extra) {
            return Some(answer);
        }
    }
    match value.get("action").and_then(serde_json::Value::as_str) {
        Some("inventory_dir" | "list_dir") => directory_listing_answer_from_value(value),
        Some("grep_text") => grep_text_matches_answer_from_value(value),
        _ => None,
    }
}

fn grep_text_matches_answer_from_value(value: &serde_json::Value) -> Option<String> {
    let matches = value.get("matches").and_then(serde_json::Value::as_array)?;
    let mut lines = Vec::new();
    for item in matches {
        let text = item
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())?;
        let line = item.get("line").and_then(serde_json::Value::as_u64);
        let projected = line
            .map(|line| format!("{line}:{text}"))
            .unwrap_or_else(|| text.to_string());
        if !lines.iter().any(|existing| existing == &projected) {
            lines.push(projected);
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn route_supports_matrix_strict_list_observed_answer(route: &crate::IntentOutputContract) -> bool {
    route_requests_exact_list(route)
}

fn route_requests_name_list(route: &crate::IntentOutputContract) -> bool {
    route_requests_exact_name_list(route)
}

fn matrix_list_selector_limit(route: &crate::IntentOutputContract) -> Option<usize> {
    route
        .selection
        .list_selector
        .limit
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn matrix_inventory_file_paths_observed_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route.requests_exact_path_list() {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let output =
            crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                output,
            );
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&output) else {
            continue;
        };
        let Some(mut paths) = inventory_file_paths_from_value(&value) else {
            continue;
        };
        if let Some(limit) = matrix_list_selector_limit(route) {
            paths.truncate(limit.min(paths.len()));
        }
        if paths.is_empty() {
            continue;
        }
        return Some((paths.join("\n"), matrix_observed_shape_summary(loop_state)));
    }
    None
}

fn inventory_file_paths_from_value(value: &serde_json::Value) -> Option<Vec<String>> {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        if let Some(paths) = inventory_file_paths_from_value(extra) {
            return Some(paths);
        }
    }
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir") {
        return None;
    }
    let entries = value.get("entries").and_then(serde_json::Value::as_array)?;
    let mut seen = HashSet::<String>::new();
    let mut paths = Vec::<String>::new();
    for entry in entries {
        let Some(map) = entry.as_object() else {
            continue;
        };
        let kind = map
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .unwrap_or("file");
        if !matches!(kind, "file" | "") {
            continue;
        }
        let Some(path) = map
            .get("path")
            .or_else(|| map.get("resolved_path"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        let key = path.replace('\\', "/").to_ascii_lowercase();
        if seen.insert(key) {
            paths.push(path.to_string());
        }
    }
    (!paths.is_empty()).then_some(paths)
}

fn matrix_ranked_size_list_observed_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requests_file_name_list(route) {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let output =
            crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                output,
            );
        if let Some(answer) = inventory_ranked_size_list_answer(&output, route) {
            return Some((answer, matrix_observed_shape_summary(loop_state)));
        }
    }
    None
}

fn collect_matrix_strict_list_items(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        collect_matrix_strict_list_items(route, extra, items);
    }
    if route_requests_directory_name_list(route) {
        collect_matrix_directory_name_items(route, value, items);
        return;
    }
    if route_requests_file_name_list(route) {
        collect_matrix_file_name_items(route, value, items);
        return;
    }
    push_matrix_string_arrays(
        route,
        value,
        items,
        &[
            "keys",
            "identity_values",
            "names",
            "name_results",
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
        .and_then(serde_json::Value::as_object)
    {
        for child in names_by_kind.values() {
            push_matrix_array_items(route, child, items);
        }
    }
    for key in ["entries", "items", "facts", "rows"] {
        if let Some(rows) = value.get(key).and_then(serde_json::Value::as_array) {
            for row in rows {
                collect_matrix_list_object_fields(route, row, items);
            }
        }
    }
}

fn route_requests_file_name_list(route: &crate::IntentOutputContract) -> bool {
    route.selection.list_selector.target_kind_specified
        && route.selection.list_selector.target_kind == crate::OutputScalarCountTargetKind::File
}

fn route_requests_directory_name_list(route: &crate::IntentOutputContract) -> bool {
    route.selection.list_selector.target_kind_specified
        && route.selection.list_selector.target_kind == crate::OutputScalarCountTargetKind::Dir
}

fn collect_matrix_file_name_items(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    if let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
    {
        push_matrix_array_items(
            route,
            names_by_kind
                .get("files")
                .unwrap_or(&serde_json::Value::Null),
            items,
        );
    }
    push_matrix_string_arrays(route, value, items, &["files"]);
    push_matrix_string_arrays(route, value, items, &["names", "results", "paths"]);
    for key in ["entries", "items", "rows"] {
        let Some(rows) = value.get(key).and_then(serde_json::Value::as_array) else {
            continue;
        };
        for row in rows {
            collect_matrix_file_name_object(route, row, items);
        }
    }
}

fn collect_matrix_file_name_object(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    let Some(map) = value.as_object() else {
        return;
    };
    let kind = map
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(kind, "file" | "") {
        return;
    }
    for key in ["name", "path", "resolved_path"] {
        if let Some(text) = map.get(key).and_then(serde_json::Value::as_str) {
            push_matrix_list_item(route, text, items);
            return;
        }
    }
}

fn collect_matrix_directory_name_items(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    if let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
    {
        push_matrix_array_items(
            route,
            names_by_kind
                .get("dirs")
                .unwrap_or(&serde_json::Value::Null),
            items,
        );
    }
    push_matrix_string_arrays(route, value, items, &["dirs", "directories"]);
    if value
        .get("dirs_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        push_matrix_string_arrays(route, value, items, &["names"]);
    }
    for key in ["entries", "items", "rows"] {
        let Some(rows) = value.get(key).and_then(serde_json::Value::as_array) else {
            continue;
        };
        for row in rows {
            collect_matrix_directory_name_object(route, row, items);
        }
    }
}

fn collect_matrix_directory_name_object(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    let Some(map) = value.as_object() else {
        return;
    };
    let kind = map
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(kind, "dir" | "directory") {
        return;
    }
    for key in ["name", "path", "resolved_path"] {
        if let Some(text) = map.get(key).and_then(serde_json::Value::as_str) {
            push_matrix_list_item(route, text, items);
            return;
        }
    }
}

fn push_matrix_string_arrays(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
    keys: &[&str],
) {
    for key in keys {
        if let Some(child) = value.get(*key) {
            push_matrix_array_items(route, child, items);
        }
    }
}

fn push_matrix_array_items(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    let Some(array) = value.as_array() else {
        return;
    };
    for item in array {
        if let Some(text) = item.as_str() {
            push_matrix_list_item(route, text, items);
        } else {
            collect_matrix_list_object_fields(route, item, items);
        }
    }
}

fn collect_matrix_list_object_fields(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    let Some(map) = value.as_object() else {
        return;
    };
    for key in [
        "name",
        "path",
        "resolved_path",
        "table",
        "table_name",
        "identity_value",
    ] {
        if let Some(text) = map.get(key).and_then(serde_json::Value::as_str) {
            push_matrix_list_item(route, text, items);
        }
    }
}

fn push_matrix_list_item(
    route: &crate::IntentOutputContract,
    raw: &str,
    items: &mut BTreeMap<String, String>,
) {
    let Some(display) = matrix_list_display_item(route, raw) else {
        return;
    };
    items.entry(display.to_ascii_lowercase()).or_insert(display);
}

fn matrix_list_display_item(route: &crate::IntentOutputContract, raw: &str) -> Option<String> {
    let item = raw.trim().trim_matches('`').trim();
    if item.is_empty() {
        return None;
    }
    if route_requests_name_list(route) {
        return std::path::Path::new(item)
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .or_else(|| Some(item.to_string()));
    }
    Some(item.to_string())
}
