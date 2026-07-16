use serde_json::Value;
use std::collections::BTreeSet;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, OutputResponseShape};

#[path = "dispatch_synthesis_local_code_fields.rs"]
mod dispatch_synthesis_local_code_fields;
#[path = "dispatch_synthesis_local_code_projection.rs"]
mod dispatch_synthesis_local_code_projection;
#[path = "dispatch_synthesis_local_code_readbacks.rs"]
mod dispatch_synthesis_local_code_readbacks;
#[path = "dispatch_synthesis_local_code_writes.rs"]
mod dispatch_synthesis_local_code_writes;
#[path = "dispatch_synthesis_markdown.rs"]
mod dispatch_synthesis_markdown;
use dispatch_synthesis_local_code_projection::{
    common_parent_path, filesystem_projection_skill, machine_code_token, normalize_projection_path,
    path_looks_like_code_or_test_file, path_looks_like_test_file, projection_paths_match,
    successful_code_readbacks, successful_fs_readbacks_after_latest_writes, FsReadback,
};
pub(super) use dispatch_synthesis_local_code_projection::{
    local_code_task_strict_json_projection, strict_json_projection_answer_satisfies_request,
};
use dispatch_synthesis_markdown::{
    markdown_heading_from_read_output, selected_markdown_title_from_read_output,
    strip_markdown_read_line_prefix,
};

pub(super) fn synthesize_answer_allows_direct_fallback(evidence_refs: &[String]) -> bool {
    evidence_refs.is_empty()
        || evidence_refs
            .iter()
            .all(|reference| reference.trim().eq_ignore_ascii_case("last_output"))
}

pub(super) fn synthesize_route_allows_direct_fallback(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.output_contract()) else {
        return true;
    };
    if crate::agent_engine::observed_output::route_disallows_direct_observation_passthrough(route) {
        return false;
    }
    if route.requires_content_evidence
        && !route.delivery_required
        && route.semantic_kind_is_unclassified()
    {
        return true;
    }
    if route.semantic_kind_is_any(&[
        crate::OutputSemanticKind::FileNames,
        crate::OutputSemanticKind::DirectoryNames,
        crate::OutputSemanticKind::FilePaths,
        crate::OutputSemanticKind::ConfigValidation,
    ]) {
        return true;
    }
    if route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && route.response_shape == crate::OutputResponseShape::Strict
    {
        return false;
    }
    matches!(
        route.response_shape,
        crate::OutputResponseShape::Scalar
            | crate::OutputResponseShape::Strict
            | crate::OutputResponseShape::FileToken
    ) || route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
}

pub(super) fn synthesize_route_prefers_model_language_observed_status(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|context| context.output_contract())
        .is_some_and(|route| {
            route.semantic_kind_is_any(&[
                crate::OutputSemanticKind::CommandOutputSummary,
                crate::OutputSemanticKind::ExecutionFailedStep,
            ]) && route.requires_content_evidence
                && !route.delivery_required
        })
}

fn output_has_count_inventory_total(output: &str) -> bool {
    let output = crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
        output.trim(),
    );
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    value.get("action").and_then(|value| value.as_str()) == Some("count_inventory")
        && value
            .get("counts")
            .and_then(|counts| counts.get("total"))
            .and_then(|value| value.as_u64())
            .is_some()
}

fn quantity_comparison_has_multiple_count_observations(loop_state: &LoopState) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .filter(|output| output_has_count_inventory_total(output))
        .take(2)
        .count()
        >= 2
}

fn synthesize_direct_fallback_blocked_by_multi_count_quantity_comparison(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|context| context.output_contract())
        .is_some_and(|route| {
            route.semantic_kind_is(crate::OutputSemanticKind::QuantityComparison)
                && quantity_comparison_has_multiple_count_observations(loop_state)
        })
}

pub(super) fn synthesize_direct_observed_fallback_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if let Some(answer) = reusable_terminal_json_after_later_observation(loop_state) {
        return Some(answer);
    }
    if let Some(answer) =
        synthesize_strict_raw_tail_read_direct_answer(loop_state, agent_run_context)
    {
        return Some(answer);
    }
    if agent_run_context
        .and_then(|context| context.output_contract())
        .is_some_and(
            crate::agent_engine::observed_output::route_disallows_direct_observation_passthrough,
        )
    {
        return None;
    }
    if route_needs_synthesis_for_multi_observation_grounded_summary(loop_state, agent_run_context) {
        return None;
    }
    if synthesize_direct_fallback_blocked_by_multi_count_quantity_comparison(
        loop_state,
        agent_run_context,
    ) {
        return None;
    }
    crate::agent_engine::observed_output::extract_answer_from_observed_output_i18n(
        loop_state,
        state,
        agent_run_context,
    )
    .or_else(|| {
        crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output_i18n(
            loop_state,
            state,
            agent_run_context,
        )
    })
    .map(|answer| answer.trim().to_string())
    .filter(|answer| !answer.is_empty())
}

fn reusable_terminal_json_after_later_observation(loop_state: &LoopState) -> Option<String> {
    let (terminal_idx, answer) = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, step)| {
            step.is_ok() && matches!(step.skill.as_str(), "synthesize_answer" | "respond")
        })
        .filter_map(|(idx, step)| {
            let answer = step.output.as_deref()?.trim();
            terminal_json_is_reusable(answer).then(|| (idx, answer.to_string()))
        })
        .next()?;
    let has_later_observation =
        loop_state
            .executed_step_results
            .iter()
            .enumerate()
            .any(|(idx, step)| {
                idx > terminal_idx
                    && step.is_ok()
                    && !matches!(
                        step.skill.as_str(),
                        "synthesize_answer" | "respond" | "think"
                    )
                    && step
                        .output
                        .as_deref()
                        .map(str::trim)
                        .is_some_and(|output| !output.is_empty())
            });
    has_later_observation.then_some(answer)
}

fn terminal_json_is_reusable(answer: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(answer.trim()) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };
    if obj.is_empty() {
        return false;
    }
    if obj.len() == 1 && obj.contains_key("steps") {
        return false;
    }
    !json_contains_unresolved_terminal_value(&value)
}

fn json_contains_unresolved_terminal_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(_) | Value::Number(_) => false,
        Value::String(text) => {
            let trimmed = text.trim();
            trimmed.is_empty()
                || trimmed.contains("{{")
                || trimmed == "<missing>"
                || trimmed == "not_observed"
                || trimmed == "null"
        }
        Value::Array(items) => items.iter().any(json_contains_unresolved_terminal_value),
        Value::Object(map) => map.values().any(json_contains_unresolved_terminal_value),
    }
}

pub(super) fn archive_database_aggregate_structured_answer(
    loop_state: &LoopState,
) -> Option<String> {
    let archive_list = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "archive_basic")
        .filter_map(|step| step.output.as_deref())
        .filter_map(skill_output_payload)
        .find(|payload| payload.get("action").and_then(Value::as_str) == Some("list"))?;
    let archive_read = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "archive_basic")
        .filter_map(|step| step.output.as_deref())
        .filter_map(skill_output_payload)
        .find(|payload| payload.get("action").and_then(Value::as_str) == Some("read"))?;
    let db_tables = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "db_basic")
        .filter_map(|step| step.output.as_deref())
        .filter_map(skill_output_payload)
        .find(|payload| payload.get("action").and_then(Value::as_str) == Some("list_tables"))?;

    let entries = archive_entry_names(&archive_list)?;
    let member = archive_read
        .get("member")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let content = archive_read
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let tables = sqlite_table_names(&db_tables)?;

    Some(
        serde_json::json!({
            "archive": {
                "entries": entries,
                "member": {
                    "name": member,
                    "content": content,
                },
            },
            "database": {
                "tables": tables,
            },
        })
        .to_string(),
    )
}

#[cfg(test)]
#[path = "dispatch_synthesis_tests.rs"]
mod tests;

pub(super) fn package_docker_probe_structured_answer(loop_state: &LoopState) -> Option<String> {
    let package = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "package_manager")
        .filter_map(|step| step.output.as_deref())
        .filter_map(skill_output_payload)
        .find(|payload| payload.get("action").and_then(Value::as_str) == Some("detect"))?;
    let docker_version = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "docker_basic")
        .filter_map(|step| step.output.as_deref())
        .filter_map(skill_output_payload)
        .find(|payload| payload.get("action").and_then(Value::as_str) == Some("version"))?;
    let docker_ps = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "docker_basic")
        .filter_map(|step| step.output.as_deref())
        .filter_map(skill_output_payload)
        .find(|payload| payload.get("action").and_then(Value::as_str) == Some("ps"))?;

    let manager = package
        .get("manager")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(
        serde_json::json!({
            "package_manager": {
                "manager": manager,
                "platform": package.get("platform").cloned().unwrap_or(Value::Null),
                "candidate_order": package.get("candidate_order").cloned().unwrap_or(Value::Null),
            },
            "docker": {
                "version": docker_probe_payload(&docker_version),
                "containers": docker_probe_payload(&docker_ps),
            },
        })
        .to_string(),
    )
}

pub(super) fn filesystem_mutation_lifecycle_structured_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route_allows = agent_run_context
        .and_then(|context| context.output_contract())
        .is_some_and(|route| {
            route.semantic_kind_is(crate::OutputSemanticKind::FilesystemMutationResult)
        });
    let effective_contract_allows = loop_state.output_contract.as_ref().is_some_and(|contract| {
        contract.semantic_kind_is(crate::OutputSemanticKind::FilesystemMutationResult)
    });
    let observed_scratch_lifecycle =
        crate::agent_engine::scratch_filesystem_lifecycle_observed_steps_match(state, loop_state);
    if !route_allows && !effective_contract_allows && !observed_scratch_lifecycle {
        return None;
    }

    let mut steps = Vec::new();
    let mut readbacks = Vec::new();
    let mut actions = Vec::new();
    let mut paths = Vec::new();
    let mut resolved_paths = Vec::new();
    let mut cleanup_observed = false;

    for step in loop_state.executed_step_results.iter().filter(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
    }) {
        let Some(payload) = step.output.as_deref().and_then(skill_output_payload) else {
            continue;
        };
        let Some(action) = payload
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !filesystem_mutation_lifecycle_action(action) {
            continue;
        }

        push_unique_string(&mut actions, action);
        if action == "remove_path" {
            cleanup_observed = true;
        }

        let path = payload.get("path").and_then(Value::as_str);
        let resolved_path = payload.get("resolved_path").and_then(Value::as_str);
        if let Some(path) = path {
            push_unique_string(&mut paths, path);
        }
        if let Some(path) = resolved_path {
            push_unique_string(&mut resolved_paths, path);
        }

        let mut step_value = serde_json::Map::new();
        step_value.insert("step_id".to_string(), Value::String(step.step_id.clone()));
        step_value.insert("skill".to_string(), Value::String(step.skill.clone()));
        step_value.insert("status".to_string(), Value::String("ok".to_string()));
        step_value.insert("action".to_string(), Value::String(action.to_string()));
        copy_string_field(&payload, &mut step_value, "path");
        copy_string_field(&payload, &mut step_value, "effective_path");
        copy_string_field(&payload, &mut step_value, "resolved_path");
        copy_value_field(&payload, &mut step_value, "append");
        copy_value_field(&payload, &mut step_value, "content_bytes");
        copy_value_field(&payload, &mut step_value, "target_kind");
        copy_value_field(&payload, &mut step_value, "recursive");
        copy_value_field(&payload, &mut step_value, "total_lines");
        steps.push(Value::Object(step_value));

        if matches!(action, "read_range" | "read_text_range") {
            if let Some(excerpt) = payload
                .get("excerpt")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let mut readback = serde_json::Map::new();
                readback.insert("step_id".to_string(), Value::String(step.step_id.clone()));
                copy_string_field(&payload, &mut readback, "path");
                copy_string_field(&payload, &mut readback, "resolved_path");
                readback.insert("excerpt".to_string(), Value::String(excerpt.to_string()));
                copy_value_field(&payload, &mut readback, "total_lines");
                readbacks.push(Value::Object(readback));
            }
        }
    }

    if steps.is_empty() {
        return None;
    }

    let mut observed = BTreeSet::new();
    for action in &actions {
        observed.insert(action.as_str());
    }

    Some(
        serde_json::json!({
            "schema_version": 1,
            "final_answer_shape": crate::evidence_policy::FinalAnswerShape::LifecycleResult.as_str(),
            "final_answer_shape_class": crate::evidence_policy::FinalAnswerShape::LifecycleResult.class().as_str(),
            "status": "ok",
            "observed_actions": actions,
            "observed_action_count": observed.len(),
            "paths": paths,
            "resolved_paths": resolved_paths,
            "steps": steps,
            "readbacks": readbacks,
            "final_state": {
                "cleanup_observed": cleanup_observed,
            },
        })
        .to_string(),
    )
}

pub(super) fn kb_filesystem_mutation_structured_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.output_contract())?;
    if !route.semantic_kind_is(crate::OutputSemanticKind::FilesystemMutationResult) {
        return None;
    }

    let mut steps = Vec::new();
    let mut actions = Vec::new();
    let mut namespaces = Vec::new();
    let mut paths = Vec::new();
    let mut result_kinds = Vec::new();
    let mut effective_statuses = Vec::new();
    let mut idempotent_success = false;
    let mut effective_success = false;

    for step in loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "kb")
    {
        let Some(payload) = step.output.as_deref().and_then(skill_output_payload) else {
            continue;
        };
        let Some(action) = kb_action_from_payload(&payload) else {
            continue;
        };

        push_unique_string(&mut actions, action);
        if let Some(namespace) = payload.get("namespace").and_then(Value::as_str) {
            push_unique_string(&mut namespaces, namespace);
        }
        if let Some(path) = payload.get("path").and_then(Value::as_str) {
            push_unique_string(&mut paths, path);
        }
        if let Some(payload_paths) = payload.get("paths").and_then(Value::as_array) {
            for path in payload_paths.iter().filter_map(Value::as_str) {
                push_unique_string(&mut paths, path);
            }
        }
        if let Some(result_kind) = payload.get("result_kind").and_then(Value::as_str) {
            push_unique_string(&mut result_kinds, result_kind);
        }
        if let Some(effective_status) = payload.get("effective_status").and_then(Value::as_str) {
            push_unique_string(&mut effective_statuses, effective_status);
        }
        idempotent_success |= payload
            .get("idempotent_success")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        effective_success |= payload
            .get("effective_success")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let mut step_value = serde_json::Map::new();
        step_value.insert("step_id".to_string(), Value::String(step.step_id.clone()));
        step_value.insert("skill".to_string(), Value::String(step.skill.clone()));
        step_value.insert("status".to_string(), Value::String("ok".to_string()));
        step_value.insert("action".to_string(), Value::String(action.to_string()));
        copy_string_field(&payload, &mut step_value, "effective_status");
        copy_string_field(&payload, &mut step_value, "result_kind");
        copy_value_field(&payload, &mut step_value, "effective_success");
        copy_value_field(&payload, &mut step_value, "idempotent_success");
        copy_string_field(&payload, &mut step_value, "namespace");
        copy_string_field(&payload, &mut step_value, "path");
        copy_value_field(&payload, &mut step_value, "paths");
        copy_value_field(&payload, &mut step_value, "stats");
        if let Some(hits) = payload.get("hits").and_then(Value::as_array) {
            step_value.insert(
                "hit_count".to_string(),
                Value::Number(serde_json::Number::from(hits.len())),
            );
            step_value.insert(
                "hits".to_string(),
                Value::Array(hits.iter().take(3).cloned().collect()),
            );
        }
        steps.push(Value::Object(step_value));
    }

    if steps.is_empty() {
        return None;
    }

    let mut observed = BTreeSet::new();
    for action in &actions {
        observed.insert(action.as_str());
    }

    Some(
        serde_json::json!({
            "schema_version": 1,
            "final_answer_shape": crate::evidence_policy::FinalAnswerShape::LifecycleResult.as_str(),
            "final_answer_shape_class": crate::evidence_policy::FinalAnswerShape::LifecycleResult.class().as_str(),
            "capability": "kb",
            "status": "ok",
            "effective_status": if effective_statuses.iter().any(|status| status != "ok") { "needs_attention" } else { "ok" },
            "effective_success": effective_success || effective_statuses.iter().any(|status| status == "ok"),
            "idempotent_success": idempotent_success,
            "result_kinds": result_kinds,
            "observed_actions": actions,
            "observed_action_count": observed.len(),
            "namespaces": namespaces,
            "paths": paths,
            "steps": steps,
        })
        .to_string(),
    )
}

fn kb_action_from_payload(payload: &Value) -> Option<&'static str> {
    match payload.get("action").and_then(Value::as_str).map(str::trim) {
        Some("ingest") => return Some("ingest"),
        Some("search") => return Some("search"),
        Some("stats") => return Some("stats"),
        Some("list_namespaces") => return Some("list_namespaces"),
        _ => {}
    }

    let stats = payload.get("stats").filter(|value| value.is_object());
    if payload.get("hits").and_then(Value::as_array).is_some() {
        return Some("search");
    }
    if payload
        .get("namespaces")
        .and_then(Value::as_array)
        .is_some()
    {
        return Some("list_namespaces");
    }
    if stats
        .and_then(|stats| stats.get("ingested_docs"))
        .and_then(Value::as_u64)
        .is_some()
        || payload.get("paths").and_then(Value::as_array).is_some()
    {
        return Some("ingest");
    }
    if stats.is_some() {
        return Some("stats");
    }
    None
}

fn filesystem_mutation_lifecycle_action(action: &str) -> bool {
    matches!(
        action,
        "make_dir"
            | "write_text"
            | "append_text"
            | "read_range"
            | "read_text_range"
            | "remove_path"
    )
}

fn copy_string_field(source: &Value, target: &mut serde_json::Map<String, Value>, field: &str) {
    if let Some(value) = source
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        target.insert(field.to_string(), Value::String(value.to_string()));
    }
}

fn copy_value_field(source: &Value, target: &mut serde_json::Map<String, Value>, field: &str) {
    if let Some(value) = source.get(field).filter(|value| !value.is_null()) {
        target.insert(field.to_string(), value.clone());
    }
}

fn docker_probe_payload(payload: &Value) -> Value {
    serde_json::json!({
        "available": payload.get("available").cloned().unwrap_or(Value::Null),
        "command_succeeded": payload.get("command_succeeded").cloned().unwrap_or(Value::Null),
        "exit_code": payload.get("exit_code").cloned().unwrap_or(Value::Null),
        "docker_args": payload.get("docker_args").cloned().unwrap_or(Value::Null),
        "output": payload.get("output").cloned().unwrap_or(Value::Null),
    })
}

fn skill_output_payload(output: &str) -> Option<Value> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        return Some(extra.clone());
    }
    Some(value)
}

fn archive_entry_names(payload: &Value) -> Option<Vec<String>> {
    let mut names = Vec::new();
    if let Some(entries) = payload.get("entries").and_then(Value::as_array) {
        for entry in entries {
            if let Some(name) = entry.get("name").and_then(Value::as_str) {
                push_unique_string(&mut names, name);
            }
        }
    }
    if names.is_empty() {
        if let Some(candidates) = payload.get("candidates").and_then(Value::as_array) {
            for candidate in candidates {
                if let Some(name) = candidate.as_str() {
                    push_unique_string(&mut names, name);
                }
            }
        }
    }
    (!names.is_empty()).then_some(names)
}

fn sqlite_table_names(payload: &Value) -> Option<Vec<String>> {
    let rows = payload
        .get("result")
        .and_then(|result| result.get("rows"))
        .or_else(|| payload.get("rows"))
        .and_then(Value::as_array)?;
    let mut names = Vec::new();
    for row in rows {
        if let Some(name) = row.get("name").and_then(Value::as_str) {
            push_unique_string(&mut names, name);
        }
    }
    (!names.is_empty()).then_some(names)
}

fn push_unique_string(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        values.push(value.to_string());
    }
}

fn synthesize_strict_raw_tail_read_direct_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.output_contract())?;
    if !route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        || route.response_shape != crate::OutputResponseShape::Strict
        || !route.requires_content_evidence
        || route.delivery_required
    {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "fs_basic" | "system_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(strict_raw_tail_read_answer_from_output)
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}

fn strict_raw_tail_read_answer_from_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    strict_raw_tail_read_answer_from_value(&value)
}

fn strict_raw_tail_read_answer_from_value(value: &Value) -> Option<String> {
    if let Some(answer) = strict_raw_tail_read_answer_from_flat_value(value) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(strict_raw_tail_read_answer_from_value)
}

fn strict_raw_tail_read_answer_from_flat_value(value: &Value) -> Option<String> {
    if !matches!(
        value.get("action").and_then(Value::as_str),
        Some("read_range" | "read_text_range")
    ) || value.get("mode").and_then(Value::as_str) != Some("tail")
    {
        return None;
    }
    let requested_n = value
        .get("requested_n")
        .or_else(|| value.get("n"))
        .or_else(|| value.get("count"))
        .and_then(Value::as_u64)?;
    if requested_n == 0 || requested_n > 50 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(Value::as_str)
        .filter(|excerpt| !excerpt.trim().is_empty())?;
    let mut candidate = value.clone();
    let obj = candidate.as_object_mut()?;
    obj.insert(
        "action".to_string(),
        Value::String("read_range".to_string()),
    );
    if !obj.contains_key("requested_n") {
        obj.insert("requested_n".to_string(), Value::Number(requested_n.into()));
    }
    crate::agent_engine::observed_output::tail_read_range_direct_answer_candidate(
        &candidate.to_string(),
        false,
    )
}

pub(super) fn synthesize_evidence_policy_direct_observed_fallback_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.output_contract())?;
    crate::evidence_policy::final_answer_shape_for_output_contract(route)?;
    if route.semantic_kind_is(crate::OutputSemanticKind::ConfigMutation) {
        return None;
    }
    if crate::agent_engine::observed_output::route_disallows_direct_observation_passthrough(route) {
        return None;
    }
    if synthesize_direct_fallback_blocked_by_multi_count_quantity_comparison(
        loop_state,
        agent_run_context,
    ) {
        return None;
    }
    if synthesize_direct_fallback_would_passthrough_multiline_read_range(
        loop_state,
        agent_run_context,
    ) {
        return None;
    }
    synthesize_direct_observed_fallback_answer(state, loop_state, agent_run_context)
}

fn route_needs_synthesis_for_multi_observation_grounded_summary(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.output_contract()) else {
        return false;
    };
    if !route.requires_content_evidence || route.delivery_required {
        return false;
    }
    let Some(shape) = crate::evidence_policy::final_answer_shape_for_output_contract(route) else {
        return false;
    };
    if shape.class() != crate::evidence_policy::FinalAnswerShapeClass::GroundedSummary {
        return false;
    }
    successful_observation_step_count(loop_state) >= 2
}

fn successful_observation_step_count(loop_state: &LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|output| !output.is_empty())
        })
        .count()
}

pub(super) fn synthesize_direct_fallback_would_passthrough_multiline_read_range(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.output_contract()) else {
        return false;
    };
    if !route.requires_content_evidence
        || route.delivery_required
        || !matches!(
            route.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::Scalar
                | OutputResponseShape::Strict
                | OutputResponseShape::OneSentence
        )
    {
        return false;
    }
    let semantic_blocks_direct_passthrough = route.semantic_kind_is_unclassified()
        || route.semantic_kind.is_content_excerpt_summary()
        || (route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
            && latest_round_plan_requests_synthesis(loop_state));
    if !semantic_blocks_direct_passthrough {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find_map(multiline_read_range_content_line_count)
        .is_some_and(|line_count| line_count > 1)
}

fn latest_round_plan_requests_synthesis(loop_state: &LoopState) -> bool {
    loop_state.round_traces.iter().rev().any(|round| {
        round.plan_result.as_ref().is_some_and(|plan| {
            plan.steps
                .iter()
                .any(|step| step.action_type == "synthesize_answer")
        })
    })
}

fn multiline_read_range_content_line_count(output: &str) -> Option<usize> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    multiline_read_range_content_line_count_from_value(&value)
}

fn multiline_read_range_content_line_count_from_value(value: &Value) -> Option<usize> {
    if value
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(|action| matches!(action, "read_range" | "read_text_range"))
    {
        if let Some(text) = value
            .get("content")
            .or_else(|| value.get("excerpt"))
            .and_then(Value::as_str)
        {
            return Some(
                text.lines()
                    .map(strip_markdown_read_line_prefix)
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .count(),
            );
        }
    }
    value
        .get("extra")
        .and_then(multiline_read_range_content_line_count_from_value)
}

pub(super) fn deterministic_scalar_markdown_heading_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context?.output_contract()?;
    if route.delivery_required
        || route.semantic_kind_is_any(&[
            crate::OutputSemanticKind::FileNames,
            crate::OutputSemanticKind::DirectoryNames,
            crate::OutputSemanticKind::FilePaths,
            crate::OutputSemanticKind::DirectoryEntryGroups,
            crate::OutputSemanticKind::ScalarCount,
            crate::OutputSemanticKind::RawCommandOutput,
        ])
    {
        return None;
    }
    let output = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find(|output| {
            output.contains("\"read_range\"") || output.contains("\"read_text_range\"")
        })?;
    if let Some(answer) = selected_markdown_title_from_read_output(output) {
        return Some(answer);
    }
    if route.response_shape != OutputResponseShape::Scalar {
        return None;
    }
    markdown_heading_from_read_output(output)
}

pub(super) fn synthesize_user_language_source<'a>(
    user_text: &'a str,
    agent_run_context: Option<&'a AgentRunContext>,
) -> &'a str {
    agent_run_context
        .and_then(|context| {
            context
                .original_user_request
                .as_deref()
                .or(context.user_request.as_deref())
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(user_text)
}

pub(super) fn route_resolved_intent(agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| {
            ctx.original_user_request
                .as_deref()
                .or(ctx.user_request.as_deref())
        })
        .map(str::trim)
        .filter(|intent| !intent.is_empty())
        .unwrap_or_default()
        .to_string()
}

pub(super) fn synthesize_failure_observed_facts(
    loop_state: &LoopState,
    refs_summary: &str,
) -> Vec<String> {
    let mut facts = vec![
        format!("synthesize_refs: {}", refs_summary.trim()),
        format!(
            "observed_steps_count: {}",
            loop_state.executed_step_results.len()
        ),
    ];
    let mut recent_steps = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
        })
        .take(4)
        .collect::<Vec<_>>();
    recent_steps.reverse();
    for step in recent_steps {
        let mut parts = vec![
            format!("skill={}", step.skill.trim()),
            format!("status={}", step.status.as_str()),
        ];
        if let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!(
                "output_excerpt={}",
                crate::truncate_for_agent_trace(output)
            ));
        }
        if let Some(error) = step
            .error
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!(
                "error_summary={}",
                crate::truncate_for_agent_trace(error)
            ));
        }
        facts.push(format!("observed_step: {}", parts.join(", ")));
    }
    facts
}

pub(super) fn step_has_observable_synthesis_fact(
    step: &crate::executor::StepExecutionResult,
) -> bool {
    if step.is_ok() {
        return step
            .output
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
    }
    step.error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|err| {
            crate::skills::is_observable_run_cmd_error(&step.skill, err)
                || crate::skills::is_recoverable_skill_error(&step.skill, err)
        })
}

pub(super) fn synthesize_failure_should_replan(loop_state: &LoopState) -> bool {
    let previous_synthesis_failures = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.skill == "synthesize_answer" && !step.is_ok())
        .count();
    if previous_synthesis_failures > 0 {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        !matches!(
            step.skill.as_str(),
            "respond" | "think" | "synthesize_answer"
        ) && step_has_observable_synthesis_fact(step)
    })
}
