use serde_json::{json, Value};

use super::{subagent_action_parts_from_args, LoopState, SubagentRuntimeConfig};
use crate::agent_runtime_contract::SubagentRole;
use crate::child_task_contract::{
    ChildTaskBudget, ChildTaskMergePolicy, ChildTaskPermissionProfile, ChildTaskSpec,
    DEFAULT_MAX_CHILDREN_PER_PARENT,
};
use crate::repo::child_tasks::{enqueue_child_task_specs, ChildTaskParentContext};
use crate::{AppState, ClaimedTask};

pub(super) const SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING: &str = "subagent_child_tasks_waiting";
pub(super) const SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED: &str =
    "subagent_child_task_schedule_failed";

const MAX_SCHEDULE_ERROR_CHARS: usize = 512;

pub(super) fn persistent_child_task_requested(args: &Value) -> bool {
    args.get("persistent_child_task")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || machine_mode_field(args, "execution_mode").is_some_and(persistent_child_task_mode_token)
        || machine_mode_field(args, "child_task_mode").is_some_and(persistent_child_task_mode_token)
        || machine_mode_field(args, "scheduler_mode").is_some_and(persistent_child_task_mode_token)
}

pub(super) fn record_persistent_child_task_from_args(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
    config: &SubagentRuntimeConfig,
) -> Result<&'static str, &'static str> {
    let specs = persistent_child_specs(task, args)?;
    let write_enabled = specs
        .iter()
        .any(|spec| spec.permission_profile == ChildTaskPermissionProfile::LocalWorktree);
    let parent = child_parent_context(task);
    let max_parallel = persistent_max_parallel(args, config);
    let recursion_depth = child_recursion_depth_from_payload(&task.payload_json);
    let enqueue = enqueue_child_task_specs(state, &parent, &specs, max_parallel, recursion_depth)
        .map_err(|err| {
        record_persistent_schedule_error(
            loop_state,
            global_step,
            step_in_round,
            "child_task_enqueue_failed",
            Some(err.to_string()),
        );
        SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED
    })?;

    if enqueue.get("status").and_then(Value::as_str) != Some("scheduled") {
        record_persistent_schedule_error(
            loop_state,
            global_step,
            step_in_round,
            "child_task_scheduler_rejected",
            None,
        );
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    }

    install_child_waiting_checkpoint(state, task, loop_state, &enqueue);
    record_persistent_schedule_observation(
        loop_state,
        global_step,
        step_in_round,
        specs.len(),
        write_enabled,
        enqueue,
    );
    Ok(SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING)
}

fn machine_mode_field<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn persistent_child_task_mode_token(token: &str) -> bool {
    matches!(
        token.trim(),
        "persistent"
            | "persistent_child_task"
            | "queued_child_task"
            | "background_child_task"
            | "child_task_queue"
    )
}

fn persistent_child_specs(
    task: &ClaimedTask,
    args: &Value,
) -> Result<Vec<ChildTaskSpec>, &'static str> {
    let child_args = args
        .get("children")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty());
    let mut specs = Vec::new();
    if let Some(children) = child_args {
        for (index, child) in children
            .iter()
            .take(DEFAULT_MAX_CHILDREN_PER_PARENT)
            .enumerate()
        {
            specs.push(persistent_child_spec(task, child, index + 1, Some(args))?);
        }
    } else {
        specs.push(persistent_child_spec(task, args, 1, None)?);
    }
    if specs.is_empty() {
        Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED)
    } else {
        Ok(specs)
    }
}

fn persistent_child_spec(
    task: &ClaimedTask,
    args: &Value,
    index: usize,
    top_level_args: Option<&Value>,
) -> Result<ChildTaskSpec, &'static str> {
    let (role, objective, context_refs, options) = subagent_action_parts_from_args(args);
    let role_kind = SubagentRole::parse_token(role.trim())
        .ok_or(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED)?;
    let objective = objective.trim();
    if objective.is_empty() {
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    }
    let permission_profile = persistent_permission_profile(args, top_level_args, role_kind)?;
    let allowed_capabilities = persistent_allowed_capabilities(&options, top_level_args)?;
    let required = args
        .get("required")
        .and_then(Value::as_bool)
        .or_else(|| top_level_args.and_then(|value| value.get("required")?.as_bool()))
        .unwrap_or(true);
    let result_contract = options
        .result_contract
        .clone()
        .or_else(|| top_level_args.and_then(|value| value.get("result_contract").cloned()))
        .unwrap_or_else(|| json!({"output_format": "machine_json"}));
    let scope = json!({
        "objective": objective,
        "context_refs": context_refs,
        "context_slice": options.context_slice,
        "allowed_capabilities": allowed_capabilities,
    });
    Ok(ChildTaskSpec {
        parent_task_id: task.task_id.clone(),
        child_task_id: format!("{}:child:{}:{index}", task.task_id, crate::now_ts_u64()),
        role,
        scope,
        permission_profile,
        required,
        budget: persistent_child_budget(args, top_level_args),
        result_contract,
        merge_policy: ChildTaskMergePolicy::StructuredFindings,
    })
}

fn persistent_permission_profile(
    args: &Value,
    top_level_args: Option<&Value>,
    role: SubagentRole,
) -> Result<ChildTaskPermissionProfile, &'static str> {
    let token = args
        .get("permission_profile")
        .and_then(Value::as_str)
        .or_else(|| {
            args.pointer("/runtime_policy/tool_permission_profile")?
                .as_str()
        })
        .or_else(|| top_level_args?.get("permission_profile")?.as_str())
        .unwrap_or(if role == SubagentRole::Writer {
            "local_worktree"
        } else {
            "read_only"
        })
        .trim();
    match (token, role) {
        ("local_worktree", SubagentRole::Writer | SubagentRole::Worker | SubagentRole::Test) => {
            Ok(ChildTaskPermissionProfile::LocalWorktree)
        }
        ("" | "read_only", SubagentRole::Writer) => {
            Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED)
        }
        ("" | "read_only", _) => Ok(ChildTaskPermissionProfile::ReadOnly),
        _ => Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED),
    }
}

fn persistent_allowed_capabilities(
    options: &super::SubagentActionOptions,
    top_level_args: Option<&Value>,
) -> Result<Vec<String>, &'static str> {
    let capabilities = if options.allowed_capabilities.is_empty() {
        top_level_args
            .and_then(|value| value.get("allowed_capabilities"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        options.allowed_capabilities.clone()
    };
    if capabilities.is_empty()
        || capabilities.len() > 32
        || capabilities
            .iter()
            .any(|capability| !machine_capability_token(capability))
    {
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    }
    Ok(capabilities)
}

fn machine_capability_token(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 160
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase()
                || ch.is_ascii_digit()
                || matches!(ch, '_' | '-' | '.' | ':' | '/')
        })
}

fn persistent_child_budget(args: &Value, top_level_args: Option<&Value>) -> ChildTaskBudget {
    let budget_value = args.get("budget").or_else(|| top_level_args?.get("budget"));
    let mut budget = ChildTaskBudget::readonly_default();
    if let Some(value) = budget_value {
        if let Some(max_rounds) = value.get("max_rounds").and_then(Value::as_u64) {
            budget.max_rounds = max_rounds.clamp(1, 12);
        }
        if let Some(max_tool_calls) = value.get("max_tool_calls").and_then(Value::as_u64) {
            budget.max_tool_calls = max_tool_calls.clamp(1, 64);
        }
        if let Some(timeout_ms) = value.get("timeout_ms").and_then(Value::as_u64) {
            budget.timeout_ms = timeout_ms.clamp(1_000, 3_600_000);
        }
    }
    budget
}

fn persistent_max_parallel(args: &Value, config: &SubagentRuntimeConfig) -> usize {
    args.get("max_parallel")
        .and_then(Value::as_u64)
        .unwrap_or(config.max_parallel_readonly)
        .clamp(1, DEFAULT_MAX_CHILDREN_PER_PARENT as u64) as usize
}

fn child_recursion_depth_from_payload(payload_json: &str) -> usize {
    serde_json::from_str::<Value>(payload_json)
        .ok()
        .filter(crate::repo::child_tasks::is_child_subagent_payload)
        .map(|_| crate::child_task_contract::DEFAULT_MAX_CHILD_DEPTH + 1)
        .unwrap_or(1)
}

fn child_parent_context(task: &ClaimedTask) -> ChildTaskParentContext {
    ChildTaskParentContext {
        parent_task_id: task.task_id.clone(),
        user_id: task.user_id,
        chat_id: task.chat_id,
        user_key: task.user_key.clone(),
        channel: task.channel.clone(),
        external_user_id: task.external_user_id.clone(),
        external_chat_id: task.external_chat_id.clone(),
    }
}

fn install_child_waiting_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    enqueue: &Value,
) {
    super::super::support::publish_agent_loop_checkpoint_progress(
        state,
        task,
        loop_state,
        SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING,
    );
    if let Some(lifecycle) = loop_state
        .task_lifecycle
        .as_mut()
        .and_then(Value::as_object_mut)
    {
        lifecycle.insert("source".to_string(), json!("subagent_child_task_enqueue"));
        lifecycle.insert(
            "message_key".to_string(),
            json!("clawd.subagent.child_tasks_waiting"),
        );
        lifecycle.insert(
            "child_task_ids".to_string(),
            enqueue
                .get("child_task_ids")
                .cloned()
                .unwrap_or_else(|| json!([])),
        );
        lifecycle.insert(
            "poll_ref".to_string(),
            json!(enqueue
                .get("child_task_ids")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str)
                .unwrap_or_default()),
        );
    }
    if let Some(boundary_context) = loop_state
        .task_checkpoint
        .as_mut()
        .and_then(|checkpoint| checkpoint.pointer_mut("/boundary_context"))
        .and_then(Value::as_object_mut)
    {
        boundary_context.insert("source".to_string(), json!("subagent_child_task_enqueue"));
        boundary_context.insert("child_task_enqueue".to_string(), enqueue.clone());
    }
}

fn record_persistent_schedule_observation(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    requested_child_count: usize,
    write_enabled: bool,
    enqueue: Value,
) {
    loop_state.task_observations.push(json!({
        "schema_version": 1,
        "owner_layer": "subagent_runtime",
        "output_format": "machine_json",
        "status": "waiting",
        "action": "subagent_child_task_enqueue",
        "execution_mode": "persistent_child_task",
        "requested_child_count": requested_child_count,
        "child_task_ids": enqueue.get("child_task_ids").cloned().unwrap_or_else(|| json!([])),
        "child_task_enqueue": enqueue,
        "task_lifecycle": loop_state.task_lifecycle,
        "write_enabled": write_enabled,
        "write_scope": if write_enabled {
            "persistent_local_worktree"
        } else {
            "read_only"
        },
        "external_publish_enabled": false,
        "failure_isolated": true,
        "global_step": global_step,
        "step_in_round": step_in_round,
        "round_no": loop_state.round_no,
    }));
}

fn record_persistent_schedule_error(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    error_code: &str,
    error_text: Option<String>,
) {
    loop_state.task_observations.push(json!({
        "schema_version": 1,
        "owner_layer": "subagent_runtime",
        "output_format": "machine_json",
        "status": "rejected",
        "action": "subagent_child_task_enqueue",
        "execution_mode": "persistent_child_task",
        "error_code": error_code,
        "error_excerpt": error_text.map(|text| bounded_error(&text)),
        "write_enabled": false,
        "external_publish_enabled": false,
        "global_step": global_step,
        "step_in_round": step_in_round,
        "round_no": loop_state.round_no,
    }));
}

fn bounded_error(value: &str) -> String {
    value.chars().take(MAX_SCHEDULE_ERROR_CHARS).collect()
}
