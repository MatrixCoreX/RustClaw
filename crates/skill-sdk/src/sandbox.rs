use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::{SkillSdkError, SkillSdkResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxNetwork {
    Deny,
    Allow,
}

#[derive(Debug)]
pub struct PreparedSandboxCommand {
    pub command: Command,
    pub backend: &'static str,
}

pub fn prepare_sandboxed_command(
    program: &Path,
    execution_root: &Path,
    writable_paths: &[PathBuf],
    network: SandboxNetwork,
) -> SkillSdkResult<PreparedSandboxCommand> {
    if !program.is_absolute() || !program.is_file() {
        return Err(
            SkillSdkError::new("sandbox_program_invalid", program.display().to_string())
                .phase("preflight"),
        );
    }
    let execution_root = canonical_directory(execution_root)?;
    let writable_paths = writable_paths
        .iter()
        .map(|path| canonical_directory(path))
        .collect::<SkillSdkResult<Vec<_>>>()?;
    #[cfg(target_os = "linux")]
    {
        prepare_bubblewrap(program, &execution_root, &writable_paths, network)
    }
    #[cfg(target_os = "macos")]
    {
        prepare_macos_seatbelt(program, &execution_root, &writable_paths, network)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (execution_root, writable_paths, network);
        Err(SkillSdkError::new(
            "sandbox_platform_unsupported",
            format!("platform={}", std::env::consts::OS),
        )
        .phase("preflight"))
    }
}

fn canonical_directory(path: &Path) -> SkillSdkResult<PathBuf> {
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        SkillSdkError::new(
            "sandbox_directory_unavailable",
            format!("path={} error={error}", path.display()),
        )
        .phase("preflight")
    })?;
    if !canonical.is_dir() {
        return Err(SkillSdkError::new(
            "sandbox_path_not_directory",
            canonical.display().to_string(),
        )
        .phase("preflight"));
    }
    Ok(canonical)
}

#[cfg(target_os = "linux")]
fn prepare_bubblewrap(
    program: &Path,
    execution_root: &Path,
    writable_paths: &[PathBuf],
    network: SandboxNetwork,
) -> SkillSdkResult<PreparedSandboxCommand> {
    let backend = [Path::new("/usr/bin/bwrap"), Path::new("/bin/bwrap")]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            SkillSdkError::new("sandbox_backend_unavailable", "bubblewrap not found")
                .phase("preflight")
        })?;
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
    let host_tmp = Path::new("/tmp");
    if !execution_root.starts_with(host_tmp)
        && !writable_paths.iter().any(|path| path.starts_with(host_tmp))
    {
        command.arg("--tmpfs").arg(host_tmp);
    }
    if network == SandboxNetwork::Deny {
        command.arg("--unshare-net");
    }
    for path in writable_paths {
        command.arg("--bind").arg(path).arg(path);
    }
    command
        .arg("--chdir")
        .arg(execution_root)
        .arg("--")
        .arg(program);
    Ok(PreparedSandboxCommand {
        command,
        backend: "bubblewrap",
    })
}

#[cfg(target_os = "macos")]
fn prepare_macos_seatbelt(
    program: &Path,
    execution_root: &Path,
    writable_paths: &[PathBuf],
    network: SandboxNetwork,
) -> SkillSdkResult<PreparedSandboxCommand> {
    let backend = Path::new("/usr/bin/sandbox-exec");
    if !backend.is_file() {
        return Err(SkillSdkError::new(
            "sandbox_backend_unavailable",
            "macOS sandbox-exec not found",
        )
        .phase("preflight"));
    }
    let mut profile = String::from(
        "(version 1)\n(deny default)\n(import \"system.sb\")\n(allow process*)\n(allow file-read*)\n",
    );
    if network == SandboxNetwork::Allow {
        profile.push_str("(allow network*)\n");
    } else {
        profile.push_str("(deny network*)\n");
    }
    for path in writable_paths {
        let value = path.to_string_lossy();
        if value
            .chars()
            .any(|value| matches!(value, '\0' | '\n' | '\r'))
        {
            return Err(SkillSdkError::new(
                "sandbox_path_invalid",
                path.display().to_string(),
            ));
        }
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        profile.push_str(&format!("(allow file-write* (subpath \"{escaped}\"))\n"));
    }
    let mut command = Command::new(backend);
    command
        .arg("-p")
        .arg(profile)
        .arg(program)
        .current_dir(execution_root);
    Ok(PreparedSandboxCommand {
        command,
        backend: "macos_seatbelt",
    })
}
