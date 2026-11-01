use super::tests::StartupFixture;
use super::*;

async fn wait_marker(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "owned handoff fixture did not progress"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn abort_during_handoff(read_input: bool) {
    let fixture = std::sync::Arc::new(StartupFixture::new());
    let ready = fixture.root.join("child-ready");
    let done = fixture.root.join("child-done");
    let release = fixture.root.join("child-release");
    let script = format!(
        "{}printf ready > \"$OWNED_READY\"; i=0; while [ ! -e \"$OWNED_RELEASE\" ] && [ \"$i\" -lt 100 ]; do i=$((i+1)); sleep 0.02; done; printf done > \"$OWNED_DONE\"",
        if read_input { "cat >/dev/null; " } else { "" },
    );
    let mut command = StartupFixture::command(&script);
    command
        .env("OWNED_READY", &ready)
        .env("OWNED_DONE", &done)
        .env("OWNED_RELEASE", &release);
    let child_fixture = fixture.clone();
    let dispatch = tokio::spawn(async move {
        let admission = RunnerAdmission::new(None);
        let request = if read_input {
            "{}".to_string()
        } else {
            "x".repeat(1024 * 1024)
        };
        child_fixture
            .start_command(command, &request, &admission)
            .await
    });
    wait_marker(&ready).await;
    assert!(!dispatch.is_finished());
    dispatch.abort();
    assert!(dispatch.await.unwrap_err().is_cancelled());
    let retained = fixture.plan.job_dir.is_dir();
    std::fs::write(release, b"release").unwrap();
    // The real child independently exits; never kill a saved PID/group, and
    // finish its owned fixture before asserting the original failure.
    wait_marker(&done).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        retained,
        "dropped dispatch erased a started job's recovery evidence"
    );
    for name in ["pid", "stdout", "stderr", "expected_execution_binding.json"] {
        assert!(
            fixture.plan.job_dir.join(name).is_file(),
            "missing retained {name}"
        );
    }
    assert!(!fixture.plan.job_dir.join("lease_ready").exists());
}

#[tokio::test]
async fn aborted_request_send_retains_started_runner_evidence() {
    abort_during_handoff(false).await;
}

#[tokio::test]
async fn aborted_lease_wait_retains_started_runner_evidence() {
    abort_during_handoff(true).await;
}

#[tokio::test]
async fn failed_spawn_retains_attempt_evidence_without_claiming_admission() {
    let fixture = StartupFixture::new();
    let command = RunnerLaunch::new(
        crate::process_invocation::ProcessInvocation::new(fixture.root.join("absent-runner")),
        &fixture.root,
        vec![],
    )
    .unwrap();
    let result = fixture
        .start_command(command, "{}", &RunnerAdmission::new(None))
        .await;
    let failure = crate::skills::parse_structured_skill_error(&result.unwrap_err()).unwrap();
    assert_eq!(failure.skill, "fixture_skill");
    assert_eq!(failure.error_code, "durable_skill_job_spawn_failed");
    let extra = failure.extra.unwrap();
    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["error_code"], failure.error_code);
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["failure_phase"], "runner_admission");
    assert_eq!(extra["completion_state"], "unknown");
    assert_eq!(extra["io_operation"], "process_spawn");
    assert!(extra["os_error_code"].as_i64().is_some());
    assert!(extra.get("side_effect_applied").is_none());
    assert!(extra.get("error_kind").is_none());
    for name in [
        "job_id",
        "job_kind",
        "stdout",
        "stderr",
        "expected_execution_binding.json",
    ] {
        assert!(fixture.plan.job_dir.join(name).is_file());
    }
    for name in ["pid", "lease_ready", "exit_code"] {
        assert!(!fixture.plan.job_dir.join(name).exists());
    }
}

#[tokio::test]
async fn unpolled_startup_cleans_only_its_unattempted_preparation() {
    let fixture = StartupFixture::new();
    let admission = RunnerAdmission::new(None);
    let binding = json!({"schema_version": 1});
    let launch = start_durable_skill_runner_process(
        StartupFixture::command("exit 99"),
        "{}",
        "fixture-request",
        "fixture_skill",
        &fixture.plan,
        super::super::DurableRunnerJobSetupGuard::new(&fixture.plan.job_dir),
        &binding,
        Path::new("fixture-runner"),
        &fixture.root,
        &fixture.root,
        &fixture.plan.job_dir,
        &fixture.plan.global_slot_root,
        &fixture.plan.skill_slot_root,
        1,
        1,
        1,
        &admission,
    );
    drop(launch);
    assert!(!fixture.plan.job_dir.exists());
    assert!(fixture.plan.global_slot_root.is_dir());
    assert!(fixture.plan.skill_slot_root.is_dir());
}
