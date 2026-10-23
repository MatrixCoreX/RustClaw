use super::*;

fn request() -> NniRemoteHeartbeatRequest {
    NniRemoteHeartbeatRequest {
        device_pubkey: "a".repeat(128),
        client_user_key: "work-test".to_string(),
        worker_capabilities: Some(NniWorkerCapabilities {
            protocol_version: 1,
            platform: "linux".to_string(),
            arch: "aarch64".to_string(),
            supported_task_types: vec!["capability_v1".to_string()],
            worker_instance_id: "11111111-2222-4333-8444-555555555555".to_string(),
            active_lease: None,
            capability_refs: vec![],
            artifact_origins: vec![],
            max_file_bytes: 0,
        }),
        work_report: None,
    }
}

#[test]
fn work_poll_challenge_binds_request_metadata_and_cannot_sign_a_heartbeat() {
    let mut request = request();
    let id = format!("nni-work-poll-{}", "1".repeat(32));
    let nonce = "2".repeat(64);
    // Fixture produced by workPollChallenge in the server implementation.
    let challenge = nni_work_sha256(&format!(
        "nni-work-poll-v1\0{id}\0{nonce}\05eeeaf634c5e394270f8e06fe9cfb8838fd169689cd6d984d865dad502394c90\074234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b"
    ));
    assert!(nni_verify_work_poll_challenge(&challenge, &id, Some(&nonce), &request).is_ok());
    request.worker_capabilities.as_mut().unwrap().platform = "macos".to_string();
    assert_eq!(
        nni_verify_work_poll_challenge(&challenge, &id, Some(&nonce), &request),
        Err("work_poll_challenge_binding_invalid")
    );
    assert!(nni_verify_work_poll_challenge(
        &challenge,
        &format!("nni-heartbeat-{}", "1".repeat(32)),
        Some(&nonce),
        &request
    )
    .is_err());
    assert!(nni_verify_work_poll_challenge(&challenge, &id, None, &request).is_err());
}

#[test]
fn work_poll_binds_the_exact_report_and_request_id() {
    let mut request = request();
    request.worker_capabilities = None;
    request.work_report = Some(NniHeartbeatWorkReport {
        task_id: format!("nni-work-{}", "3".repeat(32)),
        attempt: 1,
        lease_token: "4".repeat(64),
        result: nni_failed_work_result("test_failure", "test_failure"),
    });
    let id = format!("nni-work-poll-{}", "1".repeat(32));
    let nonce = "2".repeat(64);
    let challenge = nni_work_sha256(&format!(
        "nni-work-poll-v1\0{id}\0{nonce}\0{}\0{}",
        nni_work_sha256("null"),
        nni_work_sha256(&serde_json::to_string(&request.work_report).unwrap())
    ));
    assert!(nni_verify_work_poll_challenge(&challenge, &id, Some(&nonce), &request).is_ok());
    request.work_report.as_mut().unwrap().result.stdout = "changed".to_string();
    assert!(nni_verify_work_poll_challenge(&challenge, &id, Some(&nonce), &request).is_err());
    assert!(nni_verify_work_poll_challenge(
        &challenge,
        &format!("nni-work-poll-{}", "5".repeat(32)),
        Some(&nonce),
        &request
    )
    .is_err());
}

#[test]
fn reward_heartbeat_omits_work_and_retry_backoff_is_bounded() {
    let mut heartbeat = request();
    heartbeat.worker_capabilities = None;
    heartbeat.work_report = None;
    let encoded = serde_json::to_value(heartbeat).unwrap();
    assert!(encoded.get("worker_capabilities").is_none());
    assert!(encoded.get("work_report").is_none());
    assert_eq!(nni_work_poll_retry_delay(1), Duration::from_secs(1));
    assert_eq!(nni_work_poll_retry_delay(4), Duration::from_secs(8));
    assert_eq!(nni_work_poll_retry_delay(u32::MAX), Duration::from_secs(32));
}

#[tokio::test]
async fn work_completion_wakeup_does_not_accelerate_reward_heartbeat() {
    let notify = nni_work_poll_notify();
    notify.notify_one();
    tokio::time::timeout(Duration::from_millis(50), notify.notified())
        .await
        .unwrap();
    assert!(!nni_heartbeat_is_due(false, Some(1_590), 1_001));
}
