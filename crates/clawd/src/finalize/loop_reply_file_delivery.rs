use std::path::{Path, PathBuf};

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::AppState;

use super::{matrix_observed_shape_summary, route_requires_file_token};

pub(super) fn resolve_file_token_from_auto_locator_answer(
    answer: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let trimmed = answer.trim();
    if trimmed.is_empty()
        || trimmed.contains('\n')
        || crate::finalize::parse_delivery_file_token(trimmed).is_some()
    {
        return None;
    }
    let auto_locator_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let auto_path = Path::new(auto_locator_path);

    let resolved = if auto_path.is_file() {
        let file_name = auto_path.file_name().and_then(|v| v.to_str())?;
        if trimmed != file_name {
            return None;
        }
        auto_path
            .canonicalize()
            .unwrap_or_else(|_| auto_path.to_path_buf())
    } else if auto_path.is_dir() {
        let candidate = auto_path.join(trimmed);
        if !candidate.is_file() {
            return None;
        }
        candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf())
    } else {
        return None;
    };

    Some(format!("FILE:{}", resolved.display()))
}

pub(super) fn normalize_file_token_delivery_from_auto_locator(
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if !route_requires_file_token(agent_run_context) {
        return;
    }
    let auto_locator_path = agent_run_context.and_then(|ctx| ctx.auto_locator_path.as_deref());

    if let Some(token) = loop_state
        .last_user_visible_respond
        .as_deref()
        .and_then(|answer| resolve_file_token_from_auto_locator_answer(answer, auto_locator_path))
    {
        loop_state.last_user_visible_respond = Some(token);
    }

    for message in &mut loop_state.delivery_messages {
        if let Some(token) = resolve_file_token_from_auto_locator_answer(message, auto_locator_path)
        {
            *message = token;
        }
    }
}

pub(super) fn direct_file_token_from_observed_auto_locator_filename(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requires_file_token(agent_run_context) {
        return None;
    }
    let auto_locator_path = agent_run_context.and_then(|ctx| ctx.auto_locator_path.as_deref())?;
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|output| !output.is_empty())
        else {
            continue;
        };
        let Some(token) =
            resolve_file_token_from_auto_locator_answer(output, Some(auto_locator_path))
        else {
            continue;
        };
        return Some((
            token,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    None
}

fn bare_delivery_filename(answer: &str) -> Option<&str> {
    let trimmed = answer.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }
    let payload = crate::finalize::parse_delivery_file_token(trimmed)
        .map(|(_, payload)| payload.trim())
        .unwrap_or(trimmed);
    if payload.is_empty()
        || payload.contains('/')
        || payload.contains('\\')
        || Path::new(payload).is_absolute()
    {
        return None;
    }
    Some(payload)
}

fn observed_file_path_for_payload(
    state: &AppState,
    raw_path: &str,
    payload: &str,
) -> Option<PathBuf> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return None;
    }
    let path = Path::new(raw_path);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    let file_name = candidate.file_name()?.to_string_lossy();
    if file_name != payload {
        return None;
    }
    if !candidate.is_file() {
        return None;
    }
    Some(candidate.canonicalize().unwrap_or(candidate))
}

fn collect_observed_file_paths(
    state: &AppState,
    value: &serde_json::Value,
    payload: &str,
    out: &mut Vec<PathBuf>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if matches!(key.as_str(), "path" | "resolved_path") {
                    if let Some(raw_path) = child.as_str() {
                        if let Some(path) = observed_file_path_for_payload(state, raw_path, payload)
                        {
                            out.push(path);
                        }
                    }
                }
                collect_observed_file_paths(state, child, payload, out);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_observed_file_paths(state, child, payload, out);
            }
        }
        _ => {}
    }
}

fn resolve_file_token_from_observed_paths(
    state: &AppState,
    answer: &str,
    loop_state: &LoopState,
) -> Option<String> {
    let payload = bare_delivery_filename(answer)?;
    let mut matches = Vec::new();
    for step in loop_state.executed_step_results.iter().rev() {
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        collect_observed_file_paths(state, &value, payload, &mut matches);
    }
    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        Some(format!("FILE:{}", matches[0].display()))
    } else {
        None
    }
}

pub(super) fn normalize_file_token_delivery_from_observed_paths(
    state: &AppState,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if !route_requires_file_token(agent_run_context) {
        return;
    }

    if let Some(token) = loop_state
        .last_user_visible_respond
        .as_deref()
        .and_then(|answer| resolve_file_token_from_observed_paths(state, answer, loop_state))
    {
        loop_state.last_user_visible_respond = Some(token);
    }

    let replacements = loop_state
        .delivery_messages
        .iter()
        .map(|message| {
            resolve_file_token_from_observed_paths(state, message, loop_state)
                .unwrap_or_else(|| message.clone())
        })
        .collect::<Vec<_>>();
    loop_state.delivery_messages = replacements;
}

fn planned_file_delivery_used_unresolved_runtime_placeholder(loop_state: &LoopState) -> bool {
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|round| round.plan_result.as_ref())
        .any(|plan| {
            plan.steps.iter().any(|step| {
                if step.action_type != "respond" {
                    return false;
                }
                let content = step
                    .args
                    .get("content")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .unwrap_or_default();
                crate::finalize::parse_delivery_token(content).is_some()
                    && content.contains("{{")
                    && content.contains("}}")
            }) || {
                let raw = plan.raw_plan_text.as_str();
                raw.contains("FILE:") && raw.contains("{{") && raw.contains("}}")
            }
        })
}

fn inventory_root_path(value: &serde_json::Value) -> Option<PathBuf> {
    value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn inventory_ranked_for_single_file_selection(value: &serde_json::Value) -> bool {
    value
        .get("sort_by")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|sort_by| {
            matches!(
                sort_by,
                "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc"
            )
        })
}

fn inventory_has_deterministic_order(value: &serde_json::Value) -> bool {
    value
        .get("sort_by")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|sort_by| {
            matches!(
                sort_by,
                "name" | "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc"
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedInventorySelection {
    First,
    Last,
}

fn planned_inventory_selection_from_template_text(text: &str) -> Option<PlannedInventorySelection> {
    let mut rest = text;
    let mut selection = None;
    while let Some(start) = rest.find("{{") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            break;
        };
        let expression = after_start[..end]
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        rest = &after_start[end + 2..];

        if !expression.contains("last_output") {
            continue;
        }
        let next = if expression.contains(".last(")
            || expression.contains("[-1]")
            || expression.contains(".rev().next(")
        {
            PlannedInventorySelection::Last
        } else if expression.contains(".first(")
            || expression.contains("[0]")
            || expression.contains(".next(")
        {
            PlannedInventorySelection::First
        } else {
            continue;
        };
        if selection.is_some_and(|existing| existing != next) {
            return None;
        }
        selection = Some(next);
    }
    selection
}

fn planned_file_delivery_inventory_selection(
    loop_state: &LoopState,
) -> Option<PlannedInventorySelection> {
    for plan in loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|round| round.plan_result.as_ref())
    {
        for step in &plan.steps {
            if step.action_type != "respond" {
                continue;
            }
            let Some(content) = step
                .args
                .get("content")
                .and_then(|value| value.as_str())
                .map(str::trim)
            else {
                continue;
            };
            if crate::finalize::parse_delivery_token(content).is_some()
                || (content.contains("FILE:") && content.contains("{{"))
            {
                if let Some(selection) = planned_inventory_selection_from_template_text(content) {
                    return Some(selection);
                }
            }
        }
        let raw = plan.raw_plan_text.as_str();
        if raw.contains("FILE:") && raw.contains("{{") && raw.contains("}}") {
            if let Some(selection) = planned_inventory_selection_from_template_text(raw) {
                return Some(selection);
            }
        }
    }
    None
}

pub(super) fn planned_file_delivery_uses_runtime_selection_template(
    loop_state: &LoopState,
) -> bool {
    planned_file_delivery_used_unresolved_runtime_placeholder(loop_state)
        || planned_file_delivery_inventory_selection(loop_state).is_some()
}

fn inventory_candidate_names(value: &serde_json::Value) -> Vec<String> {
    if let Some(names) = value.get("names").and_then(|value| value.as_array()) {
        return names
            .iter()
            .filter_map(|value| value.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToString::to_string)
            .collect();
    }
    value
        .get("entries")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry
                .get("kind")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .map(|kind| kind == "file")
                .unwrap_or(true)
        })
        .filter_map(|entry| entry.get("name").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn observed_inventory_file_candidates(value: &serde_json::Value) -> Option<Vec<PathBuf>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir") {
        return None;
    }
    let root = inventory_root_path(value)?;
    let mut candidates = Vec::new();
    for name in inventory_candidate_names(value) {
        let name_path = Path::new(&name);
        let candidate = if name_path.is_absolute() {
            name_path.to_path_buf()
        } else {
            root.join(name_path)
        };
        if candidate.is_file() {
            candidates.push(candidate.canonicalize().unwrap_or(candidate));
        }
    }
    (!candidates.is_empty()).then_some(candidates)
}

fn active_anchor_bound_targets(agent_run_context: Option<&AgentRunContext>) -> Vec<String> {
    let Some(ctx) = agent_run_context else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    let sources = [
        ctx.route_result
            .as_ref()
            .map(|route| route.resolved_intent.as_str()),
        ctx.context_bundle_summary.as_deref(),
        ctx.cross_turn_recent_execution_context.as_deref(),
        ctx.user_request.as_deref(),
    ];
    for source in sources.into_iter().flatten() {
        for line in source.lines() {
            let trimmed = line.trim_start();
            let Some(target) = ["followup_bound_target:", "observed_bound_target:"]
                .iter()
                .find_map(|prefix| trimmed.strip_prefix(prefix))
                .map(str::trim)
                .filter(|target| !target.is_empty())
            else {
                continue;
            };
            if !targets
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(target))
            {
                targets.push(target.to_string());
            }
        }
    }
    targets
}

fn path_leaf_eq(left: &str, right: &str) -> bool {
    Path::new(left)
        .file_name()
        .and_then(|value| value.to_str())
        .zip(
            Path::new(right)
                .file_name()
                .and_then(|value| value.to_str()),
        )
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn inventory_root_matches_bound_parent(value: &serde_json::Value, target: &str) -> bool {
    let Some(parent_name) = Path::new(target)
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    ["resolved_path", "path"]
        .iter()
        .filter_map(|field| value.get(*field).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .any(|path| {
            Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|root_name| root_name.eq_ignore_ascii_case(parent_name))
        })
}

fn inventory_entry_path_for_bound_target(
    value: &serde_json::Value,
    target: &str,
) -> Option<String> {
    if value.get("action").and_then(|value| value.as_str()) != Some("inventory_dir")
        || !inventory_root_matches_bound_parent(value, target)
    {
        return None;
    }
    let entries = value.get("entries").and_then(|value| value.as_array())?;
    for entry in entries {
        if entry
            .get("kind")
            .and_then(|value| value.as_str())
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("dir"))
        {
            continue;
        }
        let name = entry
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let path = entry
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let matches_target = name.is_some_and(|name| path_leaf_eq(name, target))
            || path.is_some_and(|path| path_leaf_eq(path, target));
        if !matches_target {
            continue;
        }
        if let Some(path) = path {
            return Some(path.to_string());
        }
        let root = value
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let name = name?;
        return Some(Path::new(root).join(name).display().to_string());
    }
    None
}

pub(super) fn direct_path_from_active_bound_inventory(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let targets = active_anchor_bound_targets(agent_run_context);
    if targets.is_empty() {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "fs_basic" | "system_basic") {
            continue;
        }
        let Some(value) = step
            .output
            .as_deref()
            .and_then(|output| serde_json::from_str::<serde_json::Value>(output).ok())
        else {
            continue;
        };
        for target in &targets {
            if let Some(path) = inventory_entry_path_for_bound_target(&value, target) {
                return Some((
                    path,
                    crate::task_journal::TaskJournalFinalizerSummary {
                        stage: Some(
                            crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric,
                        ),
                        disposition: Some(
                            crate::finalize::FinalizerDisposition::QualifiedCompletion,
                        ),
                        contract_ok: true,
                        completion_ok: Some(true),
                        grounded_ok: Some(true),
                        format_ok: Some(true),
                        needs_clarify: Some(false),
                        used_evidence_ids_count: 1,
                        ..Default::default()
                    },
                ));
            }
        }
    }
    None
}

fn route_requests_generated_file_path_report(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            let contract = route.effective_output_contract();
            !contract.delivery_required
                && contract.response_shape == crate::OutputResponseShape::Scalar
                && route
                    .output_contract_marker_is(crate::OutputSemanticKind::GeneratedFilePathReport)
                && crate::evidence_policy::final_answer_shape_for_output_contract(&contract)
                    == Some(crate::evidence_policy::FinalAnswerShape::SinglePath)
        })
}

fn clean_machine_path_payload(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() || path.contains('\n') {
        return None;
    }
    let payload = crate::finalize::parse_delivery_file_token(path)
        .map(|(_, payload)| payload.trim())
        .unwrap_or(path)
        .trim();
    (!payload.is_empty()).then(|| payload.to_string())
}

fn single_written_file_alias_path(loop_state: &LoopState) -> Option<String> {
    let mut paths = std::collections::BTreeSet::<String>::new();
    for path in loop_state.written_file_aliases.values() {
        if let Some(path) = clean_machine_path_payload(path) {
            paths.insert(path);
        }
    }
    (paths.len() == 1)
        .then(|| paths.into_iter().next())
        .flatten()
}

pub(super) fn direct_generated_file_path_report_from_written_path(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requests_generated_file_path_report(agent_run_context) {
        return None;
    }
    let path = loop_state
        .output_vars
        .get("last_written_file_path")
        .or(loop_state.last_written_file_path.as_ref())
        .and_then(|path| clean_machine_path_payload(path))
        .or_else(|| single_written_file_alias_path(loop_state))?;
    Some((path, matrix_observed_shape_summary(loop_state)))
}

fn route_allows_dry_run_generated_file_path_report_payload(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_none_or(|route| !route.needs_clarify)
}

fn compact_machine_json(value: &serde_json::Value) -> Option<String> {
    serde_json::to_string(value)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn clean_machine_field_value(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.contains('\n'))
        .map(ToOwned::to_owned)
}

fn machine_bool_true(value: Option<&serde_json::Value>) -> bool {
    value.and_then(|value| value.as_bool()) == Some(true)
}

fn projected_planned_outputs(value: Option<&serde_json::Value>) -> Vec<serde_json::Value> {
    let Some(items) = value.and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let path = item
                .get("path")
                .and_then(clean_machine_field_value)
                .and_then(|path| clean_machine_path_payload(&path))?;
            let mut projected = serde_json::Map::new();
            projected.insert("path".to_string(), serde_json::Value::String(path));
            if let Some(kind) = item.get("type").and_then(clean_machine_field_value) {
                projected.insert("type".to_string(), serde_json::Value::String(kind));
            }
            Some(serde_json::Value::Object(projected))
        })
        .collect()
}

fn first_projected_output_path(projected: &[serde_json::Value]) -> Option<String> {
    projected.iter().find_map(|item| {
        item.get("path")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned)
    })
}

fn generated_file_path_report_from_dry_run_value(value: &serde_json::Value) -> Option<String> {
    let extra = value.get("extra").unwrap_or(value);
    if !machine_bool_true(extra.get("dry_run")) {
        return None;
    }
    let mut planned_outputs = projected_planned_outputs(extra.get("planned_outputs"));
    let output_path = extra
        .get("output_path")
        .and_then(clean_machine_field_value)
        .and_then(|path| clean_machine_path_payload(&path))
        .or_else(|| first_projected_output_path(&planned_outputs))?;
    if planned_outputs.is_empty() {
        let mut projected = serde_json::Map::new();
        projected.insert(
            "path".to_string(),
            serde_json::Value::String(output_path.clone()),
        );
        planned_outputs.push(serde_json::Value::Object(projected));
    }

    let mut fields = vec!["dry_run=true".to_string()];
    if let Some(provider) = extra.get("provider").and_then(clean_machine_field_value) {
        fields.push(format!("provider={provider}"));
    }
    if let Some(model) = extra.get("model").and_then(clean_machine_field_value) {
        fields.push(format!("model={model}"));
    }
    if let Some(model_kind) = extra.get("model_kind").and_then(clean_machine_field_value) {
        fields.push(format!("model_kind={model_kind}"));
    }
    if let Some(adapter_kind) = extra
        .get("adapter_kind")
        .and_then(clean_machine_field_value)
    {
        fields.push(format!("adapter_kind={adapter_kind}"));
    }
    fields.push(format!("output_path={output_path}"));
    if let Some(planned_outputs_json) =
        compact_machine_json(&serde_json::Value::Array(planned_outputs))
    {
        fields.push(format!("planned_outputs={planned_outputs_json}"));
    }
    if let Some(pending_contract) = extra
        .get("pending_async_job_contract")
        .filter(|value| value.is_object())
        .and_then(compact_machine_json)
    {
        fields.push(format!("pending_async_job_contract={pending_contract}"));
    }
    Some(fields.join("\n"))
}

pub(super) fn async_adapter_result_report_from_value(value: &serde_json::Value) -> Option<String> {
    let extra = value.get("extra").unwrap_or(value);
    let (adapter_result_key, adapter_result) = async_adapter_result_field(extra)?;
    let mut fields = Vec::new();
    if let Some(task_id) = extra
        .get("task_id")
        .or_else(|| {
            adapter_result
                .get("final_result_json")
                .and_then(|result| result.get("task_id"))
        })
        .or_else(|| {
            adapter_result
                .get("cancellation_result_json")
                .and_then(|result| result.get("task_id"))
        })
        .and_then(clean_machine_field_value)
    {
        fields.push(format!("task_id={task_id}"));
    }
    if let Some(job_id) = extra
        .get("job_id")
        .or_else(|| adapter_result.get("job_id"))
        .or_else(|| {
            adapter_result
                .get("final_result_json")
                .and_then(|result| result.get("job_id"))
        })
        .or_else(|| {
            adapter_result
                .get("cancellation_result_json")
                .and_then(|result| result.get("job_id"))
        })
        .and_then(clean_machine_field_value)
    {
        fields.push(format!("job_id={job_id}"));
    }
    if let Some(status) = extra
        .get("status")
        .or_else(|| adapter_result.get("status"))
        .or_else(|| {
            adapter_result
                .get("final_result_json")
                .and_then(|result| result.get("status"))
        })
        .or_else(|| {
            adapter_result
                .get("cancellation_result_json")
                .and_then(|result| result.get("status"))
        })
        .and_then(clean_machine_field_value)
    {
        fields.push(format!("status={status}"));
    }
    let adapter_json = compact_machine_json(adapter_result)?;
    fields.push(format!("{adapter_result_key}={adapter_json}"));
    (fields.len() >= 2).then(|| fields.join("\n"))
}

fn async_adapter_result_field(
    extra: &serde_json::Value,
) -> Option<(&'static str, &serde_json::Value)> {
    extra
        .get("async_cancel_adapter_result")
        .filter(|value| value.is_object())
        .map(|value| ("async_cancel_adapter_result", value))
        .or_else(|| {
            extra
                .get("async_poll_adapter_result")
                .filter(|value| value.is_object())
                .map(|value| ("async_poll_adapter_result", value))
        })
}

pub(super) fn direct_async_poll_result_report_from_payload(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_allows_dry_run_generated_file_path_report_payload(agent_run_context) {
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
        if let Some(answer) = async_adapter_result_report_from_value(&value) {
            return Some((answer, matrix_observed_shape_summary(loop_state)));
        }
    }
    None
}

pub(super) fn direct_generated_file_path_report_from_dry_run_payload(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_allows_dry_run_generated_file_path_report_payload(agent_run_context) {
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
        if let Some(answer) = generated_file_path_report_from_dry_run_value(&value) {
            return Some((answer, matrix_observed_shape_summary(loop_state)));
        }
    }
    None
}

pub(super) fn direct_created_archive_path_from_observed_archive_pack(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_requests_archive_pack(route) {
        return None;
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::ArchivePack)
        && crate::evidence_policy::final_answer_shape_for_output_contract(
            &route.effective_output_contract(),
        )?
        .as_str()
            != "created_archive_path"
    {
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
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        if let Some(path) = created_archive_path_from_observed_body(output) {
            return Some((path, matrix_observed_shape_summary(loop_state)));
        }
    }
    None
}

fn route_requests_archive_pack(route: &crate::RouteResult) -> bool {
    route.output_contract_marker_is(crate::OutputSemanticKind::ArchivePack)
        || crate::evidence_policy::final_answer_shape_for_route(route)
            == Some(crate::evidence_policy::FinalAnswerShape::CreatedArchivePath)
}

fn created_archive_path_from_observed_body(body: &str) -> Option<String> {
    const PATH_KEYS: [&str; 3] = ["archive", "archive_path", "path"];
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim()) {
        for key in PATH_KEYS {
            if let Some(path) = value
                .get(key)
                .or_else(|| value.get("extra").and_then(|extra| extra.get(key)))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| archive_output_path_candidate(value))
            {
                return Some(path.to_string());
            }
        }
    }
    for token in body.split_whitespace() {
        let token = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '。' | '，'
            )
        });
        let Some((key, rhs)) = token.split_once('=') else {
            continue;
        };
        if !PATH_KEYS
            .iter()
            .any(|candidate| key.trim().eq_ignore_ascii_case(candidate))
        {
            continue;
        }
        let rhs = rhs.trim();
        if archive_output_path_candidate(rhs) {
            return Some(rhs.to_string());
        }
    }
    None
}

fn archive_output_path_candidate(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains(|ch| matches!(ch, '\n' | '\r' | '\0'))
        && !value.contains("://")
        && (value.starts_with('/')
            || value.starts_with("./")
            || value.starts_with("../")
            || value.contains('/'))
}

fn path_batch_fact_file_path(entry: &serde_json::Value) -> Option<PathBuf> {
    let entry = entry.as_object()?;
    if entry.get("exists").and_then(|value| value.as_bool()) != Some(true) {
        return None;
    }
    let fact = entry.get("fact").and_then(|value| value.as_object());
    let kind = fact
        .and_then(|item| item.get("kind"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if kind.is_some_and(|kind| !kind.eq_ignore_ascii_case("file")) {
        return None;
    }
    let path = fact
        .and_then(|item| item.get("resolved_path"))
        .or_else(|| fact.and_then(|item| item.get("path")))
        .or_else(|| entry.get("resolved_path"))
        .or_else(|| entry.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let path = PathBuf::from(path);
    if kind.is_none() && !path.is_file() {
        return None;
    }
    Some(path.canonicalize().unwrap_or(path))
}

fn observed_path_batch_file_candidates(value: &serde_json::Value) -> Option<Vec<PathBuf>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts") {
        return None;
    }
    let facts = value.get("facts")?.as_array()?;
    let mut candidates = facts
        .iter()
        .filter_map(path_batch_fact_file_path)
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    (!candidates.is_empty()).then_some(candidates)
}

fn observed_find_entries_file_candidates(
    state: &AppState,
    value: &serde_json::Value,
) -> Option<Vec<PathBuf>> {
    let action = value.get("action").and_then(|value| value.as_str())?;
    if !matches!(action, "find_entries" | "find_name") {
        return None;
    }
    let root = value
        .get("root")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| state.skill_rt.workspace_root.clone());
    let mut candidates = value
        .get("results")
        .and_then(|value| value.as_array())?
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter_map(|path| {
            let candidate = Path::new(path);
            let resolved = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                root.join(candidate)
            };
            resolved
                .is_file()
                .then(|| resolved.canonicalize().unwrap_or(resolved))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    (!candidates.is_empty()).then_some(candidates)
}

fn route_requests_scalar_path_candidate_projection(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            let contract = route.effective_output_contract();
            !contract.delivery_required
                && contract.response_shape == crate::OutputResponseShape::Scalar
                && crate::finalize::route_matches_single_path_output_contract(route)
        })
}

fn scalar_path_candidates_from_find_name(value: &serde_json::Value) -> Option<Vec<String>> {
    let action = value.get("action").and_then(|value| value.as_str())?;
    if !matches!(action, "find_entries" | "find_name") {
        return None;
    }
    let candidates = value
        .get("results")
        .and_then(|value| value.as_array())?
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    scalar_path_candidate_list(candidates)
}

fn path_batch_fact_display_path(
    entry: &serde_json::Map<String, serde_json::Value>,
) -> Option<&str> {
    let fact = entry.get("fact").and_then(|value| value.as_object());
    fact.and_then(|item| item.get("path"))
        .or_else(|| entry.get("path"))
        .or_else(|| fact.and_then(|item| item.get("resolved_path")))
        .or_else(|| entry.get("resolved_path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

fn scalar_path_candidates_from_path_batch_facts(value: &serde_json::Value) -> Option<Vec<String>> {
    if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts") {
        return None;
    }
    let candidates = value
        .get("facts")
        .and_then(|value| value.as_array())?
        .iter()
        .filter_map(|entry| entry.as_object())
        .filter(|entry| entry.get("exists").and_then(|value| value.as_bool()) == Some(true))
        .filter_map(path_batch_fact_display_path)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    scalar_path_candidate_list(candidates)
}

fn scalar_path_candidate_list(candidates: Vec<String>) -> Option<Vec<String>> {
    let mut seen = std::collections::BTreeSet::<String>::new();
    let mut deduped = Vec::new();
    for candidate in candidates {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        let key = candidate.to_ascii_lowercase();
        if seen.insert(key) {
            deduped.push(candidate.to_string());
        }
    }
    (deduped.len() >= 2).then_some(deduped)
}

pub(super) fn direct_scalar_path_candidate_list_from_observed_outputs(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requests_scalar_path_candidate_projection(agent_run_context) {
        return None;
    }
    let mut path_batch_candidate = None;
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || !matches!(
                step.skill.as_str(),
                "fs_basic" | "system_basic" | "list_dir"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|output| !output.is_empty())
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        if let Some(candidates) = scalar_path_candidates_from_find_name(&value) {
            return Some((
                candidates.join("\n"),
                crate::task_journal::TaskJournalFinalizerSummary {
                    stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                    disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                    contract_ok: true,
                    completion_ok: Some(true),
                    grounded_ok: Some(true),
                    format_ok: Some(true),
                    needs_clarify: Some(false),
                    used_evidence_ids_count: 1,
                    ..Default::default()
                },
            ));
        }
        if path_batch_candidate.is_none() {
            path_batch_candidate = scalar_path_candidates_from_path_batch_facts(&value);
        }
    }
    path_batch_candidate.map(|candidates| {
        (
            candidates.join("\n"),
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        )
    })
}

pub(super) fn direct_file_token_from_observed_path_batch_facts(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requires_file_token(agent_run_context) {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "fs_basic" | "system_basic") {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|output| !output.is_empty())
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        let candidates = observed_path_batch_file_candidates(&value)?;
        if candidates.len() != 1 {
            return None;
        }
        return Some((
            format!("FILE:{}", candidates[0].display()),
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    None
}

pub(super) fn direct_file_token_from_observed_find_entries(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requires_file_token(agent_run_context) {
        return None;
    }
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || !matches!(step.skill.as_str(), "fs_basic" | "system_basic") {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|output| !output.is_empty())
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        let candidates = observed_find_entries_file_candidates(state, &value)?;
        if candidates.len() != 1 {
            return None;
        }
        return Some((
            format!("FILE:{}", candidates[0].display()),
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    None
}

pub(super) fn direct_file_token_from_observed_inventory(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requires_file_token(agent_run_context) {
        return None;
    }
    let malformed_placeholder_delivery =
        planned_file_delivery_used_unresolved_runtime_placeholder(loop_state);
    let planned_inventory_selection = planned_file_delivery_inventory_selection(loop_state);
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok()
            || !matches!(
                step.skill.as_str(),
                "fs_basic" | "system_basic" | "list_dir"
            )
        {
            continue;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|output| !output.is_empty())
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        let Some(candidates) = observed_inventory_file_candidates(&value) else {
            continue;
        };
        let selected = if candidates.len() == 1 {
            candidates.first()
        } else if planned_inventory_selection.is_some() && inventory_has_deterministic_order(&value)
        {
            match planned_inventory_selection? {
                PlannedInventorySelection::First => candidates.first(),
                PlannedInventorySelection::Last => candidates.last(),
            }
        } else if malformed_placeholder_delivery
            && inventory_ranked_for_single_file_selection(&value)
        {
            candidates.first()
        } else {
            None
        }?;
        return Some((
            format!("FILE:{}", selected.display()),
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    None
}
