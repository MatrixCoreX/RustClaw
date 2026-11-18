use super::*;

fn reference() -> OwnerReference {
    OwnerReference::new(
        Uuid::new_v4(),
        super::super::instance::observe_process(std::process::id())
            .unwrap()
            .birth,
        [9; 32],
    )
    .unwrap()
}

#[test]
fn owner_reference_authentication_and_private_credential_recover_without_persisting_nonce() {
    let key = OwnerAuthorityKey::new([7; 32]).unwrap();
    let reference = reference();
    let credential = key.credential(&reference).unwrap();
    credential.validate().unwrap();
    let bytes = key.sign(reference.clone()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(&credential.nonce.to_string()));
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value["reference"].get("nonce").is_none());
    let reopened_key = OwnerAuthorityKey::new([7; 32]).unwrap();
    let reopened = reopened_key.verify(&bytes).unwrap();
    assert!(reopened == reference);
    assert!(reopened_key.credential(&reopened).unwrap() == credential);
}

#[test]
fn owner_reference_wrong_key_or_changed_authenticated_fields_never_verify() {
    let key = OwnerAuthorityKey::new([7; 32]).unwrap();
    let bytes = key.sign(reference()).unwrap();
    assert!(matches!(
        OwnerAuthorityKey::new([8; 32]).unwrap().verify(&bytes),
        Err(OwnerReferenceFailure::AuthenticationFailed)
    ));
    for field in ["job_id", "owner", "binding_digest"] {
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let changed = reference();
        let other = serde_json::to_value(changed).unwrap();
        if field == "owner" {
            value["reference"][field]["boot_id"] = serde_json::json!(Uuid::new_v4());
        } else if field == "binding_digest" {
            value["reference"][field] = serde_json::json!(vec![8; 32]);
        } else {
            value["reference"][field] = other[field].clone();
        }
        assert!(matches!(
            key.verify(&serde_json::to_vec(&value).unwrap()),
            Err(OwnerReferenceFailure::AuthenticationFailed)
        ));
    }
}

#[test]
fn owner_reference_credentials_are_separated_by_job_birth_and_binding() {
    let key = OwnerAuthorityKey::new([7; 32]).unwrap();
    let initial = reference();
    let expected = key.credential(&initial).unwrap();
    for field in 0..4 {
        let mut changed = initial.clone();
        match field {
            0 => changed.job_id = Uuid::new_v4(),
            1 => changed.owner.boot_id = Uuid::new_v4(),
            2 => changed.owner.pid = if changed.owner.pid == 2 { 3 } else { 2 },
            _ => changed.binding_digest = [8; 32],
        }
        assert!(key.credential(&changed).unwrap().nonce != expected.nonce);
    }
    assert!(key.sign(initial).unwrap().len() <= MAX_REFERENCE_BYTES);
}

#[test]
fn owner_reference_closed_schema_and_size_limits_reject_untrusted_records() {
    let key = OwnerAuthorityKey::new([7; 32]).unwrap();
    for bytes in [vec![], vec![b' '; MAX_REFERENCE_BYTES + 1], b"{}".to_vec()] {
        assert!(key.verify(&bytes).is_err());
    }
    let bytes = key.sign(reference()).unwrap();
    for nested in [false, true] {
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if nested {
            value["reference"]["command"] = serde_json::json!("unexpected");
        } else {
            value["nonce"] = serde_json::json!(Uuid::new_v4());
        }
        assert!(matches!(
            key.verify(&serde_json::to_vec(&value).unwrap()),
            Err(OwnerReferenceFailure::InvalidReference)
        ));
    }
}

#[test]
fn owner_reference_refuses_default_key_empty_job_and_missing_binding() {
    assert!(matches!(
        OwnerAuthorityKey::new([0; 32]),
        Err(OwnerReferenceFailure::InvalidKey)
    ));
    let valid = reference();
    assert!(OwnerReference::new(Uuid::nil(), valid.owner.clone(), [9; 32]).is_err());
    assert!(OwnerReference::new(valid.job_id, valid.owner, [0; 32]).is_err());
}

#[test]
fn owner_reference_fingerprint_is_stable_nonzero_and_domain_separated() {
    let key = OwnerAuthorityKey::new([7; 32]).unwrap();
    let fingerprint = key.fingerprint().unwrap();
    assert_eq!(
        fingerprint,
        OwnerAuthorityKey::new([7; 32])
            .unwrap()
            .fingerprint()
            .unwrap()
    );
    assert_ne!(
        fingerprint,
        OwnerAuthorityKey::new([8; 32])
            .unwrap()
            .fingerprint()
            .unwrap()
    );
    assert_ne!(fingerprint.as_bytes(), &[7; 32]);
    assert!(AuthorityFingerprint::from_slice(&[]).is_none());
    assert!(AuthorityFingerprint::from_slice(&[0; 32]).is_none());
    assert!(AuthorityFingerprint::from_slice(&[7; 31]).is_none());
    assert!(AuthorityFingerprint::from_slice(&[7; 33]).is_none());
    let reference = reference();
    let mut record_mac = Authenticator::new_from_slice(&[7; 32]).unwrap();
    record_mac.update(b"owner-reference-v1\0");
    record_mac.update(&serde_json::to_vec(&reference).unwrap());
    let record_mac: [u8; 32] = record_mac.finalize().into_bytes().into();
    assert_ne!(fingerprint.as_bytes(), &record_mac);
}
