#![allow(dead_code)]

use serde_json::{json, Value};

pub(crate) const CHILD_TASK_SCHEMA_VERSION: u64 = 1;
pub(crate) const DEFAULT_MAX_CHILDREN_PER_PARENT: usize = 16;
pub(crate) const DEFAULT_MAX_CHILD_DEPTH: usize = 1;

#[path = "child_task_contract_policy.rs"]
mod child_task_contract_policy;

pub(crate) use child_task_contract_policy::{child_scheduler_decision, merge_child_task_results};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildTaskLifecycleEvent {
    Queued,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
}

impl ChildTaskLifecycleEvent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "subagent_queued",
            Self::Running => "subagent_running",
            Self::Waiting => "subagent_waiting",
            Self::Succeeded => "subagent_succeeded",
            Self::Failed => "subagent_failed",
            Self::Cancelled => "subagent_cancelled",
        }
    }

    pub(crate) fn status(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildTaskPermissionProfile {
    ReadOnly,
    LocalCurrentWorkspace,
    LocalWorktree,
    LocalTempWorkspace,
    RemoteExecutor,
}

impl ChildTaskPermissionProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::LocalCurrentWorkspace => "local_current_workspace",
            Self::LocalWorktree => "local_worktree",
            Self::LocalTempWorkspace => "local_temp_workspace",
            Self::RemoteExecutor => "remote_executor",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildTaskMergePolicy {
    RequiredAll,
    OptionalIsolated,
    StructuredFindings,
}

impl ChildTaskMergePolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RequiredAll => "required_all",
            Self::OptionalIsolated => "optional_isolated",
            Self::StructuredFindings => "structured_findings",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChildTaskBudget {
    pub(crate) max_rounds: u64,
    pub(crate) max_tool_calls: u64,
    pub(crate) timeout_ms: u64,
}

impl ChildTaskBudget {
    pub(crate) fn readonly_default() -> Self {
        Self {
            max_rounds: 2,
            max_tool_calls: 8,
            timeout_ms: 120_000,
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "max_rounds": self.max_rounds,
            "max_tool_calls": self.max_tool_calls,
            "timeout_ms": self.timeout_ms,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChildTaskSpec {
    pub(crate) parent_task_id: String,
    pub(crate) child_task_id: String,
    pub(crate) role: String,
    pub(crate) scope: Value,
    pub(crate) permission_profile: ChildTaskPermissionProfile,
    pub(crate) required: bool,
    pub(crate) budget: ChildTaskBudget,
    pub(crate) result_contract: Value,
    pub(crate) merge_policy: ChildTaskMergePolicy,
}

impl ChildTaskSpec {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "schema_version": CHILD_TASK_SCHEMA_VERSION,
            "parent_task_id": stable_machine_ref(&self.parent_task_id),
            "child_task_id": stable_machine_ref(&self.child_task_id),
            "role": stable_machine_ref(&self.role),
            "scope": self.scope,
            "permission_profile": self.permission_profile.as_str(),
            "required": self.required,
            "budget": self.budget.to_json(),
            "result_contract": self.result_contract,
            "merge_policy": self.merge_policy.as_str(),
        })
    }
}

pub(crate) fn child_task_event_json(
    event: ChildTaskLifecycleEvent,
    spec: &ChildTaskSpec,
    reason_code: Option<&str>,
) -> Value {
    json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "event_type": event.as_str(),
        "parent_task_id": stable_machine_ref(&spec.parent_task_id),
        "child_task_id": stable_machine_ref(&spec.child_task_id),
        "role": stable_machine_ref(&spec.role),
        "status": event.status(),
        "required": spec.required,
        "permission_profile": spec.permission_profile.as_str(),
        "merge_policy": spec.merge_policy.as_str(),
        "reason_code": reason_code.map(stable_machine_ref),
    })
}

pub(crate) fn parent_cancel_child_directive(
    parent_task_id: &str,
    child_task_ids: &[String],
) -> Value {
    json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "directive": "cancel_child_tasks",
        "parent_task_id": stable_machine_ref(parent_task_id),
        "reason_code": "parent_cancelled",
        "child_task_ids": child_task_ids
            .iter()
            .map(|task_id| stable_machine_ref(task_id))
            .collect::<Vec<_>>(),
    })
}

fn stable_machine_ref(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/'))
        .take(160)
        .collect()
}

#[cfg(test)]
#[path = "child_task_contract_tests.rs"]
mod tests;
