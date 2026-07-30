use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::{DiscoveryBudget, RipgrepStatus};

const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub(crate) struct RipgrepBinary {
    pub(crate) path: PathBuf,
    pub(crate) version: String,
}

pub(crate) struct CapturedOutput {
    pub(crate) stdout: Vec<u8>,
    pub(crate) output_truncated: bool,
    pub(crate) timed_out: bool,
    pub(crate) cancelled: bool,
    pub(crate) status: ExitStatus,
}

static RESOLVED: OnceLock<Result<RipgrepBinary, String>> = OnceLock::new();

pub(crate) fn status() -> RipgrepStatus {
    match resolve_binary() {
        Ok(binary) => RipgrepStatus {
            available: true,
            executable: Some(binary.path.display().to_string()),
            version: Some(binary.version.clone()),
            reason_code: None,
        },
        Err(reason_code) => RipgrepStatus {
            available: false,
            executable: None,
            version: None,
            reason_code: Some(reason_code),
        },
    }
}

pub(crate) fn resolve_binary() -> Result<&'static RipgrepBinary, String> {
    RESOLVED
        .get_or_init(resolve_binary_uncached)
        .as_ref()
        .map_err(Clone::clone)
}

pub(crate) fn base_command(binary: &RipgrepBinary, cwd: &Path) -> Command {
    let mut command = Command::new(&binary.path);
    command.env_clear().env("LANG", "C").current_dir(cwd);
    command
}

pub(crate) fn run_bounded(
    mut command: Command,
    budget: &DiscoveryBudget,
    max_capture_bytes: usize,
    allow_no_match_exit: bool,
) -> Result<CapturedOutput, String> {
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "ripgrep_spawn_failed".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ripgrep_stdout_missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "ripgrep_stderr_missing".to_string())?;
    let stdout_reader = thread::spawn(move || read_limited(stdout, max_capture_bytes));
    let stderr_reader = thread::spawn(move || read_limited(stderr, MAX_DIAGNOSTIC_BYTES));
    let started = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        if budget
            .deadline
            .is_some_and(|deadline| started.elapsed() >= deadline)
        {
            timed_out = true;
            terminate(&mut child);
            break child
                .wait()
                .map_err(|_| "ripgrep_wait_failed".to_string())?;
        }
        if budget
            .cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
        {
            cancelled = true;
            terminate(&mut child);
            break child
                .wait()
                .map_err(|_| "ripgrep_wait_failed".to_string())?;
        }
        match child
            .try_wait()
            .map_err(|_| "ripgrep_wait_failed".to_string())?
        {
            Some(status) => break status,
            None => thread::sleep(Duration::from_millis(5)),
        }
    };
    let (stdout, output_truncated) = stdout_reader
        .join()
        .map_err(|_| "ripgrep_stdout_reader_failed".to_string())??;
    let (stderr, _) = stderr_reader
        .join()
        .map_err(|_| "ripgrep_stderr_reader_failed".to_string())??;
    let accepted_no_match = allow_no_match_exit && status.code() == Some(1);
    if !status.success() && !accepted_no_match && !timed_out && !cancelled && !output_truncated {
        let _diagnostic = String::from_utf8_lossy(&stderr);
        return Err("ripgrep_nonzero_exit".to_string());
    }
    Ok(CapturedOutput {
        stdout,
        output_truncated,
        timed_out,
        cancelled,
        status,
    })
}

fn resolve_binary_uncached() -> Result<RipgrepBinary, String> {
    let candidates = explicit_candidate()
        .into_iter()
        .chain(path_candidates())
        .collect::<Vec<_>>();
    for candidate in candidates {
        let Ok(path) = candidate.canonicalize() else {
            continue;
        };
        if !path.is_file() || !is_executable(&path) {
            continue;
        }
        let output = Command::new(&path)
            .env_clear()
            .env("LANG", "C")
            .arg("--version")
            .output();
        let Ok(output) = output else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let first_line = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if first_line.starts_with("ripgrep ") {
            return Ok(RipgrepBinary {
                path,
                version: first_line,
            });
        }
    }
    Err("ripgrep_not_found_or_unverified".to_string())
}

fn explicit_candidate() -> Option<PathBuf> {
    std::env::var_os("APP_RG_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn path_candidates() -> impl Iterator<Item = PathBuf> {
    let executable = if cfg!(windows) { "rg.exe" } else { "rg" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .map(move |directory| directory.join(executable))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn read_limited(mut reader: impl Read, limit: usize) -> Result<(Vec<u8>, bool), String> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|_| "ripgrep_output_read_failed".to_string())?;
        if read == 0 {
            return Ok((out, false));
        }
        let remaining = limit.saturating_sub(out.len());
        out.extend_from_slice(&chunk[..read.min(remaining)]);
        if read > remaining || out.len() >= limit {
            return Ok((out, true));
        }
    }
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}
