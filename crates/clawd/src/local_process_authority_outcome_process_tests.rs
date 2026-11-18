use super::*;

fn invoke_outcome(fixture: &Fixture, request: &serde_json::Value) -> serde_json::Value {
    invoke_response(fixture, request, "outcome")
}

fn invoke_response(
    fixture: &Fixture,
    request: &serde_json::Value,
    success_status: &str,
) -> serde_json::Value {
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
    assert_eq!(status.success(), value["status"] == success_status);
    if success_status == "outcome" {
        assert!(value.get("credential").is_none());
        assert!(!value.to_string().contains("\"nonce\""));
    }
    value
}

pub(super) fn run() {
    let fixture = Fixture::new();
    let pin = host()
        .initialize(directories(&fixture), deadline())
        .unwrap();
    // The protected publisher itself supplies a real birth for this reference
    // fixture. It exits without starting a payload or publishing a job outcome.
    let (mut child, channel) = helper(&fixture);
    let birth = crate::local_process_owner::instance::observe_process(child.0.id())
        .unwrap()
        .birth;
    let job = uuid::Uuid::new_v4();
    let binding = [29_u8; 32];
    let publish = serde_json::json!({
        "schema_version": 1,
        "action": {
            "kind": "publish_reference",
            "authority_fingerprint": pin.as_bytes(),
            "job_id": job,
            "owner": birth,
            "binding_digest": binding,
        }
    });
    let channel = channel
        .send_bootstrap(
            &serde_json::to_vec(&publish).unwrap(),
            [
                File::open(&fixture.keys).unwrap().into(),
                File::open(&fixture.journal).unwrap().into(),
            ],
            deadline(),
        )
        .unwrap();
    let response = channel.receive_control(deadline()).unwrap();
    drop(channel);
    assert!(child.wait().success());
    let original = match AuthorityResponse::decode(&response).unwrap() {
        AuthorityResponse::Reference { credential, .. } => credential,
        _ => panic!("protected publication returned wrong role"),
    };
    let mut load = publish.clone();
    load["action"]["kind"] = serde_json::json!("load_reference");
    let load_template = load.clone();
    let mut read = publish;
    read["action"]["kind"] = serde_json::json!("read_outcome");
    read["action"].as_object_mut().unwrap().remove("job_id");
    read["action"].as_object_mut().unwrap().remove("owner");
    read["action"]["credential"] = serde_json::to_value(&original).unwrap();
    let reference = fixture
        .keys
        .join(crate::local_process_owner::owner_authority::DIRECTORY)
        .join(format!("{}.owner", job.simple()));
    let saved = fs::read(&reference).unwrap();
    let pending = invoke_outcome(&fixture, &read);
    assert_eq!(
        pending["status"], "outcome",
        "authenticated outcome read refused"
    );
    assert_eq!(pending["observation"]["kind"], "pending");
    super::record("keyless_outcome_pending_after_owner_exit");

    for name in ["exit_code", "startup_failed"] {
        fs::write(fixture.root.join(name), b"0").unwrap();
    }
    let pending = invoke_outcome(&fixture, &read);
    assert_eq!(pending["status"], "outcome");
    assert_eq!(pending["observation"]["kind"], "pending");
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_outcome_ignores_unsigned_terminal_markers");

    let mut missing = read.clone();
    let absent = uuid::Uuid::new_v4();
    missing["action"]["credential"]["job_id"] = serde_json::json!(absent);
    let refusal = invoke_outcome(&fixture, &missing);
    assert_eq!(refusal["status"], "error");
    assert_eq!(refusal["error"]["reference_failure"], "invalid_reference");
    assert!(!reference
        .with_file_name(format!("{}.owner", absent.simple()))
        .exists());
    super::record("keyless_outcome_missing_reference_is_not_terminal");

    let loaded = invoke_response(&fixture, &load, "reference");
    assert_eq!(loaded["status"], "reference");
    let restored = serde_json::from_value::<
        crate::local_process_owner::owner_protocol::OwnerCredential,
    >(loaded["credential"].clone())
    .unwrap();
    assert!(restored == original, "recovery changed private credential");
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_reference_load_after_owner_exit");

    let (mut waiting, barrier) = helper(&fixture);
    let live_birth = crate::local_process_owner::instance::observe_process(waiting.0.id())
        .unwrap()
        .birth;
    let live_job = uuid::Uuid::new_v4();
    let mut live_missing = load.clone();
    live_missing["action"]["job_id"] = serde_json::json!(live_job);
    live_missing["action"]["owner"] = serde_json::to_value(live_birth).unwrap();
    let refusal = invoke_response(&fixture, &live_missing, "reference");
    assert_eq!(refusal["status"], "error");
    assert_eq!(refusal["error"]["reference_failure"], "invalid_reference");
    assert!(refusal.get("credential").is_none());
    assert!(!reference
        .with_file_name(format!("{}.owner", live_job.simple()))
        .exists());
    assert_eq!(fs::read(&reference).unwrap(), saved);
    drop(barrier);
    assert!(!waiting.wait().success());
    super::record("keyless_reference_load_never_creates_for_live_owner");

    let mut wrong = read.clone();
    wrong["action"]["binding_digest"] = serde_json::to_value([37_u8; 32]).unwrap();
    let refusal = invoke_outcome(&fixture, &wrong);
    assert_eq!(refusal["status"], "error");
    assert_eq!(refusal["error"]["reference_failure"], "conflict");
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_outcome_rejects_wrong_binding");

    let mut wrong_credential = read.clone();
    wrong_credential["action"]["credential"]["nonce"] = serde_json::json!(uuid::Uuid::new_v4());
    let refusal = invoke_outcome(&fixture, &wrong_credential);
    assert_eq!(refusal["status"], "error");
    assert_eq!(refusal["error"]["reference_failure"], "conflict");
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_outcome_rejects_wrong_private_credential");

    load["action"]["binding_digest"] = serde_json::to_value([37_u8; 32]).unwrap();
    let refusal = invoke_response(&fixture, &load, "reference");
    assert_eq!(refusal["status"], "error");
    assert_eq!(refusal["error"]["reference_failure"], "conflict");
    assert!(refusal.get("credential").is_none());
    assert_eq!(fs::read(&reference).unwrap(), saved);
    super::record("keyless_reference_load_rejects_wrong_binding");

    let mut invalid_pin = *pin.as_bytes();
    invalid_pin[0] ^= 1;
    invalid_pin[1] |= 1;
    for (kind, role, mut request) in [
        ("read_outcome", "outcome", read.clone()),
        ("load_reference", "reference", load_template.clone()),
    ] {
        request["action"]["authority_fingerprint"] = serde_json::to_value(invalid_pin).unwrap();
        let refusal = invoke_response(&fixture, &request, role);
        assert_eq!(refusal["status"], "error");
        assert_eq!(refusal["error"]["error_code"], "authority_mismatch");
        assert!(refusal.get("credential").is_none());
        assert!(refusal.get("observation").is_none());
        assert_eq!(fs::read(&reference).unwrap(), saved);
        super::record(&format!("keyless_{kind}_requires_expected_epoch"));
    }

    let mut tampered: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    tampered["authenticator"][0] =
        serde_json::json!(tampered["authenticator"][0].as_u64().unwrap() ^ 1);
    for (fault, bytes, error_code) in [
        (
            "malformed_reference",
            b"partial".to_vec(),
            "invalid_reference",
        ),
        (
            "corrupt_reference",
            serde_json::to_vec(&tampered).unwrap(),
            "authentication_failed",
        ),
    ] {
        fs::write(&reference, &bytes).unwrap();
        for (kind, role, request) in [
            ("read_outcome", "outcome", read.clone()),
            ("load_reference", "reference", load_template.clone()),
        ] {
            let refusal = invoke_response(&fixture, &request, role);
            assert_eq!(refusal["status"], "error");
            assert_eq!(refusal["error"]["reference_failure"], error_code);
            assert!(refusal.get("credential").is_none());
            assert!(refusal.get("observation").is_none());
            assert_eq!(fs::read(&reference).unwrap(), bytes);
            super::record(&format!("keyless_{kind}_rejects_{fault}"));
        }
        // Only the fixture restores its owned record between independent probes.
        fs::write(&reference, &saved).unwrap();
    }

    let fake = serde_json::json!({
        "kind": "terminal",
        "receipt": {
            "schema_version": 1,
            "reference": {
                "schema_version": 1,
                "job_id": job,
                "owner": original.owner,
                "binding_digest": binding,
            },
            "snapshot": {
                "phase": "terminal",
                "completion": "completed",
                "root_exit": {"code": 0, "signal": null},
            },
        },
        "authenticator": serde_json::to_value([0_u8; 32]).unwrap(),
    });
    let outcome = reference.with_file_name(format!("{}.outcome", job.simple()));
    let bytes = serde_json::to_vec(&fake).unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&outcome)
        .unwrap();
    std::io::Write::write_all(&mut file, &bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let refusal = invoke_outcome(&fixture, &read);
    assert_eq!(refusal["status"], "error");
    assert_eq!(
        refusal["error"]["reference_failure"],
        "authentication_failed"
    );
    assert!(refusal.get("observation").is_none());
    assert_eq!(fs::read(outcome).unwrap(), bytes);
    assert_eq!(fs::read(reference).unwrap(), saved);
    super::record("keyless_outcome_rejects_forged_terminal");
}
