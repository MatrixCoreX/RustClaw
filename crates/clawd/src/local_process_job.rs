use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static ATOMIC_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessIdentityState {
    AliveVerified,
    Missing,
    IdentityMismatch,
    Unknown,
}

impl ProcessIdentityState {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::AliveVerified => "alive_verified",
            Self::Missing => "missing",
            Self::IdentityMismatch => "identity_mismatch",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn alive(self) -> Option<bool> {
        match self {
            Self::AliveVerified => Some(true),
            Self::Missing | Self::IdentityMismatch => Some(false),
            Self::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutputSnapshot {
    pub(crate) text: String,
    pub(crate) start_cursor: u64,
    pub(crate) end_cursor: u64,
    pub(crate) total_bytes: u64,
    pub(crate) truncated: bool,
    pub(crate) cursor_reset: bool,
    pub(crate) encoding: &'static str,
}

pub(crate) fn read_pid(job_dir: &Path) -> Option<u32> {
    std::fs::read_to_string(job_dir.join("pid"))
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid > 0)
}

pub(crate) fn read_i64(job_dir: &Path, name: &str) -> Option<i64> {
    std::fs::read_to_string(job_dir.join(name))
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()
}

pub(crate) fn read_u64(job_dir: &Path, name: &str) -> Option<u64> {
    std::fs::read_to_string(job_dir.join(name))
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

pub(crate) fn read_runtime_timeout_seconds(job_dir: &Path) -> Option<u64> {
    std::fs::read_to_string(job_dir.join("runtime_timeout_seconds"))
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
}

pub(crate) fn read_retention_seconds(job_dir: &Path) -> Option<u64> {
    read_u64(job_dir, "retention_seconds").filter(|seconds| *seconds > 0)
}

pub(crate) fn process_identity_state(job_dir: &Path, pid: u32) -> ProcessIdentityState {
    #[cfg(unix)]
    {
        let alive = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match alive {
            Ok(status) if status.success() => {}
            Ok(_) => {
                return if process_group_exists(pid) == Some(true) {
                    ProcessIdentityState::AliveVerified
                } else {
                    ProcessIdentityState::Missing
                };
            }
            Err(_) => return ProcessIdentityState::Unknown,
        }
        let pid_string = pid.to_string();
        let output = match Command::new("ps")
            .args(["-o", "command=", "-p", pid_string.as_str()])
            .output()
        {
            Ok(output) if output.status.success() => output,
            Ok(_) => return ProcessIdentityState::Missing,
            Err(_) => return ProcessIdentityState::Unknown,
        };
        let command = String::from_utf8_lossy(&output.stdout);
        let run_script = job_dir.join("run.sh").to_string_lossy().to_string();
        if command.contains(&run_script) {
            ProcessIdentityState::AliveVerified
        } else {
            ProcessIdentityState::IdentityMismatch
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (job_dir, pid);
        ProcessIdentityState::Unknown
    }
}

fn process_group_exists(process_group_id: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", "--", &format!("-{process_group_id}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()
            .map(|status| status.success())
    }
    #[cfg(not(unix))]
    {
        let _ = process_group_id;
        None
    }
}

/// Requires a durable second observation before treating a vanished process as
/// lost. A short-lived command can exit immediately before its wrapper commits
/// the terminal `exit_code` record, so the first missing observation is only a
/// pending terminal-record state. An identity mismatch gets the same bounded
/// observation grace because an exiting wrapper can briefly appear as a zombie
/// without its original command line. This grace never authorizes signalling:
/// cancellation still refuses to target an identity mismatch.
pub(crate) fn process_loss_is_stable(
    job_dir: &Path,
    identity_state: ProcessIdentityState,
    now_ts: i64,
    observation_grace_seconds: u64,
) -> bool {
    let marker = job_dir.join("process_missing_observed_at");
    match identity_state {
        ProcessIdentityState::Missing | ProcessIdentityState::IdentityMismatch => {
            let Some(first_observed_at) = read_i64(job_dir, "process_missing_observed_at") else {
                let _ = write_atomic(&marker, &now_ts.to_string());
                return false;
            };
            now_ts
                >= first_observed_at
                    .saturating_add(observation_grace_seconds.max(1).min(i64::MAX as u64) as i64)
        }
        ProcessIdentityState::AliveVerified => {
            let _ = std::fs::remove_file(marker);
            false
        }
        ProcessIdentityState::Unknown => false,
    }
}

pub(crate) fn read_output_delta(
    path: &Path,
    cursor_path: &Path,
    max_bytes: usize,
) -> OutputSnapshot {
    let total_bytes = std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let max_bytes = max_bytes.max(1) as u64;
    let requested_cursor = std::fs::read_to_string(cursor_path)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let cursor_reset = requested_cursor > total_bytes;
    let start_cursor = if cursor_reset { 0 } else { requested_cursor };
    let mut bytes = Vec::with_capacity(
        usize::try_from(total_bytes.saturating_sub(start_cursor).min(max_bytes)).unwrap_or(0),
    );
    if let Ok(mut file) = File::open(path) {
        let _ = file.seek(SeekFrom::Start(start_cursor));
        let _ = file
            .take(max_bytes.saturating_add(3))
            .read_to_end(&mut bytes);
    }
    let (visible_len, encoding) = utf8_delta_boundary(&bytes, max_bytes as usize);
    let visible_bytes = &bytes[..visible_len];
    let end_cursor = start_cursor.saturating_add(visible_bytes.len() as u64);
    let _ = write_atomic(cursor_path, &end_cursor.to_string());
    OutputSnapshot {
        text: String::from_utf8_lossy(visible_bytes).to_string(),
        start_cursor,
        end_cursor,
        total_bytes,
        truncated: end_cursor < total_bytes,
        cursor_reset,
        encoding,
    }
}

fn utf8_delta_boundary(bytes: &[u8], requested_max: usize) -> (usize, &'static str) {
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            let target = requested_max.min(bytes.len());
            let end = text
                .char_indices()
                .map(|(index, ch)| index + ch.len_utf8())
                .find(|end| *end >= target)
                .unwrap_or(bytes.len());
            (end, "utf-8")
        }
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            if valid == 0 {
                (0, "utf-8")
            } else {
                (valid, "utf-8")
            }
        }
        Err(_) => (requested_max.min(bytes.len()), "utf-8-lossy"),
    }
}

pub(crate) fn write_atomic(path: &Path, value: &str) -> std::io::Result<()> {
    let sequence = ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    std::fs::write(&temp_path, value)?;
    std::fs::rename(temp_path, path)
}

pub(crate) fn terminate_verified_process_group(job_dir: &Path, pid: u32, signal: &str) -> bool {
    if process_identity_state(job_dir, pid) != ProcessIdentityState::AliveVerified {
        return false;
    }
    signal_process_group_or_pid(pid, signal)
}

fn signal_process_group_or_pid(pid: u32, signal: &str) -> bool {
    signal_process_group(pid, signal) || signal_process(pid, signal)
}

fn signal_process_group(pid: u32, signal: &str) -> bool {
    #[cfg(unix)]
    {
        let signal = match signal {
            "KILL" => "-KILL",
            "INT" => "-INT",
            _ => "-TERM",
        };
        Command::new("kill")
            .arg(signal)
            .arg("--")
            .arg(format!("-{pid}"))
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, signal);
        false
    }
}

fn signal_process(pid: u32, signal: &str) -> bool {
    #[cfg(unix)]
    {
        let signal = match signal {
            "KILL" => "-KILL",
            "INT" => "-INT",
            _ => "-TERM",
        };
        Command::new("kill")
            .arg(signal)
            .arg(pid.to_string())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, signal);
        false
    }
}

pub(crate) fn schedule_kill_after_grace(job_dir: &Path, pid: u32, grace_seconds: u64) {
    let job_dir = job_dir.to_path_buf();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(grace_seconds.max(1)));
        if job_dir.join("cancel_requested_at").exists()
            && !job_dir.join("exit_code").exists()
            && signal_process_group(pid, "KILL")
        {
            let _ = write_atomic(&job_dir.join("cancel_escalated_signal"), "KILL");
        }
    });
}

/// Replays cancellation escalation from durable metadata after a service restart.
pub(crate) fn maybe_escalate_cancel(job_dir: &Path, now_ts: i64) -> &'static str {
    let Some(cancel_requested_at) = read_i64(job_dir, "cancel_requested_at") else {
        return "not_requested";
    };
    if job_dir.join("exit_code").exists() {
        return "terminal_observed";
    }
    if job_dir.join("cancel_escalated_signal").exists() {
        return "already_escalated";
    }
    let grace_seconds = read_u64(job_dir, "terminate_grace_seconds")
        .unwrap_or(5)
        .max(1)
        .min(i64::MAX as u64) as i64;
    if now_ts < cancel_requested_at.saturating_add(grace_seconds) {
        return "term_grace_active";
    }
    let Some(pid) = read_pid(job_dir) else {
        return "pid_missing";
    };
    match process_identity_state(job_dir, pid) {
        ProcessIdentityState::AliveVerified => {
            if terminate_verified_process_group(job_dir, pid, "KILL") {
                let _ = write_atomic(&job_dir.join("cancel_escalated_signal"), "KILL");
                "kill_sent"
            } else {
                "kill_failed"
            }
        }
        ProcessIdentityState::Missing => {
            // The verified group leader may have exited after TERM while a child
            // ignored it. Only use the durable group id inside a short fresh
            // cancellation window; after that, avoid any PID/PGID reuse risk.
            let latest_safe_escalation = cancel_requested_at
                .saturating_add(grace_seconds)
                .saturating_add(60);
            if now_ts <= latest_safe_escalation && signal_process_group(pid, "KILL") {
                let _ = write_atomic(&job_dir.join("cancel_escalated_signal"), "KILL");
                "kill_sent"
            } else {
                "process_gone"
            }
        }
        ProcessIdentityState::IdentityMismatch => "identity_mismatch",
        ProcessIdentityState::Unknown => "identity_unknown",
    }
}

#[cfg(test)]
#[path = "local_process_job_tests.rs"]
mod tests;
