use serde_json::Value;
use std::path::Path;

use super::{AgentLoopGuardPolicy, AppState, LoopState, WriteFileEffectivePath};
use crate::agent_engine::{
    CLAWD_CONTINUE_ON_ERROR_ARG, CLAWD_LITERAL_COMMAND_ARG, CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG,
    CLAWD_MISSING_TARGET_REPAIRABLE_ARG, CLAWD_RUNTIME_ASYNC_JOB_START_ARG,
    CLAWD_USER_NAMED_OUTPUT_PATH_ARG,
};

pub(super) fn strip_internal_execution_args(args: &mut Value) {
    if let Some(obj) = args.as_object_mut() {
        obj.remove(CLAWD_CONTINUE_ON_ERROR_ARG);
        obj.remove(CLAWD_LITERAL_COMMAND_ARG);
        obj.remove(CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG);
        obj.remove(CLAWD_MISSING_TARGET_REPAIRABLE_ARG);
        obj.remove(CLAWD_RUNTIME_ASYNC_JOB_START_ARG);
        obj.remove(CLAWD_USER_NAMED_OUTPUT_PATH_ARG);
        obj.remove(crate::execution_recipe::CLAWD_VALIDATION_ARG);
    }
}

pub(super) fn strip_unsupported_planner_metadata_args(
    state: &AppState,
    canonical_skill: &str,
    args: &mut Value,
) -> Vec<String> {
    let Some(obj) = args.as_object_mut() else {
        return Vec::new();
    };
    let schema = if let Some(tool) = state.mcp_tool(canonical_skill) {
        tool.input_schema
    } else {
        let Some(manifest) = state.skill_manifest(canonical_skill) else {
            return Vec::new();
        };
        let Some(schema) = manifest.input_schema else {
            return Vec::new();
        };
        schema
    };
    let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for key in ["confirm", "confirmation", "requires_confirmation"] {
        if !properties.contains_key(key) && obj.remove(key).is_some() {
            removed.push(key.to_string());
        }
    }
    removed
}

pub(super) fn read_file_requested_path(skill_name: &str, args: &Value) -> Option<String> {
    if skill_name != "read_file" {
        return None;
    }
    args.get("path")
        .and_then(|v| v.as_str())
        .map(|path| path.to_string())
}

pub(super) fn write_file_effective_path(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> Option<WriteFileEffectivePath> {
    if normalized_skill != "write_file" {
        return None;
    }
    args.get("path").and_then(|v| v.as_str()).map(|path| {
        let effective = crate::ensure_default_file_path(&state.skill_rt.workspace_root, path);
        let user_visible = if Path::new(&effective).is_absolute() {
            effective.clone()
        } else {
            state
                .skill_rt
                .workspace_root
                .join(&effective)
                .display()
                .to_string()
        };
        (path.to_string(), effective, user_visible)
    })
}

pub(super) fn apply_recipe_run_cmd_overrides(
    state: &AppState,
    loop_state: &LoopState,
    policy: &AgentLoopGuardPolicy,
    normalized_skill: &str,
    args: &mut Value,
) {
    if normalized_skill != "run_cmd" || !loop_state.execution_recipe.is_active() {
        return;
    }
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    if obj.get("timeout_seconds").is_some() {
        return;
    }
    let raw_effect = crate::execution_recipe::classify_skill_action_effect(
        state,
        normalized_skill,
        &Value::Object(obj.clone()),
    );
    let effect = crate::execution_recipe::effective_action_effect_for_recipe(
        loop_state.execution_recipe,
        raw_effect,
    );
    let Some(timeout_seconds) =
        policy.run_cmd_timeout_override(loop_state.execution_recipe, effect)
    else {
        return;
    };
    obj.insert(
        "timeout_seconds".to_string(),
        Value::Number(serde_json::Number::from(timeout_seconds)),
    );
}

pub(super) fn record_latest_run_cmd_command_output_vars(
    loop_state: &mut LoopState,
    normalized_skill: &str,
    args: &Value,
) {
    if normalized_skill != "run_cmd" {
        return;
    }
    let Some(command) = args
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
    else {
        return;
    };
    loop_state.output_vars.insert(
        "agent_loop.latest_run_cmd_command".to_string(),
        command.to_string(),
    );
    record_run_cmd_command_list_output_var(loop_state, command);
    if let Some(cwd) = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
    {
        loop_state
            .output_vars
            .insert("agent_loop.latest_run_cmd_cwd".to_string(), cwd.to_string());
    }
}

fn record_run_cmd_command_list_output_var(loop_state: &mut LoopState, command: &str) {
    let mut commands = loop_state
        .output_vars
        .get("agent_loop.run_cmd_commands")
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default();
    if commands.iter().any(|existing| existing == command) {
        return;
    }
    commands.push(command.to_string());
    if let Ok(serialized) = serde_json::to_string(&commands) {
        loop_state
            .output_vars
            .insert("agent_loop.run_cmd_commands".to_string(), serialized);
    }
}
