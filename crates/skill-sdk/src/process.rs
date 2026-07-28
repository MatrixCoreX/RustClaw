use std::io::{Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::{SkillSdkError, SkillSdkResult};

const MAX_CAPTURE_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub(crate) struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub(crate) fn run_command(
    command: &mut Command,
    stdin: Option<&[u8]>,
    timeout: Duration,
    phase: &str,
) -> SkillSdkResult<ProcessOutput> {
    run_command_controlled(command, stdin, timeout, phase, None)
}

pub(crate) fn run_command_controlled(
    command: &mut Command,
    stdin: Option<&[u8]>,
    timeout: Duration,
    phase: &str,
    cancelled: Option<&AtomicBool>,
) -> SkillSdkResult<ProcessOutput> {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| {
        SkillSdkError::new("process_start_failed", error.to_string()).phase(phase)
    })?;
    if let Some(input) = stdin {
        let mut handle = child.stdin.take().ok_or_else(|| {
            SkillSdkError::new("process_stdin_unavailable", "stdin pipe missing").phase(phase)
        })?;
        handle.write_all(input).map_err(|error| {
            SkillSdkError::new("process_stdin_failed", error.to_string()).phase(phase)
        })?;
    }
    let stdout = child.stdout.take().ok_or_else(|| {
        SkillSdkError::new("process_stdout_unavailable", "stdout pipe missing").phase(phase)
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        SkillSdkError::new("process_stderr_unavailable", "stderr pipe missing").phase(phase)
    })?;
    let stdout_thread = thread::spawn(move || read_bounded(stdout));
    let stderr_thread = thread::spawn(move || read_bounded(stderr));
    let started = Instant::now();
    let status = loop {
        if cancelled.is_some_and(|value| value.load(Ordering::Acquire)) {
            terminate_process_group(&mut child);
            let _ = child.wait();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return Err(
                SkillSdkError::new("process_cancelled", "cancellation requested").phase(phase),
            );
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                terminate_process_group(&mut child);
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(SkillSdkError::new(
                    "process_timeout",
                    format!("timeout_seconds={}", timeout.as_secs()),
                )
                .retryable(true)
                .phase(phase));
            }
            Err(error) => {
                terminate_process_group(&mut child);
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(
                    SkillSdkError::new("process_wait_failed", error.to_string()).phase(phase)
                );
            }
        }
    };
    let stdout = stdout_thread.join().map_err(|_| {
        SkillSdkError::new("process_stdout_join_failed", "stdout reader panicked").phase(phase)
    })??;
    let stderr = stderr_thread.join().map_err(|_| {
        SkillSdkError::new("process_stderr_join_failed", "stderr reader panicked").phase(phase)
    })??;
    Ok(ProcessOutput {
        status,
        stdout,
        stderr,
    })
}

fn read_bounded(reader: impl Read) -> SkillSdkResult<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_CAPTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| SkillSdkError::new("process_output_read_failed", error.to_string()))?;
    if bytes.len() as u64 > MAX_CAPTURE_BYTES {
        return Err(SkillSdkError::new(
            "process_output_oversized",
            format!("limit_bytes={MAX_CAPTURE_BYTES}"),
        ));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn terminate_process_group(child: &mut std::process::Child) {
    let process_group = -(child.id() as i32);
    // SAFETY: `kill` is called with the child process group created above.
    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn terminate_process_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
