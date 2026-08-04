use serde_json::{json, Value};

use super::{LoopState, SubagentRuntimeConfig};
use crate::{AppState, ClaimedTask};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CheckpointChildGraphOutcome {
    Waiting,
    Ready,
    Blocked,
}

pub(super) fn reuse_checkpoint_child_graph(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    config: &SubagentRuntimeConfig,
) -> Result<Option<CheckpointChildGraphOutcome>, &'static str> {
    if !is_child_wait_checkpoint(loop_state) {
        return Ok(None);
    }
    reuse_parent_child_graph(
        state,
        task,
        loop_state,
        global_step,
        step_in_round,
        config,
        "checkpoint_child_tasks",
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reuse_existing_parent_child_graph(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    config: &SubagentRuntimeConfig,
) -> Result<Option<CheckpointChildGraphOutcome>, &'static str> {
    reuse_parent_child_graph(
        state,
        task,
        loop_state,
        global_step,
        step_in_round,
        config,
        "existing_parent_child_graph",
    )
}

#[allow(clippy::too_many_arguments)]
fn reuse_parent_child_graph(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    config: &SubagentRuntimeConfig,
    reuse_reason_prefix: &str,
) -> Result<Option<CheckpointChildGraphOutcome>, &'static str> {
    let graph = {
        let db = state.core.db.get().map_err(|_| {
            super::record_persistent_schedule_error(
                loop_state,
                global_step,
                step_in_round,
                "child_task_graph_read_failed",
                None,
            );
            super::SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED
        })?;
        crate::repo::child_task_graph::graph_snapshot(&db, &task.task_id).map_err(|_| {
            super::record_persistent_schedule_error(
                loop_state,
                global_step,
                step_in_round,
                "child_task_graph_read_failed",
                None,
            );
            super::SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED
        })?
    };
    let Some(graph) = graph else {
        return Ok(None);
    };
    let nodes = graph
        .get("nodes")
        .and_then(Value::as_array)
        .filter(|nodes| !nodes.is_empty());
    let Some(nodes) = nodes else {
        return Ok(None);
    };
    let child_task_ids = nodes
        .iter()
        .filter_map(|node| node.get("child_task_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let pending_count = nodes
        .iter()
        .filter(|node| !terminal_execution_state(node))
        .count();
    if pending_count > 0 {
        record_reused_waiting_graph(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            config,
            &graph,
            &child_task_ids,
            pending_count,
            reuse_reason_prefix,
        );
        return Ok(Some(CheckpointChildGraphOutcome::Waiting));
    }

    let merge = crate::repo::child_tasks::refresh_parent_child_task_merge(state, &task.task_id)
        .map_err(|_| {
            super::record_persistent_schedule_error(
                loop_state,
                global_step,
                step_in_round,
                "child_task_merge_refresh_failed",
                None,
            );
            super::SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED
        })?
        .ok_or_else(|| {
            super::record_persistent_schedule_error(
                loop_state,
                global_step,
                step_in_round,
                "child_task_merge_missing",
                None,
            );
            super::SUBAGENT_STOP_SIGNAL_CHILD_TASK_SCHEDULE_FAILED
        })?;
    let continuation = merge
        .pointer("/parent_continuation/status")
        .and_then(Value::as_str)
        .unwrap_or("blocked");
    if continuation == "waiting" {
        let pending_count = merge
            .get("pending_child_count")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize;
        record_reused_waiting_graph(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            config,
            &graph,
            &child_task_ids,
            pending_count,
            reuse_reason_prefix,
        );
        return Ok(Some(CheckpointChildGraphOutcome::Waiting));
    }
    let outcome = if continuation == "ready" {
        CheckpointChildGraphOutcome::Ready
    } else {
        CheckpointChildGraphOutcome::Blocked
    };
    loop_state.task_checkpoint = None;
    loop_state.task_lifecycle = None;
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some(merge.to_string());
    loop_state.task_observations.push(json!({
        "schema_version": 2,
        "owner_layer": "subagent_runtime",
        "output_format": "machine_json",
        "status": continuation,
        "action": "subagent_child_task_merge_reused",
        "admission_reused": true,
        "reuse_reason": format!("{reuse_reason_prefix}_terminal"),
        "parent_task_id": task.task_id,
        "child_task_ids": child_task_ids,
        "child_task_merge": merge,
        "global_step": global_step,
        "step_in_round": step_in_round,
        "round_no": loop_state.round_no,
    }));
    Ok(Some(outcome))
}

#[allow(clippy::too_many_arguments)]
fn record_reused_waiting_graph(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    config: &SubagentRuntimeConfig,
    graph: &Value,
    child_task_ids: &[String],
    pending_count: usize,
    reuse_reason_prefix: &str,
) {
    let nodes = graph
        .get("nodes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let join_wait_ms = loop_state
        .task_checkpoint
        .as_ref()
        .and_then(|checkpoint| {
            checkpoint
                .pointer("/boundary_context/join_wait_ms")
                .and_then(Value::as_u64)
        })
        .unwrap_or_else(|| super::effective_join_wait_ms(&json!({}), config));
    let enqueue = json!({
        "schema_version": 2,
        "status": "reused",
        "admission_reused": true,
        "reuse_reason": format!("{reuse_reason_prefix}_pending"),
        "parent_task_id": task.task_id,
        "child_task_ids": child_task_ids,
        "queued_child_count": pending_count,
        "child_task_graph": graph,
        "scheduler": {
            "schema_version": 2,
            "decision": "reuse_existing_child_graph",
            "reason_code": format!("{reuse_reason_prefix}_pending"),
            "max_parallel": graph.get("max_parallel"),
            "blocked_child_count": nodes
                .iter()
                .filter(|node| node.get("thread_state").and_then(Value::as_str) == Some("queued_capacity"))
                .count(),
            "main_agent_counted": false,
        },
    });
    super::install_child_waiting_checkpoint(state, task, loop_state, &enqueue, join_wait_ms);
    super::record_persistent_schedule_observation(
        loop_state,
        global_step,
        step_in_round,
        child_task_ids.len(),
        nodes.iter().any(|node| {
            node.get("permission_profile").and_then(Value::as_str) == Some("local_worktree")
        }),
        enqueue,
        config,
        join_wait_ms,
        state.worker.active_running_task_count(),
    );
}

fn is_child_wait_checkpoint(loop_state: &LoopState) -> bool {
    loop_state
        .task_checkpoint
        .as_ref()
        .is_some_and(|checkpoint| {
            checkpoint
                .pointer("/boundary_context/source")
                .and_then(Value::as_str)
                == Some("subagent_child_task_enqueue")
                || checkpoint
                    .pointer("/boundary_context/context_compaction_trigger/resume_reason")
                    .and_then(Value::as_str)
                    == Some(super::SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING)
        })
}

fn terminal_execution_state(node: &Value) -> bool {
    matches!(
        node.get("execution_state").and_then(Value::as_str),
        Some("succeeded" | "failed" | "cancelled" | "timed_out")
    )
}
