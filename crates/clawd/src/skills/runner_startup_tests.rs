use super::*;
use crate::skills::place_subprocess_in_own_process_group;
use std::path::PathBuf;

pub(super) struct StartupFixture {
    pub(super) root: PathBuf,
    pub(super) plan: DurableRunnerJobPlan,
}

impl StartupFixture {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("runner-admission-{}", uuid::Uuid::new_v4()));
        let plan = DurableRunnerJobPlan::new(&root, "fixture_skill", 600, None);
        plan.create_directories()
            .expect("owned startup directories");
        Self { root, plan }
    }

    async fn start(&self, request: &str, budget: Duration) -> Result<Value, String> {
        self.start_command(
            Self::command("exec sleep 2"),
            request,
            &RunnerAdmission {
                deadline: Instant::now() + budget,
                cancellation: None,
            },
        )
        .await
    }

    pub(super) fn command(script: &str) -> RunnerLaunch {
        Self::command_in(script, &std::env::current_dir().unwrap())
    }

    pub(super) fn command_in(script: &str, directory: &Path) -> RunnerLaunch {
        let mut invocation = crate::process_invocation::ProcessInvocation::new("sh");
        invocation.arg("-c").arg(script);
        RunnerLaunch::new(
            invocation,
            directory,
            vec![("PATH".into(), "/usr/bin:/bin".into())],
        )
        .unwrap()
    }

    fn warm_command(script: &str) -> tokio::process::Command {
        let mut command = tokio::process::Command::from(
            Self::command(script)
                .finish(PayloadIo {
                    stdin: None,
                    stdout: None,
                    stderr: None,
                })
                .unwrap()
                .into_command(),
        );
        place_subprocess_in_own_process_group(&mut command);
        command
    }

    pub(super) async fn start_command(
        &self,
        command: RunnerLaunch,
        request: &str,
        admission: &RunnerAdmission<'_>,
    ) -> Result<Value, String> {
        start_durable_skill_runner_process(
            command,
            request,
            "fixture-request",
            "fixture_skill",
            &self.plan,
            super::super::DurableRunnerJobSetupGuard::new(&self.plan.job_dir),
            &json!({"schema_version": 1}),
            Path::new("fixture-runner"),
            &self.root,
            &self.root,
            &self.plan.job_dir,
            &self.plan.global_slot_root,
            &self.plan.skill_slot_root,
            1,
            1,
            1,
            admission,
        )
        .await
    }

    fn assert_reaped(&self) {
        let pid: libc::pid_t = std::fs::read_to_string(self.plan.job_dir.join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}

impl Drop for StartupFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).expect("remove owned startup fixture");
    }
}

#[tokio::test]
async fn non_reading_runner_cannot_outlive_its_admission_budget() {
    let fixture = StartupFixture::new();
    let request = "x".repeat(1024 * 1024);
    let result = fixture.start(&request, Duration::from_millis(80)).await;
    assert_eq!(
        result.unwrap_err(),
        "durable_skill_job_lease_handoff_timeout"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.plan.job_dir.join("startup_failed")).unwrap(),
        "lease_handoff_timeout"
    );
    fixture.assert_reaped();
}

#[tokio::test]
async fn reading_budget_already_limits_missing_lease_handoff() {
    let fixture = StartupFixture::new();
    let result = fixture.start("{}", Duration::from_millis(80)).await;
    assert_eq!(
        result.unwrap_err(),
        "durable_skill_job_lease_handoff_timeout"
    );
    fixture.assert_reaped();
}

#[tokio::test]
async fn expired_or_cancelled_admission_never_spawns_a_durable_child() {
    for cancelled in [false, true] {
        let fixture = StartupFixture::new();
        let token = CancellationToken::new();
        if cancelled {
            token.cancel();
        }
        let admission = RunnerAdmission {
            deadline: Instant::now(),
            cancellation: Some(&token),
        };
        let result = fixture
            .start_command(StartupFixture::command("exit 99"), "{}", &admission)
            .await;
        assert_eq!(
            result.unwrap_err(),
            if cancelled {
                crate::agent_engine::TASK_CANCELED_ERR
            } else {
                "durable_skill_job_lease_handoff_timeout"
            }
        );
        assert!(!fixture.plan.job_dir.exists());
    }
}

#[tokio::test]
async fn task_cancellation_interrupts_write_and_lease_wait_without_retry() {
    for request in ["{}".to_string(), "x".repeat(1024 * 1024)] {
        let fixture = StartupFixture::new();
        let token = CancellationToken::new();
        let admission = RunnerAdmission::new(Some(&token));
        let (result, ()) = tokio::join!(
            fixture.start_command(
                StartupFixture::command("exec sleep 2"),
                &request,
                &admission
            ),
            async {
                tokio::time::sleep(Duration::from_millis(80)).await;
                token.cancel();
            }
        );
        assert_eq!(result.unwrap_err(), crate::agent_engine::TASK_CANCELED_ERR);
        assert_eq!(
            std::fs::read_to_string(fixture.plan.job_dir.join("startup_failed")).unwrap(),
            "task_cancelled"
        );
        fixture.assert_reaped();
    }
}

#[tokio::test]
async fn accepted_operation_outlives_admission_and_receives_exactly_one_record() {
    let fixture = StartupFixture::new();
    let admission = RunnerAdmission {
        deadline: Instant::now() + Duration::from_millis(500),
        cancellation: None,
    };
    let request = r#"{"fixture":"value with \\n escaped newline"}"#;
    let script = r#"
IFS= read -r line || exit 11
printf '%s' "$line" > "$APP_DURABLE_JOB_DIR/request"
if IFS= read -r extra; then exit 12; fi
printf ready > "$APP_DURABLE_JOB_DIR/lease_ready"
sleep 1
printf completed > "$APP_DURABLE_JOB_DIR/completed"
"#;
    let result = fixture
        .start_command(StartupFixture::command(script), request, &admission)
        .await
        .unwrap();
    assert_eq!(result["status"], "ok");
    assert_eq!(result["extra"]["pending_async_job"]["status"], "accepted");
    assert_eq!(
        result["extra"]["pending_async_job"]["runtime_deadline_at"],
        Value::Null
    );
    assert_eq!(
        std::fs::read_to_string(fixture.plan.job_dir.join("request")).unwrap(),
        request
    );
    tokio::time::sleep_until(admission.deadline).await;
    assert!(!fixture.plan.job_dir.join("startup_failed").exists());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !fixture.plan.job_dir.join("completed").exists() {
        assert!(
            Instant::now() < deadline,
            "accepted operation did not finish"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        std::fs::read_to_string(fixture.plan.job_dir.join("completed")).unwrap(),
        "completed"
    );
}

#[tokio::test]
async fn warm_request_timeout_and_cancellation_discard_the_partial_stream() {
    for cancelled in [false, true] {
        let mut process = WarmRunnerProcess::spawn(StartupFixture::warm_command("exec sleep 2"))
            .expect("owned non-reading warm process");
        let token = CancellationToken::new();
        let admission = RunnerAdmission {
            deadline: Instant::now() + Duration::from_millis(if cancelled { 1000 } else { 80 }),
            cancellation: Some(&token),
        };
        let request = "x".repeat(1024 * 1024);
        let (result, ()) = tokio::join!(
            send_runner_request(&mut process, &request, "fixture_skill", &admission, None),
            async {
                if cancelled {
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    token.cancel();
                }
            }
        );
        assert_eq!(
            result.unwrap_err(),
            if cancelled {
                crate::agent_engine::TASK_CANCELED_ERR
            } else {
                "skill_runner_admission_timeout"
            }
        );
        assert!(!process.is_alive());
        assert_eq!(process.id(), None);
    }
}

#[tokio::test]
async fn warm_io_failure_is_not_timeout_and_failed_process_is_reaped() {
    let mut process =
        WarmRunnerProcess::spawn(StartupFixture::warm_command("exec 0<&-; exec sleep 2"))
            .expect("owned closed-input warm process");
    let result = send_runner_request(
        &mut process,
        &"x".repeat(1024 * 1024),
        "fixture_skill",
        &RunnerAdmission::new(None),
        None,
    )
    .await;
    let failure = crate::skills::parse_structured_skill_error(&result.unwrap_err())
        .expect("canonical IO failure");
    assert_eq!(failure.error_code, "skill_runner_stdin_write_failed");
    assert_eq!(failure.skill, "fixture_skill");
    let extra = failure.extra.unwrap();
    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["error_code"], failure.error_code);
    assert_eq!(extra["message_key"], "clawd.skill.runner_io_failed");
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["failure_phase"], "runner_admission");
    assert_eq!(extra["completion_state"], "unknown");
    assert_eq!(extra["io_operation"], "stdin_write");
    assert!(extra["os_error_code"].as_i64().is_some());
    assert!(extra.get("side_effect_applied").is_none());
    assert!(extra.get("error_kind").is_none());
    assert!(!process.is_alive());
    assert_eq!(process.id(), None);
}

#[tokio::test]
async fn successful_warm_requests_keep_distinct_admission_windows() {
    use futures_util::StreamExt;
    let mut process = WarmRunnerProcess::spawn(StartupFixture::warm_command(
        r#"while IFS= read -r line; do printf '%s\n' "$line"; done"#,
    ))
    .expect("owned echo warm process");
    for request in [r#"{"fixture":1}"#, r#"{"fixture":2}"#] {
        let admission = RunnerAdmission {
            deadline: Instant::now() + Duration::from_millis(100),
            cancellation: None,
        };
        send_runner_request(&mut process, request, "fixture_skill", &admission, None)
            .await
            .unwrap();
        let line = tokio::time::timeout(Duration::from_secs(1), process.records.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(line, request);
        tokio::time::sleep_until(admission.deadline).await;
        assert!(process.is_alive());
    }
    process.shutdown().await.unwrap();
    assert!(!process.is_alive());
}

#[tokio::test]
async fn one_window_is_not_reset_between_handoff_stages() {
    let admission = RunnerAdmission {
        deadline: Instant::now() + Duration::from_millis(100),
        cancellation: None,
    };
    admission
        .wait(tokio::time::sleep(Duration::from_millis(30)))
        .await
        .unwrap();
    assert_eq!(
        admission
            .wait(tokio::time::sleep(Duration::from_millis(100)))
            .await,
        Err(AdmissionFailure::DeadlineElapsed)
    );
}

#[tokio::test]
async fn expired_or_cancelled_window_does_not_poll_a_ready_write() {
    for cancelled in [false, true] {
        let polled = std::cell::Cell::new(false);
        let token = CancellationToken::new();
        if cancelled {
            token.cancel();
        }
        let admission = RunnerAdmission {
            deadline: Instant::now(),
            cancellation: Some(&token),
        };
        let result = admission
            .wait(async {
                polled.set(true);
            })
            .await;
        assert_eq!(
            result,
            Err(if cancelled {
                AdmissionFailure::Cancelled
            } else {
                AdmissionFailure::DeadlineElapsed
            })
        );
        assert!(!polled.get());
    }
}

struct HeldShutdown(Vec<u8>);

impl tokio::io::AsyncWrite for HeldShutdown {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.get_mut().0.extend_from_slice(bytes);
        std::task::Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Pending
    }
}

#[tokio::test]
async fn admission_deadline_includes_a_stalled_shutdown() {
    let admission = RunnerAdmission {
        deadline: Instant::now() + Duration::from_millis(40),
        cancellation: None,
    };
    let mut writer = HeldShutdown(Vec::new());
    let result = admission
        .wait(async {
            writer.write_all(b"{}\n").await.unwrap();
            writer.shutdown().await.unwrap();
        })
        .await;
    assert_eq!(result, Err(AdmissionFailure::DeadlineElapsed));
    assert_eq!(writer.0, b"{}\n");
}

#[tokio::test]
async fn read_only_conversation_interrupt_preempts_a_blocked_request_send() {
    let mut process = WarmRunnerProcess::spawn(StartupFixture::warm_command("exec sleep 2"))
        .expect("owned non-reading warm process");
    let interrupt = CancellationToken::new();
    let request = "x".repeat(1024 * 1024);
    let admission = RunnerAdmission::new(None);
    let (result, ()) = tokio::join!(
        send_runner_request(
            &mut process,
            &request,
            "fixture_skill",
            &admission,
            Some(&interrupt)
        ),
        async {
            tokio::time::sleep(Duration::from_millis(80)).await;
            interrupt.cancel();
        }
    );
    assert_eq!(
        result.unwrap_err(),
        crate::llm_gateway::CONVERSATION_INPUT_INTERRUPTED_ERR
    );
    assert!(!process.is_alive());
    assert_eq!(process.id(), None);
}

#[tokio::test]
async fn explicit_task_cancellation_keeps_priority_over_a_conversation_interrupt() {
    let mut process = WarmRunnerProcess::spawn(StartupFixture::warm_command("exec sleep 2"))
        .expect("owned non-reading warm process");
    let cancellation = CancellationToken::new();
    let interrupt = CancellationToken::new();
    cancellation.cancel();
    interrupt.cancel();
    let result = send_runner_request(
        &mut process,
        "{}",
        "fixture_skill",
        &RunnerAdmission::new(Some(&cancellation)),
        Some(&interrupt),
    )
    .await;
    assert_eq!(result.unwrap_err(), crate::agent_engine::TASK_CANCELED_ERR);
    assert!(!process.is_alive());
}
