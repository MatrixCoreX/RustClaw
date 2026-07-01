use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::AppState;

use super::{plan_step_for_execution, raw_command_arg_from_plan_step};

pub(super) fn looks_like_structured_machine_output(answer: &str) -> bool {
    let trimmed = answer.trim();
    serde_json::from_str::<serde_json::Value>(trimmed)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
        || looks_like_contract_evidence_projection(trimmed)
        || looks_like_structured_key_path_projection(trimmed)
}

fn looks_like_contract_evidence_projection(answer: &str) -> bool {
    let mut has_path = false;
    let mut has_evidence_field = false;
    for line in answer.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("path=") || line.starts_with("resolved_path=") {
            has_path = true;
            continue;
        }
        if matches!(
            line,
            "content_excerpt:" | "field_value:" | "command_output:" | "candidates:" | "results:"
        ) {
            has_evidence_field = true;
        }
    }
    has_path && has_evidence_field
}

fn looks_like_structured_key_path_projection(answer: &str) -> bool {
    let lines = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return false;
    }
    let mut assignment_count = 0usize;
    let mut key_path_assignment_count = 0usize;
    for line in &lines {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let key = key.trim();
        if key.is_empty()
            || value.trim().is_empty()
            || !key
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '[' | ']'))
        {
            return false;
        }
        assignment_count += 1;
        if key.contains('.') || key.contains('[') || key.contains(']') {
            key_path_assignment_count += 1;
        }
    }
    assignment_count == lines.len() && key_path_assignment_count > 0
}

pub(super) fn looks_like_raw_command_snapshot(answer: &str) -> bool {
    let trimmed = answer.trim();
    trimmed.starts_with("exit=")
        && trimmed.contains('\n')
        && (trimmed.contains("\nCOMMAND ")
            || trimmed.contains("(LISTEN)")
            || trimmed.contains("\nLISTEN ")
            || trimmed.contains("State  Recv-Q")
            || trimmed.contains("%CPU")
            || trimmed.contains("PID PPID"))
}

pub(super) fn direct_raw_command_output_projection(
    state: &AppState,
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let outputs = latest_run_cmd_output_group(loop_state);
    if outputs.is_empty() {
        if let Some((answer, used_evidence_ids_count)) =
            direct_read_range_raw_command_projection(state, route, loop_state)
        {
            return Some((
                answer,
                crate::task_journal::TaskJournalFinalizerSummary {
                    stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                    disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                    parsed: true,
                    contract_ok: true,
                    completion_ok: Some(true),
                    grounded_ok: Some(true),
                    format_ok: Some(true),
                    needs_clarify: Some(false),
                    used_evidence_ids_count,
                    ..Default::default()
                },
            ));
        }
        return None;
    }
    if let Some(path) = latest_run_cmd_redirect_existing_file_path(state, loop_state) {
        return Some((
            path,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                parsed: true,
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
    let projected = latest_raw_command_structural_projection(loop_state)
        .and_then(|projection| apply_raw_command_structural_projection(&outputs, &projection));
    let (answer, used_evidence_ids_count) = if let Some(answer) = projected {
        (answer, outputs.len())
    } else if let Some(answer) = collapse_identical_raw_command_outputs(&outputs) {
        (answer, 1)
    } else {
        (outputs.join("\n"), outputs.len())
    };
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count,
            ..Default::default()
        },
    ))
}

fn direct_read_range_raw_command_projection(
    state: &AppState,
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, usize)> {
    if !loop_state.executed_step_results.iter().rev().any(|step| {
        step.is_ok()
            && matches!(step.skill.as_str(), "fs_basic" | "system_basic")
            && step
                .output
                .as_deref()
                .is_some_and(output_contains_read_range_observation)
    }) {
        return None;
    }
    let agent_run_context = AgentRunContext {
        route_result: Some(route.clone()),
        ..Default::default()
    };
    let answer =
        crate::agent_engine::observed_output::extract_direct_answer_from_generic_output_i18n(
            loop_state,
            state,
            Some(&agent_run_context),
        )?;
    let answer = answer.trim_end().to_string();
    (!answer.trim().is_empty()).then_some((answer, 1))
}

fn output_contains_read_range_observation(output: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(output)
        .ok()
        .and_then(|value| read_range_action_value(&value))
        .is_some()
}

fn read_range_action_value(value: &serde_json::Value) -> Option<()> {
    let action = value
        .get("action")
        .and_then(serde_json::Value::as_str)
        .map(|action| action.trim().to_ascii_lowercase());
    if matches!(action.as_deref(), Some("read_range" | "read_text_range")) {
        return Some(());
    }
    if value
        .get("extra")
        .and_then(read_range_action_value)
        .is_some()
    {
        return Some(());
    }
    None
}

fn collapse_identical_raw_command_outputs(outputs: &[String]) -> Option<String> {
    let first = outputs.first()?.trim();
    if first.is_empty() {
        return None;
    }
    outputs
        .iter()
        .all(|output| output.trim() == first)
        .then(|| first.to_string())
}

fn latest_run_cmd_output_group(loop_state: &LoopState) -> Vec<String> {
    let mut outputs = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut seen_outputs = HashSet::new();
    let mut started = false;
    for step in loop_state.executed_step_results.iter().rev() {
        if matches!(
            step.skill.as_str(),
            "respond" | "synthesize_answer" | "think"
        ) {
            if started {
                break;
            }
            continue;
        }
        if step.skill == "run_cmd" {
            let Some(output) = step.output.as_deref().map(str::trim) else {
                if started {
                    break;
                }
                return Vec::new();
            };
            if !step.is_ok() || output.is_empty() {
                if started {
                    break;
                }
                return Vec::new();
            }
            started = true;
            let output_key = output.to_string();
            let command_key = plan_step_for_execution(loop_state, step)
                .and_then(|plan_step| raw_command_arg_from_plan_step(Some(plan_step)))
                .or_else(|| run_cmd_machine_command_from_output(output))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("command:{value}"))
                .unwrap_or_else(|| format!("step:{}", step.step_id));
            if !seen_keys.insert(command_key) || !seen_outputs.insert(output_key) {
                break;
            }
            outputs.push(output.to_string());
            continue;
        }
        if started {
            break;
        }
    }
    outputs.reverse();
    outputs
}

fn latest_run_cmd_redirect_existing_file_path(
    state: &AppState,
    loop_state: &LoopState,
) -> Option<String> {
    let workspace_root = state.skill_rt.workspace_root.canonicalize().ok()?;
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "run_cmd")
        .find_map(|step| {
            let output = step.output.as_deref().unwrap_or_default();
            let plan_command = plan_step_for_execution(loop_state, step)
                .and_then(|plan_step| raw_command_arg_from_plan_step(Some(plan_step)));
            plan_command
                .into_iter()
                .chain(run_cmd_machine_command_from_output(output))
                .find_map(|command| {
                    shell_stdout_redirect_target_path(command).and_then(|path| {
                        normalize_workspace_existing_file_path(&workspace_root, path)
                    })
                })
        })
}

fn run_cmd_machine_command_from_output(output: &str) -> Option<&str> {
    let line = output
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("exit=0 command="))?;
    line.strip_prefix("exit=0 command=")
        .map(str::trim)
        .filter(|command| !command.is_empty())
}

fn normalize_workspace_existing_file_path(workspace_root: &Path, path: PathBuf) -> Option<String> {
    let absolute = if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    };
    let canonical = absolute.canonicalize().ok()?;
    if !canonical.is_file() || canonical.strip_prefix(workspace_root).is_err() {
        return None;
    }
    Some(canonical.display().to_string())
}

pub(super) fn shell_stdout_redirect_target_path(command: &str) -> Option<PathBuf> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (idx, ch) in command.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_double && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if in_single || in_double || ch != '>' {
            continue;
        }
        let before = command[..idx].trim_end();
        if shell_redirect_is_non_stdout_fd(before) {
            continue;
        }
        let mut rest = &command[idx + ch.len_utf8()..];
        if rest.starts_with('>') {
            rest = &rest[1..];
        }
        let rest = rest.trim_start();
        if rest.starts_with('&') {
            continue;
        }
        if let Some(word) = parse_shell_redirect_word(rest) {
            return Some(PathBuf::from(word));
        }
    }
    None
}

fn shell_redirect_is_non_stdout_fd(before: &str) -> bool {
    let Some(last) = before.chars().last() else {
        return false;
    };
    if !last.is_ascii_digit() {
        return false;
    }
    if last == '1' {
        return false;
    }
    let prefix = &before[..before.len() - last.len_utf8()];
    prefix
        .chars()
        .last()
        .is_none_or(|ch| ch.is_whitespace() || matches!(ch, ';' | '|' | '&'))
}

fn parse_shell_redirect_word(rest: &str) -> Option<String> {
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    let (word, consumed_to_end) = if first == '\'' || first == '"' {
        parse_quoted_shell_word(rest, first)?
    } else {
        let end = rest
            .char_indices()
            .find_map(|(idx, ch)| {
                (idx > 0 && (ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>')))
                    .then_some(idx)
            })
            .unwrap_or(rest.len());
        (rest[..end].to_string(), end == rest.len())
    };
    let trimmed = word.trim();
    if trimmed.is_empty()
        || trimmed.contains('\0')
        || trimmed.contains('$')
        || trimmed.contains('`')
        || trimmed.contains('{')
        || trimmed.contains('}')
    {
        return None;
    }
    if !consumed_to_end {
        return Some(trimmed.to_string());
    }
    Some(trimmed.to_string())
}

fn parse_quoted_shell_word(rest: &str, quote: char) -> Option<(String, bool)> {
    let mut word = String::new();
    let mut escaped = false;
    for (idx, ch) in rest[quote.len_utf8()..].char_indices() {
        if escaped {
            word.push(ch);
            escaped = false;
            continue;
        }
        if quote == '"' && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote {
            let consumed = quote.len_utf8() + idx + ch.len_utf8();
            let trailing = rest[consumed..].trim_start();
            return Some((word, trailing.is_empty()));
        }
        word.push(ch);
    }
    None
}

#[derive(Debug, Clone, Default)]
struct RawCommandStructuralProjection {
    limit: Option<usize>,
    sort_by: Option<String>,
}

fn latest_raw_command_structural_projection(
    loop_state: &LoopState,
) -> Option<RawCommandStructuralProjection> {
    loop_state.round_traces.iter().rev().find_map(|round| {
        let plan = round.plan_result.as_ref()?;
        plan.steps
            .iter()
            .find_map(|step| raw_command_projection_from_value(&step.args))
            .or_else(|| {
                serde_json::from_str::<serde_json::Value>(&plan.raw_plan_text)
                    .ok()
                    .and_then(|value| raw_command_projection_from_value(&value))
            })
    })
}

fn raw_command_projection_from_value(
    value: &serde_json::Value,
) -> Option<RawCommandStructuralProjection> {
    match value {
        serde_json::Value::Object(map) => {
            let limit = map
                .get("max_entries")
                .or_else(|| map.get("max_results"))
                .or_else(|| map.get("limit"))
                .or_else(|| map.get("n"))
                .and_then(|value| value.as_u64())
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0);
            let sort_by = map
                .get("sort_by")
                .or_else(|| map.get("sort_order"))
                .or_else(|| map.get("order"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            if limit.is_some() || sort_by.is_some() {
                return Some(RawCommandStructuralProjection { limit, sort_by });
            }
            map.values().find_map(raw_command_projection_from_value)
        }
        serde_json::Value::Array(items) => items.iter().find_map(raw_command_projection_from_value),
        _ => None,
    }
}

fn apply_raw_command_structural_projection(
    outputs: &[String],
    projection: &RawCommandStructuralProjection,
) -> Option<String> {
    let mut lines = outputs
        .iter()
        .flat_map(|output| output.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    if let Some(sort_by) = projection.sort_by.as_deref() {
        let normalized = sort_by.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "name" | "name_asc" | "asc" => lines.sort(),
            "name_desc" | "desc" => {
                lines.sort();
                lines.reverse();
            }
            _ => {}
        }
    }
    if let Some(limit) = projection.limit {
        lines.truncate(limit);
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

pub(super) fn route_explicitly_requests_command_result(route: &crate::RouteResult) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route.output_contract.response_shape != crate::OutputResponseShape::Strict
}

pub(super) fn raw_command_output_needs_structural_projection(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput {
        return false;
    }
    let latest_is_run_cmd = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
        })
        .is_some_and(|step| step.skill == "run_cmd");
    latest_is_run_cmd && latest_plan_declares_structural_projection(loop_state)
}

fn latest_plan_declares_structural_projection(loop_state: &LoopState) -> bool {
    loop_state.round_traces.iter().rev().any(|round| {
        let Some(plan) = round.plan_result.as_ref() else {
            return false;
        };
        plan.steps
            .iter()
            .any(|step| value_declares_structural_projection(&step.args))
            || serde_json::from_str::<serde_json::Value>(&plan.raw_plan_text)
                .ok()
                .is_some_and(|value| value_declares_structural_projection(&value))
    })
}

fn value_declares_structural_projection(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            if map
                .get("max_entries")
                .or_else(|| map.get("max_results"))
                .or_else(|| map.get("limit"))
                .or_else(|| map.get("n"))
                .is_some_and(json_value_is_positive_number)
            {
                return true;
            }
            if map
                .get("sort_by")
                .or_else(|| map.get("sort_order"))
                .or_else(|| map.get("order"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .is_some_and(|text| !text.eq_ignore_ascii_case("name"))
            {
                return true;
            }
            if map
                .get("ext_filter")
                .or_else(|| map.get("exclude"))
                .or_else(|| map.get("exclude_names"))
                .or_else(|| map.get("exclude_patterns"))
                .is_some_and(json_value_is_non_empty)
            {
                return true;
            }
            if map
                .get("files_only")
                .or_else(|| map.get("dirs_only"))
                .is_some_and(|value| value.as_bool() == Some(true))
            {
                return true;
            }
            map.values().any(value_declares_structural_projection)
        }
        serde_json::Value::Array(items) => items.iter().any(value_declares_structural_projection),
        _ => false,
    }
}

fn json_value_is_positive_number(value: &serde_json::Value) -> bool {
    value.as_u64().is_some_and(|number| number > 0)
        || value.as_i64().is_some_and(|number| number > 0)
        || value.as_f64().is_some_and(|number| number > 0.0)
}

fn json_value_is_non_empty(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => !text.trim().is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(map) => !map.is_empty(),
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(_) => true,
        serde_json::Value::Null => false,
    }
}

pub(super) fn output_contract_requests_exact_delivery(route: &crate::RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) || matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::DirectoryEntryGroups
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::GitCommitSubject
            | crate::OutputSemanticKind::GitRepositoryState
            | crate::OutputSemanticKind::StructuredKeys
    ) || (route.output_contract.semantic_kind == crate::OutputSemanticKind::CommandOutputSummary
        && route.output_contract.response_shape == crate::OutputResponseShape::Strict)
}
