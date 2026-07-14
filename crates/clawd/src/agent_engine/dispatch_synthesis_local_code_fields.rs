use serde_json::Value;
use std::collections::BTreeMap;

use crate::agent_engine::LoopState;

#[derive(Debug, Clone)]
pub(super) struct RunCmdFailureProjection {
    pub(super) failed_command: String,
    pub(super) failure_evidence: Value,
    pub(super) fix_summary: Value,
}

pub(super) fn local_code_json_projection_field_value_supported(field: &str, value: &Value) -> bool {
    if field != "error_codes" {
        return true;
    }
    let values = string_or_array_values(value);
    !values.is_empty()
        && values
            .iter()
            .all(|value| machine_error_code_token(value.as_str()))
}

pub(super) fn run_cmd_commands_from_task_observations(loop_state: &LoopState) -> Vec<String> {
    let mut commands = Vec::new();
    for observation in &loop_state.task_observations {
        if observation.get("stage").and_then(Value::as_str) != Some("post_tool_use")
            || observation.get("tool_or_skill").and_then(Value::as_str) != Some("run_cmd")
            || observation
                .get("status")
                .or_else(|| observation.get("step_status"))
                .and_then(Value::as_str)
                != Some("ok")
        {
            continue;
        }
        let Some(command) = observation
            .get("args")
            .and_then(Value::as_object)
            .and_then(|args| args.get("command"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|command| !command.is_empty())
        else {
            continue;
        };
        commands.push(command.to_string());
    }
    commands
}

pub(super) fn current_request_surface(user_text: &str) -> &str {
    user_text
        .split_once("\n\n### ")
        .map(|(current, _)| current)
        .unwrap_or(user_text)
}

pub(super) fn run_cmd_failure_projection(
    loop_state: &LoopState,
    commands: &[String],
) -> Option<RunCmdFailureProjection> {
    let observations = run_cmd_projection_observations(loop_state, commands);
    let (failed_index, failed) = observations
        .iter()
        .enumerate()
        .find(|(_, observation)| observation.exit_code.is_some_and(|code| code != 0))?;
    let validation = observations
        .iter()
        .skip(failed_index + 1)
        .find(|observation| observation.exit_code == Some(0))?;

    let mut evidence = serde_json::Map::new();
    evidence.insert(
        "exit_code".to_string(),
        Value::Number(serde_json::Number::from(failed.exit_code?)),
    );
    evidence.insert(
        "output_excerpt".to_string(),
        Value::String(truncated_machine_excerpt(&failed.output, 800)),
    );
    evidence.insert(
        "output_chars".to_string(),
        Value::Number(serde_json::Number::from(
            failed.output.chars().count() as u64
        )),
    );

    let mut fix_summary = serde_json::Map::new();
    fix_summary.insert(
        "status_code".to_string(),
        Value::String("post_failure_validation_passed".to_string()),
    );
    fix_summary.insert(
        "validation_command".to_string(),
        Value::String(validation.command.clone()),
    );
    fix_summary.insert(
        "validation_exit_code".to_string(),
        Value::Number(serde_json::Number::from(0)),
    );

    Some(RunCmdFailureProjection {
        failed_command: failed.command.clone(),
        failure_evidence: Value::Object(evidence),
        fix_summary: Value::Object(fix_summary),
    })
}

pub(super) fn path_size_bytes_by_path(loop_state: &LoopState) -> BTreeMap<String, u64> {
    let mut sizes = BTreeMap::new();
    for step in &loop_state.executed_step_results {
        let Some(payload) = step.output.as_deref().and_then(super::skill_output_payload) else {
            continue;
        };
        collect_path_size_bytes_from_payload(&payload, &mut sizes);
    }
    sizes
}

pub(super) fn diff_summary_projection_value(
    readbacks: &[super::FsReadback],
    changed_paths: &[String],
    functions: &[String],
    error_codes: &[String],
    path_size_bytes: &BTreeMap<String, u64>,
) -> Option<Value> {
    if changed_paths.is_empty() {
        return None;
    }
    let mut items = Vec::new();
    for path in changed_paths {
        let mut item = serde_json::Map::new();
        item.insert("path".to_string(), Value::String(path.to_string()));
        item.insert(
            "summary_code".to_string(),
            Value::String(diff_summary_code_for_path(path).to_string()),
        );
        if let Some(size_bytes) = size_bytes_for_path(path_size_bytes, path) {
            item.insert(
                "size_bytes".to_string(),
                Value::Number(serde_json::Number::from(size_bytes)),
            );
        }
        if !super::path_looks_like_test_file(path) && !functions.is_empty() {
            item.insert(
                "functions".to_string(),
                Value::Array(functions.iter().cloned().map(Value::String).collect()),
            );
        }
        if !error_codes.is_empty() {
            item.insert(
                "error_codes".to_string(),
                Value::Array(error_codes.iter().cloned().map(Value::String).collect()),
            );
        }
        if readbacks
            .iter()
            .any(|readback| super::projection_paths_match(&readback.path, path))
        {
            item.insert("has_readback".to_string(), Value::Bool(true));
        }
        items.push(Value::Object(item));
    }
    Some(Value::Array(items))
}

pub(super) fn machine_error_code_token(value: &str) -> bool {
    let value = value.trim();
    if !super::machine_code_token(value) {
        return false;
    }
    !matches!(
        value.to_ascii_lowercase().as_str(),
        "error_code" | "ok" | "value" | "true" | "false" | "none" | "null"
    )
}

fn collect_path_size_bytes_from_payload(payload: &Value, out: &mut BTreeMap<String, u64>) {
    let Some(action) = payload.get("action").and_then(Value::as_str) else {
        return;
    };
    if action == "path_batch_facts" {
        if let Some(facts) = payload.get("facts").and_then(Value::as_array) {
            for item in facts {
                collect_path_size_bytes_from_fact(item, out);
            }
        }
    } else {
        collect_path_size_bytes_from_fact(payload, out);
    }
}

fn collect_path_size_bytes_from_fact(value: &Value, out: &mut BTreeMap<String, u64>) {
    let fact = value.get("fact").unwrap_or(value);
    let Some(size_bytes) = fact.get("size_bytes").and_then(Value::as_u64) else {
        return;
    };
    for path in [
        fact.get("resolved_path").and_then(Value::as_str),
        value.get("path").and_then(Value::as_str),
        fact.get("path").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    {
        let path = path.trim();
        if !path.is_empty() {
            out.insert(super::normalize_projection_path(path), size_bytes);
        }
    }
}

fn size_bytes_for_path(values: &BTreeMap<String, u64>, path: &str) -> Option<u64> {
    let normalized = super::normalize_projection_path(path);
    values.iter().find_map(|(candidate, size)| {
        super::projection_paths_match(candidate, &normalized).then_some(*size)
    })
}

fn diff_summary_code_for_path(path: &str) -> &'static str {
    if super::path_looks_like_test_file(path) {
        "test_file_updated"
    } else {
        "source_file_updated"
    }
}

fn string_or_array_values(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => vec![text.trim().to_string()],
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

struct RunCmdProjectionObservation {
    command: String,
    output: String,
    exit_code: Option<i64>,
}

fn run_cmd_projection_observations(
    loop_state: &LoopState,
    commands: &[String],
) -> Vec<RunCmdProjectionObservation> {
    let mut observations = Vec::new();
    let mut run_cmd_index = 0usize;
    for step in &loop_state.executed_step_results {
        if step.skill != "run_cmd" {
            continue;
        }
        let output = step
            .output
            .as_deref()
            .or(step.error.as_deref())
            .unwrap_or_default()
            .trim()
            .to_string();
        let command = commands
            .get(run_cmd_index)
            .map(String::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let exit_code = run_cmd_observed_exit_code(&output);
        observations.push(RunCmdProjectionObservation {
            command,
            output,
            exit_code,
        });
        run_cmd_index += 1;
    }
    observations
        .into_iter()
        .filter(|observation| {
            !observation.command.is_empty()
                && !observation.output.is_empty()
                && observation.exit_code.is_some()
        })
        .collect()
}

fn run_cmd_observed_exit_code(output: &str) -> Option<i64> {
    structured_run_cmd_exit_code(output).or_else(|| machine_exit_code_marker(output))
}

fn structured_run_cmd_exit_code(output: &str) -> Option<i64> {
    if let Some(value) = super::skill_output_payload(output) {
        if let Some(exit_code) = value.get("exit_code").and_then(Value::as_i64) {
            return Some(exit_code);
        }
        if let Some(exit_code) = value
            .get("extra")
            .and_then(|extra| extra.get("exit_code"))
            .and_then(Value::as_i64)
        {
            return Some(exit_code);
        }
    }
    let raw = output.trim().strip_prefix("__RC_SKILL_ERROR__:")?;
    let value = serde_json::from_str::<Value>(raw.trim()).ok()?;
    value
        .get("extra")
        .and_then(|extra| extra.get("exit_code"))
        .and_then(Value::as_i64)
        .or_else(|| value.get("exit_code").and_then(Value::as_i64))
}

fn machine_exit_code_marker(output: &str) -> Option<i64> {
    for line in output.lines() {
        let line = line.trim();
        for prefix in ["EXIT_CODE=", "EXIT_CODE:", "exit_code=", "exit_code:"] {
            let Some(raw) = line.strip_prefix(prefix) else {
                continue;
            };
            let token = raw.trim().trim_matches('"').trim_matches('\'');
            if let Ok(value) = token.parse::<i64>() {
                return Some(value);
            }
        }
    }
    None
}

fn truncated_machine_excerpt(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect()
}
