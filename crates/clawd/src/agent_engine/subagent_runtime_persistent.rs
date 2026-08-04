use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{subagent_action_parts_from_args, LoopState, SubagentRuntimeConfig};
use crate::agent_runtime_contract::SubagentRoleDefinition;
use crate::child_task_contract::{
    ChildTaskBudget, ChildTaskMergePolicy, ChildTaskPermissionProfile, ChildTaskSpec,
};
use crate::repo::child_tasks::{enqueue_child_task_specs, ChildTaskParentContext};
use crate::{AppState, ClaimedTask};

#[path = "subagent_runtime_persistent_resume.rs"]
mod resume;

pub(super) const SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING: &str = "subagent_child_tasks_waiting";
pub(super) const SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED: &str =
    "subagent_child_task_schedule_failed";

const MAX_SCHEDULE_ERROR_CHARS: usize = 512;

pub(super) fn persistent_child_task_requested(args: &Value) -> bool {
    args.get("action").and_then(Value::as_str) == Some("persistent_child_task")
}

pub(super) fn record_persistent_child_task_from_args(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
    config: &SubagentRuntimeConfig,
) -> Result<Option<&'static str>, &'static str> {
    schedule_child_task_specs(
        state,
        task,
        loop_state,
        global_step,
        step_in_round,
        args,
        config,
        false,
    )
}

pub(super) fn record_durable_readonly_child_task_from_args(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
    config: &SubagentRuntimeConfig,
) -> Result<Option<&'static str>, &'static str> {
    schedule_child_task_specs(
        state,
        task,
        loop_state,
        global_step,
        step_in_round,
        args,
        config,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn schedule_child_task_specs(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
    config: &SubagentRuntimeConfig,
    force_readonly: bool,
) -> Result<Option<&'static str>, &'static str> {
    if !config.enabled {
        record_persistent_schedule_error(
            loop_state,
            global_step,
            step_in_round,
            "subagent_runtime_disabled",
            None,
        );
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    }
    if let Some(outcome) = resume::reuse_checkpoint_child_graph(
        state,
        task,
        loop_state,
        global_step,
        step_in_round,
        config,
    )? {
        return match outcome {
            resume::CheckpointChildGraphOutcome::Waiting => {
                Ok(Some(SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING))
            }
            resume::CheckpointChildGraphOutcome::Ready => Ok(None),
            resume::CheckpointChildGraphOutcome::Blocked => {
                Err(super::SUBAGENT_STOP_SIGNAL_REQUIRED_CHILD_FAILED)
            }
        };
    }
    if let Some(outcome) = resume::reuse_existing_parent_child_graph(
        state,
        task,
        loop_state,
        global_step,
        step_in_round,
        config,
    )? {
        return match outcome {
            resume::CheckpointChildGraphOutcome::Waiting => {
                Ok(Some(SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING))
            }
            resume::CheckpointChildGraphOutcome::Ready => Ok(None),
            resume::CheckpointChildGraphOutcome::Blocked => {
                Err(super::SUBAGENT_STOP_SIGNAL_REQUIRED_CHILD_FAILED)
            }
        };
    }
    let mut specs = child_specs(task, args, config, force_readonly)?;
    let allocation_ids = allocate_persistent_child_budgets(loop_state, &mut specs)?;
    let write_enabled = specs
        .iter()
        .any(|spec| spec.permission_profile == ChildTaskPermissionProfile::LocalWorktree);
    let parent = child_parent_context(state, task);
    let max_parallel = persistent_max_parallel(args, config);
    let join_wait_ms = effective_join_wait_ms(args, config);
    let global_running_count = state.worker.active_running_task_count();
    let recursion_depth = child_recursion_depth_from_payload(&task.payload_json);
    let enqueue = enqueue_child_task_specs(
        state,
        &parent,
        &specs,
        max_parallel,
        recursion_depth,
        config.max_spawn_depth as usize,
    )
    .map_err(|err| {
        settle_rejected_child_allocations(loop_state, &allocation_ids);
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
        settle_rejected_child_allocations(loop_state, &allocation_ids);
        record_persistent_schedule_error(
            loop_state,
            global_step,
            step_in_round,
            "child_task_scheduler_rejected",
            None,
        );
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    }

    install_child_waiting_checkpoint(state, task, loop_state, &enqueue, join_wait_ms);
    record_persistent_schedule_observation(
        loop_state,
        global_step,
        step_in_round,
        specs.len(),
        write_enabled,
        enqueue,
        config,
        join_wait_ms,
        global_running_count,
    );
    Ok(Some(SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING))
}

pub(super) fn persistent_child_specs(
    task: &ClaimedTask,
    args: &Value,
    config: &SubagentRuntimeConfig,
) -> Result<Vec<ChildTaskSpec>, &'static str> {
    child_specs(task, args, config, false)
}

fn child_specs(
    task: &ClaimedTask,
    args: &Value,
    config: &SubagentRuntimeConfig,
    force_readonly: bool,
) -> Result<Vec<ChildTaskSpec>, &'static str> {
    let child_args = args
        .get("children")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty());
    let mut specs = Vec::new();
    if let Some(children) = child_args {
        for (index, child) in children.iter().enumerate() {
            specs.push(persistent_child_spec(
                task,
                child,
                index + 1,
                Some(args),
                config,
                force_readonly,
            )?);
        }
    } else {
        specs.push(persistent_child_spec(
            task,
            args,
            1,
            None,
            config,
            force_readonly,
        )?);
    }
    if specs.is_empty() {
        Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED)
    } else {
        resolve_persistent_dependency_refs(&mut specs)?;
        Ok(specs)
    }
}

pub(super) fn allocate_persistent_child_budgets(
    loop_state: &mut LoopState,
    specs: &mut [ChildTaskSpec],
) -> Result<Vec<String>, &'static str> {
    let Some(slice) = loop_state.task_budget_slice.as_mut() else {
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    };
    let mut allocation_ids: Vec<String> = Vec::with_capacity(specs.len());
    for spec in specs {
        let allocation_id = format!("child:{}", spec.child_task_id);
        let requested = crate::task_budget_contract::BudgetUnits {
            model_turns: spec.budget.max_rounds,
            tool_calls: spec.budget.max_tool_calls,
            tokens: spec.budget.max_tokens,
            elapsed_ms: spec.budget.runtime_deadline_ms.unwrap_or_default(),
        };
        let Some(allocation) = slice.allocate(
            allocation_id.clone(),
            format!("child_task:{}", spec.child_task_id),
            crate::task_budget_contract::BudgetAllocationKind::ChildTask,
            requested,
        ) else {
            for allocated in &allocation_ids {
                slice.settle_allocation(
                    allocated,
                    crate::task_budget_contract::BudgetUnits::default(),
                );
            }
            return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
        };
        spec.budget.max_rounds = allocation.granted.model_turns.max(1);
        spec.budget.max_tool_calls = allocation.granted.tool_calls.max(1);
        spec.budget.max_tokens = allocation.granted.tokens.max(1);
        spec.budget.runtime_deadline_ms = spec
            .budget
            .runtime_deadline_ms
            .map(|_| allocation.granted.elapsed_ms.max(1_000));
        if let Some(scope) = spec.scope.as_object_mut() {
            scope.insert(
                "budget_allocation_id".to_string(),
                json!(allocation_id.clone()),
            );
            scope.insert("budget_owner".to_string(), json!("task_budget_manager"));
        }
        allocation_ids.push(allocation_id);
    }
    Ok(allocation_ids)
}

fn settle_rejected_child_allocations(loop_state: &mut LoopState, allocation_ids: &[String]) {
    let Some(slice) = loop_state.task_budget_slice.as_mut() else {
        return;
    };
    for allocation_id in allocation_ids {
        slice.settle_allocation(
            allocation_id,
            crate::task_budget_contract::BudgetUnits::default(),
        );
    }
}

fn persistent_child_spec(
    task: &ClaimedTask,
    args: &Value,
    index: usize,
    top_level_args: Option<&Value>,
    config: &SubagentRuntimeConfig,
    force_readonly: bool,
) -> Result<ChildTaskSpec, &'static str> {
    let (role, objective, context_refs, options) = subagent_action_parts_from_args(args);
    let role_kind = config
        .resolve_role(role.trim())
        .cloned()
        .ok_or(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED)?;
    let objective = objective.trim();
    if objective.is_empty() {
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    }
    let permission_profile = if force_readonly {
        ChildTaskPermissionProfile::ReadOnly
    } else {
        persistent_permission_profile(args, top_level_args, &role_kind)?
    };
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
    let node_id = persistent_node_id(args, index)?;
    let dependencies =
        persistent_scope_value(args, top_level_args, "depends_on").unwrap_or_else(|| json!([]));
    let owned_paths =
        persistent_scope_value(args, top_level_args, "owned_paths").unwrap_or_else(|| json!([]));
    let model_policy = role_kind.model_policy.clone();
    let mut tool_policy = role_kind.tool_policy.clone();
    let Some(tool_policy_object) = tool_policy.as_object_mut() else {
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    };
    tool_policy_object.insert(
        "allowed_capabilities".to_string(),
        json!(allowed_capabilities.clone()),
    );
    let session_ref = child_session_ref(task);
    let scope = json!({
        "node_id": node_id.clone(),
        "objective": objective,
        "context_refs": context_refs,
        "context_slice": options.context_slice,
        "allowed_capabilities": allowed_capabilities,
        "dependencies": dependencies,
        "owned_paths": owned_paths,
        "model_policy": model_policy,
        "tool_policy": tool_policy,
        "recursion_depth": child_recursion_depth_from_payload(&task.payload_json),
        "session_ref": session_ref,
        "session_open_capacity": config.max_concurrent_threads_per_session,
    });
    Ok(ChildTaskSpec {
        parent_task_id: task.task_id.clone(),
        child_task_id: format!(
            "{}:child:{}:{}",
            task.task_id,
            node_id,
            uuid::Uuid::new_v4().simple()
        ),
        role,
        scope,
        permission_profile,
        required,
        budget: persistent_child_budget(task, args, top_level_args),
        result_contract,
        merge_policy: ChildTaskMergePolicy::StructuredFindings,
    })
}

fn persistent_node_id(args: &Value, index: usize) -> Result<String, &'static str> {
    let candidate = args
        .get("node_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("node_{index}"));
    if machine_capability_token(&candidate) {
        Ok(candidate)
    } else {
        Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED)
    }
}

fn persistent_scope_value(
    args: &Value,
    top_level_args: Option<&Value>,
    key: &str,
) -> Option<Value> {
    args.get(key)
        .cloned()
        .or_else(|| top_level_args.and_then(|value| value.get(key).cloned()))
}

fn resolve_persistent_dependency_refs(specs: &mut [ChildTaskSpec]) -> Result<(), &'static str> {
    let node_ids = specs
        .iter()
        .filter_map(|spec| {
            Some((
                spec.scope.get("node_id")?.as_str()?.to_string(),
                spec.child_task_id.clone(),
            ))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    if node_ids.len() != specs.len() {
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    }
    for spec in specs {
        let Some(items) = spec
            .scope
            .get_mut("dependencies")
            .and_then(Value::as_array_mut)
        else {
            return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
        };
        for item in items {
            match item {
                Value::String(reference) => {
                    let Some(child_task_id) = node_ids.get(reference.trim()) else {
                        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
                    };
                    *reference = child_task_id.clone();
                }
                Value::Object(object) => {
                    let reference = object
                        .get("node_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .ok_or(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED)?;
                    let Some(child_task_id) = node_ids.get(reference) else {
                        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
                    };
                    object.remove("node_id");
                    object.insert("child_task_id".to_string(), json!(child_task_id));
                }
                _ => return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED),
            }
        }
    }
    Ok(())
}

fn persistent_permission_profile(
    _args: &Value,
    _top_level_args: Option<&Value>,
    role: &SubagentRoleDefinition,
) -> Result<ChildTaskPermissionProfile, &'static str> {
    let token = role.default_permission_profile.trim();
    if !role.allows_permission_profile(token) {
        return Err(SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED);
    }
    match token {
        "read_only" => Ok(ChildTaskPermissionProfile::ReadOnly),
        "local_current_workspace" => Ok(ChildTaskPermissionProfile::LocalCurrentWorkspace),
        "local_worktree" => Ok(ChildTaskPermissionProfile::LocalWorktree),
        "local_temp_workspace" => Ok(ChildTaskPermissionProfile::LocalTempWorkspace),
        "remote_executor" => Ok(ChildTaskPermissionProfile::RemoteExecutor),
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

fn persistent_child_budget(
    task: &ClaimedTask,
    args: &Value,
    top_level_args: Option<&Value>,
) -> ChildTaskBudget {
    let budget_value = args.get("budget").or_else(|| top_level_args?.get("budget"));
    let mut budget = ChildTaskBudget::readonly_default();
    if let Some(value) = budget_value {
        if let Some(max_rounds) = value.get("max_rounds").and_then(Value::as_u64) {
            budget.max_rounds = max_rounds.max(1);
        }
        if let Some(max_tool_calls) = value.get("max_tool_calls").and_then(Value::as_u64) {
            budget.max_tool_calls = max_tool_calls.max(1);
        }
        if let Some(max_tokens) = value.get("max_tokens").and_then(Value::as_u64) {
            budget.max_tokens = max_tokens.max(1);
        }
    }
    // A planner may choose the parent wait window, but it cannot terminate a
    // whole child operation. Only a structured field on the original parent
    // task submission can opt into an operation deadline. Persisted v1 child
    // tasks already carry their frozen deadline in their child contract and
    // are handled by the worker compatibility reader.
    budget.runtime_deadline_ms = serde_json::from_str::<Value>(&task.payload_json)
        .ok()
        .and_then(|payload| {
            payload
                .pointer("/subagent_execution/runtime_deadline_ms")
                .and_then(Value::as_u64)
        })
        .map(|deadline| deadline.max(1_000));
    budget
}

fn persistent_max_parallel(args: &Value, config: &SubagentRuntimeConfig) -> usize {
    args.get("max_parallel")
        .and_then(Value::as_u64)
        .unwrap_or(config.max_concurrent_threads_per_session)
        .min(config.max_concurrent_threads_per_session.max(1))
        .max(1) as usize
}

fn effective_join_wait_ms(args: &Value, config: &SubagentRuntimeConfig) -> u64 {
    args.pointer("/wait_policy/join_wait_ms")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(100, 300_000))
        .unwrap_or(config.join_wait_ms)
}

fn child_recursion_depth_from_payload(payload_json: &str) -> usize {
    serde_json::from_str::<Value>(payload_json)
        .ok()
        .and_then(|payload| {
            payload
                .pointer("/child_task_contract/scope/recursion_depth")
                .and_then(Value::as_u64)
        })
        .and_then(|depth| usize::try_from(depth).ok())
        .unwrap_or(0)
        .saturating_add(1)
}

fn child_session_ref(task: &ClaimedTask) -> String {
    let payload = serde_json::from_str::<Value>(&task.payload_json).unwrap_or(Value::Null);
    if let Some(inherited) = payload
        .pointer("/child_task_contract/scope/session_ref")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        return inherited.to_string();
    }
    let conversation_ref = ["thread_id", "conversation_id", "session_id"]
        .into_iter()
        .find_map(|key| payload.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| task.external_chat_id.clone())
        .unwrap_or_else(|| task.chat_id.to_string());
    let principal_ref = task
        .user_key
        .clone()
        .or_else(|| task.external_user_id.clone())
        .unwrap_or_else(|| task.user_id.to_string());
    let digest = Sha256::digest(
        format!(
            "{}\0{}\0{}\0{}",
            task.channel, principal_ref, task.user_id, conversation_ref
        )
        .as_bytes(),
    );
    format!("subagent-session:{:x}", digest)[..41].to_string()
}

fn child_parent_context(state: &AppState, task: &ClaimedTask) -> ChildTaskParentContext {
    ChildTaskParentContext {
        parent_task_id: task.task_id.clone(),
        user_id: task.user_id,
        chat_id: task.chat_id,
        user_key: task.user_key.clone(),
        channel: task.channel.clone(),
        external_user_id: task.external_user_id.clone(),
        external_chat_id: task.external_chat_id.clone(),
        execution_policy_stamp: crate::task_execution_policy::inheritable_policy_stamp(state, task),
        interactive_approval_available:
            crate::repo::child_tasks::parent_interactive_approval_available(&task.payload_json),
    }
}

pub(super) fn install_child_waiting_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    enqueue: &Value,
    join_wait_ms: u64,
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
        lifecycle.insert("join_wait_ms".to_string(), json!(join_wait_ms));
        lifecycle.insert("join_wait_expires_child".to_string(), json!(false));
        lifecycle.insert(
            "waiting_reason".to_string(),
            json!("child_tasks_running_or_queued"),
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
        boundary_context.insert("join_wait_ms".to_string(), json!(join_wait_ms));
    }
}

pub(super) fn record_persistent_schedule_observation(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    requested_child_count: usize,
    write_enabled: bool,
    enqueue: Value,
    config: &SubagentRuntimeConfig,
    join_wait_ms: u64,
    global_running_count: usize,
) {
    loop_state.task_observations.push(json!({
        "schema_version": 2,
        "owner_layer": "subagent_runtime",
        "output_format": "machine_json",
        "status": "waiting",
        "action": "subagent_child_task_enqueue",
        "execution_mode": "persistent_child_task",
        "requested_child_count": requested_child_count,
        "child_task_ids": enqueue.get("child_task_ids").cloned().unwrap_or_else(|| json!([])),
        "child_task_enqueue": enqueue.clone(),
        "capacity": {
            "schema_version": 2,
            "main_agent_counted": false,
            "session_open_capacity_source": "agent_guard.subagents.max_concurrent_threads_per_session",
            "effective_session_open_capacity": config.max_concurrent_threads_per_session,
            "effective_parent_parallel_capacity": enqueue
                .pointer("/scheduler/max_parallel")
                .and_then(Value::as_u64),
            "queued_count": enqueue
                .pointer("/scheduler/blocked_child_count")
                .and_then(Value::as_u64),
            "global_running_capacity_source": "worker_provider_admission",
            "effective_global_running_capacity": config.max_running_threads_global,
            "global_running_count": global_running_count,
        },
        "wait_policy": {
            "join_wait_ms": join_wait_ms,
            "join_wait_expires_child": false,
        },
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
