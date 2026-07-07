use std::collections::{BTreeMap, BTreeSet, HashSet};

use tracing::info;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    build_loop_journal, direct_created_archive_path_from_observed_archive_pack,
    direct_file_token_from_observed_auto_locator_filename,
    direct_file_token_from_observed_find_entries, direct_file_token_from_observed_inventory,
    direct_generated_file_path_report_from_dry_run_payload,
    direct_generated_file_path_report_from_written_path, direct_path_from_active_bound_inventory,
    direct_scalar_observed_answer, direct_scalar_path_candidate_list_from_observed_outputs,
    direct_structured_observed_answer_allowing_implicit_metadata_path_facts,
    directory_entry_groups_prefers_observed_groups, final_answer_text_from_delivery,
    inventory_ranked_size_list_answer, latest_grounded_synthesis_for_mixed_listing_contract,
    latest_plan_requested_synthesis, log_deterministic_delivery_record,
    looks_like_structured_machine_output,
    successful_content_observation_should_precede_status_summary,
};

fn evidence_policy_final_answer_shape_class(
    route: &crate::RouteResult,
) -> Option<crate::evidence_policy::FinalAnswerShapeClass> {
    if route_requests_docker_text_list_projection(route) {
        return Some(crate::evidence_policy::FinalAnswerShapeClass::StrictList);
    }
    crate::evidence_policy::final_answer_shape_for_route(route).map(|shape| shape.class())
}

pub(super) fn route_requires_evidence_policy_deterministic_final_answer(
    route: &crate::RouteResult,
) -> bool {
    evidence_policy_final_answer_shape_class(route)
        .is_some_and(|class| !class.allows_model_language())
}

pub(super) fn agent_context_allows_observed_output_language_fallback(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_none_or(|route| !route_requires_evidence_policy_deterministic_final_answer(route))
}

pub(super) fn should_try_observed_output_language_fallback(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(crate::agent_engine::observed_output::route_requires_synthesized_delivery)
        || agent_context_allows_observed_output_language_fallback(agent_run_context)
        || latest_plan_requested_synthesis(loop_state)
        || successful_content_observation_should_precede_status_summary(
            agent_run_context,
            loop_state,
        )
}

#[cfg(test)]
pub(super) fn route_has_evidence_policy_final_shape(route: &crate::RouteResult) -> bool {
    evidence_policy_final_answer_shape_class(route).is_some()
}

pub(super) fn route_requires_observed_semantic_projection(route: &crate::RouteResult) -> bool {
    matches!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::QuantityComparison
            | crate::OutputSemanticKind::ServiceStatus
    )
}

pub(super) fn evidence_policy_candidate_satisfies_final_shape(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    route: &crate::RouteResult,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    if route_requests_docker_text_list_projection(route)
        && docker_text_list_candidate_is_observed(route, loop_state, candidate)
    {
        return true;
    }
    let delivery_messages = vec![candidate.to_string()];
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        crate::task_journal::delivery_payload_consistent(candidate, &delivery_messages),
        candidate,
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    crate::answer_verifier::structurally_satisfies_answer_contract(route, &journal, candidate)
}

pub(super) fn synthetic_task_for_evidence_policy_shape_check(task_id: &str) -> ClaimedTask {
    ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 0,
        chat_id: 0,
        user_key: None,
        channel: "finalize".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

pub(super) fn current_synthesis_satisfies_evidence_policy_shape(
    task_id: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route_requires_evidence_policy_deterministic_final_answer(route) {
        return true;
    }
    let Some(message) = delivery_messages.last() else {
        return false;
    };
    if directory_entry_groups_prefers_observed_groups(route, loop_state) {
        return false;
    }
    if archive_member_list_prefers_observed_projection(route) {
        return false;
    }
    if file_name_list_prefers_observed_projection(route, loop_state) {
        return false;
    }
    let task = synthetic_task_for_evidence_policy_shape_check(task_id);
    evidence_policy_candidate_satisfies_final_shape(
        &task,
        &route.resolved_intent,
        loop_state,
        agent_run_context,
        finalizer_summary,
        route,
        message,
    )
}

pub(super) fn archive_member_list_prefers_observed_projection(route: &crate::RouteResult) -> bool {
    route_requests_archive_list(route)
        && (crate::evidence_policy::final_answer_shape_for_route(route)
            == Some(crate::evidence_policy::FinalAnswerShape::ArchiveMemberList)
            || route.output_contract.response_shape == crate::OutputResponseShape::Strict)
}

fn route_requests_archive_list(route: &crate::RouteResult) -> bool {
    route.output_contract_marker_is(crate::OutputSemanticKind::ArchiveList)
        || crate::evidence_policy::final_answer_shape_for_route(route)
            == Some(crate::evidence_policy::FinalAnswerShape::ArchiveMemberList)
}

pub(super) fn file_name_list_prefers_observed_projection(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::FileNames)
        || route.output_contract.response_shape != crate::OutputResponseShape::Strict
        || route
            .output_contract
            .self_extension
            .list_selector
            .sort_by
            .as_deref()
            .is_some_and(matrix_size_ranked_sort_token)
    {
        return false;
    }

    loop_state.executed_step_results.iter().any(|step| {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        {
            return false;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return false;
        };
        let output =
            crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                output,
            );
        serde_json::from_str::<serde_json::Value>(&output)
            .ok()
            .is_some_and(|value| value_contains_observed_file_name_list(&value))
    })
}

fn value_contains_observed_file_name_list(value: &serde_json::Value) -> bool {
    if value
        .get("sort_by")
        .and_then(serde_json::Value::as_str)
        .is_some_and(matrix_size_ranked_sort_token)
    {
        return false;
    }
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        if value_contains_observed_file_name_list(extra) {
            return true;
        }
    }
    value_string_array_has_items(value, &["names", "results", "files", "paths"])
        || value
            .pointer("/names_by_kind/files")
            .is_some_and(json_array_has_string_item)
        || value
            .get("entries")
            .is_some_and(value_entries_include_file_name)
}

fn value_string_array_has_items(value: &serde_json::Value, keys: &[&str]) -> bool {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .any(json_array_has_string_item)
}

fn value_entries_include_file_name(value: &serde_json::Value) -> bool {
    let Some(entries) = value.as_array() else {
        return false;
    };
    entries.iter().any(|entry| {
        let Some(map) = entry.as_object() else {
            return false;
        };
        if map
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| kind != "file")
        {
            return false;
        }
        ["name", "path", "resolved_path"].iter().any(|key| {
            map.get(*key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .is_some_and(|text| !text.is_empty())
        })
    })
}

fn json_array_has_string_item(value: &serde_json::Value) -> bool {
    value.as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item.as_str()
                .map(str::trim)
                .is_some_and(|text| !text.is_empty())
        })
    })
}

fn matrix_size_ranked_sort_token(sort_by: &str) -> bool {
    matches!(sort_by.trim(), "size_desc" | "size_asc")
}

fn matrix_observed_answer_candidate_for_shape(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    shape_class: crate::evidence_policy::FinalAnswerShapeClass,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    match shape_class {
        crate::evidence_policy::FinalAnswerShapeClass::DeliveryArtifact => {
            direct_file_token_from_observed_auto_locator_filename(loop_state, agent_run_context)
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
        crate::evidence_policy::FinalAnswerShapeClass::ScalarValue
        | crate::evidence_policy::FinalAnswerShapeClass::SinglePath => {
            direct_generated_file_path_report_from_dry_run_payload(loop_state, agent_run_context)
                .or_else(|| {
                    direct_generated_file_path_report_from_written_path(
                        loop_state,
                        agent_run_context,
                    )
                })
                .or_else(|| {
                    direct_created_archive_path_from_observed_archive_pack(
                        loop_state,
                        agent_run_context,
                    )
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
            .and_then(|route| {
                matrix_grouped_name_list_observed_answer(route, loop_state)
                    .or_else(|| matrix_docker_text_list_observed_answer(route, loop_state))
                    .or_else(|| matrix_strict_list_observed_answer(route, loop_state))
            })
            .or_else(|| {
                direct_structured_observed_answer_allowing_implicit_metadata_path_facts(
                    Some(state),
                    loop_state,
                    agent_run_context,
                )
            }),
        crate::evidence_policy::FinalAnswerShapeClass::Table => route
            .and_then(|route| matrix_table_observed_answer(route, loop_state))
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

pub(super) fn matrix_strict_list_observed_answer(
    route: &crate::RouteResult,
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
        if route.output_contract_marker_is(crate::OutputSemanticKind::HiddenEntriesCheck) {
            collect_matrix_hidden_entries(&value, &mut items);
        } else {
            collect_matrix_strict_list_items(route, &value, &mut items);
        }
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

fn route_supports_matrix_strict_list_observed_answer(route: &crate::RouteResult) -> bool {
    route_requests_archive_list(route)
        || route_requests_filesystem_path_list(route)
        || matches!(
            route.effective_output_contract_semantic_kind(),
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::HiddenEntriesCheck
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::StructuredKeys
                | crate::OutputSemanticKind::SqliteTableNamesOnly
        )
}

fn route_requests_filesystem_path_list(route: &crate::RouteResult) -> bool {
    crate::evidence_policy::final_answer_shape_for_route(route)
        == Some(crate::evidence_policy::FinalAnswerShape::PathList)
}

fn matrix_list_selector_limit(route: &crate::RouteResult) -> Option<usize> {
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn matrix_inventory_file_paths_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths) {
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
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::FileNames) {
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

fn collect_matrix_hidden_entries(value: &serde_json::Value, items: &mut BTreeMap<String, String>) {
    if let Some(entries) = value.get("entries").and_then(serde_json::Value::as_array) {
        for entry in entries {
            let Some(map) = entry.as_object() else {
                continue;
            };
            let hidden = map
                .get("hidden")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if !hidden {
                continue;
            }
            for key in ["name", "path"] {
                if let Some(text) = map.get(key).and_then(serde_json::Value::as_str) {
                    push_matrix_hidden_entry_item(text, items);
                    break;
                }
            }
        }
    }
    if let Some(names) = value.get("names").and_then(serde_json::Value::as_array) {
        for name in names {
            if let Some(text) = name.as_str() {
                push_matrix_hidden_entry_item(text, items);
            }
        }
    }
    if let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
    {
        for child in names_by_kind.values() {
            if let Some(array) = child.as_array() {
                for name in array {
                    if let Some(text) = name.as_str() {
                        push_matrix_hidden_entry_item(text, items);
                    }
                }
            }
        }
    }
}

fn push_matrix_hidden_entry_item(raw: &str, items: &mut BTreeMap<String, String>) {
    let item = raw.trim().trim_matches('`').trim();
    if item.is_empty() || item == "." || item == ".." {
        return;
    }
    let display = std::path::Path::new(item)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(item);
    if !display.starts_with('.') || matches!(display, "." | "..") {
        return;
    }
    items
        .entry(display.to_ascii_lowercase())
        .or_insert_with(|| display.to_string());
}

pub(super) fn matrix_grouped_name_list_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !crate::finalize::route_prefers_grouped_name_list_output(route) {
        return None;
    }
    let mut dirs = BTreeMap::<String, String>::new();
    let mut files = BTreeMap::<String, String>::new();
    let mut other = BTreeMap::<String, String>::new();
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
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        if let Some(answer) = ordered_matrix_grouped_name_list_from_value(route, &value) {
            return Some((answer, matrix_observed_shape_summary(loop_state)));
        }
        collect_matrix_grouped_name_items(route, &value, &mut dirs, &mut files, &mut other);
    }
    if dirs.is_empty() && files.is_empty() && other.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    push_matrix_grouped_name_lines("dirs", dirs, &mut lines);
    push_matrix_grouped_name_lines("files", files, &mut lines);
    push_matrix_grouped_name_lines("other", other, &mut lines);
    Some((lines.join("\n"), matrix_observed_shape_summary(loop_state)))
}

fn ordered_matrix_grouped_name_list_from_value(
    route: &crate::RouteResult,
    value: &serde_json::Value,
) -> Option<String> {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        if let Some(answer) = ordered_matrix_grouped_name_list_from_value(route, extra) {
            return Some(answer);
        }
    }
    let sort_by = value
        .get("sort_by")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|sort_by| !sort_by.is_empty())?;
    if sort_by == "name"
        && route
            .output_contract
            .self_extension
            .list_selector
            .sort_by
            .is_none()
    {
        return None;
    }
    let names_by_kind = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)?;
    let mut lines = Vec::new();
    push_ordered_matrix_grouped_name_lines(route, "dirs", names_by_kind.get("dirs"), &mut lines);
    push_ordered_matrix_grouped_name_lines(route, "files", names_by_kind.get("files"), &mut lines);
    push_ordered_matrix_grouped_name_lines(route, "other", names_by_kind.get("other"), &mut lines);
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn push_ordered_matrix_grouped_name_lines(
    route: &crate::RouteResult,
    title: &str,
    value: Option<&serde_json::Value>,
    lines: &mut Vec<String>,
) {
    let Some(array) = value.and_then(serde_json::Value::as_array) else {
        return;
    };
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for item in array {
        let Some(raw) = item.as_str() else {
            continue;
        };
        let Some(display) = matrix_list_display_item(route, raw) else {
            continue;
        };
        if seen.insert(display.to_ascii_lowercase()) {
            items.push(display);
        }
    }
    if items.is_empty() {
        return;
    }
    lines.push(format!("{title}:"));
    lines.extend(items.into_iter().map(|item| format!("- {item}")));
}

fn matrix_docker_text_list_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requests_docker_text_list_projection(route) {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "docker_basic" | "run_cmd") {
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
        if looks_like_structured_machine_output(output)
            || crate::finalize::looks_like_planner_artifact(output)
            || crate::finalize::looks_like_internal_trace_artifact(output)
        {
            continue;
        }
        return Some((
            output.to_string(),
            matrix_observed_shape_summary(loop_state),
        ));
    }
    None
}

fn docker_text_list_candidate_is_observed(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    candidate: &str,
) -> bool {
    if !route_requests_docker_text_list_projection(route) {
        return false;
    }
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    loop_state.executed_step_results.iter().rev().any(|step| {
        step.is_ok()
            && matches!(step.skill.as_str(), "docker_basic" | "run_cmd")
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| output == candidate)
    })
}

fn route_requests_docker_text_list_projection(route: &crate::RouteResult) -> bool {
    matches!(
        crate::evidence_policy::final_answer_shape_for_route(route),
        Some(
            crate::evidence_policy::FinalAnswerShape::ContainerList
                | crate::evidence_policy::FinalAnswerShape::ImageList
        )
    )
}

fn collect_matrix_grouped_name_items(
    route: &crate::RouteResult,
    value: &serde_json::Value,
    dirs: &mut BTreeMap<String, String>,
    files: &mut BTreeMap<String, String>,
    other: &mut BTreeMap<String, String>,
) {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        collect_matrix_grouped_name_items(route, extra, dirs, files, other);
    }
    if let Some(names_by_kind) = value
        .get("names_by_kind")
        .and_then(serde_json::Value::as_object)
    {
        push_matrix_grouped_name_array(route, names_by_kind.get("dirs"), dirs);
        push_matrix_grouped_name_array(route, names_by_kind.get("files"), files);
        push_matrix_grouped_name_array(route, names_by_kind.get("other"), other);
    }
}

fn push_matrix_grouped_name_array(
    route: &crate::RouteResult,
    value: Option<&serde_json::Value>,
    items: &mut BTreeMap<String, String>,
) {
    let Some(array) = value.and_then(serde_json::Value::as_array) else {
        return;
    };
    for item in array {
        if let Some(text) = item.as_str() {
            push_matrix_grouped_name_item(route, text, items);
        }
    }
}

fn push_matrix_grouped_name_item(
    route: &crate::RouteResult,
    raw: &str,
    items: &mut BTreeMap<String, String>,
) {
    let Some(display) = matrix_list_display_item(route, raw) else {
        return;
    };
    items.entry(display.to_ascii_lowercase()).or_insert(display);
}

fn push_matrix_grouped_name_lines(
    label: &str,
    items: BTreeMap<String, String>,
    lines: &mut Vec<String>,
) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{label}:"));
    lines.extend(items.into_values().map(|item| format!("- {item}")));
}

fn collect_matrix_strict_list_items(
    route: &crate::RouteResult,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        collect_matrix_strict_list_items(route, extra, items);
    }
    if route_requests_archive_list(route) {
        collect_matrix_archive_member_items(route, value, items);
        return;
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::DirectoryNames) {
        collect_matrix_directory_name_items(route, value, items);
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

fn collect_matrix_directory_name_items(
    route: &crate::RouteResult,
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
    route: &crate::RouteResult,
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

fn collect_matrix_archive_member_items(
    route: &crate::RouteResult,
    value: &serde_json::Value,
    items: &mut BTreeMap<String, String>,
) {
    let archive_hint = value
        .get("archive")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|archive| !archive.is_empty());
    for key in ["entries", "candidates"] {
        let Some(array) = value.get(key).and_then(serde_json::Value::as_array) else {
            continue;
        };
        for item in array {
            match item {
                serde_json::Value::String(raw) => {
                    push_matrix_archive_member_item(route, archive_hint, raw, None, items);
                }
                serde_json::Value::Object(map) => {
                    let kind = map.get("kind").and_then(serde_json::Value::as_str);
                    let raw = map
                        .get("name")
                        .or_else(|| map.get("path"))
                        .and_then(serde_json::Value::as_str);
                    if let Some(raw) = raw {
                        push_matrix_archive_member_item(route, archive_hint, raw, kind, items);
                    }
                }
                _ => {}
            }
        }
    }
}

fn push_matrix_archive_member_item(
    route: &crate::RouteResult,
    archive_hint: Option<&str>,
    raw: &str,
    kind: Option<&str>,
    items: &mut BTreeMap<String, String>,
) {
    if !archive_member_matches_list_selector(route, raw, kind) {
        return;
    }
    let Some(display) = matrix_archive_member_display_item(raw, archive_hint) else {
        return;
    };
    items.entry(display.to_ascii_lowercase()).or_insert(display);
}

fn archive_member_matches_list_selector(
    route: &crate::RouteResult,
    raw: &str,
    kind: Option<&str>,
) -> bool {
    let selector = &route.output_contract.self_extension.list_selector;
    let target_kind = if selector.target_kind == crate::OutputScalarCountTargetKind::Any
        && !selector.target_kind_specified
    {
        crate::OutputScalarCountTargetKind::File
    } else {
        selector.target_kind
    };
    match target_kind {
        crate::OutputScalarCountTargetKind::Any => true,
        crate::OutputScalarCountTargetKind::File => kind
            .map(|kind| kind.trim().eq_ignore_ascii_case("file"))
            .unwrap_or_else(|| !raw.trim().ends_with('/')),
        crate::OutputScalarCountTargetKind::Dir => kind
            .map(|kind| kind.trim().eq_ignore_ascii_case("dir"))
            .unwrap_or_else(|| raw.trim().ends_with('/')),
    }
}

fn matrix_archive_member_display_item(raw: &str, archive_hint: Option<&str>) -> Option<String> {
    let item = raw.trim().trim_matches('`').trim().replace('\\', "/");
    if item.is_empty() {
        return None;
    }
    let item_no_leading = item.trim_start_matches('/');
    if let Some(parent_prefix) = archive_hint.and_then(archive_parent_prefix_for_member_display) {
        if let Some(rest) = item_no_leading.strip_prefix(&parent_prefix) {
            let rest = rest.trim_start_matches('/');
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    Some(item)
}

fn archive_parent_prefix_for_member_display(archive_hint: &str) -> Option<String> {
    let parent = std::path::Path::new(archive_hint.trim()).parent()?;
    let prefix = parent
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string();
    (!prefix.is_empty()).then_some(prefix)
}

fn push_matrix_string_arrays(
    route: &crate::RouteResult,
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
    route: &crate::RouteResult,
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
    route: &crate::RouteResult,
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
    route: &crate::RouteResult,
    raw: &str,
    items: &mut BTreeMap<String, String>,
) {
    let Some(display) = matrix_list_display_item(route, raw) else {
        return;
    };
    items.entry(display.to_ascii_lowercase()).or_insert(display);
}

fn matrix_list_display_item(route: &crate::RouteResult, raw: &str) -> Option<String> {
    let item = raw.trim().trim_matches('`').trim();
    if item.is_empty() {
        return None;
    }
    if matches!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::FileNames | crate::OutputSemanticKind::DirectoryNames
    ) {
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

fn matrix_table_observed_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requests_table_listing(route) {
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
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        if let Some(answer) = matrix_markdown_table_from_json(&value) {
            return Some((answer, matrix_observed_shape_summary(loop_state)));
        }
    }
    None
}

fn route_requests_table_listing(route: &crate::RouteResult) -> bool {
    crate::evidence_policy::final_answer_shape_for_route(route)
        == Some(crate::evidence_policy::FinalAnswerShape::TableListing)
        || route.output_contract_marker_is(crate::OutputSemanticKind::SqliteTableListing)
}

fn matrix_markdown_table_from_json(value: &serde_json::Value) -> Option<String> {
    let rows = value
        .get("rows")
        .or_else(|| value.pointer("/result/rows"))?
        .as_array()?;
    if rows.is_empty() {
        return None;
    }
    let columns = matrix_table_columns(value, rows)?;
    let mut table = String::new();
    table.push('|');
    for column in &columns {
        table.push(' ');
        table.push_str(column);
        table.push_str(" |");
    }
    table.push('\n');
    table.push('|');
    for _ in &columns {
        table.push_str(" --- |");
    }
    for row in rows {
        let cells = matrix_table_row_cells(row, &columns)?;
        table.push('\n');
        table.push('|');
        for cell in cells {
            table.push(' ');
            table.push_str(&cell);
            table.push_str(" |");
        }
    }
    Some(table)
}

fn matrix_table_columns(
    value: &serde_json::Value,
    rows: &[serde_json::Value],
) -> Option<Vec<String>> {
    let mut columns = value
        .get("columns")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for row in rows {
        if let Some(map) = row.as_object() {
            for key in map.keys() {
                if !columns.iter().any(|column| column == key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    (!columns.is_empty()).then_some(columns)
}

fn matrix_table_row_cells(row: &serde_json::Value, columns: &[String]) -> Option<Vec<String>> {
    match row {
        serde_json::Value::Object(map) => {
            let mut cells = Vec::new();
            for column in columns {
                let cell = map
                    .get(column)
                    .and_then(matrix_table_cell_text)
                    .unwrap_or_default();
                if cell.contains(['\n', '|']) {
                    return None;
                }
                cells.push(cell);
            }
            Some(cells)
        }
        serde_json::Value::Array(values) => values
            .iter()
            .map(matrix_table_cell_text)
            .collect::<Option<Vec<_>>>(),
        value => matrix_table_cell_text(value).map(|cell| vec![cell]),
    }
}

fn matrix_table_cell_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Null => Some(String::new()),
        _ => None,
    }
}

pub(super) fn matrix_observed_shape_summary(
    loop_state: &LoopState,
) -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    }
}

pub(super) fn replace_delivery_with_matrix_observed_shape_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if loop_state.pending_user_input_required {
        return false;
    }
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_requires_evidence_policy_deterministic_final_answer(route) {
        return false;
    }
    if let Some((candidate, summary)) =
        direct_path_from_active_bound_inventory(loop_state, agent_run_context)
    {
        let answer = candidate.trim().to_string();
        if answer.is_empty() {
            return false;
        }
        if final_answer_text_from_delivery(delivery_messages).trim() == answer {
            *finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer);
            return true;
        }
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        log_deterministic_delivery_record(
            &task.task_id,
            "matrix_replace_active_bound_inventory_path",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    let Some(shape_class) = evidence_policy_final_answer_shape_class(route) else {
        return false;
    };
    let current_answer = final_answer_text_from_delivery(delivery_messages);
    if !current_answer.trim().is_empty()
        && !directory_entry_groups_prefers_observed_groups(route, loop_state)
        && !archive_member_list_prefers_observed_projection(route)
        && !file_name_list_prefers_observed_projection(route, loop_state)
        && evidence_policy_candidate_satisfies_final_shape(
            task,
            user_text,
            loop_state,
            agent_run_context,
            finalizer_summary.clone(),
            route,
            &current_answer,
        )
    {
        return false;
    }
    if let Some((answer, summary)) =
        latest_grounded_synthesis_for_mixed_listing_contract(route, loop_state)
    {
        let answer = answer.trim().to_string();
        if !answer.is_empty() && current_answer.trim() == answer {
            loop_state.last_user_visible_respond = Some(answer);
            *finalizer_summary = Some(summary);
            return true;
        }
    }

    let Some((candidate, summary)) = matrix_observed_answer_candidate_for_shape(
        state,
        loop_state,
        agent_run_context,
        shape_class,
    ) else {
        return false;
    };
    if !archive_member_list_prefers_observed_projection(route)
        && !file_name_list_prefers_observed_projection(route, loop_state)
        && !evidence_policy_candidate_satisfies_final_shape(
            task,
            user_text,
            loop_state,
            agent_run_context,
            Some(summary.clone()),
            route,
            &candidate,
        )
    {
        return false;
    }

    let answer = candidate.trim().to_string();
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    info!(
        "delivery matrix_shape_from_observed task_id={} shape_class={} answer={}",
        task.task_id,
        shape_class.as_str(),
        crate::truncate_for_log(&candidate)
    );
    log_deterministic_delivery_record(
        &task.task_id,
        "matrix_shape_from_observed",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn finalizer_summary_requires_matrix_observed_replacement(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(summary) = summary else {
        return false;
    };
    summary.needs_clarify == Some(true)
        || !summary.contract_ok
        || summary.format_ok == Some(false)
        || summary.grounded_ok == Some(false)
}

pub(crate) fn deterministic_matrix_observed_shape_answer(
    state: &AppState,
    _task: &ClaimedTask,
    _user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_requires_evidence_policy_deterministic_final_answer(route) {
        return None;
    }
    let shape_class = evidence_policy_final_answer_shape_class(route)?;
    let (candidate, summary) = matrix_observed_answer_candidate_for_shape(
        state,
        loop_state,
        agent_run_context,
        shape_class,
    )?;
    let candidate = candidate.trim().to_string();
    if candidate.is_empty() {
        return None;
    }
    Some((candidate, summary))
}
