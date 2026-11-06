//! Compare metadata with the real runtime factory; never contact a provider.
use super::*;
use std::sync::Arc;

#[path = "model_catalog_runtime_credential_tests.rs"]
mod credentials;
#[path = "model_catalog_runtime_projection_tests.rs"]
mod projection;

fn isolated(module: &str, name: &str, values: &[(&str, &str)]) -> bool {
    let full_name = format!("http::ui_routes::model_catalog_read_tests::truthfulness::runtime_contracts::{module}::{name}");
    isolated_process_case(&full_name, name, values)
}

fn configure(fixture: &Fixture, llm: &str) {
    fixture.write("configs/config.toml", &format!(
        "[server]\nrequest_timeout_seconds=30\n[database]\nsqlite_path='data/fixture.db'\nbusy_timeout_ms=2000\n[worker]\n{llm}"));
}

fn load_config(fixture: &Fixture) -> claw_core::config::AppConfig {
    claw_core::config::AppConfig::load(fixture.root.join("configs/config.toml").to_str().unwrap())
        .unwrap()
}

fn runtimes(fixture: &Fixture) -> Vec<Arc<crate::LlmProviderRuntime>> {
    let config = load_config(fixture);
    crate::llm_gateway::build_providers_for_selection(&config, None, None)
}

fn only_runtime(fixture: &Fixture) -> Arc<crate::LlmProviderRuntime> {
    let mut values = runtimes(fixture);
    assert_eq!(values.len(), 1, "fixture must produce one actual runtime");
    values.pop().unwrap()
}

fn explicit(protocol: &str) -> String {
    format!(
        r#"
[llm]
selected_vendor = "fixture-explicit"
selected_model = "fixture-runtime-model"
[[llm.providers]]
name = "fixture-explicit"
type = "{protocol}"
base_url = "https://fixture.invalid/v1"
api_key = "{SECRET}"
model = "fixture-runtime-model"
priority = 1
timeout_seconds = 30
max_concurrency = 1
"#
    )
}

fn standard(vendor: &str, format: &str, selected_model: bool) -> String {
    let selected = if selected_model {
        "selected_model='fixture-runtime-model'\n"
    } else {
        ""
    };
    format!("[llm]\nselected_vendor='{vendor}'\n{selected}\n\
             [llm.{vendor}]\nbase_url='https://fixture.invalid/v1'\nmodel='fixture-runtime-model'\n{format}\n")
}

async fn entry(fixture: &Fixture, provider: &str) -> Value {
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    fixture.assert_private(&response.1);
    let found = response.1["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider"] == provider && entry["model"] == "fixture-runtime-model")
        .cloned();
    assert!(
        found.is_some(),
        "actual configured runtime must be represented"
    );
    found.unwrap()
}

#[tokio::test]
async fn model_catalog_runtime_explicit_provider_preserves_legitimate_inline_credential() {
    struct NoLookup;
    impl claw_core::secrets::SecretsBroker for NoLookup {
        fn lookup(
            &self,
            _: &str,
        ) -> Result<Option<claw_core::secrets::SecretValue>, claw_core::secrets::SecretsError>
        {
            panic!("an arbitrary explicit runtime name must retain its own credential contract");
        }
    }
    let fixture = Fixture::new();
    configure(&fixture, &explicit("openai_compat"));
    let runtime = only_runtime(&fixture);
    assert_eq!(runtime.config.name, "fixture-explicit");
    assert_eq!(runtime.api_key_using(&NoLookup), SECRET);
    let row = entry(&fixture, "fixture-explicit").await;
    assert_eq!(row["supports_text"], true);
    assert_eq!(row["credential_state"], "configured_inline");
}

#[tokio::test]
async fn model_catalog_runtime_explicit_list_does_not_advertise_inactive_vendor_templates() {
    let fixture = Fixture::new();
    configure(
        &fixture,
        &(explicit("openai_compat")
            + r#"
[llm.minimax]
base_url = "https://fixture.invalid/v1"
model = "fixture-inactive-model"
"#),
    );
    assert_eq!(only_runtime(&fixture).config.name, "fixture-explicit");
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    assert!(response.1["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["provider"] != "minimax" || entry["supports_text"] != true));
    fixture.assert_private(&response.1);
}

#[tokio::test]
async fn model_catalog_runtime_google_default_protocol_matches_the_actual_adapter() {
    let fixture = Fixture::new();
    configure(&fixture, &standard("google", "", true));
    assert_eq!(only_runtime(&fixture).config.provider_type, "google_gemini");
    assert_eq!(
        entry(&fixture, "google").await["api_style"],
        "google_gemini"
    );
}

#[tokio::test]
async fn model_catalog_runtime_anthropic_default_protocol_matches_the_actual_adapter() {
    let fixture = Fixture::new();
    configure(&fixture, &standard("anthropic", "", true));
    assert_eq!(
        only_runtime(&fixture).config.provider_type,
        "anthropic_claude"
    );
    assert_eq!(
        entry(&fixture, "anthropic").await["api_style"],
        "anthropic_messages"
    );
}

#[tokio::test]
async fn model_catalog_runtime_supported_protocol_alias_matches_the_actual_adapter() {
    let fixture = Fixture::new();
    configure(
        &fixture,
        &standard("minimax", "api_format='anthropic'", true),
    );
    assert_eq!(
        only_runtime(&fixture).config.provider_type,
        "anthropic_claude"
    );
    assert_eq!(
        entry(&fixture, "minimax").await["api_style"],
        "anthropic_messages"
    );
}

fn relay(fixture: &Fixture, vendor: &str, format: &str, provider_base: &str) {
    let llm = standard(vendor, format, true).replace("https://fixture.invalid/v1", provider_base);
    configure(fixture, &format!("{llm}\n[llm.hosted_relay]\nenabled=true\nvendor='{vendor}'\nmodel='fixture-runtime-model'\nbase_url='https://relay.fixture.invalid/v1'\n"));
}

#[tokio::test]
async fn model_catalog_runtime_relay_endpoint_slash_normalization_matches_enrollment() {
    let fixture = Fixture::new();
    relay(&fixture, "custom", "", "https://relay.fixture.invalid/v1/");
    assert!(only_runtime(&fixture).config.params.device_key_enrollment);
    assert_eq!(
        entry(&fixture, "custom").await["credential_state"],
        "device_enrollment"
    );
}

#[tokio::test]
async fn model_catalog_runtime_non_relay_protocol_does_not_imply_device_enrollment() {
    let fixture = Fixture::new();
    relay(
        &fixture,
        "minimax",
        "api_format='anthropic'",
        "https://relay.fixture.invalid/v1",
    );
    assert!(!only_runtime(&fixture).config.params.device_key_enrollment);
    assert_ne!(
        entry(&fixture, "minimax").await["credential_state"],
        "device_enrollment"
    );
}

#[tokio::test]
async fn model_catalog_runtime_absent_selected_model_uses_the_configured_default() {
    let fixture = Fixture::new();
    configure(&fixture, &standard("minimax", "", false));
    assert_eq!(only_runtime(&fixture).config.model, "fixture-runtime-model");
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(
        response.1["data"]["selected_model"],
        "fixture-runtime-model"
    );
    assert_eq!(
        entry(&fixture, "minimax").await["active_text_provider"],
        true
    );
    fixture.assert_private(&response.1);
}

#[tokio::test]
async fn model_catalog_runtime_unsupported_explicit_protocol_remains_unavailable() {
    let fixture = Fixture::new();
    configure(&fixture, &explicit("fixture-unsupported"));
    assert!(runtimes(&fixture).is_empty());
    let response = fixture.catalog(ADMIN).await;
    assert_eq!(response.0, StatusCode::OK);
    assert!(response.1["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["supports_text"] != true));
    fixture.assert_private(&response.1);
}
