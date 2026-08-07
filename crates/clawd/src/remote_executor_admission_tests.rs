use super::*;

fn digest(fill: char) -> String {
    std::iter::repeat_n(fill, 64).collect()
}

#[test]
fn disabled_or_insecure_remote_transport_never_admits() {
    let mut config = claw_core::config::RemoteExecutorConfig::default();
    assert!(validate_feature_config(&config).is_err());
    config.enabled = true;
    config.endpoint = Some("http://executor.invalid".to_string());
    config.trusted_attestation_digests = vec![digest('a')];
    assert!(validate_feature_config(&config)
        .expect_err("plain HTTP transport must fail")
        .to_string()
        .contains("requires_tls"));
}

#[test]
fn admission_requires_allowlisted_attestation_and_protocol() {
    let trusted = digest('a');
    let config = claw_core::config::RemoteExecutorConfig {
        enabled: true,
        endpoint: Some("https://executor.example".to_string()),
        trusted_attestation_digests: vec![trusted.clone()],
    };
    let mut admission = RemoteExecutorAdmission {
        schema_version: 1,
        worker_id: "worker-1".to_string(),
        supported_protocol_versions: vec![1],
        capability_digests: vec![digest('b')],
        attestation_digest: trusted,
    };
    validate_admission(&config, &admission).expect("trusted worker should admit");
    admission.attestation_digest = digest('c');
    assert!(validate_admission(&config, &admission)
        .expect_err("untrusted worker must fail")
        .to_string()
        .contains("attestation_untrusted"));
}
