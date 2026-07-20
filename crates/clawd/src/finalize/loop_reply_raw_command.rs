use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

pub(super) fn looks_like_structured_machine_output(answer: &str) -> bool {
    let trimmed = answer.trim();
    serde_json::from_str::<serde_json::Value>(trimmed)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
        || looks_like_contract_evidence_projection(trimmed)
        || looks_like_structured_key_path_projection(trimmed)
        || looks_like_multiline_machine_field_projection(trimmed)
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

fn looks_like_multiline_machine_field_projection(answer: &str) -> bool {
    let lines = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }
    let mut anchored = false;
    for line in &lines {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let key = key.trim();
        let value = value.trim();
        if !valid_machine_projection_key(key) || value.is_empty() {
            return false;
        }
        if key.contains('.')
            || key.contains('[')
            || key.contains(']')
            || value.starts_with('{')
            || value.starts_with('[')
            || matches!(
                key,
                "async_cancel_adapter_result"
                    | "async_poll_adapter_result"
                    | "dry_run"
                    | "job_id"
                    | "model"
                    | "model_kind"
                    | "output_path"
                    | "planned_outputs"
                    | "provider"
                    | "status"
                    | "task_id"
            )
        {
            anchored = true;
        }
    }
    anchored
}

fn valid_machine_projection_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '[' | ']'))
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
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        || route.delivery_required
        || matches!(route.response_shape, crate::OutputResponseShape::FileToken)
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
    let requested_machine_fields = requested_raw_command_machine_fields(route);
    if !requested_machine_fields.is_empty() {
        let all_run_cmd_outputs = successful_run_cmd_outputs(loop_state);
        let mut used_evidence_ids_count = outputs.len();
        let mut answer = requested_raw_command_machine_field_projection_for_loop_state(
            state,
            &requested_machine_fields,
            loop_state,
            &outputs,
        );
        if answer.is_none() && all_run_cmd_outputs != outputs {
            answer = requested_raw_command_machine_field_projection_for_loop_state(
                state,
                &requested_machine_fields,
                loop_state,
                &all_run_cmd_outputs,
            );
            used_evidence_ids_count = all_run_cmd_outputs.len();
        }
        if let Some(answer) = answer {
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

pub(crate) fn raw_command_machine_field_delivery_satisfies_request(
    route: &crate::IntentOutputContract,
    delivery: &str,
) -> bool {
    let fields = requested_raw_command_machine_fields(route);
    if fields.is_empty() {
        return false;
    }
    fields
        .iter()
        .all(|field| raw_command_machine_field_value(delivery, field).is_some())
}

pub(crate) fn raw_command_machine_field_projection_from_journal(
    route: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if !route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        || route.delivery_required
        || matches!(route.response_shape, crate::OutputResponseShape::FileToken)
    {
        return None;
    }
    let fields = requested_raw_command_machine_fields(route);
    if fields.is_empty() {
        return None;
    }
    let outputs = successful_run_cmd_outputs_from_journal(journal);
    let answer = requested_raw_command_machine_field_projection_for_fields(&fields, &outputs)?;
    raw_command_machine_field_delivery_satisfies_request(route, &answer).then_some(answer)
}

pub(super) fn replace_final_delivery_with_raw_command_machine_field_projection(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let projected = direct_raw_command_output_projection(state, route, loop_state);
    let current = super::final_answer_text_from_delivery(delivery_messages);
    let answer = projected
        .as_ref()
        .and_then(|(answer, _)| {
            raw_command_machine_field_delivery_satisfies_request(route, answer)
                .then(|| answer.clone())
        })
        .or_else(|| normalize_raw_command_machine_field_delivery(route, &current));
    let Some(answer) = answer else {
        return false;
    };
    let summary = projected
        .map(|(_, summary)| summary)
        .unwrap_or_else(raw_command_machine_field_finalizer_summary);
    if delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer.trim())
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return false;
    }
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    super::log_deterministic_delivery_record(
        &task.task_id,
        "final_raw_command_machine_field_projection",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn normalize_raw_command_machine_field_delivery(
    route: &crate::IntentOutputContract,
    delivery: &str,
) -> Option<String> {
    let fields = requested_raw_command_machine_fields(route);
    if fields.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    for field in fields {
        match field.as_str() {
            "exit_code" => {
                let value = raw_command_machine_field_assignment_value(delivery, "exit_code")?;
                let value = value
                    .split_whitespace()
                    .next()
                    .filter(|value| value.chars().all(|ch| ch.is_ascii_digit()))?;
                lines.push(format!("exit_code={value}"));
            }
            "stdout_path" => {
                let value = raw_command_machine_field_assignment_value(delivery, "stdout_path")?
                    .split_whitespace()
                    .next()
                    .filter(|value| raw_command_stdout_value_is_path(value))?;
                lines.push(format!("stdout_path={value}"));
            }
            "stdout" => {
                let value = raw_command_machine_field_assignment_value(delivery, "stdout")?;
                lines.push(format!("stdout={}", value.trim()));
            }
            _ => return None,
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn raw_command_machine_field_assignment_value<'a>(
    delivery: &'a str,
    field: &str,
) -> Option<&'a str> {
    let equals_prefix = format!("{field}=");
    let colon_prefix = format!("{field}:");
    delivery.lines().map(str::trim).find_map(|line| {
        line.strip_prefix(equals_prefix.as_str())
            .or_else(|| line.strip_prefix(colon_prefix.as_str()))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn raw_command_machine_field_finalizer_summary() -> crate::task_journal::TaskJournalFinalizerSummary
{
    crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        ..Default::default()
    }
}

fn raw_command_machine_field_value<'a>(delivery: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{field}=");
    let value = delivery
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    match field {
        "exit_code" => value.chars().all(|ch| ch.is_ascii_digit()).then_some(value),
        "stdout_path" => (raw_command_stdout_value_is_path(value)
            && !value.contains(char::is_whitespace))
        .then_some(value),
        _ => Some(value),
    }
}

fn direct_read_range_raw_command_projection(
    state: &AppState,
    route: &crate::IntentOutputContract,
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
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let answer = crate::agent_engine::observed_output::extract_answer_from_observed_output_i18n(
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
            let command_key =
                crate::agent_engine::successful_run_cmd_command_for_step(loop_state, &step.step_id)
                    .as_deref()
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

fn successful_run_cmd_outputs(loop_state: &LoopState) -> Vec<String> {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "run_cmd")
        .filter_map(|step| step.output.as_deref().map(str::trim))
        .filter(|output| !output.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn successful_run_cmd_outputs_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter(|step| step.skill == "run_cmd")
        .filter_map(|step| step.output_excerpt.as_deref().map(str::trim))
        .filter(|output| !output.is_empty())
        .map(ToOwned::to_owned)
        .collect()
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
            let recorded_command =
                crate::agent_engine::successful_run_cmd_command_for_step(loop_state, &step.step_id);
            recorded_command
                .as_deref()
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

fn requested_raw_command_machine_field_projection_for_fields(
    fields: &[String],
    outputs: &[String],
) -> Option<String> {
    if outputs.is_empty() {
        return None;
    }
    let stdout = raw_command_stdout_value(outputs);
    let mut lines = Vec::new();
    for field in fields {
        match field.as_str() {
            "exit_code" => lines.push("exit_code=0".to_string()),
            "stdout" => {
                let value = stdout.as_deref()?;
                lines.push(format!("stdout={value}"));
            }
            "stdout_path" => {
                let value = stdout
                    .as_deref()
                    .filter(|value| raw_command_stdout_value_is_path(value))?;
                lines.push(format!("stdout_path={value}"));
            }
            "status" => lines.push("status=ok".to_string()),
            _ => return None,
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn requested_raw_command_machine_field_projection_for_loop_state(
    state: &AppState,
    fields: &[String],
    loop_state: &LoopState,
    outputs: &[String],
) -> Option<String> {
    if outputs.is_empty() {
        return None;
    }
    let stdout = raw_command_stdout_value(outputs);
    let command = latest_successful_run_cmd_command(loop_state);
    let created_path = latest_run_cmd_redirect_existing_file_path(state, loop_state);
    let mut lines = Vec::new();
    for field in fields {
        match field.as_str() {
            "command" => lines.push(format!("command={}", command.as_deref()?)),
            "created_path" => {
                lines.push(format!("created_path={}", created_path.as_deref()?));
            }
            "exit_code" => lines.push("exit_code=0".to_string()),
            "status" => lines.push("status=ok".to_string()),
            "stdout" => lines.push(format!("stdout={}", stdout.as_deref()?)),
            "stdout_path" => {
                let value = stdout
                    .as_deref()
                    .filter(|value| raw_command_stdout_value_is_path(value))?;
                lines.push(format!("stdout_path={value}"));
            }
            _ => return None,
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn latest_successful_run_cmd_command(loop_state: &LoopState) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "run_cmd")
        .find_map(|step| {
            let output = step.output.as_deref().unwrap_or_default();
            crate::agent_engine::successful_run_cmd_command_for_step(loop_state, &step.step_id)
                .as_deref()
                .or_else(|| run_cmd_machine_command_from_output(output))
                .map(str::trim)
                .filter(|command| {
                    !command.is_empty() && !command.contains('\n') && !command.contains('\r')
                })
                .map(ToOwned::to_owned)
        })
}

fn requested_raw_command_machine_fields(route: &crate::IntentOutputContract) -> Vec<String> {
    let mut fields = Vec::new();
    if let Some(selector) = route.selection.structured_field_selector.as_deref() {
        for token in selector.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')) {
            if matches!(
                token,
                "command" | "created_path" | "exit_code" | "status" | "stdout" | "stdout_path"
            ) && !fields.iter().any(|field| field == token)
            {
                fields.push(token.to_string());
            }
        }
    }
    if fields.iter().any(|field| field == "stdout_path") {
        fields.retain(|field| field != "stdout");
    }
    fields
}

fn raw_command_stdout_value(outputs: &[String]) -> Option<String> {
    let candidates = outputs
        .iter()
        .flat_map(|output| output.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !raw_command_line_is_execution_metadata(line))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    let unique_candidates = candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.clone()))
        .collect::<Vec<_>>();
    match unique_candidates.as_slice() {
        [value] => Some(value.clone()),
        _ => None,
    }
}

fn raw_command_line_is_execution_metadata(line: &str) -> bool {
    line.starts_with("exit=")
        || line.starts_with("EXIT=")
        || line.starts_with("exit_code=")
        || line.starts_with("stdout_path=")
        || line.starts_with("command=")
        || line.starts_with("detached=")
        || line == "---"
}

fn raw_command_stdout_value_is_path(value: &str) -> bool {
    !value.contains('\n') && Path::new(value).is_absolute()
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

pub(super) fn route_explicitly_requests_command_result(
    route: &crate::IntentOutputContract,
) -> bool {
    route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && route.response_shape != crate::OutputResponseShape::Strict
}

pub(super) fn raw_command_output_needs_structural_projection(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    if !route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput) {
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
    latest_is_run_cmd
        && (!requested_raw_command_machine_fields(route).is_empty()
            || latest_plan_declares_structural_projection(loop_state))
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

pub(super) fn output_contract_requests_exact_delivery(route: &crate::IntentOutputContract) -> bool {
    route.requests_exact_name_list()
        || matches!(
            route.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || route.semantic_kind_is_any(&[
            crate::OutputSemanticKind::RawCommandOutput,
            crate::OutputSemanticKind::FilePaths,
        ])
}
