use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::AgentAction;

use super::dispatch_synthesis_local_code_fields::{
    diff_summary_projection_value, local_code_json_projection_field_value_supported,
    machine_error_code_token, path_size_bytes_by_path, run_cmd_commands_from_task_observations,
    run_cmd_failure_projection,
};
use super::dispatch_synthesis_local_code_readbacks::readbacks_for_local_code_projection;
use super::dispatch_synthesis_local_code_writes::{
    successful_fs_changed_write_paths, successful_fs_write_paths,
    successful_fs_write_readbacks_from_plan_trace,
};
use super::{push_unique_string, skill_output_payload};
use crate::read_range_utils::strip_read_range_line_prefix;

pub(in crate::agent_engine) fn local_code_task_strict_json_projection(
    _user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let requested_fields = requested_local_code_json_fields(loop_state, agent_run_context);
    if requested_fields.is_empty() {
        return None;
    }
    if !local_code_task_projection_allowed(loop_state, agent_run_context, &requested_fields) {
        return None;
    }

    let current_write_paths = successful_fs_write_paths(loop_state);
    let changed_write_paths = successful_fs_changed_write_paths(loop_state);
    let has_current_writes = !current_write_paths.is_empty();
    let mut readbacks = readbacks_for_local_code_projection(loop_state, &current_write_paths);
    for readback in successful_fs_write_readbacks_from_plan_trace(loop_state, &current_write_paths)
    {
        if !readbacks
            .iter()
            .any(|existing| projection_paths_match(&existing.path, &readback.path))
        {
            readbacks.push(readback);
        }
    }
    let evidence_paths = if has_current_writes {
        current_write_paths.clone()
    } else {
        paths_from_readbacks(&readbacks)
    };
    if evidence_paths.is_empty() {
        return None;
    }
    let latest_run_cmd_ok = latest_successful_run_cmd_observed(loop_state);
    let run_cmd_commands = successful_run_cmd_commands(loop_state);
    let functions = functions_from_readbacks(&readbacks, &evidence_paths);
    let error_codes = error_codes_from_readbacks(&readbacks);
    let projected_changed_paths = if has_current_writes {
        changed_write_paths.clone()
    } else {
        evidence_paths.clone()
    };
    let path_size_bytes = path_size_bytes_by_path(loop_state);
    let failure_projection = run_cmd_failure_projection(loop_state, &run_cmd_commands);

    let mut object = serde_json::Map::new();
    for field in &requested_fields {
        match field.as_str() {
            "created_files" => {
                if !has_current_writes {
                    return None;
                }
                object.insert(field.clone(), string_array_value(&evidence_paths));
            }
            "changed_files" => {
                object.insert(field.clone(), string_array_value(&projected_changed_paths));
            }
            "failed_command" => {
                object.insert(
                    field.clone(),
                    Value::String(failure_projection.as_ref()?.failed_command.clone()),
                );
            }
            "failure_observed" => {
                object.insert(field.clone(), Value::Bool(failure_projection.is_some()));
            }
            "failure_evidence" => {
                object.insert(
                    field.clone(),
                    failure_projection.as_ref()?.failure_evidence.clone(),
                );
            }
            "fix_summary" => {
                object.insert(
                    field.clone(),
                    failure_projection.as_ref()?.fix_summary.clone(),
                );
            }
            "test_command" | "verification_command" => {
                object.insert(
                    field.clone(),
                    test_command_projection_value(&run_cmd_commands)?,
                );
            }
            "test_status" => {
                if !latest_run_cmd_ok {
                    return None;
                }
                object.insert(field.clone(), Value::String("passed".to_string()));
            }
            "functions" => {
                if functions.is_empty() {
                    return None;
                }
                object.insert(field.clone(), string_array_value(&functions));
            }
            "error_codes" => {
                if error_codes.is_empty() {
                    return None;
                }
                object.insert(field.clone(), string_array_value(&error_codes));
            }
            "evidence_files" => {
                let evidence_files = readbacks
                    .iter()
                    .map(|readback| readback.path.as_str())
                    .collect::<Vec<_>>();
                if evidence_files.is_empty() {
                    return None;
                }
                object.insert(field.clone(), string_array_value(&evidence_files));
            }
            "project_dir" => {
                object.insert(
                    field.clone(),
                    Value::String(common_parent_path(&evidence_paths)?),
                );
            }
            "diff_summary" => {
                object.insert(
                    field.clone(),
                    diff_summary_projection_value(
                        &readbacks,
                        &projected_changed_paths,
                        &functions,
                        &error_codes,
                        &path_size_bytes,
                    )?,
                );
            }
            _ => return None,
        }
    }

    (!object.is_empty()).then(|| Value::Object(object).to_string())
}

pub(in crate::agent_engine) fn strict_json_projection_answer_satisfies_request(
    _user_text: &str,
    answer: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let requested_fields = requested_local_code_json_fields(loop_state, agent_run_context);
    if requested_fields.is_empty() {
        return false;
    }
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(answer.trim()) else {
        return false;
    };
    !object.is_empty()
        && object.len() <= requested_fields.len()
        && object.iter().all(|(key, value)| {
            requested_fields.iter().any(|field| field == key)
                && local_code_json_projection_field_supported(key)
                && local_code_json_projection_field_value_supported(key, value)
                && json_projection_value_has_payload(value)
        })
        && requested_fields.iter().all(|field| {
            object
                .get(field)
                .is_some_and(json_projection_value_has_payload)
        })
}

fn local_code_task_projection_allowed(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    requested_fields: &[String],
) -> bool {
    let Some(route) = loop_state
        .output_contract
        .as_ref()
        .or_else(|| agent_run_context.and_then(|context| context.output_contract()))
    else {
        return true;
    };
    if !matches!(route.response_shape, crate::OutputResponseShape::FileToken) {
        return true;
    }
    let local_code_route = route.locator_kind == crate::OutputLocatorKind::CurrentWorkspace;
    local_code_route
        && requested_fields
            .iter()
            .any(|field| local_code_json_projection_field_supported(field))
}

pub(in crate::agent_engine) fn requested_local_code_json_fields(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Vec<String> {
    if let Some(selector) = loop_state
        .output_contract
        .as_ref()
        .or_else(|| agent_run_context.and_then(|context| context.output_contract()))
        .and_then(|contract| contract.selection.structured_field_selector.as_deref())
    {
        let fields = requested_local_code_json_fields_from_selector(selector);
        if !fields.is_empty() {
            return fields;
        }
    }

    if let Some(state_patch) = agent_run_context
        .and_then(|context| context.turn_analysis.as_ref())
        .and_then(|analysis| analysis.state_patch.as_ref())
    {
        let mut structured_surfaces = Vec::new();
        crate::machine_kv_projection::collect_requested_machine_kv_surfaces_from_state_patch(
            state_patch,
            &mut structured_surfaces,
        );
        for surface in structured_surfaces {
            let fields = requested_local_code_json_fields_from_surface(&surface);
            if !fields.is_empty() {
                return fields;
            }
        }
    }
    Vec::new()
}

fn requested_local_code_json_fields_from_selector(selector: &str) -> Vec<String> {
    let fields = selector
        .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, ',' | ';'))
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if fields.is_empty()
        || fields
            .iter()
            .any(|field| !local_code_json_projection_field_supported(field))
    {
        return Vec::new();
    }
    fields
}

fn requested_local_code_json_fields_from_surface(surface: &str) -> Vec<String> {
    local_code_json_field_segments(surface)
        .into_iter()
        .rev()
        .find_map(|segment| {
            let fields =
                crate::machine_kv_projection::requested_machine_markers_for_projection(&segment);
            (!fields.is_empty()
                && fields
                    .iter()
                    .all(|field| local_code_json_projection_field_supported(field)))
            .then_some(fields)
        })
        .unwrap_or_default()
}

fn local_code_json_field_segments(surface: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut start = 0usize;
    for (index, ch) in surface.char_indices() {
        if !local_code_json_field_surface_boundary(surface, index, ch) {
            continue;
        }
        let segment = surface[start..index].trim();
        if !segment.is_empty() {
            segments.push(segment.to_string());
        }
        start = index + ch.len_utf8();
    }
    let segment = surface[start..].trim();
    if !segment.is_empty() {
        segments.push(segment.to_string());
    }
    segments
}

fn local_code_json_field_surface_boundary(surface: &str, index: usize, ch: char) -> bool {
    if matches!(
        ch,
        '\n' | '\r' | '。' | ';' | '；' | '!' | '?' | '！' | '？'
    ) {
        return true;
    }
    ch == '.'
        && surface[index + ch.len_utf8()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
}

fn local_code_json_projection_field_supported(field: &str) -> bool {
    matches!(
        field,
        "created_files"
            | "changed_files"
            | "failed_command"
            | "failure_observed"
            | "failure_evidence"
            | "fix_summary"
            | "test_command"
            | "verification_command"
            | "test_status"
            | "functions"
            | "error_codes"
            | "evidence_files"
            | "project_dir"
            | "diff_summary"
    )
}

#[derive(Debug, Clone)]
pub(super) struct FsReadback {
    pub(super) path: String,
    pub(super) excerpt: String,
}

pub(super) fn successful_fs_readbacks_after_latest_writes(
    loop_state: &LoopState,
    write_paths: &[String],
) -> Vec<FsReadback> {
    let mut latest_write_index_by_path = BTreeMap::<String, usize>::new();
    for (index, step) in loop_state.executed_step_results.iter().enumerate() {
        if !step.is_ok() || !filesystem_projection_skill(&step.skill) {
            continue;
        }
        let Some(payload) = step.output.as_deref().and_then(skill_output_payload) else {
            continue;
        };
        if !matches!(
            payload.get("action").and_then(Value::as_str),
            Some("write_text" | "append_text")
        ) {
            continue;
        }
        let Some(path) = payload_path(&payload) else {
            continue;
        };
        latest_write_index_by_path.insert(normalize_projection_path(&path), index);
    }

    let mut readbacks = Vec::new();
    for (index, step) in loop_state.executed_step_results.iter().enumerate() {
        if !step.is_ok() || !filesystem_projection_skill(&step.skill) {
            continue;
        }
        let Some(payload) = step.output.as_deref().and_then(skill_output_payload) else {
            continue;
        };
        if !matches!(
            payload.get("action").and_then(Value::as_str),
            Some("read_range" | "read_text_range")
        ) {
            continue;
        }
        let Some(path) = payload_path(&payload) else {
            continue;
        };
        if !write_paths.iter().any(|write_path| {
            projection_paths_match(&path, write_path)
                && latest_write_index_by_path
                    .get(&normalize_projection_path(write_path))
                    .is_some_and(|write_index| index > *write_index)
        }) {
            continue;
        }
        let Some(excerpt) = payload
            .get("excerpt")
            .or_else(|| payload.get("content"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|excerpt| !excerpt.is_empty())
        else {
            continue;
        };
        readbacks.push(FsReadback {
            path,
            excerpt: excerpt.to_string(),
        });
    }
    readbacks
}

pub(super) fn successful_code_readbacks(loop_state: &LoopState) -> Vec<FsReadback> {
    let mut readbacks = Vec::new();
    for step in &loop_state.executed_step_results {
        if !step.is_ok() || !filesystem_projection_skill(&step.skill) {
            continue;
        }
        let Some(payload) = step.output.as_deref().and_then(skill_output_payload) else {
            continue;
        };
        if !matches!(
            payload.get("action").and_then(Value::as_str),
            Some("read_range" | "read_text_range")
        ) {
            continue;
        }
        let Some(path) = payload_path(&payload) else {
            continue;
        };
        if !path_looks_like_code_or_test_file(&path) {
            continue;
        }
        let Some(excerpt) = payload
            .get("excerpt")
            .or_else(|| payload.get("content"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|excerpt| !excerpt.is_empty())
        else {
            continue;
        };
        readbacks.push(FsReadback {
            path,
            excerpt: excerpt.to_string(),
        });
    }
    readbacks
}

fn paths_from_readbacks(readbacks: &[FsReadback]) -> Vec<String> {
    let mut paths = Vec::new();
    for readback in readbacks {
        push_unique_string(&mut paths, &readback.path);
    }
    paths
}

pub(super) fn filesystem_projection_skill(skill: &str) -> bool {
    matches!(skill, "fs_basic" | "system_basic" | "write_file")
}

fn payload_path(payload: &Value) -> Option<String> {
    payload
        .get("resolved_path")
        .or_else(|| payload.get("effective_path"))
        .or_else(|| payload.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

pub(super) fn normalize_projection_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

pub(super) fn projection_paths_match(candidate: &str, expected: &str) -> bool {
    let candidate = normalize_projection_path(candidate);
    let expected = normalize_projection_path(expected);
    if candidate == expected {
        return true;
    }
    let (shorter, longer) = if candidate.len() <= expected.len() {
        (candidate.as_str(), expected.as_str())
    } else {
        (expected.as_str(), candidate.as_str())
    };
    !shorter.starts_with('/') && !shorter.is_empty() && longer.ends_with(&format!("/{shorter}"))
}

fn latest_successful_run_cmd_observed(loop_state: &LoopState) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .any(|step| step.is_ok() && step.skill == "run_cmd")
}

fn successful_run_cmd_commands(loop_state: &LoopState) -> Vec<String> {
    let successful_count = successful_run_cmd_count(loop_state);
    if successful_count == 0 {
        return Vec::new();
    }
    let mut commands = loop_state
        .output_vars
        .get("agent_loop.run_cmd_commands")
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|command| command.trim().to_string())
        .filter(|command| !command.is_empty())
        .collect::<Vec<_>>();
    if commands.is_empty() {
        if let Some(command) = loop_state
            .output_vars
            .get("agent_loop.latest_run_cmd_command")
            .map(String::as_str)
            .map(str::trim)
            .filter(|command| !command.is_empty())
        {
            commands.push(command.to_string());
        }
    }
    if commands.len() < successful_count {
        let plan_commands = run_cmd_commands_from_plan_trace(loop_state);
        if plan_commands.len() >= successful_count {
            commands = plan_commands;
        } else {
            for command in plan_commands {
                push_unique_string(&mut commands, &command);
            }
        }
    }
    if commands.len() < successful_count {
        for command in run_cmd_commands_from_task_observations(loop_state) {
            push_unique_string(&mut commands, &command);
        }
    }
    if commands.len() > successful_count {
        commands.truncate(successful_count);
    }
    commands
}

fn successful_run_cmd_count(loop_state: &LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "run_cmd")
        .count()
}

fn run_cmd_commands_from_plan_trace(loop_state: &LoopState) -> Vec<String> {
    let mut commands = Vec::new();
    for round in &loop_state.round_traces {
        let Some(plan) = round.plan_result.as_ref() else {
            continue;
        };
        for action in plan
            .steps
            .iter()
            .filter_map(crate::PlanStep::to_agent_action)
        {
            let Some(command) = run_cmd_command_from_action(&action) else {
                continue;
            };
            commands.push(command);
        }
    }
    commands
}

fn run_cmd_command_from_action(action: &AgentAction) -> Option<String> {
    let (tool_or_skill, args) = match action {
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => return None,
    };
    if !machine_ref_is_run_cmd(tool_or_skill) {
        return None;
    }
    args.get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
}

fn machine_ref_is_run_cmd(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    matches!(normalized.as_str(), "run_cmd" | "process_basic")
}

fn test_command_projection_value(commands: &[String]) -> Option<Value> {
    match commands {
        [] => None,
        [command] => Some(Value::String(command.clone())),
        _ => Some(Value::Array(
            commands.iter().cloned().map(Value::String).collect(),
        )),
    }
}

fn functions_from_readbacks(readbacks: &[FsReadback], write_paths: &[String]) -> Vec<String> {
    let primary_readbacks = readbacks
        .iter()
        .filter(|readback| !path_looks_like_test_file(&readback.path))
        .collect::<Vec<_>>();
    let source_readbacks = if primary_readbacks.is_empty() {
        readbacks.iter().collect::<Vec<_>>()
    } else {
        primary_readbacks
    };
    let mut source_functions = Vec::new();
    for line in source_readbacks
        .into_iter()
        .flat_map(|readback| readback.excerpt.lines())
    {
        if let Some(name) = function_name_from_code_line(strip_read_range_line_prefix(line)) {
            push_unique_string(&mut source_functions, name);
        }
    }

    let test_import_functions = functions_from_test_import_readbacks(readbacks, write_paths);
    if test_import_functions.len() > source_functions.len() {
        let mut functions = test_import_functions;
        for function in source_functions {
            push_unique_string(&mut functions, &function);
        }
        return functions;
    }

    let mut functions = source_functions;
    for function in test_import_functions {
        push_unique_string(&mut functions, &function);
    }
    functions
}

fn functions_from_test_import_readbacks(
    readbacks: &[FsReadback],
    write_paths: &[String],
) -> Vec<String> {
    let source_modules = source_module_names_from_write_paths(write_paths);
    if source_modules.is_empty() {
        return Vec::new();
    }

    let mut functions = Vec::new();
    for line in readbacks
        .iter()
        .filter(|readback| path_looks_like_test_file(&readback.path))
        .flat_map(|readback| readback.excerpt.lines())
    {
        for name in python_from_import_names_for_written_module(
            strip_read_range_line_prefix(line),
            &source_modules,
        ) {
            push_unique_string(&mut functions, &name);
        }
    }
    functions
}

fn source_module_names_from_write_paths(write_paths: &[String]) -> BTreeSet<String> {
    let mut modules = BTreeSet::new();
    for path in write_paths {
        if path_looks_like_test_file(path) {
            continue;
        }
        let normalized = normalize_projection_path(path);
        let Some(stem) = Path::new(&normalized)
            .file_stem()
            .and_then(|stem| stem.to_str())
        else {
            continue;
        };
        let stem = stem.trim();
        if !stem.is_empty()
            && stem
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            && stem.chars().any(|ch| ch.is_ascii_alphabetic() || ch == '_')
        {
            modules.insert(stem.to_string());
        }
    }
    modules
}

fn python_from_import_names_for_written_module(
    line: &str,
    source_modules: &BTreeSet<String>,
) -> Vec<String> {
    let line = line.trim();
    let Some(rest) = line.strip_prefix("from ") else {
        return Vec::new();
    };
    let Some((module, imports)) = rest.split_once(" import ") else {
        return Vec::new();
    };
    let module = module.trim();
    let module_leaf = module.rsplit('.').next().unwrap_or(module).trim();
    if !source_modules.contains(module_leaf) {
        return Vec::new();
    }

    let mut names = Vec::new();
    for part in imports.split(',') {
        let name = part
            .trim()
            .split_ascii_whitespace()
            .next()
            .unwrap_or_default()
            .trim();
        if function_identifier_token(name) {
            push_unique_string(&mut names, name);
        }
    }
    names
}

pub(super) fn path_looks_like_test_file(path: &str) -> bool {
    let path = normalize_projection_path(path);
    let basename = Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path.as_str())
        .to_ascii_lowercase();
    basename.starts_with("test_")
        || basename.ends_with("_test.py")
        || basename.ends_with(".test.js")
        || basename.ends_with(".spec.js")
        || basename.ends_with(".test.ts")
        || basename.ends_with(".spec.ts")
        || basename.ends_with("_test.rs")
}

pub(super) fn path_looks_like_code_or_test_file(path: &str) -> bool {
    if path_looks_like_test_file(path) {
        return true;
    }
    let path = normalize_projection_path(path);
    let extension = Path::new(&path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        extension.as_str(),
        "py" | "rs"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "go"
            | "java"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
            | "swift"
            | "kt"
            | "scala"
            | "sh"
    )
}

fn function_name_from_code_line(line: &str) -> Option<&str> {
    let line = line.trim_start();
    for prefix in ["def ", "fn ", "function "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return identifier_prefix(rest);
        }
    }
    None
}

fn identifier_prefix(value: &str) -> Option<&str> {
    let end = value
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '_')
        .map(|(idx, ch)| idx + ch.len_utf8())
        .last()?;
    let ident = value.get(..end)?.trim();
    (!ident.is_empty() && ident.chars().any(|ch| ch.is_ascii_alphabetic())).then_some(ident)
}

fn function_identifier_token(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && value
            .chars()
            .any(|ch| ch.is_ascii_alphabetic() || ch == '_')
}

fn error_codes_from_readbacks(readbacks: &[FsReadback]) -> Vec<String> {
    let mut codes = Vec::new();
    for readback in readbacks {
        for (idx, _) in readback.excerpt.match_indices("error_code") {
            let tail = &readback.excerpt[idx + "error_code".len()..];
            for value in quoted_machine_strings(tail)
                .into_iter()
                .chain(machine_code_tokens(tail))
                .take(8)
            {
                if machine_error_code_token(&value) {
                    push_unique_string(&mut codes, &value);
                    break;
                }
            }
        }
    }
    codes
}

fn quoted_machine_strings(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut start: Option<(usize, char)> = None;
    for (idx, ch) in text.char_indices() {
        if !matches!(ch, '\'' | '"') {
            continue;
        }
        if let Some((start_idx, quote)) = start {
            if ch == quote {
                if let Some(value) = text.get(start_idx + quote.len_utf8()..idx) {
                    values.push(value.to_string());
                }
                start = None;
            }
        } else {
            start = Some((idx, ch));
        }
    }
    values
}

fn machine_code_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .filter(|token| !matches!(*token, "error_code" | "ok" | "true" | "false" | "return"))
        .map(str::to_string)
        .collect()
}

pub(super) fn machine_code_token(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 96
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && value
            .chars()
            .any(|ch| ch.is_ascii_alphabetic() || ch == '_')
}

fn string_array_value(values: &[impl AsRef<str>]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| Value::String(value.as_ref().to_string()))
            .collect(),
    )
}

pub(super) fn common_parent_path(paths: &[String]) -> Option<String> {
    let mut parents = paths
        .iter()
        .filter_map(|path| Path::new(path).parent())
        .map(|path| path.display().to_string());
    let first = parents.next()?;
    parents.all(|parent| parent == first).then_some(first)
}

fn json_projection_value_has_payload(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => json_projection_string_has_payload(text),
        Value::Array(items) => {
            !items.is_empty() && items.iter().all(json_projection_value_has_payload)
        }
        Value::Object(object) => {
            !object.is_empty() && object.values().all(json_projection_value_has_payload)
        }
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn json_projection_string_has_payload(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && !trimmed.contains("{{")
        && !matches!(trimmed, "<missing>" | "not_observed" | "null")
}
