use super::*;
use crate::task_result_retention::test_support::Fixture;
use k256::ecdsa::{signature::Signer, Signature, SigningKey};
use k256::elliptic_curve::rand_core::OsRng;

fn local(fixture: &mut Fixture) -> NniLocalWorkState {
    let payload = json!({"capability":fixture.enrollment.capability,"args":{"action":"download"},"artifact_delivery":{
        "schema_version":1,"job_id":fixture.enrollment.job_id,"origin":fixture.enrollment.origin,
        "upload_token":"b".repeat(64),"required_kinds":["media"],"max_file_bytes":1024}});
    let canonical = payload.to_string();
    fixture.enrollment.payload_sha256 = nni_work_sha256(&canonical);
    NniLocalWorkState {
        schema_version: 1,
        node_url: "https://coordinator.example.org".to_string(),
        status: "completed_pending_report".to_string(),
        assignment: NniWorkAssignment {
            schema_version: 2,
            heartbeat_task_id: "assignment-poll".to_string(),
            task_id: fixture.enrollment.work_id.clone(),
            task_type: "capability_v1".to_string(),
            target_device_pubkey: "a".repeat(128),
            target_platform: nni_work_platform().unwrap().to_string(),
            target_arch: nni_work_arch().unwrap().to_string(),
            attempt: 1,
            lease_token: "c".repeat(64),
            lease_expires_at_unix: u64::try_from(current_unix_ts()).unwrap() + 60,
            payload_canonical: canonical,
            payload_sha256: fixture.enrollment.payload_sha256.clone(),
            issued_at_unix: 0,
        },
        local_task_id: Some(fixture.task_id.clone()),
        started_at_unix: Some(current_unix_ts()),
        result: Some(NniWorkResult {
            status: "success".to_string(),
            exit_code: None,
            duration_ms: 123,
            stdout: "private-result".to_string(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            error_code: None,
        }),
        cleanup: Some(fixture.enrollment.clone()),
        cleanup_plan: None,
    }
}
fn report(local: &NniLocalWorkState) -> NniHeartbeatWorkReport {
    NniHeartbeatWorkReport {
        task_id: local.assignment.task_id.clone(),
        attempt: local.assignment.attempt,
        lease_token: local.assignment.lease_token.clone(),
        result: local.result.clone().unwrap(),
    }
}
fn put_file(fixture: &Fixture) -> PathBuf {
    let path = claw_core::workspace_state::workspace_artifacts_root(&fixture.root)
        .join("delivery")
        .join(&fixture.task_id)
        .join("video.mp4");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"media-bytes").unwrap();
    path
}
fn setup(fixture: &Fixture) {
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
}

#[test]
fn cleanup_is_forward_only_default_off_and_bound_to_opt_in_origin() {
    let mut fixture = Fixture::new();
    let mut work = local(&mut fixture);
    fs::create_dir_all(nni_work_dir(&fixture.state)).unwrap();
    let mut policy = json!({"schema_version":1,"principal_id":"fixture-principal","capabilities":[fixture.enrollment.capability],
        "artifact_origins":[fixture.enrollment.origin],"max_file_bytes":1024});
    fs::write(
        nni_work_dir(&fixture.state).join("policy.json"),
        policy.to_string(),
    )
    .unwrap();
    assert!(nni_cleanup_enrollment(&fixture.state, &work.assignment).is_none());
    policy["artifact_cleanup_grants"] =
        json!([{"origin":fixture.enrollment.origin,"capability":fixture.enrollment.capability}]);
    fs::write(
        nni_work_dir(&fixture.state).join("policy.json"),
        policy.to_string(),
    )
    .unwrap();
    assert!(nni_cleanup_enrollment(&fixture.state, &work.assignment).is_some());
    let mut unrelated = work.assignment.clone();
    let mut payload: Value = serde_json::from_str(&unrelated.payload_canonical).unwrap();
    payload["capability"] = json!("fixture.unrelated");
    unrelated.payload_canonical = payload.to_string();
    unrelated.payload_sha256 = nni_work_sha256(&unrelated.payload_canonical);
    policy["capabilities"] = json!([fixture.enrollment.capability, "fixture.unrelated"]);
    fs::write(
        nni_work_dir(&fixture.state).join("policy.json"),
        policy.to_string(),
    )
    .unwrap();
    assert!(nni_cleanup_enrollment(&fixture.state, &unrelated).is_none());
    let mut other: Value = serde_json::from_str(&work.assignment.payload_canonical).unwrap();
    other["artifact_delivery"]["origin"] = json!("https://other.example.org");
    work.assignment.payload_canonical = other.to_string();
    work.assignment.payload_sha256 = nni_work_sha256(&work.assignment.payload_canonical);
    assert!(nni_cleanup_enrollment(&fixture.state, &work.assignment).is_none());
    let mut old = serde_json::to_value(&work).unwrap();
    old.as_object_mut().unwrap().remove("cleanup");
    old.as_object_mut().unwrap().remove("cleanup_plan");
    let old: NniLocalWorkState = serde_json::from_value(old).unwrap();
    assert!(old.cleanup.is_none());
}

#[test]
fn matching_ack_cleans_and_partial_cleanup_recovers_without_result_or_upload() {
    let mut fixture = Fixture::new();
    let work = local(&mut fixture);
    setup(&fixture);
    let path = put_file(&fixture);
    write_nni_local_work_state(&fixture.state, &work).unwrap();
    let mut pending = work.clone();
    pending.status = "cleanup_pending".to_string();
    pending.cleanup_plan = Some(
        crate::task_result_retention::files::prepare(&fixture.root, &fixture.task_id).unwrap(),
    );
    pending.result.as_mut().unwrap().stdout.clear();
    write_nni_local_work_state(&fixture.state, &pending).unwrap();
    // Crash after the DB commit and after some files were already removed.
    crate::repo::distributed_task_cleanup::retire_result(
        &fixture.state,
        &fixture.task_id,
        &fixture.enrollment,
    )
    .unwrap();
    fs::remove_file(&path).unwrap();
    let restored = read_nni_local_work_state(&fixture.state).unwrap().unwrap();
    nni_finish_acknowledged_cleanup(&fixture.state, &restored).unwrap();
    nni_finish_acknowledged_cleanup(&fixture.state, &restored).unwrap();
    assert!(!path.exists());
    assert!(!nni_work_state_path(&fixture.state).exists());
    assert_eq!(
        serde_json::from_str::<Value>(&fixture.row(&fixture.task_id).1).unwrap()
            ["result_retention"],
        "removed_after_delivery_ack"
    );
    assert!(!fs::read_to_string(nni_work_audit_path(&fixture.state))
        .unwrap()
        .contains("private-result"));
}

#[test]
fn failed_upload_missing_or_mismatched_submitted_report_preserves_every_byte() {
    let mut fixture = Fixture::new();
    let work = local(&mut fixture);
    setup(&fixture);
    let path = put_file(&fixture);
    let before = fixture.row(&fixture.task_id);
    assert!(nni_acknowledge_cleanup(&fixture.state, &work, None).is_err());
    let mut wrong = report(&work);
    wrong.attempt += 1;
    assert!(nni_acknowledge_cleanup(&fixture.state, &work, Some(&wrong)).is_err());
    wrong = report(&work);
    wrong.result.stdout.push_str("different");
    assert!(nni_acknowledge_cleanup(&fixture.state, &work, Some(&wrong)).is_err());
    let mut failed = work.clone();
    failed.result.as_mut().unwrap().status = "failed".to_string();
    failed.result.as_mut().unwrap().error_code = Some("artifact_upload_failed".to_string());
    assert!(nni_acknowledge_cleanup(&fixture.state, &failed, None).is_err());
    assert_eq!(fixture.row(&fixture.task_id), before);
    assert_eq!(fs::read(path).unwrap(), b"media-bytes");
}

#[test]
fn signed_control_is_required_and_success_ack_uses_the_actual_poll_report() {
    // These environment keys are process-local to this deterministic test.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = LOCK.lock().unwrap();
    let key = SigningKey::random(&mut OsRng);
    let public = key.verifying_key().to_encoded_point(true);
    let old = std::env::var_os(NNI_WORK_ASSIGNMENT_VERIFY_KEYS_ENV);
    std::env::set_var(
        NNI_WORK_ASSIGNMENT_VERIFY_KEYS_ENV,
        hex::encode(public.as_bytes()),
    );
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(v) = self.0.take() {
                std::env::set_var(NNI_WORK_ASSIGNMENT_VERIFY_KEYS_ENV, v);
            } else {
                std::env::remove_var(NNI_WORK_ASSIGNMENT_VERIFY_KEYS_ENV);
            }
        }
    }
    let _restore = Restore(old);
    let mut fixture = Fixture::new();
    let work = local(&mut fixture);
    setup(&fixture);
    let path = put_file(&fixture);
    write_nni_local_work_state(&fixture.state, &work).unwrap();
    let control = NniWorkControl {
        schema_version: 2,
        heartbeat_task_id: "ack-poll".to_string(),
        device_pubkey: work.assignment.target_device_pubkey.clone(),
        work_report_status: "accepted".to_string(),
        work_report_error_code: None,
        work_poll_status: "idle".to_string(),
        work_poll_error_code: None,
        work_assignment: None,
        issued_at_unix: u64::try_from(current_unix_ts()).unwrap(),
    };
    let raw = serde_json::to_string(&control).unwrap();
    let signature: Signature =
        key.sign(format!("{NNI_WORK_CONTROL_SIGNATURE_DOMAIN}{raw}").as_bytes());
    let envelope = NniWorkAssignmentEnvelope {
        schema_version: 2,
        algorithm: NNI_WORK_ASSIGNMENT_SIGNATURE_ALGORITHM.to_string(),
        signing_key_id: nni_work_sha256_bytes(public.as_bytes()),
        payload_canonical: raw,
        signature: hex::encode(signature.to_bytes()),
    };
    let mut heartbeat = json!({"task_id":"ack-poll","node_url":work.node_url,"local_device_pubkey":control.device_pubkey,"work_control":envelope});
    heartbeat["work_control"]["signature"] = json!("0".repeat(128));
    nni_handle_work_response(&fixture.state, &heartbeat, Some(&report(&work)));
    assert!(path.exists());
    heartbeat["work_control"] = serde_json::to_value(envelope).unwrap();
    nni_handle_work_response(&fixture.state, &heartbeat, None);
    assert!(path.exists());
    nni_handle_work_response(&fixture.state, &heartbeat, Some(&report(&work)));
    assert!(!path.exists());
    assert!(!nni_work_state_path(&fixture.state).exists());
}
fn nni_work_sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn accepted_failed_cancelled_and_timeout_attempts_retire_but_unconfirmed_failure_does_not() {
    for status in ["failed", "canceled", "timeout"] {
        let mut fixture = Fixture::new();
        let mut work = local(&mut fixture);
        setup(&fixture);
        fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE tasks SET status=?2 WHERE task_id=?1",
                rusqlite::params![fixture.task_id, status],
            )
            .unwrap();
        work.result.as_mut().unwrap().status = "failed".to_string();
        work.result.as_mut().unwrap().error_code = Some("artifact_upload_failed".to_string());
        let path = put_file(&fixture);
        write_nni_local_work_state(&fixture.state, &work).unwrap();
        let before = fixture.row(&fixture.task_id);
        assert!(nni_acknowledge_cleanup(&fixture.state, &work, None).is_err());
        assert_eq!(before, fixture.row(&fixture.task_id));
        assert!(path.exists());
        nni_acknowledge_cleanup(&fixture.state, &work, Some(&report(&work))).unwrap();
        assert!(!path.exists());
        assert!(!nni_work_state_path(&fixture.state).exists());
        assert!(!fixture.row(&fixture.task_id).1.contains("private-article"));
    }
    assert!(nni_cleanup_terminal_lease_error("nni_work_lease_expired"));
    assert!(nni_cleanup_terminal_lease_error(
        "nni_work_lease_not_active"
    ));
    assert!(!nni_cleanup_terminal_lease_error("nni_work_report_invalid"));
    assert!(!nni_cleanup_terminal_lease_error("nni_work_result_invalid"));
    assert!(!nni_cleanup_terminal_lease_error(
        "nni_work_server_unavailable"
    ));
}

#[tokio::test]
async fn in_flight_artifact_writer_defers_ack_cleanup_and_late_writers_cannot_recreate_result() {
    let mut fixture = Fixture::new();
    let work = local(&mut fixture);
    setup(&fixture);
    let path = put_file(&fixture);
    write_nni_local_work_state(&fixture.state, &work).unwrap();
    let producer =
        crate::task_result_retention::write_gate::begin_writing(&fixture.state, &fixture.task_id)
            .await
            .unwrap();
    assert!(nni_acknowledge_cleanup(&fixture.state, &work, Some(&report(&work))).is_err());
    assert!(path.exists());
    assert!(fixture.row(&fixture.task_id).1.contains("private-article"));
    let pending = read_nni_local_work_state(&fixture.state).unwrap().unwrap();
    assert_eq!(pending.status, "cleanup_pending");
    assert!(pending.result.as_ref().unwrap().stdout.is_empty());
    drop(producer);
    nni_finish_acknowledged_cleanup(&fixture.state, &pending).unwrap();
    assert!(!path.exists());
    assert!(crate::task_result_retention::write_gate::begin_writing(
        &fixture.state,
        &fixture.task_id
    )
    .await
    .is_err());
    assert!(crate::task_result_retention::write_gate::try_begin_writing(
        &fixture.state,
        &fixture.task_id
    )
    .is_err());
}

#[test]
fn recovery_finds_only_pre_enrolled_task_and_handles_pre_enqueue_failure_without_touching_history()
{
    let mut fixture = Fixture::new();
    let mut work = local(&mut fixture);
    setup(&fixture);
    let path = put_file(&fixture);
    work.local_task_id = None;
    write_nni_local_work_state(&fixture.state, &work).unwrap();
    assert_eq!(
        nni_recover_enrolled_task(&fixture.state, &fixture.enrollment).unwrap(),
        Some(fixture.task_id.clone())
    );
    nni_acknowledge_cleanup(&fixture.state, &work, Some(&report(&work))).unwrap();
    assert!(!path.exists());
    let mut fixture = Fixture::new();
    let mut work = local(&mut fixture);
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    // A matching payload/idempotency alone cannot retroactively enroll history.
    let before = fixture.row(&fixture.task_id);
    let path = put_file(&fixture);
    work.local_task_id = None;
    work.result.as_mut().unwrap().status = "failed".to_string();
    write_nni_local_work_state(&fixture.state, &work).unwrap();
    assert!(
        nni_recover_enrolled_task(&fixture.state, &fixture.enrollment)
            .unwrap()
            .is_none()
    );
    nni_acknowledge_cleanup(&fixture.state, &work, Some(&report(&work))).unwrap();
    assert_eq!(before, fixture.row(&fixture.task_id));
    assert!(path.exists());
}
