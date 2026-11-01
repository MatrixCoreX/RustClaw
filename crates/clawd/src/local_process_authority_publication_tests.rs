use super::*;
use crate::local_process_owner::instance::{observe_process, ProcessBirthIdentity};

fn request(pin: [u8; 32], job: uuid::Uuid, owner: &ProcessBirthIdentity) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "action": {
            "kind": "publish_reference",
            "authority_fingerprint": pin,
            "job_id": job,
            "owner": owner,
            "binding_digest": serde_json::to_value([19_u8; 32]).unwrap(),
        }
    })
}

fn invoke_publication(fixture: &Fixture, request: &serde_json::Value) -> serde_json::Value {
    let (mut child, channel) = helper(fixture);
    let channel = channel
        .send_bootstrap(
            &serde_json::to_vec(request).unwrap(),
            [
                File::open(&fixture.keys).unwrap().into(),
                File::open(&fixture.journal).unwrap().into(),
            ],
            deadline(),
        )
        .unwrap();
    let bytes = channel.receive_control(deadline()).unwrap();
    drop(channel);
    let status = child.wait();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(status.success(), value["status"] == "reference");
    value
}

fn expect_published(value: &serde_json::Value, job: uuid::Uuid, owner: &ProcessBirthIdentity) {
    assert_eq!(
        value["status"], "reference",
        "protected publication refused"
    );
    assert_eq!(value["credential"]["job_id"], job.to_string());
    assert_eq!(
        value["credential"]["owner"],
        serde_json::to_value(owner).unwrap()
    );
    let nonce = value["credential"]["nonce"].as_str().unwrap();
    assert_eq!(uuid::Uuid::parse_str(nonce).unwrap().get_version_num(), 4);
    assert_eq!(
        value["binding_digest"],
        serde_json::to_value([19_u8; 32]).unwrap()
    );
    assert!(value.get("key").is_none());
}

pub(super) fn run() {
    // This fixture observes one owned child; it is not host-policy admission
    // or proof that an arbitrary process satisfies the protected-owner contract.
    let mut owner = OwnedHelper(
        Command::new("/bin/sh")
            .args(["-c", "read -r _"])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let input = owner.0.stdin.take().unwrap();
    let birth = observe_process(owner.0.id()).unwrap().birth;
    let fixture = Fixture::new();
    let pin = host()
        .initialize(directories(&fixture), deadline())
        .unwrap();
    let job = uuid::Uuid::new_v4();
    let wanted = request(*pin.as_bytes(), job, &birth);
    let published = invoke_publication(&fixture, &wanted);
    expect_published(&published, job, &birth);
    let reference = fixture
        .keys
        .join(crate::local_process_owner::owner_authority::DIRECTORY)
        .join(format!("{}.owner", job.simple()));
    let saved = fs::read(&reference).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(record["reference"]["job_id"], job.to_string());
    assert!(record.get("nonce").is_none());
    assert!(!String::from_utf8_lossy(&saved)
        .contains(published["credential"]["nonce"].as_str().unwrap()));
    super::record("keyless_reference_publication");

    let reopened = invoke_publication(&fixture, &wanted);
    expect_published(&reopened, job, &birth);
    assert!(reopened == published, "private reference response changed");
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_reference_reopen_same_binding");

    let mut changed = wanted.clone();
    changed["action"]["binding_digest"] = serde_json::to_value([23_u8; 32]).unwrap();
    let refused = invoke_publication(&fixture, &changed);
    assert_eq!(refused["status"], "error");
    assert_eq!(refused["error"]["reference_failure"], "conflict");
    assert!(refused.get("credential").is_none());
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_reference_conflict_retains_original");

    let mut stale = wanted;
    stale["action"]["job_id"] = serde_json::json!(uuid::Uuid::new_v4());
    stale["action"]["owner"]["boot_id"] = serde_json::json!(uuid::Uuid::new_v4());
    let refused = invoke_publication(&fixture, &stale);
    assert_eq!(refused["status"], "error");
    assert_eq!(refused["error"]["error_code"], "owner_identity_mismatch");
    assert!(refused.get("credential").is_none());
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_reference_rejects_stale_birth");

    let mut wrong_pin = request(*pin.as_bytes(), uuid::Uuid::new_v4(), &birth);
    let mut changed_pin = *pin.as_bytes();
    changed_pin[0] ^= 1;
    wrong_pin["action"]["authority_fingerprint"] = serde_json::to_value(changed_pin).unwrap();
    let refused = invoke_publication(&fixture, &wrong_pin);
    assert_eq!(refused["status"], "error");
    assert_eq!(refused["error"]["error_code"], "authority_mismatch");
    assert!(refused.get("credential").is_none());
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_reference_requires_expected_epoch");

    let lost_job = uuid::Uuid::new_v4();
    let lost_request = request(*pin.as_bytes(), lost_job, &birth);
    let (mut helper, channel) = helper(&fixture);
    let channel = channel
        .send_bootstrap(
            &serde_json::to_vec(&lost_request).unwrap(),
            [
                File::open(&fixture.keys).unwrap().into(),
                File::open(&fixture.journal).unwrap().into(),
            ],
            deadline(),
        )
        .unwrap();
    drop(channel);
    assert!(!helper.wait().success());
    let lost_path = reference.with_file_name(format!("{}.owner", lost_job.simple()));
    let retained = fs::read(&lost_path).unwrap();
    let recovered = invoke_publication(&fixture, &lost_request);
    expect_published(&recovered, lost_job, &birth);
    assert_eq!(fs::read(&lost_path).unwrap(), retained);
    super::record("keyless_reference_lost_response_retains_publication");

    // Both actual helpers are live behind this owned DB lock before release.
    // This proves concurrent callers, not a forced race at rename itself.
    let concurrent_job = uuid::Uuid::new_v4();
    let concurrent_request = request(*pin.as_bytes(), concurrent_job, &birth);
    let sequence = fixture.sequence();
    let lock = rusqlite::Connection::open(fixture.database()).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut pending = Vec::new();
    for _ in 0..2 {
        let (child, channel) = super::helper(&fixture);
        let channel = channel
            .send_bootstrap(
                &serde_json::to_vec(&concurrent_request).unwrap(),
                [
                    File::open(&fixture.keys).unwrap().into(),
                    File::open(&fixture.journal).unwrap().into(),
                ],
                deadline(),
            )
            .unwrap();
        pending.push((child, channel));
    }
    for (child, _) in &mut pending {
        assert!(child.0.try_wait().unwrap().is_none());
    }
    lock.execute_batch("COMMIT").unwrap();
    lock.close().unwrap();
    let mut first = None;
    for (mut child, channel) in pending {
        let bytes = channel.receive_control(deadline()).unwrap();
        drop(channel);
        assert!(child.wait().success());
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        expect_published(&value, concurrent_job, &birth);
        if let Some(previous) = &first {
            assert!(*previous == value, "concurrent private response changed");
        } else {
            first = Some(value);
        }
    }
    assert_eq!(fixture.sequence(), sequence + 2);
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_reference_concurrent_publication");

    drop(input);
    assert!(!owner.wait().success());
}
