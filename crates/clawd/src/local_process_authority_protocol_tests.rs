use super::*;

#[test]
fn authority_bootstrap_is_bounded_versioned_and_role_specific() {
    let zero_fingerprint = [0_u8; 32];
    let valid = serde_json::json!({"schema_version":1,"action":{"kind":"initialize"}});
    assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&valid).unwrap()).is_ok());
    for invalid in [
        serde_json::json!({"schema_version":2,"action":{"kind":"initialize"}}),
        serde_json::json!({"schema_version":1,"action":{"kind":"resume"}}),
        serde_json::json!({"schema_version":1,"action":{"kind":"resume","authority_fingerprint":zero_fingerprint}}),
        serde_json::json!({"schema_version":1,"action":{"kind":"initialize","key":"not_admitted"}}),
        serde_json::json!({"schema_version":1,"action":{"kind":"initialize"},"path":"not_admitted"}),
        serde_json::json!({"schema_version":1,"job_id":"not_authority_bootstrap"}),
    ] {
        assert_eq!(
            AuthorityBootstrap::decode(&serde_json::to_vec(&invalid).unwrap())
                .err()
                .unwrap()
                .error_code,
            AuthorityAdmissionCode::InvalidRequest
        );
    }
    assert!(AuthorityBootstrap::decode(&vec![b' '; 4097]).is_err());
    assert!(AuthorityBootstrap::decode(
        br#"{"schema_version":1,"schema_version":1,"action":{"kind":"initialize"}}"#
    )
    .is_err());
}

#[test]
fn authority_success_contains_only_the_public_fingerprint() {
    let response =
        AuthorityResponse::from_result(Ok(AuthorityFingerprint::from_slice(&[1; 32]).unwrap()));
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 3);
    assert_eq!(value["status"], "ok");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(
        value["authority_fingerprint"],
        serde_json::to_value([1_u8; 32]).unwrap()
    );
    let mut invalid = value;
    invalid["key"] = serde_json::json!("not_admitted");
    assert!(serde_json::from_value::<AuthorityResponse>(invalid).is_err());
}

#[test]
fn authority_failures_preserve_machine_fields_without_exception_prose() {
    let error = AuthorityAdmissionError::io(
        AuthorityAdmissionCode::StorageUnavailable,
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "private fixture prose",
        ),
    );
    let value = serde_json::to_value(AuthorityResponse::from_result(Err(error))).unwrap();
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["error_code"], "storage_unavailable");
    assert!(!value.to_string().contains("private fixture prose"));
    let error = AuthorityAdmissionError::from(AdmissionFailure::Sqlite {
        stage: AdmissionStage::AdmitExisting,
        sqlite_code: Some(rusqlite::ErrorCode::DatabaseBusy),
        sqlite_extended_code: Some(5),
    });
    let value = serde_json::to_value(error).unwrap();
    assert_eq!(value["admission_stage"], "admit_existing");
    assert_eq!(value["sqlite_extended_code"], 5);
}

#[test]
fn host_response_decoder_rejects_future_schema_zero_pin_and_unknown_fields() {
    let valid = serde_json::to_value(AuthorityResponse::from_result(Ok(
        AuthorityFingerprint::from_slice(&[1; 32]).unwrap(),
    )))
    .unwrap();
    assert!(AuthorityResponse::decode(&serde_json::to_vec(&valid).unwrap()).is_ok());
    let mut future = valid.clone();
    future["schema_version"] = serde_json::json!(2);
    assert_eq!(
        AuthorityResponse::decode(&serde_json::to_vec(&future).unwrap()).err(),
        Some(AuthorityResponseFailure::UnsupportedSchema)
    );
    let mut zero = valid.clone();
    zero["authority_fingerprint"] = serde_json::to_value([0_u8; 32]).unwrap();
    assert_eq!(
        AuthorityResponse::decode(&serde_json::to_vec(&zero).unwrap()).err(),
        Some(AuthorityResponseFailure::InvalidFingerprint)
    );
    let mut unknown = valid;
    unknown["key"] = serde_json::json!("not_admitted");
    assert_eq!(
        AuthorityResponse::decode(&serde_json::to_vec(&unknown).unwrap()).err(),
        Some(AuthorityResponseFailure::InvalidFrame)
    );
    assert_eq!(
        AuthorityResponse::decode(&vec![b' '; 4097]).err(),
        Some(AuthorityResponseFailure::InvalidFrame)
    );
    let mut error = serde_json::to_value(AuthorityResponse::from_result(Err(
        AuthorityAdmissionError::code(AuthorityAdmissionCode::InvalidJournal),
    )))
    .unwrap();
    error["schema_version"] = serde_json::json!(2);
    assert_eq!(
        AuthorityResponse::decode(&serde_json::to_vec(&error).unwrap()).err(),
        Some(AuthorityResponseFailure::UnsupportedSchema)
    );
}

fn publication_request() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "action": {
            "kind": "publish_reference",
            "authority_fingerprint": serde_json::to_value([1_u8; 32]).unwrap(),
            "job_id": uuid::Uuid::new_v4(),
            "owner": {
                "pid": 123,
                "boot_id": uuid::Uuid::new_v4(),
                "started": { "kind": "linux_ticks", "ticks": 10 }
            },
            "binding_digest": serde_json::to_value([2_u8; 32]).unwrap(),
        }
    })
}

#[test]
fn publication_schema_rejects_unbound_invalid_or_extra_machine_fields() {
    let valid = publication_request();
    assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&valid).unwrap()).is_ok());
    for (path, replacement) in [
        ("/action/job_id", serde_json::json!(uuid::Uuid::nil())),
        ("/action/owner/pid", serde_json::json!(1)),
        (
            "/action/owner/boot_id",
            serde_json::json!(uuid::Uuid::nil()),
        ),
        (
            "/action/binding_digest",
            serde_json::to_value([0_u8; 32]).unwrap(),
        ),
        (
            "/action/authority_fingerprint",
            serde_json::to_value([0_u8; 32]).unwrap(),
        ),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(path).unwrap() = replacement;
        assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    for path in ["/action", "/action/owner", "/action/owner/started"] {
        let mut invalid = valid.clone();
        invalid.pointer_mut(path).unwrap()["extra"] = serde_json::json!(true);
        assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
}

#[test]
fn resource_request_schema_rejects_invalid_budget_and_unknown_fields() {
    let mut request = publication_request();
    request["action"]["kind"] = serde_json::json!("hold_resources");
    let job = request["action"]
        .as_object_mut()
        .unwrap()
        .remove("job_id")
        .unwrap();
    let owner = request["action"]
        .as_object_mut()
        .unwrap()
        .remove("owner")
        .unwrap();
    request["action"]["credential"] =
        serde_json::json!({ "job_id": job, "owner": owner, "nonce": uuid::Uuid::new_v4() });
    request["action"]["budget"] = serde_json::json!({
        "memory_mib": 512, "cpu_cores": 1, "network_slots": 0, "provider_slots": 0,
        "browser_slots": 0, "heavy": true, "background": true,
    });
    assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&request).unwrap()).is_ok());
    for (field, value) in [
        ("memory_mib", serde_json::json!(0)),
        ("cpu_cores", serde_json::json!(0)),
        ("browser_slots", serde_json::json!(65)),
        ("extra", serde_json::json!(true)),
    ] {
        let mut changed = request.clone();
        changed["action"]["budget"][field] = value;
        assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    request["action"]["kind"] = serde_json::json!("read_resources");
    request["action"].as_object_mut().unwrap().remove("budget");
    assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&request).unwrap()).is_ok());
}

#[test]
fn resource_response_schema_is_strict() {
    let request = publication_request();
    let response = serde_json::json!({
        "schema_version": 1, "status": "resources",
        "authority_fingerprint": request["action"]["authority_fingerprint"],
        "reference": { "schema_version": 1, "job_id": request["action"]["job_id"],
            "owner": request["action"]["owner"], "binding_digest": request["action"]["binding_digest"] },
        "budget": { "memory_mib": 512, "cpu_cores": 1, "network_slots": 0, "provider_slots": 0,
            "browser_slots": 0, "heavy": false, "background": true },
        "observation": { "kind": "pending" },
    });
    assert!(AuthorityResponse::decode(&serde_json::to_vec(&response).unwrap()).is_ok());
    for path in ["", "/reference", "/budget", "/observation"] {
        let mut changed = response.clone();
        changed.pointer_mut(path).unwrap()["extra"] = serde_json::json!(true);
        assert!(AuthorityResponse::decode(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    let mut changed = response.clone();
    changed["budget"]["memory_mib"] = serde_json::json!(0);
    assert!(AuthorityResponse::decode(&serde_json::to_vec(&changed).unwrap()).is_err());
}

#[test]
fn private_reference_response_is_strict_and_never_accepted_as_a_public_reply() {
    let request = publication_request();
    let response = serde_json::json!({
        "schema_version": 1,
        "status": "reference",
        "authority_fingerprint": request["action"]["authority_fingerprint"],
        "credential": {
            "job_id": request["action"]["job_id"],
            "owner": request["action"]["owner"],
            "nonce": uuid::Uuid::new_v4()
        },
        "binding_digest": request["action"]["binding_digest"],
    });
    let decoded = AuthorityResponse::decode(&serde_json::to_vec(&response).unwrap()).unwrap();
    assert!(matches!(decoded, AuthorityResponse::Reference { .. }));
    for (path, replacement) in [
        ("/schema_version", serde_json::json!(2)),
        ("/credential/nonce", serde_json::json!(uuid::Uuid::nil())),
        ("/credential/owner/pid", serde_json::json!(0)),
        ("/binding_digest", serde_json::to_value([0_u8; 32]).unwrap()),
    ] {
        let mut invalid = response.clone();
        *invalid.pointer_mut(path).unwrap() = replacement;
        assert!(AuthorityResponse::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    let mut invalid = response;
    invalid["credential"]["key"] = serde_json::json!("not_admitted");
    assert!(AuthorityResponse::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
}

#[test]
fn outcome_read_request_requires_a_complete_existing_reference_binding() {
    for kind in ["read_outcome", "load_reference"] {
        let mut valid = publication_request();
        valid["action"]["kind"] = serde_json::json!(kind);
        let fields = if kind == "read_outcome" {
            let job = valid["action"]
                .as_object_mut()
                .unwrap()
                .remove("job_id")
                .unwrap();
            let owner = valid["action"]
                .as_object_mut()
                .unwrap()
                .remove("owner")
                .unwrap();
            valid["action"]["credential"] =
                serde_json::json!({ "job_id": job, "owner": owner, "nonce": uuid::Uuid::new_v4() });
            vec!["authority_fingerprint", "credential", "binding_digest"]
        } else {
            vec!["authority_fingerprint", "job_id", "owner", "binding_digest"]
        };
        assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&valid).unwrap()).is_ok());
        for field in fields {
            let mut invalid = valid.clone();
            invalid["action"].as_object_mut().unwrap().remove(field);
            assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let mut invalid = valid;
        invalid["action"]["create"] = serde_json::json!(true);
        assert!(AuthorityBootstrap::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
}

#[test]
fn outcome_reply_decoder_validates_nested_reference_observation_and_schema() {
    let request = publication_request();
    let valid = serde_json::json!({
        "schema_version":1,
        "status":"outcome",
        "authority_fingerprint":request["action"]["authority_fingerprint"],
        "reference": {
            "schema_version":1,
            "job_id":request["action"]["job_id"],
            "owner":request["action"]["owner"],
            "binding_digest":request["action"]["binding_digest"]
        },
        "observation":{"kind":"pending"}
    });
    assert!(AuthorityResponse::decode(&serde_json::to_vec(&valid).unwrap()).is_ok());
    for path in ["", "/reference", "/reference/owner", "/observation"] {
        let mut invalid = valid.clone();
        invalid.pointer_mut(path).unwrap()["nonce"] = serde_json::json!(uuid::Uuid::new_v4());
        assert!(AuthorityResponse::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    for path in ["/schema_version", "/reference/schema_version"] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(path).unwrap() = serde_json::json!(2);
        assert!(AuthorityResponse::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    let mut invalid = valid.clone();
    invalid["observation"] = serde_json::json!({
        "kind":"terminal",
        "snapshot":{"phase":"running","completion":null,"root_exit":{"code":0,"signal":null}}
    });
    assert!(AuthorityResponse::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    let mut terminal = valid;
    terminal["observation"] = serde_json::json!({
        "kind":"terminal",
        "snapshot":{"phase":"terminal","completion":"completed","root_exit":{"code":0,"signal":null}}
    });
    assert!(AuthorityResponse::decode(&serde_json::to_vec(&terminal).unwrap()).is_ok());
    for path in ["/observation/snapshot", "/observation/snapshot/root_exit"] {
        let mut extra = terminal.clone();
        extra.pointer_mut(path).unwrap()["unexpected"] = serde_json::json!(true);
        assert!(AuthorityResponse::decode(&serde_json::to_vec(&extra).unwrap()).is_err());
    }
}
