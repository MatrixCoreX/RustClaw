//! Credential contracts compared against the actual runtime, without requests.
use super::*;
use claw_core::secrets::{EnvFileSecretsBroker, SecretValue, SecretsBroker, SecretsError};

struct Miss;
impl SecretsBroker for Miss {
    fn lookup(&self, _: &str) -> Result<Option<SecretValue>, SecretsError> {
        Ok(None)
    }
}

fn broker(fixture: &Fixture) -> EnvFileSecretsBroker {
    EnvFileSecretsBroker::new(claw_core::git_remote_config::git_credential_store_path(
        &fixture.root,
    ))
}

fn managed(fixture: &Fixture, key: &str) {
    claw_core::secrets::model_environment::save(&fixture.root, "minimax", key).unwrap();
}

fn vendor_explicit(key: &str) -> String {
    explicit("openai_compat")
        .replace("fixture-explicit", "vendor-minimax")
        .replace(SECRET, key)
}

#[tokio::test]
async fn model_catalog_runtime_explicit_vendor_literal_precedes_unrelated_vendor_environment() {
    let name =
        "model_catalog_runtime_explicit_vendor_literal_precedes_unrelated_vendor_environment";
    if isolated(
        "credentials",
        name,
        &[(
            "MINIMAX_API_KEY",
            "fixture_catalog_private_source_line-vendor",
        )],
    ) {
        return;
    }
    let fixture = Fixture::new();
    configure(&fixture, &vendor_explicit(SECRET));
    assert_eq!(only_runtime(&fixture).api_key_using(&Miss), SECRET);
    assert_eq!(
        entry(&fixture, "minimax").await["credential_state"],
        "configured_inline"
    );
}

#[tokio::test]
async fn model_catalog_runtime_explicit_vendor_managed_secret_retains_broker_precedence() {
    let name = "model_catalog_runtime_explicit_vendor_managed_secret_retains_broker_precedence";
    if isolated("credentials", name, &[]) {
        return;
    }
    let fixture = Fixture::new();
    configure(&fixture, &vendor_explicit(SECRET));
    let key = format!("{SECRET}-managed");
    managed(&fixture, &key);
    assert_eq!(
        only_runtime(&fixture)
            .api_key_using(&broker(&fixture))
            .as_ref(),
        key.as_str()
    );
    assert_eq!(
        entry(&fixture, "minimax").await["credential_state"],
        "configured_env"
    );
}

#[tokio::test]
async fn model_catalog_runtime_arbitrary_explicit_key_does_not_consult_corrupt_vendor_storage() {
    struct Forbidden;
    impl SecretsBroker for Forbidden {
        fn lookup(&self, _: &str) -> Result<Option<SecretValue>, SecretsError> {
            panic!("arbitrary provider cannot consult vendor storage")
        }
    }
    let name =
        "model_catalog_runtime_arbitrary_explicit_key_does_not_consult_corrupt_vendor_storage";
    if isolated("credentials", name, &[]) {
        return;
    }
    let fixture = Fixture::new();
    configure(&fixture, &explicit("openai_compat"));
    let path = claw_core::secrets::model_environment::path(&fixture.root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "invalid managed credential document").unwrap();
    assert_eq!(only_runtime(&fixture).api_key_using(&Forbidden), SECRET);
    assert_eq!(
        entry(&fixture, "fixture-explicit").await["credential_state"],
        "configured_inline"
    );
}

#[tokio::test]
async fn model_catalog_runtime_media_lookup_cannot_promote_an_explicit_text_credential() {
    let name = "model_catalog_runtime_media_lookup_cannot_promote_an_explicit_text_credential";
    if isolated("credentials", name, &[("MINIMAX_API_KEY", SECRET)]) {
        return;
    }
    let fixture = Fixture::new();
    configure(&fixture, &vendor_explicit(""));
    fixture.write(
        "configs/audio.toml",
        "[audio_transcribe]\ndefault_vendor='minimax'\ndefault_model='aaa-media'\n\
        [audio_transcribe.providers.minimax]\nbase_url='https://fixture.invalid/audio'\n",
    );
    assert!(only_runtime(&fixture).api_key_using(&Miss).is_empty());
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    let rows = response.1["data"]["entries"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["model"], "aaa-media");
    assert_eq!(rows[0]["credential_state"], "configured_env");
    assert_eq!(rows[1]["model"], "fixture-runtime-model");
    assert_eq!(rows[1]["credential_state"], "missing");
    fixture.assert_private(&response.1);
}

#[tokio::test]
async fn model_catalog_runtime_file_flags_cannot_select_resolved_or_enrolled_credentials() {
    let name = "model_catalog_runtime_file_flags_cannot_select_resolved_or_enrolled_credentials";
    if isolated("credentials", name, &[]) {
        return;
    }
    let fixture = Fixture::new();
    configure(&fixture, &(vendor_explicit(SECRET) + "\n[llm.providers.params]\ncredential_source='Resolved'\ndevice_key_enrollment=true\n"));
    let key = format!("{SECRET}-managed");
    managed(&fixture, &key);
    let runtime = only_runtime(&fixture);
    assert_eq!(
        runtime.config.params.credential_source,
        claw_core::config::LlmCredentialSource::Managed
    );
    assert!(!runtime.config.params.device_key_enrollment);
    assert_eq!(
        runtime.api_key_using(&broker(&fixture)).as_ref(),
        key.as_str()
    );
    assert_eq!(
        entry(&fixture, "minimax").await["credential_state"],
        "configured_env"
    );
}

#[tokio::test]
async fn model_catalog_runtime_relay_mapping_does_not_enroll_an_arbitrary_explicit_name() {
    let name = "model_catalog_runtime_relay_mapping_does_not_enroll_an_arbitrary_explicit_name";
    if isolated("credentials", name, &[]) {
        return;
    }
    let fixture = Fixture::new();
    let configured = explicit("openai_compat").replace("fixture-explicit", "custom")
        + "\n[llm.hosted_relay]\nenabled=true\nvendor='custom'\nmodel='fixture-runtime-model'\nbase_url='https://fixture.invalid/v1'\n";
    configure(&fixture, &configured);
    let runtime = only_runtime(&fixture);
    assert!(!runtime.config.params.device_key_enrollment);
    assert_eq!(runtime.api_key_using(&Miss), SECRET);
    assert_eq!(
        entry(&fixture, "custom").await["credential_state"],
        "configured_inline"
    );
}

#[tokio::test]
async fn model_catalog_runtime_loopback_does_not_override_the_actual_relay_enrollment_mode() {
    let fixture = Fixture::new();
    let configured = standard("custom", "", true).replace("https://fixture.invalid/v1", "http://127.0.0.1:4/v1")
        + "\n[llm.hosted_relay]\nenabled=true\nvendor='custom'\nmodel='fixture-runtime-model'\nbase_url='http://127.0.0.1:4/v1/'\n";
    configure(&fixture, &configured);
    assert!(only_runtime(&fixture).config.params.device_key_enrollment);
    assert_eq!(
        entry(&fixture, "custom").await["credential_state"],
        "device_enrollment"
    );
}
