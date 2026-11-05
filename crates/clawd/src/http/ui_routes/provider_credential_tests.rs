//! Isolated effective-credential regressions; no live credentials or providers.
use super::model_settings_integrity_tests::support::{Fixture, ADMIN, USER};
use super::*;
use axum::http::Method;
use claw_core::secrets::{model_environment, EnvFileSecretsBroker, SecretsBroker};

const OLD_KEY: &str = "fixture-effective-credential-original-73519";
const NEW_KEY: &str = "fixture-effective-credential-replacement-84620";
const ENDPOINT: &str = "/v1/admin/provider-keys";

fn fixture(vendors: &[&str]) -> Fixture {
    let fixture = Fixture::new();
    let mut raw = String::from("[llm]\nselected_vendor='custom'\nselected_model='fixture-model'\n");
    for vendor in vendors {
        raw.push_str(&format!(
            "[llm.{vendor}]\napi_key=''\nmodel='fixture-model'\nbase_url='https://fixture.invalid/v1'\n"
        ));
        model_environment::save(&fixture.root, vendor, OLD_KEY).unwrap();
    }
    fixture.write("config.toml", &raw);
    fixture
}

fn effective(fixture: &Fixture, vendor: &str) -> Option<String> {
    let broker = EnvFileSecretsBroker::new(
        claw_core::git_remote_config::git_credential_store_path(&fixture.root),
    );
    broker
        .lookup(&claw_core::secrets::text_secret_name_for_vendor(vendor))
        .unwrap()
        .map(|value| value.expose().to_string())
}

fn credential_bytes(fixture: &Fixture) -> Vec<u8> {
    fs::read(model_environment::path(&fixture.root)).unwrap()
}

fn assert_private(fixture: &Fixture, response: &Value) {
    let raw = response.to_string();
    for secret in [OLD_KEY, NEW_KEY, fixture.root.to_str().unwrap()] {
        assert!(
            !raw.contains(secret),
            "credential response exposed private data"
        );
    }
}

#[tokio::test]
async fn provider_credentials_modern_save_updates_the_effective_runtime_key() {
    let fixture = fixture(&["custom"]);
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"custom":NEW_KEY}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        effective(&fixture, "custom").as_deref(),
        Some(NEW_KEY),
        "successful modern save must update the same credential the runtime resolves"
    );
    assert_private(&fixture, &response.1);
}

#[tokio::test]
async fn provider_credentials_new_text_secrets_are_never_persisted_in_toml() {
    let fixture = fixture(&["custom"]);
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"custom":NEW_KEY}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    let raw = fs::read_to_string(fixture.root.join("configs/config.toml")).unwrap();
    assert!(
        !raw.contains(NEW_KEY),
        "text secrets belong to the existing credential broker"
    );
    let parsed: toml::Value = toml::from_str(&raw).unwrap();
    assert_eq!(parsed["llm"]["custom"]["api_key"].as_str(), Some(""));
}

#[tokio::test]
async fn provider_credentials_modern_readback_masks_the_effective_managed_key() {
    let fixture = fixture(&["custom"]);
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    let response = fixture.api(ADMIN, Method::GET, ENDPOINT, None).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        response.1["data"]["llm"]["custom"],
        mask_secret(OLD_KEY),
        "literal empty TOML must not conceal an active broker credential"
    );
    assert_private(&fixture, &response.1);
    assert_eq!(fixture.files(), before);
    assert_eq!(credential_bytes(&fixture), credentials);
}

#[tokio::test]
async fn provider_credentials_one_request_updates_all_supported_text_vendors() {
    let fixture = fixture(&llm_vendor_names());
    let mut payload = json!({"llm":{}});
    for vendor in llm_vendor_names() {
        payload["llm"][vendor] = json!(format!("{NEW_KEY}-{vendor}"));
    }
    let response = fixture
        .api(ADMIN, Method::POST, ENDPOINT, Some(payload))
        .await;
    assert_eq!(response.0, StatusCode::OK);
    for vendor in llm_vendor_names() {
        let expected = format!("{NEW_KEY}-{vendor}");
        assert_eq!(
            effective(&fixture, vendor).as_deref(),
            Some(expected.as_str()),
            "vendor={vendor}"
        );
        assert_eq!(response.1["data"]["llm"][vendor], mask_secret(&expected));
    }
    assert_private(&fixture, &response.1);
}

#[tokio::test]
async fn provider_credentials_roundtripped_effective_mask_is_not_saved_as_a_new_secret() {
    let fixture = fixture(&["custom"]);
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"custom":mask_secret(OLD_KEY)}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(effective(&fixture, "custom").as_deref(), Some(OLD_KEY));
    assert_eq!(
        fixture.files(),
        before,
        "a mask is not a configuration edit"
    );
    assert_eq!(credential_bytes(&fixture), credentials);
}

#[tokio::test]
async fn provider_credentials_unsupported_text_vendor_rejects_the_entire_update_before_writes() {
    let fixture = fixture(&["custom"]);
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"custom":NEW_KEY,"unsupported_fixture_vendor":NEW_KEY}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::BAD_REQUEST);
    assert_eq!(fixture.files(), before);
    assert_eq!(credential_bytes(&fixture), credentials);
    assert_private(&fixture, &response.1);
}

#[tokio::test]
async fn provider_credentials_invalid_text_value_rejects_the_entire_update_before_writes() {
    for value in ["fixture\ninvalid-value", "fixture\0invalid-value"] {
        let fixture = fixture(&["custom"]);
        let before = fixture.files();
        let credentials = credential_bytes(&fixture);
        let response = fixture
            .api(
                ADMIN,
                Method::POST,
                ENDPOINT,
                Some(json!({"llm":{"custom":value}})),
            )
            .await;
        assert_eq!(response.0, StatusCode::BAD_REQUEST);
        assert_eq!(fixture.files(), before);
        assert_eq!(credential_bytes(&fixture), credentials);
        assert_private(&fixture, &response.1);
    }
}

#[tokio::test]
async fn provider_credentials_blank_input_preserves_all_existing_credentials_and_files() {
    let fixture = fixture(&["custom"]);
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"custom":""}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(effective(&fixture, "custom").as_deref(), Some(OLD_KEY));
    assert_eq!(fixture.files(), before);
    assert_eq!(credential_bytes(&fixture), credentials);
}

#[tokio::test]
async fn provider_credentials_malformed_modality_config_does_not_rotate_text_credentials() {
    let fixture = fixture(&["custom"]);
    fixture.write("audio.toml", "[fixture-invalid\n");
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"custom":NEW_KEY}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(fixture.files(), before);
    assert_eq!(credential_bytes(&fixture), credentials);
    assert_private(&fixture, &response.1);
}

#[tokio::test]
async fn provider_credentials_non_admin_cannot_rotate_or_rewrite_credentials() {
    let fixture = fixture(&["custom"]);
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    let response = fixture
        .api(
            USER,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"custom":NEW_KEY}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::FORBIDDEN);
    assert_eq!(fixture.files(), before);
    assert_eq!(credential_bytes(&fixture), credentials);
    assert_private(&fixture, &response.1);
}

#[tokio::test]
async fn provider_credentials_empty_patch_is_a_read_only_control() {
    let fixture = fixture(&["custom"]);
    let before = fixture.files();
    let credentials = credential_bytes(&fixture);
    let response = fixture
        .api(ADMIN, Method::POST, ENDPOINT, Some(json!({})))
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(fixture.files(), before);
    assert_eq!(credential_bytes(&fixture), credentials);
    assert_private(&fixture, &response.1);
}
