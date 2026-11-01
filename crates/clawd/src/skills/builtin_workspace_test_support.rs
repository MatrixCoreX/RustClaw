pub(super) use super::builtin_workspace_mutation::MutationAfterShape;
use crate::runtime::host_filesystem::HostFilesystemPolicy;
use claw_core::config::ToolSandboxBackend;
use serde_json::{Map, Value};
use std::path::Path;

pub(super) fn atomic_write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    HostFilesystemPolicy::without_private_authority()
        .atomic_write_file_with_options(None, path, bytes, false)
}

pub(super) fn run_checkpointed_workspace_mutation<F>(
    root: &Path,
    task_id: &str,
    action: &str,
    target: &Path,
    after_shape: MutationAfterShape,
    operation: F,
) -> Result<String, String>
where
    F: FnOnce() -> Result<(), String>,
{
    super::builtin_workspace_mutation::run_checkpointed_workspace_mutation_with_policy(
        root,
        task_id,
        action,
        target,
        after_shape,
        &HostFilesystemPolicy::without_private_authority(),
        operation,
    )
}

pub(super) fn run_authorized_mutation<F>(
    root: &Path,
    task_id: &str,
    action: &str,
    target: &Path,
    after_shape: MutationAfterShape,
    authorized: bool,
    operation: F,
) -> Result<String, String>
where
    F: FnOnce() -> Result<(), String>,
{
    super::builtin_workspace_mutation::run_authorized_mutation_with_policy(
        root,
        task_id,
        action,
        target,
        after_shape,
        authorized,
        &HostFilesystemPolicy::without_private_authority(),
        operation,
    )
}

pub(super) fn structured_mutation_diff(root: &Path, id: &str) -> Result<String, String> {
    super::builtin_workspace_mutation::structured_mutation_diff_with_policy(
        root,
        id,
        &HostFilesystemPolicy::without_private_authority(),
    )
}

pub(super) fn rewind_structured_mutation(root: &Path, id: &str) -> Result<String, String> {
    super::builtin_workspace_mutation::rewind_structured_mutation_with_policy(
        root,
        id,
        &HostFilesystemPolicy::without_private_authority(),
    )
}

pub(super) fn execute_workspace_patch_for_root(
    root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
) -> Result<String, String> {
    super::builtin_workspace_patch::execute_workspace_patch_with_policy(
        root,
        task_id,
        args,
        &HostFilesystemPolicy::without_private_authority(),
        ToolSandboxBackend::Auto,
    )
}

pub(super) fn execute_workspace_replace_for_root(
    root: &Path,
    task_id: &str,
    args: &Map<String, Value>,
) -> Result<String, String> {
    super::builtin_workspace_replace::execute_workspace_replace_with_policy(
        root,
        task_id,
        args,
        &HostFilesystemPolicy::without_private_authority(),
    )
}
