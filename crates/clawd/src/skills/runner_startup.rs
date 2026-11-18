use std::future::Future;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::process::Child;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::super::runner_pool::WarmRunnerProcess;
use super::launch_spec::{launch_error, RunnerLaunch};
use super::{
    open_durable_job_output, read_last_runner_response, terminate_subprocess_group,
    write_durable_job_metadata, DurableRunnerJobPlan, DurableRunnerJobSetupGuard,
};
use crate::local_process_owner::process_launch::PayloadIo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmissionFailure {
    Cancelled,
    DeadlineElapsed,
}

// One window covers request transmission and the durable lease handoff. It
// never becomes the runtime deadline of an accepted background operation.
pub(super) struct RunnerAdmission<'a> {
    deadline: Instant,
    cancellation: Option<&'a CancellationToken>,
}

impl<'a> RunnerAdmission<'a> {
    pub(super) fn new(cancellation: Option<&'a CancellationToken>) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(120),
            cancellation,
        }
    }

    fn check(&self) -> Result<(), AdmissionFailure> {
        if self
            .cancellation
            .is_some_and(CancellationToken::is_cancelled)
        {
            Err(AdmissionFailure::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(AdmissionFailure::DeadlineElapsed)
        } else {
            Ok(())
        }
    }

    async fn wait<F: Future>(&self, operation: F) -> Result<F::Output, AdmissionFailure> {
        self.check()?;
        tokio::select! {
            biased;
            _ = super::wait_for_optional_cancellation(self.cancellation) => {
                Err(AdmissionFailure::Cancelled)
            }
            _ = tokio::time::sleep_until(self.deadline) => {
                Err(AdmissionFailure::DeadlineElapsed)
            }
            result = operation => Ok(result),
        }
    }
}

pub(super) async fn send_runner_request(
    runner: &mut WarmRunnerProcess,
    request: &str,
    skill_name: &str,
    admission: &RunnerAdmission<'_>,
    conversation_interrupt: Option<&CancellationToken>,
) -> Result<(), String> {
    let send = async {
        tokio::select! {
            biased;
            _ = super::wait_for_optional_cancellation(conversation_interrupt) => {
                Err(crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR.to_string())
            }
            sent = runner.send(request) => sent.map_err(|error| runner_io_error(
                skill_name, "skill_runner_stdin_write_failed", "stdin_write", error,
            )),
        }
    };
    let error = match admission.wait(send).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error,
        Err(AdmissionFailure::Cancelled) => crate::agent_engine::TASK_CANCELED_ERR.to_string(),
        Err(AdmissionFailure::DeadlineElapsed) => "skill_runner_admission_timeout".to_string(),
    };
    // A partial record must never be retried or returned to the warm pool.
    let _ = terminate_subprocess_group(runner.id()).await;
    runner.kill_and_wait().await;
    Err(error)
}

fn runner_io_error(
    skill_name: &str,
    error_code: &str,
    operation: &str,
    error: std::io::Error,
) -> String {
    super::super::structured_skill_error_from_parts(
        skill_name,
        error_code,
        &error.to_string(),
        Some(std::env::consts::OS),
        Some(json!({
            "message_key": "clawd.skill.runner_io_failed",
            "retryable": false,
            "failure_phase": "runner_admission",
            "completion_state": "unknown",
            "io_operation": operation,
            "os_error_code": error.raw_os_error(),
        })),
    )
}

async fn stop_starting_child(child: &mut Child) {
    let _ = terminate_subprocess_group(child.id()).await;
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn durable_admission_error(error: AdmissionFailure) -> &'static str {
    match error {
        AdmissionFailure::Cancelled => crate::agent_engine::TASK_CANCELED_ERR,
        AdmissionFailure::DeadlineElapsed => "durable_skill_job_lease_handoff_timeout",
    }
}

async fn fail_durable_admission(
    child: &mut Child,
    plan: &DurableRunnerJobPlan,
    error: AdmissionFailure,
) -> Result<Value, String> {
    stop_starting_child(child).await;
    let reason = match error {
        AdmissionFailure::Cancelled => "task_cancelled",
        AdmissionFailure::DeadlineElapsed => "lease_handoff_timeout",
    };
    write_durable_job_metadata(&plan.job_dir, "startup_failed", reason)?;
    Err(durable_admission_error(error).to_string())
}

pub(super) async fn start_durable_skill_runner_process(
    mut launch: RunnerLaunch,
    req_line: &str,
    request_id: &str,
    canonical_skill_name: &str,
    plan: &DurableRunnerJobPlan,
    mut setup_guard: DurableRunnerJobSetupGuard,
    expected_execution_binding: &Value,
    process_command_marker: &Path,
    host_artifact_output_directory: &Path,
    sandbox_artifact_output_directory: &Path,
    sandbox_job_dir: &Path,
    sandbox_global_slot_root: &Path,
    sandbox_skill_slot_root: &Path,
    global_max_concurrency: usize,
    skill_max_concurrency: usize,
    terminate_grace_seconds: u64,
    admission: &RunnerAdmission<'_>,
) -> Result<Value, String> {
    admission.check().map_err(durable_admission_error)?;
    for (name, value) in [
        ("job_id", plan.job_id.as_str()),
        ("job_kind", "skill_runner"),
        ("skill_name", canonical_skill_name),
        ("runtime_timeout_seconds", "disabled"),
        ("poll_after_seconds", &plan.poll_after_seconds.to_string()),
        ("retention_seconds", &plan.retention_seconds.to_string()),
        (
            "retention_deadline_at",
            &plan.retention_deadline_at.to_string(),
        ),
        (
            "terminate_grace_seconds",
            &terminate_grace_seconds.max(1).to_string(),
        ),
        (
            "process_command_marker",
            &process_command_marker.display().to_string(),
        ),
        (
            "host_artifact_output_directory",
            &host_artifact_output_directory.display().to_string(),
        ),
        (
            "sandbox_artifact_output_directory",
            &sandbox_artifact_output_directory.display().to_string(),
        ),
    ] {
        write_durable_job_metadata(&plan.job_dir, name, value)?;
    }
    write_durable_job_metadata(
        &plan.job_dir,
        "expected_execution_binding.json",
        &expected_execution_binding.to_string(),
    )?;

    let stdout = open_durable_job_output(&plan.job_dir.join("stdout"))?;
    let stderr = open_durable_job_output(&plan.job_dir.join("stderr"))?;
    launch
        .env_remove("SKILL_TIMEOUT_SECONDS")
        .env("APP_DURABLE_BACKGROUND", "1")
        .env("APP_DURABLE_JOB_DIR", sandbox_job_dir)
        .env("APP_DURABLE_GLOBAL_SLOT_ROOT", sandbox_global_slot_root)
        .env(
            "APP_DURABLE_GLOBAL_MAX_CONCURRENCY",
            global_max_concurrency.max(1).to_string(),
        )
        .env("APP_DURABLE_SKILL_SLOT_ROOT", sandbox_skill_slot_root)
        .env(
            "APP_DURABLE_SKILL_MAX_CONCURRENCY",
            skill_max_concurrency.max(1).to_string(),
        );
    let mut cmd = tokio::process::Command::from(
        launch
            .finish(PayloadIo {
                stdin: None,
                stdout: Some(stdout),
                stderr: Some(stderr),
            })
            .map_err(|error| launch_error(canonical_skill_name, error))?
            .into_command(),
    );
    super::place_subprocess_in_own_process_group(&mut cmd);
    cmd.stdin(Stdio::piped()).kill_on_drop(false);
    admission.check().map_err(durable_admission_error)?;
    setup_guard.begin_launch_attempt();
    let mut child = cmd.spawn().map_err(|error| {
        runner_io_error(
            canonical_skill_name,
            "durable_skill_job_spawn_failed",
            "process_spawn",
            error,
        )
    })?;
    let pid = child
        .id()
        .ok_or_else(|| "durable_skill_job_pid_missing".to_string())?;
    if let Err(error) = write_durable_job_metadata(&plan.job_dir, "pid", &pid.to_string()) {
        stop_starting_child(&mut child).await;
        return Err(error);
    }
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "durable_skill_job_stdin_missing".to_string())?;
    let input = admission
        .wait(async {
            stdin
                .write_all(req_line.as_bytes())
                .await
                .map_err(|error| {
                    runner_io_error(
                        canonical_skill_name,
                        "durable_skill_job_stdin_write_failed",
                        "stdin_write",
                        error,
                    )
                })?;
            stdin.write_all(b"\n").await.map_err(|error| {
                runner_io_error(
                    canonical_skill_name,
                    "durable_skill_job_stdin_write_failed",
                    "stdin_write",
                    error,
                )
            })?;
            stdin.shutdown().await.map_err(|error| {
                runner_io_error(
                    canonical_skill_name,
                    "durable_skill_job_stdin_close_failed",
                    "stdin_close",
                    error,
                )
            })
        })
        .await;
    drop(stdin);
    match input {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            stop_starting_child(&mut child).await;
            return Err(error);
        }
        Err(error) => return fail_durable_admission(&mut child, plan, error).await,
    }

    loop {
        if let Err(error) = admission.check() {
            return fail_durable_admission(&mut child, plan, error).await;
        }
        if plan.job_dir.join("lease_ready").exists() {
            return Ok(json!({
                "request_id": request_id,
                "status": "ok",
                "text": "",
                "buttons": null,
                "validation": null,
                "extra": {
                    "schema_version": 1,
                    "source_skill": "skill_runner",
                    "status": "ok",
                    "action": "durable_background_start",
                    "execution_binding": expected_execution_binding,
                    "runner_dispatch": {
                        "schema_version": 1,
                        "mode": "durable_background",
                        "fallback_reason": null
                    },
                    "pending_async_job": {
                        "job_id": plan.job_id,
                        "provider": "local_process",
                        "status": "accepted",
                        "poll_after_seconds": plan.poll_after_seconds,
                        "poll_after_ms": plan.poll_after_seconds.saturating_mul(1_000),
                        "expires_at": plan.retention_deadline_at,
                        "runtime_deadline_at": null,
                        "retention_deadline_at": plan.retention_deadline_at,
                        "cancel_ref": format!("local_process:{}", plan.job_dir.display()),
                        "cancel_token": format!("local_process:{}", plan.job_dir.display()),
                        "result_ref": plan.job_id,
                        "retryable": true,
                        "message_key": "clawd.task.async_job_pending"
                    }
                },
                "error_text": null
            }));
        }
        let exited = plan.job_dir.join("exit_code").exists()
            || child
                .try_wait()
                .map_err(|error| {
                    runner_io_error(
                        canonical_skill_name,
                        "durable_skill_job_wait_failed",
                        "process_wait",
                        error,
                    )
                })?
                .is_some();
        if exited {
            let _ = child.wait().await;
            return read_last_runner_response(&plan.job_dir.join("stdout"));
        }
        if let Err(error) = admission
            .wait(tokio::time::sleep(Duration::from_millis(25)))
            .await
        {
            return fail_durable_admission(&mut child, plan, error).await;
        }
    }
}

#[cfg(test)]
#[path = "runner_startup_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runner_startup_path_tests.rs"]
mod path_tests;

#[cfg(test)]
#[path = "runner_handoff_tests.rs"]
mod handoff_tests;
