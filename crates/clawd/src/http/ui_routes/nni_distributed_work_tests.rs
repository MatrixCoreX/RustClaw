use super::*;
use k256::ecdsa::{signature::Signer, Signature, SigningKey};
use k256::elliptic_curve::Generate;

fn assignment(task_type: &str, payload: Value) -> NniWorkAssignment {
    let payload_canonical = serde_json::to_string(&payload).unwrap();
    let now = u64::try_from(current_unix_ts()).unwrap();
    NniWorkAssignment {
        schema_version: NNI_WORK_ASSIGNMENT_SCHEMA_VERSION,
        heartbeat_task_id: "nni-heartbeat-0123456789abcdef0123456789abcdef".to_string(),
        task_id: "nni-work-0123456789abcdef0123456789abcdef".to_string(),
        task_type: task_type.to_string(),
        target_device_pubkey: "a".repeat(128),
        target_platform: nni_work_platform().unwrap().to_string(),
        target_arch: nni_work_arch().unwrap().to_string(),
        attempt: 1,
        lease_token: "b".repeat(64),
        lease_expires_at_unix: now + 60,
        payload_sha256: nni_work_sha256(&payload_canonical),
        payload_canonical,
        issued_at_unix: now,
    }
}

#[test]
fn program_validation_rejects_shells_and_paths() {
    assert!(nni_work_program_name_valid("uname"));
    assert!(nni_work_program_name_valid("printf"));
    assert!(!nni_work_program_name_valid("sh"));
    assert!(!nni_work_program_name_valid("/usr/bin/uname"));
    assert!(!nni_work_program_name_valid("uname;id"));
}

#[test]
fn signed_assignment_rejects_tampering_unknown_keys_and_heartbeat_replay() {
    use sha2::{Digest, Sha256};

    let signing_key = SigningKey::generate();
    let public_key = signing_key.verifying_key().to_sec1_point(true);
    let public_key_hex = hex::encode(public_key.as_bytes());
    let keys =
        nni_parse_work_assignment_verifying_keys(&public_key_hex).expect("parse verifying key");
    let key_id = hex::encode(Sha256::digest(public_key.as_bytes()));
    let expected_heartbeat = "nni-heartbeat-0123456789abcdef0123456789abcdef";
    let work = assignment(
        "command_v1",
        json!({
            "program": "uname",
            "args": ["-a"],
            "timeout_seconds": 30,
        }),
    );
    let payload_canonical = serde_json::to_string(&work).expect("serialize assignment");
    let signed = format!(
        "{}{}",
        NNI_WORK_ASSIGNMENT_SIGNATURE_DOMAIN, payload_canonical
    );
    let signature: Signature = signing_key.sign(signed.as_bytes());
    let envelope = NniWorkAssignmentEnvelope {
        schema_version: NNI_WORK_ASSIGNMENT_SCHEMA_VERSION,
        algorithm: NNI_WORK_ASSIGNMENT_SIGNATURE_ALGORITHM.to_string(),
        signing_key_id: key_id,
        payload_canonical,
        signature: hex::encode(signature.to_bytes()),
    };

    let verified = nni_verify_work_assignment_envelope(&envelope, expected_heartbeat, &keys)
        .expect("verify signed assignment");
    assert_eq!(verified.task_id, work.task_id);

    let mut tampered = envelope.clone();
    tampered.payload_canonical = tampered.payload_canonical.replace("uname", "false");
    assert_eq!(
        nni_verify_work_assignment_envelope(&tampered, expected_heartbeat, &keys)
            .expect_err("tampering must fail"),
        "assignment_signature_invalid"
    );
    assert_eq!(
        nni_verify_work_assignment_envelope(
            &envelope,
            "nni-heartbeat-ffffffffffffffffffffffffffffffff",
            &keys,
        )
        .expect_err("heartbeat replay must fail"),
        "assignment_binding_invalid"
    );

    let other_key = SigningKey::generate();
    let other_hex = hex::encode(other_key.verifying_key().to_sec1_point(true).as_bytes());
    let other_keys =
        nni_parse_work_assignment_verifying_keys(&other_hex).expect("parse other verifying key");
    assert_eq!(
        nni_verify_work_assignment_envelope(&envelope, expected_heartbeat, &other_keys)
            .expect_err("unknown key must fail"),
        "assignment_signing_key_unknown"
    );

    let control_payload = serde_json::to_string(&NniWorkControl {
        schema_version: NNI_WORK_ASSIGNMENT_SCHEMA_VERSION,
        heartbeat_task_id: expected_heartbeat.to_string(),
        device_pubkey: "a".repeat(128),
        work_report_status: "accepted".to_string(),
        work_report_error_code: None,
        work_poll_status: "leased".to_string(),
        work_poll_error_code: None,
        work_assignment: Some(envelope.clone()),
        issued_at_unix: u64::try_from(current_unix_ts()).unwrap(),
    })
    .expect("serialize work control");
    let control_signature: Signature = signing_key
        .sign(format!("{}{}", NNI_WORK_CONTROL_SIGNATURE_DOMAIN, control_payload).as_bytes());
    let control_envelope = NniWorkAssignmentEnvelope {
        schema_version: NNI_WORK_ASSIGNMENT_SCHEMA_VERSION,
        algorithm: NNI_WORK_ASSIGNMENT_SIGNATURE_ALGORITHM.to_string(),
        signing_key_id: envelope.signing_key_id.clone(),
        payload_canonical: control_payload,
        signature: hex::encode(control_signature.to_bytes()),
    };
    let control = nni_verify_work_control_envelope(
        &control_envelope,
        expected_heartbeat,
        &"a".repeat(128),
        &keys,
    )
    .expect("verify signed work control");
    assert_eq!(control.work_report_status, "accepted");
    assert!(
        nni_verify_work_assignment_envelope(&control_envelope, expected_heartbeat, &keys,).is_err()
    );
    assert_eq!(
        nni_verify_work_control_envelope(
            &control_envelope,
            "nni-heartbeat-ffffffffffffffffffffffffffffffff",
            &"a".repeat(128),
            &keys,
        )
        .expect_err("work-control replay must fail"),
        "work_control_binding_invalid"
    );
}

#[test]
fn verifies_node_crypto_assignment_signature_fixture() {
    let keys = nni_parse_work_assignment_verifying_keys(
        "0216451a0615c57cdb9caddc0b90a2acf727340d6ddced7e305331627c0d10cc46",
    )
    .expect("parse Node.js fixture key");
    let envelope = NniWorkAssignmentEnvelope {
        schema_version: 2,
        algorithm: "secp256k1-sha256".to_string(),
        signing_key_id:
            "fa7f8c2e1ed9da5d4b2ce637e54d87a5e6a78f60c1c6912875c272a8d0eac341"
                .to_string(),
        payload_canonical: r#"{"schema_version":2,"heartbeat_task_id":"nni-heartbeat-11111111111111111111111111111111","task_id":"nni-work-22222222222222222222222222222222","task_type":"command_v1","target_device_pubkey":"33333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333","target_platform":"linux","target_arch":"aarch64","attempt":1,"lease_token":"4444444444444444444444444444444444444444444444444444444444444444","lease_expires_at_unix":4102444800,"payload_canonical":"{\"program\":\"uname\",\"args\":[\"-a\"],\"timeout_seconds\":30}","payload_sha256":"5555555555555555555555555555555555555555555555555555555555555555","issued_at_unix":1}"#.to_string(),
        signature:
            "aafacd24156612e1e738b8510667261d4b65a31d2cd2f68b5872d4a5852386115ebe590153788c744818c37895e744f5678b454df53fa963c864e9157f793538"
                .to_string(),
    };
    let verified = nni_verify_work_assignment_envelope(
        &envelope,
        "nni-heartbeat-11111111111111111111111111111111",
        &keys,
    )
    .expect("verify Node.js signature in Rust");
    assert_eq!(
        verified.task_id,
        "nni-work-22222222222222222222222222222222"
    );
    let signature = Signature::from_slice(
        &hex::decode(&envelope.signature).expect("fixture signature bytes"),
    )
    .expect("P1363 fixture signature");
    let mut der = envelope.clone();
    der.signature = hex::encode(signature.to_der().as_bytes());
    assert_eq!(
        nni_verify_work_assignment_envelope(
            &der,
            "nni-heartbeat-11111111111111111111111111111111",
            &keys,
        )
        .expect_err("DER is not the work signature wire format"),
        "assignment_envelope_invalid"
    );
    let mut high_s = envelope.clone();
    high_s.signature = hex::encode(
        Signature::from_scalars(signature.r().to_bytes(), (-signature.s()).to_bytes())
            .expect("equivalent high-S signature")
            .to_bytes(),
    );
    assert_eq!(
        nni_verify_work_assignment_envelope(
            &high_s,
            "nni-heartbeat-11111111111111111111111111111111",
            &keys,
        )
        .expect_err("high-S signatures must remain rejected"),
        "assignment_signature_invalid"
    );
    let mut tampered = envelope;
    let mut signature = hex::decode(&tampered.signature).expect("fixture signature bytes");
    signature[0] ^= 1;
    tampered.signature = hex::encode(signature);
    assert_eq!(
        nni_verify_work_assignment_envelope(
            &tampered,
            "nni-heartbeat-11111111111111111111111111111111",
            &keys,
        )
        .expect_err("altered Node.js signature must fail"),
        "assignment_signature_invalid"
    );
}

#[test]
fn capability_assignment_uses_the_existing_direct_capability_contract() {
    let parsed = nni_validate_work_assignment(&assignment(
        "capability_v1",
        json!({
            "capability": "web.search",
            "args": {"query": "distributed inference"},
        }),
    ))
    .expect("valid capability assignment");
    match parsed {
        NniWorkPayload::Capability(payload) => {
            assert_eq!(payload.capability, "web.search");
            assert_eq!(payload.args["query"], "distributed inference");
        }
        NniWorkPayload::Command(_) => panic!("expected capability payload"),
    }
    assert!(nni_validate_work_assignment(&assignment(
        "capability_v1",
        json!({"capability": "Browser Web", "args": {}}),
    ))
    .is_err());
}

#[test]
fn capability_assignment_enters_the_authenticated_local_task_queue() {
    let root = std::env::temp_dir().join(format!("work-policy-test-{}", uuid::Uuid::new_v4()));
    let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let registry_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path).unwrap();
    let enabled = registry.all_names().into_iter().collect();
    *state.core.skill_views_snapshot.write().unwrap() = std::sync::Arc::new(crate::SkillViewsSnapshot {
        binding: Default::default(), registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    state.skill_rt.workspace_root = root.clone();
    state.seed_test_auth_identity("rk-nni-capability-admin", "admin");
    let identity = resolve_auth_identity_by_key(&state, "rk-nni-capability-admin").unwrap().unwrap();
    std::fs::create_dir_all(nni_work_dir(&state)).unwrap();
    std::fs::write(nni_work_dir(&state).join("policy.json"), serde_json::to_vec(&json!({
        "schema_version": 1, "principal_id": identity.principal_id,
        "capabilities": ["web.search"], "artifact_origins": [], "max_file_bytes": 1024,
        "argument_constraints": {"web.search": {"query": "distributed inference"}}
    })).unwrap()).unwrap();
    let assignment = assignment(
        "capability_v1",
        json!({
            "capability": "web.search",
            "args": {"query": "distributed inference"},
        }),
    );
    let payload =
        match nni_validate_work_assignment(&assignment).expect("valid capability assignment") {
            NniWorkPayload::Capability(payload) => payload,
            NniWorkPayload::Command(_) => panic!("expected capability payload"),
        };

    let advertised = nni_advertised_capability_grants(&state);
    assert_eq!(advertised.0, vec!["web.search".to_string()]);
    let (local_task_id, enrolled) = nni_enqueue_capability_task(&state, &assignment, &payload, None)
        .expect("enqueue local capability task");
    assert!(!enrolled);
    let (payload_json, status, user_key, principal_id): (String, String, String, String) = state
        .core
        .db
        .get()
        .expect("get test database")
        .query_row(
            "SELECT payload_json, status, user_key, principal_id FROM tasks WHERE task_id = ?1",
            [&local_task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read queued local capability task");
    let queued: Value = serde_json::from_str(&payload_json).expect("parse queued payload");

    assert_eq!(status, "queued");
    assert_eq!(user_key, "rk-nni-capability-admin");
    assert!(!principal_id.is_empty());
    assert!(queued.get("entrypoint").is_none());
    assert_eq!(queued["skill_name"], "web_search_extract");
    assert_eq!(queued["args"]["action"], "search");
    assert_eq!(queued["_requested_capability"], "web.search");
    assert_eq!(queued["_execution_only"], true);
    let kind: String = state.core.db.get().unwrap().query_row(
        "SELECT kind FROM tasks WHERE task_id=?1", [&local_task_id], |row| row.get(0)).unwrap();
    assert_eq!(kind, "run_skill");
    assert_eq!(queued["args"]["query"], "distributed inference");
    assert_eq!(queued["_agent_execution_policy"]["mode"], "yolo");
    assert_eq!(
        queued["_agent_execution_policy"]["authority"],
        "authenticated_admin"
    );
    let registry = state.get_skills_registry();
    *state.core.skill_views_snapshot.write().unwrap() = std::sync::Arc::new(crate::SkillViewsSnapshot {
        binding: Default::default(), registry, skills_list: std::sync::Arc::new(Default::default()),
    });
    assert!(nni_advertised_capability_grants(&state).0.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capability_result_prefers_readable_text_and_redacts_secrets() {
    let text = nni_capability_result_text(Some(&json!({
        "text": "done api_key=secret-value",
        "extra": {"internal": true},
    })));
    assert!(text.starts_with("done"));
    assert!(!text.contains("secret-value"));
}

#[tokio::test]
async fn command_executor_captures_success_without_a_shell() {
    let result = nni_execute_work_command(
        &NniWorkCommandPayload {
            program: "printf".to_string(),
            args: vec!["heartbeat-work-ok".to_string()],
            timeout_seconds: 5,
        },
        &BTreeSet::from(["printf".to_string()]),
        5,
    )
    .await;
    assert_eq!(result.status, "success");
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout, "heartbeat-work-ok");
    assert!(result.error_code.is_none());
}

#[tokio::test]
async fn command_executor_timeout_is_a_work_failure() {
    let result = nni_execute_work_command(
        &NniWorkCommandPayload {
            program: "sleep".to_string(),
            args: vec!["2".to_string()],
            timeout_seconds: 1,
        },
        &BTreeSet::from(["sleep".to_string()]),
        1,
    )
    .await;
    assert_eq!(result.status, "failed");
    assert_eq!(result.error_code.as_deref(), Some("command_timeout"));
}

#[test]
fn report_json_is_bounded_even_when_every_byte_requires_escaping() {
    let result = nni_bound_work_result_for_heartbeat(NniWorkResult {
        status: "success".to_string(),
        exit_code: Some(0),
        duration_ms: 1,
        stdout: "\\\"".repeat(NNI_WORK_RESULT_STREAM_BYTES / 2),
        stderr: "\\\"".repeat(NNI_WORK_RESULT_STREAM_BYTES / 2),
        stdout_truncated: false,
        stderr_truncated: false,
        error_code: None,
    });
    assert!(serde_json::to_vec(&result).unwrap().len() <= NNI_WORK_REPORT_JSON_BYTES);
    assert!(result.stdout_truncated || result.stderr_truncated);
}
