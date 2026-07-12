use serde_json::Value;

use crate::agent_engine::{AgentRunContext, LoopState};

pub(super) const LOCAL_CODE_PROJECTION_PENDING_READBACK: &str =
    "local_code_strict_json_projection_pending_readback";
pub(super) const LOCAL_CODE_PROJECTION_PENDING_VALIDATION: &str =
    "local_code_strict_json_projection_pending_validation";

pub(super) fn local_code_strict_json_projection_should_defer_observed_synthesis(
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    if super::local_code_strict_json_projection_from_machine_evidence(
        user_text,
        loop_state,
        agent_run_context,
    )
    .is_some()
    {
        return false;
    }
    if !local_code_strict_json_requested(agent_run_context, user_text) {
        return false;
    }
    if !agent_run_context
        .and_then(|context| context.route_result.as_ref())
        .is_none_or(|route| {
            route.output_contract.response_shape != crate::OutputResponseShape::FileToken
        })
    {
        return false;
    }
    local_code_has_successful_validation(loop_state)
        && local_code_has_post_write_readback_gap(loop_state)
}

pub(super) fn local_code_strict_json_projection_should_defer_until_validation(
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    if super::local_code_strict_json_projection_from_machine_evidence(
        user_text,
        loop_state,
        agent_run_context,
    )
    .is_some()
    {
        return false;
    }
    if !local_code_validation_field_requested(agent_run_context, user_text) {
        return false;
    }
    if !agent_run_context
        .and_then(|context| context.route_result.as_ref())
        .is_none_or(|route| {
            route.output_contract.response_shape != crate::OutputResponseShape::FileToken
        })
    {
        return false;
    }
    !local_code_has_successful_validation(loop_state)
        && (local_code_has_code_or_test_readback(loop_state)
            || local_code_has_code_or_test_write(loop_state))
}

fn local_code_strict_json_requested(
    agent_run_context: Option<&AgentRunContext>,
    user_text: &str,
) -> bool {
    local_code_requested_fields(agent_run_context, user_text)
        .iter()
        .any(|field| local_code_json_projection_field_supported(field))
}

fn local_code_validation_field_requested(
    agent_run_context: Option<&AgentRunContext>,
    user_text: &str,
) -> bool {
    local_code_requested_fields(agent_run_context, user_text)
        .iter()
        .any(|field| matches!(field.as_str(), "test_command" | "test_status"))
}

fn local_code_requested_fields(
    agent_run_context: Option<&AgentRunContext>,
    user_text: &str,
) -> Vec<String> {
    let mut surfaces = Vec::new();
    if let Some(context) = agent_run_context {
        if let Some(state_patch) = context
            .turn_analysis
            .as_ref()
            .and_then(|analysis| analysis.state_patch.as_ref())
        {
            crate::machine_kv_projection::collect_requested_machine_kv_surfaces_from_state_patch(
                state_patch,
                &mut surfaces,
            );
        }
        for value in [
            context.original_user_request.as_deref(),
            context.user_request.as_deref(),
            context
                .route_result
                .as_ref()
                .map(|route| route.resolved_intent.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, value);
        }
    }
    crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, user_text);
    let mut fields = Vec::new();
    for field in surfaces.iter().flat_map(|surface| {
        crate::machine_kv_projection::requested_machine_markers_for_projection(surface)
    }) {
        if !fields.iter().any(|existing| existing == &field) {
            fields.push(field);
        }
    }
    fields
}

fn local_code_json_projection_field_supported(field: &str) -> bool {
    matches!(
        field,
        "created_files"
            | "changed_files"
            | "test_command"
            | "test_status"
            | "functions"
            | "error_codes"
            | "evidence_files"
            | "project_dir"
    )
}

fn local_code_has_successful_validation(loop_state: &LoopState) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .any(|step| step.is_ok() && matches!(step.skill.as_str(), "run_cmd" | "process_basic"))
}

fn local_code_has_code_or_test_readback(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().any(|step| {
        if !step.is_ok() || !local_code_filesystem_skill(&step.skill) {
            return false;
        }
        let Some(payload) = step
            .output
            .as_deref()
            .and_then(local_code_skill_output_payload)
        else {
            return false;
        };
        if !matches!(
            payload.get("action").and_then(Value::as_str),
            Some("read_range" | "read_text_range" | "read_text")
        ) {
            return false;
        }
        let Some(path) = local_code_payload_path(&payload) else {
            return false;
        };
        local_code_path_looks_like_code_or_test(&path)
            && payload
                .get("excerpt")
                .or_else(|| payload.get("content"))
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|content| !content.is_empty())
    })
}

fn local_code_has_code_or_test_write(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().any(|step| {
        if !step.is_ok() || !local_code_filesystem_skill(&step.skill) {
            return false;
        }
        let Some(payload) = step
            .output
            .as_deref()
            .and_then(local_code_skill_output_payload)
        else {
            return false;
        };
        if !matches!(
            payload.get("action").and_then(Value::as_str),
            Some("write_text" | "append_text")
        ) {
            return false;
        }
        local_code_payload_path(&payload)
            .is_some_and(|path| local_code_path_looks_like_code_or_test(&path))
    })
}

fn local_code_has_post_write_readback_gap(loop_state: &LoopState) -> bool {
    let mut latest_write_by_path = std::collections::BTreeMap::<String, usize>::new();
    for (index, step) in loop_state.executed_step_results.iter().enumerate() {
        if !step.is_ok() || !local_code_filesystem_skill(&step.skill) {
            continue;
        }
        let Some(payload) = step
            .output
            .as_deref()
            .and_then(local_code_skill_output_payload)
        else {
            continue;
        };
        if !matches!(
            payload.get("action").and_then(Value::as_str),
            Some("write_text" | "append_text")
        ) {
            continue;
        }
        let Some(path) = local_code_payload_path(&payload) else {
            continue;
        };
        if local_code_path_looks_like_code_or_test(&path) {
            latest_write_by_path.insert(local_code_normalize_path(&path), index);
        }
    }
    latest_write_by_path.iter().any(|(path, write_index)| {
        !local_code_has_readback_after_write(loop_state, path, *write_index)
    })
}

fn local_code_has_readback_after_write(
    loop_state: &LoopState,
    expected_path: &str,
    write_index: usize,
) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .skip(write_index + 1)
        .any(|(_, step)| {
            if !step.is_ok() || !local_code_filesystem_skill(&step.skill) {
                return false;
            }
            let Some(payload) = step
                .output
                .as_deref()
                .and_then(local_code_skill_output_payload)
            else {
                return false;
            };
            if !matches!(
                payload.get("action").and_then(Value::as_str),
                Some("read_range" | "read_text_range" | "read_text")
            ) {
                return false;
            }
            let Some(path) = local_code_payload_path(&payload) else {
                return false;
            };
            local_code_paths_match(&path, expected_path)
                && payload
                    .get("excerpt")
                    .or_else(|| payload.get("content"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_some_and(|content| !content.is_empty())
        })
}

fn local_code_filesystem_skill(skill: &str) -> bool {
    matches!(skill, "fs_basic" | "system_basic" | "write_file")
}

fn local_code_skill_output_payload(output: &str) -> Option<Value> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    value
        .get("extra")
        .filter(|extra| extra.is_object())
        .cloned()
        .or(Some(value))
}

fn local_code_payload_path(payload: &Value) -> Option<String> {
    payload
        .get("resolved_path")
        .or_else(|| payload.get("effective_path"))
        .or_else(|| payload.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

fn local_code_normalize_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn local_code_paths_match(candidate: &str, expected: &str) -> bool {
    let candidate = local_code_normalize_path(candidate);
    let expected = local_code_normalize_path(expected);
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

fn local_code_path_looks_like_code_or_test(path: &str) -> bool {
    let path = local_code_normalize_path(path);
    let Some(extension) = std::path::Path::new(&path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
    else {
        return false;
    };
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

#[cfg(test)]
#[path = "dispatch_local_code_projection_gate_tests.rs"]
mod tests;
