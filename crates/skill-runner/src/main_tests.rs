use super::*;

#[test]
fn missing_runtime_timeout_limit_uses_manifest_timeout() {
    let configured = parse_configured_timeout_limit(None).expect("missing limit");
    assert_eq!(configured, None);
    assert_eq!(effective_timeout_seconds(900, configured), 900);
}

#[test]
fn explicit_runtime_timeout_can_only_tighten_manifest_timeout() {
    let shorter = parse_configured_timeout_limit(Some("45")).expect("shorter limit");
    let longer = parse_configured_timeout_limit(Some("1200")).expect("longer limit");

    assert_eq!(effective_timeout_seconds(900, shorter), 45);
    assert_eq!(effective_timeout_seconds(900, longer), 900);
}

#[test]
fn invalid_runtime_timeout_limit_is_rejected() {
    for value in ["", "0", "not-a-number", "86401"] {
        assert!(
            parse_configured_timeout_limit(Some(value)).is_err(),
            "value should be rejected: {value:?}"
        );
    }
}

#[test]
fn runner_overwrites_child_binding_with_the_verified_actual_binding() {
    let response = SkillResponse {
        request_id: "request-1".to_string(),
        status: "ok".to_string(),
        text: "done".to_string(),
        buttons: None,
        error_code: None,
        platform: None,
        exit_code: None,
        validation: None,
        extra: Some(serde_json::json!({
            "value": 7,
            "execution_binding": {"version": "untrusted-child-value"}
        })),
        error_text: None,
    };
    let binding = ExecutionBinding {
        skill_name: "fixture_skill".to_string(),
        version: "1.2.3".to_string(),
        manifest_digest: "a".repeat(64),
        receipt_digest: "b".repeat(64),
        registry_generation: 4,
        registry_generation_digest: Some("c".repeat(64)),
        base_registry_digest: Some("f".repeat(64)),
        overlay_generation_digest: Some("0".repeat(64)),
        policy_digest: Some("d".repeat(64)),
        admission_receipt_digest: Some("e".repeat(64)),
    };

    let response = attach_execution_binding(response, Some(&binding));
    assert_eq!(response.extra.as_ref().unwrap()["value"], 7);
    assert_eq!(
        response.extra.as_ref().unwrap()["execution_binding"],
        serde_json::to_value(binding).expect("serialize binding")
    );
}

#[tokio::test]
async fn installed_child_observes_the_effective_runner_timeout() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut launch = ChildLaunch::legacy("/bin/sh");
    launch.args = vec![
        "-c".to_string(),
        "printf '%s\\n' \"$SKILL_TIMEOUT_SECONDS\"".to_string(),
    ];
    launch.installed = true;
    launch.strict_protocol = true;
    launch.working_directory = Some(root.path().to_path_buf());
    launch.timeout_seconds = 777;
    launch
        .environment
        .insert("SKILL_TIMEOUT_SECONDS".to_string(), "999".to_string());

    let output = run_child_skill(&launch, "ignored", Duration::from_secs(2))
        .await
        .expect("installed child should observe the effective timeout");

    assert_eq!(output, b"777\n");
}

#[tokio::test]
async fn durable_child_has_no_inherited_or_runner_runtime_deadline() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut launch = ChildLaunch::legacy("/bin/sh");
    launch.args = vec![
        "-c".to_string(),
        "printf '%s\n' \"${SKILL_TIMEOUT_SECONDS-unset}\"".to_string(),
    ];
    launch.installed = true;
    launch.strict_protocol = true;
    launch.working_directory = Some(root.path().to_path_buf());
    launch.runtime_deadline_enabled = false;
    launch
        .environment
        .insert("SKILL_TIMEOUT_SECONDS".to_string(), "3600".to_string());

    let output = run_child_skill_streaming(&launch, "ignored", "ignored", None, None)
        .await
        .expect("durable child should complete without a runner deadline")
        .final_output;

    assert_eq!(output, b"unset\n");
}

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
    let Some(child) = ["/bin/sh", "/usr/bin/sh"]
        .into_iter()
        .find(|path| Path::new(path).is_file())
    else {
        eprintln!("skipping nonzero-exit assertion: no shell executable found");
        return;
    };
    let mut launch = ChildLaunch::legacy(child);
    launch.args = vec!["-c".to_string(), "read _; exit 2".to_string()];
    let result = run_child_skill(&launch, "ignored", Duration::from_secs(2)).await;
    assert!(
        matches!(result, Err(ref e) if e.error_code == "child_nonzero_exit" && e.exit_code.is_some_and(|code| code != 0)),
        "expected a nonzero child exit, got {result:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn run_child_skill_reports_terminating_signal() {
    let mut launch = ChildLaunch::legacy("/bin/sh");
    launch.args = vec!["-c".to_string(), "read _; kill -TERM $$".to_string()];
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
            skill_sdk::MAX_PROTOCOL_LINE_BYTES + 1
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
async fn framed_child_forwards_machine_progress_and_preserves_the_final_response() {
    let (mut writer, reader) = tokio::io::duplex(64 * 1024);
    let (progress_tx, mut progress_rx) = mpsc::channel(8);
    let progress = r#"{"schema_version":1,"record_type":"skill_progress","request_id":"r1","sequence":1,"kind":"progress","detail_key":"media.download.fetching","params":{"item":1},"current":1,"total":2}"#;
    let final_response = r#"{"request_id":"r1","status":"ok","text":"done","error_text":null}"#;
    let body = format!("{progress}\n{final_response}\n");
    let write = tokio::spawn(async move {
        writer
            .write_all(body.as_bytes())
            .await
            .expect("write fixture");
    });

    let execution = read_framed_child_output(reader, "r1", Some(progress_tx))
        .await
        .expect("framed output");
    write.await.expect("writer task");

    assert_eq!(
        execution.final_output,
        format!("{final_response}\n").as_bytes()
    );
    assert_eq!(execution.progress_diagnostics.accepted, 1);
    assert_eq!(progress_rx.try_recv().expect("forwarded frame"), progress);
}

#[tokio::test]
async fn framed_child_rejects_missing_final_wrong_request_overrate_and_oversized_frames() {
    let valid = |sequence| {
        format!(
            "{{\"schema_version\":1,\"record_type\":\"skill_progress\",\"request_id\":\"r1\",\"sequence\":{sequence},\"kind\":\"heartbeat\",\"detail_key\":\"skill.fixture.working\",\"params\":{{}}}}"
        )
    };
    let wrong_request = r#"{"schema_version":1,"record_type":"skill_progress","request_id":"other","sequence":1,"kind":"heartbeat","detail_key":"skill.fixture.working","params":{}}"#;
    let oversized = "x".repeat(skill_sdk::MAX_PROGRESS_FRAME_LINE_BYTES + 1);
    let final_response = r#"{"request_id":"r1","status":"ok","text":"done","error_text":null}"#;
    let mut records = vec![wrong_request.to_string()];
    records.extend((1..=9).map(valid));
    records.push(oversized);
    records.push(final_response.to_string());
    let body = format!("{}\n", records.join("\n"));
    let (mut writer, reader) = tokio::io::duplex(body.len() + 1);
    let (progress_tx, mut progress_rx) = mpsc::channel(16);
    let write = tokio::spawn(async move {
        writer
            .write_all(body.as_bytes())
            .await
            .expect("write fixture");
    });

    let execution = read_framed_child_output(reader, "r1", Some(progress_tx))
        .await
        .expect("final response remains authoritative");
    write.await.expect("writer task");
    assert_eq!(execution.progress_diagnostics.accepted, 8);
    assert_eq!(
        execution.progress_diagnostics.dropped_request_id_mismatch,
        1
    );
    assert_eq!(execution.progress_diagnostics.dropped_rate_limited, 1);
    assert_eq!(execution.progress_diagnostics.dropped_oversized, 1);
    let mut forwarded = 0;
    while progress_rx.try_recv().is_ok() {
        forwarded += 1;
    }
    assert_eq!(forwarded, 8);

    let only_frame = format!("{}\n", valid(1));
    let (mut writer, reader) = tokio::io::duplex(4096);
    let write = tokio::spawn(async move {
        writer
            .write_all(only_frame.as_bytes())
            .await
            .expect("write fixture");
    });
    let failure = read_framed_child_output(reader, "r1", None)
        .await
        .expect_err("a final response is mandatory");
    write.await.expect("writer task");
    assert_eq!(failure.error_code, "child_final_response_missing");
    assert_eq!(
        failure
            .progress_diagnostics
            .expect("machine diagnostics")
            .accepted,
        1
    );
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

#[test]
fn inherited_parent_sandbox_accepts_only_runtime_backend_tokens() {
    assert_eq!(
        inherited_parent_sandbox_backend_token("bubblewrap"),
        Some("bubblewrap")
    );
    assert_eq!(
        inherited_parent_sandbox_backend_token("macos_seatbelt"),
        Some("macos_seatbelt")
    );
    assert_eq!(inherited_parent_sandbox_backend_token("direct"), None);
    assert_eq!(inherited_parent_sandbox_backend_token("unknown"), None);
}

#[test]
fn unrestricted_admin_flag_is_exact_and_server_owned() {
    assert!(!super::environment_flag_value_is_enabled(None));
    assert!(!super::environment_flag_value_is_enabled(Some("true")));
    assert!(!super::environment_flag_value_is_enabled(Some("0")));
    assert!(super::environment_flag_value_is_enabled(Some("1")));
}

#[test]
fn installed_child_inherits_secret_token_store_without_manifest_declaration() {
    assert!(RUNTIME_CHILD_ENV_ALLOWLIST.contains(&"APP_SECRET_TOKEN_DIR"));
}

#[test]
fn declared_private_storage_is_writable_for_a_read_only_installed_skill() {
    let storage = tempfile::tempdir().expect("storage tempdir");
    let mut launch = ChildLaunch::legacy("/bin/true");
    launch.installed = true;
    launch.sandbox_profile = SandboxProfile::Required;

    let paths = installed_writable_paths_from(&launch, None, Some(storage.path()))
        .expect("declared storage path");

    assert_eq!(
        paths,
        vec![std::fs::canonicalize(storage.path()).expect("canonical storage")]
    );
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
        Some(Duration::from_millis(100)),
    )
    .await
    .expect_err("network permission must be required");
    assert_eq!(error.error_code, "http_runtime_network_denied");
    assert!(error.detail.contains("not allowed by the receipt"));
}

#[tokio::test]
async fn http_json_launch_posts_protocol_json_with_structured_idempotency() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local HTTP fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept HTTP request");
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).await.expect("read HTTP request");
            assert!(
                read > 0,
                "HTTP client closed before sending a complete request"
            );
            request.extend_from_slice(&chunk[..read]);
            let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let body = br#"{"request_id":"fixture","status":"ok","text":"done","error_text":null,"buttons":null,"extra":{"transport":"http_json"}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write HTTP headers");
        stream.write_all(body).await.expect("write HTTP body");
        String::from_utf8(request).expect("HTTP request is UTF-8")
    });

    let request = serde_json::json!({
        "request_id": "fixture",
        "args": {"value": 7},
        "context": {"execution": {"idempotency_key": "task-42:attempt-1"}}
    });
    let mut launch = ChildLaunch::legacy("unused");
    launch.installed = true;
    launch.launcher = LauncherKind::HttpJson;
    launch.runtime_network = true;
    launch.remote_endpoint = Some(format!("http://{address}/skill"));

    let output = run_http_json_skill(&launch, &request, Some(Duration::from_secs(2)))
        .await
        .expect("HTTP transport succeeds");
    let response = validate_response_line(&output, "fixture").expect("valid protocol response");
    assert_eq!(response.extra.unwrap()["transport"], "http_json");

    let raw_request = server.await.expect("HTTP fixture task");
    assert!(raw_request.starts_with("POST /skill HTTP/1.1\r\n"));
    assert!(raw_request
        .to_ascii_lowercase()
        .contains("idempotency-key: task-42:attempt-1\r\n"));
    assert!(raw_request.contains("\"request_id\":\"fixture\""));
    assert!(raw_request.contains("\"value\":7"));
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
