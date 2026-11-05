//! Real API checks in isolated child environments, never process-global mutation.
use super::*;

const ENV_KEY: &str = "fixture-environment-credential-24680";
const CANONICAL_KEY: &str = "fixture-canonical-credential-35791";
const PRIVATE_KEY: &str = "fixture-private-credential-46802";
const CHILD: &str = "APP_PROVIDER_CREDENTIAL_FIXTURE_CHILD";

fn in_child(name: &str, values: &[(&str, &str)]) -> bool {
    if std::env::var(CHILD).as_deref() == Ok(name) {
        return false;
    }
    let full_name = format!("http::ui_routes::provider_credential_tests::environment::{name}");
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.env_clear().env(CHILD, name).args([
        "--exact",
        &full_name,
        "--nocapture",
        "--test-threads=1",
    ]);
    // Keep only executable-loader/temp/identity configuration, never real keys.
    for name in [
        "PATH",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "DYLD_FALLBACK_LIBRARY_PATH",
        "SYSTEMROOT",
        "TMPDIR",
        "TEMP",
        "APP_PRODUCT_IDENTITY_CONFIG",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.envs(values.iter().copied());
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "isolated credential case {name} failed\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("running 1 test"));
    assert!(stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"));
    true
}

fn without_managed(vendors: &[&str]) -> Fixture {
    let fixture = fixture(vendors);
    fs::remove_file(model_environment::path(&fixture.root)).unwrap();
    fixture
}

fn credentials(fixture: &Fixture) -> [Option<Vec<u8>>; 2] {
    [
        fs::read(model_environment::path(&fixture.root)).ok(),
        fs::read(claw_core::git_remote_config::git_credential_store_path(
            &fixture.root,
        ))
        .ok(),
    ]
}

async fn assert_read(fixture: &Fixture, vendor: &str, key: Option<&str>, source: &str) {
    let before = fixture.files();
    let saved = credentials(fixture);
    let modern = fixture.api(ADMIN, Method::GET, ENDPOINT, None).await;
    assert_eq!(modern.0, StatusCode::OK);
    assert_eq!(
        modern.1["data"]["llm"][vendor],
        mask_secret(key.unwrap_or(""))
    );
    let legacy = fixture
        .api(ADMIN, Method::GET, "/v1/llm/config", None)
        .await;
    assert_eq!(legacy.0, StatusCode::OK);
    let info = legacy.1["data"]["vendors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["name"] == vendor)
        .unwrap();
    assert_eq!(info["api_key_configured"], key.is_some());
    assert_eq!(info["api_key_source"], source);
    for value in [&modern.1, &legacy.1] {
        for secret in [ENV_KEY, CANONICAL_KEY, PRIVATE_KEY, OLD_KEY] {
            assert!(!value.to_string().contains(secret), "raw credential leaked");
        }
    }
    assert_eq!(fixture.files(), before);
    assert_eq!(credentials(fixture), saved);
}

#[tokio::test]
async fn provider_credentials_environment_all_vendor_keys_are_read_without_writes() {
    let values = llm_vendor_names().map(|vendor| {
        (
            *claw_core::config::llm_vendor_api_key_env_names(vendor)
                .last()
                .unwrap(),
            ENV_KEY,
        )
    });
    if in_child(
        "provider_credentials_environment_all_vendor_keys_are_read_without_writes",
        &values,
    ) {
        return;
    }
    let fixture = without_managed(&llm_vendor_names());
    for vendor in llm_vendor_names() {
        assert_read(&fixture, vendor, Some(ENV_KEY), "environment").await;
    }
}

#[tokio::test]
async fn provider_credentials_environment_vendor_aliases_use_runtime_precedence() {
    for values in [
        vec![("XIAOMI_API_KEY", ENV_KEY)],
        vec![("XIAOMI_API_KEY", ENV_KEY), ("MIMO_API_KEY", CANONICAL_KEY)],
    ] {
        if in_child(
            "provider_credentials_environment_vendor_aliases_use_runtime_precedence",
            &values,
        ) {
            continue;
        }
        // The child runs the whole function; assert the value selected from its
        // own explicitly supplied environment, not the parent's loop position.
        let fixture = without_managed(&["mimo"]);
        let actual = std::env::var("MIMO_API_KEY").unwrap_or_else(|_| ENV_KEY.into());
        assert_read(&fixture, "mimo", Some(&actual), "environment").await;
        break;
    }
}

#[tokio::test]
async fn provider_credentials_environment_canonical_broker_key_precedes_vendor_fallback() {
    if in_child(
        "provider_credentials_environment_canonical_broker_key_precedes_vendor_fallback",
        &[
            ("MINIMAX_API_KEY", ENV_KEY),
            ("TEXT_MINIMAX_API_KEY", CANONICAL_KEY),
        ],
    ) {
        return;
    }
    let fixture = without_managed(&["minimax"]);
    assert_read(&fixture, "minimax", Some(CANONICAL_KEY), "environment").await;
}

#[tokio::test]
async fn provider_credentials_environment_private_broker_key_precedes_vendor_fallback() {
    if in_child(
        "provider_credentials_environment_private_broker_key_precedes_vendor_fallback",
        &[("MINIMAX_API_KEY", ENV_KEY)],
    ) {
        return;
    }
    let fixture = without_managed(&["minimax"]);
    claw_core::secrets::set_file_secret(
        &claw_core::git_remote_config::git_credential_store_path(&fixture.root),
        "text_minimax_api_key",
        PRIVATE_KEY,
    )
    .unwrap();
    assert_read(
        &fixture,
        "minimax",
        Some(PRIVATE_KEY),
        "private_file_fallback",
    )
    .await;
}

#[tokio::test]
async fn provider_credentials_environment_managed_override_precedes_both_environment_forms() {
    if in_child(
        "provider_credentials_environment_managed_override_precedes_both_environment_forms",
        &[
            ("MINIMAX_API_KEY", ENV_KEY),
            ("TEXT_MINIMAX_API_KEY", CANONICAL_KEY),
        ],
    ) {
        return;
    }
    let fixture = fixture(&["minimax"]);
    assert_read(&fixture, "minimax", Some(OLD_KEY), "environment_file").await;
}

#[tokio::test]
async fn provider_credentials_environment_roundtripped_vendor_mask_is_not_a_new_secret() {
    if in_child(
        "provider_credentials_environment_roundtripped_vendor_mask_is_not_a_new_secret",
        &[("MINIMAX_API_KEY", ENV_KEY)],
    ) {
        return;
    }
    let fixture = without_managed(&["minimax"]);
    let before = fixture.files();
    let saved = credentials(&fixture);
    let response = fixture
        .api(
            ADMIN,
            Method::POST,
            ENDPOINT,
            Some(json!({"llm":{"minimax":mask_secret(ENV_KEY)}})),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["data"]["llm"]["minimax"], mask_secret(ENV_KEY));
    assert_eq!(fixture.files(), before);
    assert_eq!(credentials(&fixture), saved);
}

#[tokio::test]
async fn provider_credentials_environment_placeholder_and_empty_aliases_are_not_credentials() {
    if in_child(
        "provider_credentials_environment_placeholder_and_empty_aliases_are_not_credentials",
        &[
            ("XIAOMI_API_KEY", "REPLACE_ME_FIXTURE"),
            ("MIMO_API_KEY", "  "),
        ],
    ) {
        return;
    }
    let fixture = without_managed(&["mimo"]);
    assert_read(&fixture, "mimo", None, "none").await;
}

#[tokio::test]
async fn provider_credentials_environment_corrupt_managed_storage_cannot_hide_behind_fallback() {
    if in_child(
        "provider_credentials_environment_corrupt_managed_storage_cannot_hide_behind_fallback",
        &[("MINIMAX_API_KEY", ENV_KEY)],
    ) {
        return;
    }
    let fixture = fixture(&["minimax"]);
    fs::write(
        model_environment::path(&fixture.root),
        "MINIMAX_API_KEY=bad\nMINIMAX_API_KEY=duplicate\n",
    )
    .unwrap();
    let before = fixture.files();
    let saved = credentials(&fixture);
    let modern = fixture.api(ADMIN, Method::GET, ENDPOINT, None).await;
    assert_eq!(modern.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(modern.1["error"], "provider_keys_read_failed");
    assert!(!modern.1.to_string().contains(ENV_KEY));
    assert_eq!(fixture.files(), before);
    assert_eq!(credentials(&fixture), saved);
}
