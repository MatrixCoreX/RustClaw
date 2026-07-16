use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use claw_core::config::ToolSandboxMode;
use tokio::process::Command;

const BUBBLEWRAP_BACKEND: &str = "bubblewrap";
const DIRECT_BACKEND: &str = "direct";
const INTERNAL_WRITABLE_ROOT: &str = "/run/rustclaw-writable";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessNetworkPolicy {
    Deny,
    Inherit,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessSandboxRequest<'a> {
    pub(crate) mode: ToolSandboxMode,
    pub(crate) workspace_root: &'a Path,
    pub(crate) execution_root: &'a Path,
    pub(crate) network: ProcessNetworkPolicy,
    pub(crate) additional_writable_paths: &'a [PathBuf],
}

pub(crate) struct PreparedProcessCommand {
    pub(crate) command: Command,
    pub(crate) backend: &'static str,
    pub(crate) additional_writable_targets: Vec<PathBuf>,
}

pub(crate) fn prepare_process_command(
    program: impl AsRef<OsStr>,
    request: ProcessSandboxRequest<'_>,
) -> Result<PreparedProcessCommand, &'static str> {
    if request.mode == ToolSandboxMode::DangerFull {
        return Ok(PreparedProcessCommand {
            command: Command::new(program),
            backend: DIRECT_BACKEND,
            additional_writable_targets: request.additional_writable_paths.to_vec(),
        });
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = program;
        let _ = request;
        return Err("sandbox_backend_unsupported_platform");
    }

    #[cfg(target_os = "linux")]
    {
        let backend = find_bubblewrap().ok_or("sandbox_backend_unavailable")?;
        let workspace_root = canonical_directory(request.workspace_root)?;
        let execution_root = canonical_directory(request.execution_root)?;
        if matches!(
            request.mode,
            ToolSandboxMode::WorkspaceWrite | ToolSandboxMode::IsolatedWorktree
        ) && !execution_root.starts_with(&workspace_root)
        {
            return Err("sandbox_execution_root_outside_workspace");
        }

        let mut command = Command::new(backend);
        command
            .arg("--die-with-parent")
            .arg("--new-session")
            .arg("--unshare-pid")
            .arg("--unshare-ipc")
            .arg("--unshare-uts")
            .arg("--ro-bind")
            .arg("/")
            .arg("/")
            .arg("--proc")
            .arg("/proc")
            .arg("--dev")
            .arg("/dev");
        if request.network == ProcessNetworkPolicy::Deny {
            command.arg("--unshare-net");
        }
        if matches!(
            request.mode,
            ToolSandboxMode::WorkspaceWrite | ToolSandboxMode::IsolatedWorktree
        ) {
            command
                .arg("--bind")
                .arg(&workspace_root)
                .arg(&workspace_root);
        }
        let mut additional_writable_targets = Vec::new();
        if !request.additional_writable_paths.is_empty() {
            command
                .arg("--tmpfs")
                .arg("/run")
                .arg("--dir")
                .arg(INTERNAL_WRITABLE_ROOT);
        }
        for (index, path) in request.additional_writable_paths.iter().enumerate() {
            let source = canonical_directory(path)?;
            let target = Path::new(INTERNAL_WRITABLE_ROOT).join(index.to_string());
            command
                .arg("--dir")
                .arg(&target)
                .arg("--bind")
                .arg(&source)
                .arg(&target);
            additional_writable_targets.push(target);
        }
        command
            .arg("--tmpfs")
            .arg("/tmp")
            .arg("--chdir")
            .arg(&execution_root)
            .arg("--")
            .arg(program);
        Ok(PreparedProcessCommand {
            command,
            backend: BUBBLEWRAP_BACKEND,
            additional_writable_targets,
        })
    }
}

#[cfg(target_os = "linux")]
fn canonical_directory(path: &Path) -> Result<PathBuf, &'static str> {
    let canonical = path
        .canonicalize()
        .map_err(|_| "sandbox_directory_unavailable")?;
    if !canonical.is_dir() {
        return Err("sandbox_path_not_directory");
    }
    Ok(canonical)
}

#[cfg(target_os = "linux")]
fn find_bubblewrap() -> Option<PathBuf> {
    [Path::new("/usr/bin/bwrap"), Path::new("/bin/bwrap")]
        .into_iter()
        .find(|path| path.is_file())
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|root| root.join("bwrap"))
                    .find(|path| path.is_file())
            })
        })
}

#[cfg(test)]
#[path = "process_sandbox_tests.rs"]
mod tests;
