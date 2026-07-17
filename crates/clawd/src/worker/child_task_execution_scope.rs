use anyhow::{bail, Context};
use serde_json::{json, Value};

#[cfg(test)]
use crate::execution_isolation::ExecutionIsolationPlan;
use crate::execution_isolation::ExecutionIsolationRuntime;
use crate::{AppState, ClaimedTask};
use claw_core::skill_registry::CapabilityIsolationProfile;

pub(super) struct ChildTaskExecutionScope {
    scoped_state: Option<AppState>,
    permission_profile: Option<String>,
    runtime: Option<ExecutionIsolationRuntime>,
}

impl ChildTaskExecutionScope {
    pub(super) fn prepare(
        state: &AppState,
        task: &ClaimedTask,
        payload: &Value,
    ) -> anyhow::Result<Self> {
        if !crate::repo::child_tasks::is_child_subagent_payload(payload) {
            return Ok(Self {
                scoped_state: None,
                permission_profile: None,
                runtime: None,
            });
        }
        let permission_profile = payload
            .pointer("/child_task_contract/permission_profile")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("read_only");
        match permission_profile {
            "read_only" => Ok(Self {
                scoped_state: None,
                permission_profile: Some(permission_profile.to_string()),
                runtime: None,
            }),
            "local_worktree" => {
                let plan = crate::execution_isolation::plan_execution_isolation(
                    &state.skill_rt.workspace_root,
                    &task.task_id,
                    CapabilityIsolationProfile::LocalWorktree,
                )
                .context("child_worktree_plan_failed")?;
                let runtime = crate::execution_isolation::create_or_reuse_execution_isolation(
                    &plan,
                    crate::now_ts_u64(),
                )
                .context("child_worktree_allocation_failed")?;
                let mut scoped_state = state.clone();
                scoped_state.skill_rt.workspace_root = runtime.plan.execution_root.clone();
                scoped_state.skill_rt.default_locator_search_dir =
                    runtime.plan.execution_root.clone();
                Ok(Self {
                    scoped_state: Some(scoped_state),
                    permission_profile: Some(permission_profile.to_string()),
                    runtime: Some(runtime),
                })
            }
            _ => bail!("child_permission_profile_unsupported"),
        }
    }

    pub(super) fn state<'a>(&'a self, fallback: &'a AppState) -> &'a AppState {
        self.scoped_state.as_ref().unwrap_or(fallback)
    }

    pub(super) fn projection(&self, fallback: &AppState) -> Option<Value> {
        let permission_profile = self.permission_profile.as_deref()?;
        let workspace_root = self
            .runtime
            .as_ref()
            .map(|runtime| runtime.plan.execution_root.as_path())
            .unwrap_or(fallback.skill_rt.workspace_root.as_path());
        let patch_artifact = self.runtime.as_ref().map(|runtime| {
            crate::execution_isolation::build_child_worktree_patch_artifact(&runtime.plan)
                .unwrap_or_else(|_| {
                    json!({
                        "schema_version": 1,
                        "kind": "child_worktree_patch",
                        "status": "error",
                        "error_code": "child_worktree_patch_artifact_failed",
                        "reason_code": "child_worktree_patch_artifact_failed",
                        "failure_stage": "patch_artifact_build",
                        "apply_owner": "parent_agent",
                        "apply_policy": "parent_review_required",
                        "cleanup_ref": runtime.plan.cleanup_ref,
                    })
                })
        });
        let mut artifact_refs = self
            .runtime
            .as_ref()
            .map(|runtime| runtime.artifact_refs.clone())
            .unwrap_or_default();
        if let Some(patch_artifact) = patch_artifact.as_ref() {
            artifact_refs.push(patch_artifact.clone());
        }
        Some(json!({
            "schema_version": 1,
            "owner_layer": "child_task_execution_scope",
            "status": "bound",
            "permission_profile": permission_profile,
            "workspace_binding": if self.runtime.is_some() {
                "isolated_worktree"
            } else {
                "primary_workspace_read_only"
            },
            "workspace_root": workspace_root.display().to_string(),
            "allocation_reused": self.runtime.as_ref().is_some_and(|runtime| runtime.reused),
            "artifact_refs": artifact_refs,
            "patch_artifact": patch_artifact,
            "cleanup_policy": if self.runtime.is_some() {
                "parent_owned_after_patch_decision"
            } else {
                "not_required"
            },
        }))
    }

    #[cfg(test)]
    pub(super) fn plan(&self) -> Option<&ExecutionIsolationPlan> {
        self.runtime.as_ref().map(|runtime| &runtime.plan)
    }
}

#[cfg(test)]
#[path = "child_task_execution_scope_tests.rs"]
mod tests;
