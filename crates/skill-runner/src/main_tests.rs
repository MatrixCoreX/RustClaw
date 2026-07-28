use super::*;

#[tokio::test]
async fn run_child_skill_times_out_and_kills_child() {
    let Some(child) = ["/bin/sh", "/usr/bin/sh"]
        .into_iter()
        .find(|path| Path::new(path).exists())
    else {
        eprintln!("skipping timeout assertion: no shell executable found");
        return;
    };
    let mut launch = ChildLaunch::legacy(child);
    launch.args = vec!["-c".to_string(), "sleep 30".to_string()];
    let result = run_child_skill(&launch, "ignored", Duration::from_millis(150)).await;
    assert!(
        matches!(result, Err(ref e) if e.error_code == "child_timeout" && e.timed_out && e.retryable),
        "expected timeout, got {:?}",
        result
    );
}

#[tokio::test]
async fn run_child_skill_reports_nonzero_exit() {
    let result = run_child_skill(
        &ChildLaunch::legacy("/bin/false"),
        "ignored",
        Duration::from_secs(2),
    )
    .await;
    assert!(
        matches!(result, Err(ref e) if e.error_code == "child_nonzero_exit" && e.exit_code == Some(1))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn run_child_skill_reports_terminating_signal() {
    let mut launch = ChildLaunch::legacy("/bin/sh");
    launch.args = vec!["-c".to_string(), "kill -TERM $$".to_string()];
    let error = run_child_skill(&launch, "ignored", Duration::from_secs(2))
        .await
        .expect_err("signal termination");
    assert_eq!(error.error_code, "child_nonzero_exit");
    assert_eq!(error.exit_code, None);
    assert_eq!(error.signal, Some(15));
}

#[tokio::test]
async fn run_child_skill_reports_bounded_stdout_truncation() {
    let mut launch = ChildLaunch::legacy("/bin/sh");
    launch.args = vec![
        "-c".to_string(),
        format!(
            "head -c {} /dev/zero",
            rustclaw_skill_sdk::MAX_PROTOCOL_LINE_BYTES + 1
        ),
    ];
    let error = run_child_skill(&launch, "ignored", Duration::from_secs(5))
        .await
        .expect_err("oversized stdout");
    assert_eq!(error.error_code, "child_output_truncated");
    assert!(error.truncated);
}

#[tokio::test]
async fn run_child_skill_returns_stdout_record() {
    let result = run_child_skill(
        &ChildLaunch::legacy("/bin/cat"),
        "hello-from-stdin",
        Duration::from_secs(2),
    )
    .await
    .expect("cat should echo stdin");
    assert_eq!(result, b"hello-from-stdin\n");
}

#[tokio::test]
async fn installed_process_runs_through_the_required_sandbox_boundary() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut launch = ChildLaunch::legacy("/bin/cat");
    launch.installed = true;
    launch.strict_protocol = true;
    launch.working_directory = Some(root.path().to_path_buf());
    let output = run_child_skill(&launch, "sandboxed", Duration::from_secs(2))
        .await
        .expect("sandboxed cat");
    assert_eq!(output, b"sandboxed\n");
}

#[tokio::test]
async fn installed_process_fails_closed_when_launch_metadata_is_invalid() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut launch = ChildLaunch::legacy(root.path().join("missing"));
    launch.installed = true;
    launch.working_directory = Some(root.path().to_path_buf());
    let error = run_child_skill(&launch, "ignored", Duration::from_secs(2))
        .await
        .expect_err("invalid installed program must fail closed");
    assert_eq!(error.error_code, "child_launch_invalid");
    assert!(error.detail.contains("sandbox failed closed"));
}

#[tokio::test]
async fn http_json_launch_requires_receipt_network_permission() {
    let mut launch = ChildLaunch::legacy("unused");
    launch.installed = true;
    launch.launcher = LauncherKind::HttpJson;
    launch.remote_endpoint = Some("https://example.invalid/skill".to_string());
    let error = run_http_json_skill(
        &launch,
        &serde_json::json!({"request_id": "fixture"}),
        Duration::from_millis(100),
    )
    .await
    .expect_err("network permission must be required");
    assert_eq!(error.error_code, "http_runtime_network_denied");
    assert!(error.detail.contains("not allowed by the receipt"));
}

#[test]
fn http_json_idempotency_header_comes_only_from_structured_execution_context() {
    let request = serde_json::json!({
        "context": {
            "execution": {
                "idempotency_key": "task-42:attempt-1"
            }
        }
    });
    let header = http_idempotency_header(&request)
        .expect("valid header")
        .expect("idempotency header");
    assert_eq!(header, "task-42:attempt-1");

    let invalid = serde_json::json!({
        "context": {
            "execution": {
                "idempotency_key": "unsafe\nheader"
            }
        }
    });
    let error = http_idempotency_header(&invalid).expect_err("invalid header must fail closed");
    assert_eq!(error.error_code, "http_idempotency_key_invalid");
}
