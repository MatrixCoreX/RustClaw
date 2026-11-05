//! Isolated metadata regressions; no real credentials or provider requests.
use super::*;
use claw_core::model_catalog::build_model_catalog_from_workspace;
use std::path::Path;

struct Workspace(Fixture);
impl AsRef<Path> for Workspace {
    fn as_ref(&self) -> &Path {
        self.0.root.as_path()
    }
}
impl std::ops::Deref for Workspace {
    type Target = Path;
    fn deref(&self) -> &Path {
        self.as_ref()
    }
}
fn temp_workspace_root() -> Workspace {
    Workspace(Fixture::new())
}

const CHILD: &str = "APP_MODEL_CATALOG_FIXTURE_CHILD";
const SECRET: &str = "fixture-catalog-source-secret-53197";

fn in_child(name: &str, values: &[(&str, &str)]) -> bool {
    if std::env::var(CHILD).as_deref() == Ok(name) {
        return false;
    }
    let full_name =
        format!("http::ui_routes::model_catalog_read_tests::truthfulness::state_contracts::{name}");
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.env_clear().env(CHILD, name).args([
        "--exact",
        &full_name,
        "--nocapture",
        "--test-threads=1",
    ]);
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
    let output = command.envs(values.iter().copied()).output().unwrap();
    assert!(
        output.status.success(),
        "isolated catalogue case {name} failed\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("running 1 test"));
    assert!(stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"));
    true
}

fn write_text(root: &Path, key: &str, base_url: &str) {
    std::fs::create_dir_all(root.join("configs")).unwrap();
    std::fs::write(
        root.join("configs/config.toml"),
        format!(
            "[server]\nrequest_timeout_seconds=30\n\
         [database]\nsqlite_path='data/fixture.db'\nbusy_timeout_ms=2000\n\
         [worker]\n\
         [llm]\nselected_vendor='minimax'\nselected_model='fixture-text'\n\
         [llm.minimax]\nbase_url='{base_url}'\napi_key='{key}'\nmodel='fixture-text'\n"
        ),
    )
    .unwrap();
}

fn text_state(root: &Path) -> String {
    let catalog = build_model_catalog_from_workspace(root).unwrap();
    assert!(!serde_json::to_string(&catalog).unwrap().contains(SECRET));
    catalog
        .entries
        .iter()
        .find(|entry| entry.provider == "minimax" && entry.model == "fixture-text")
        .unwrap()
        .credential_state
        .clone()
}

#[tokio::test]
async fn catalog_truthfulness_legacy_text_literal_is_not_an_effective_credential() {
    if in_child(
        "catalog_truthfulness_legacy_text_literal_is_not_an_effective_credential",
        &[],
    ) {
        return;
    }
    let root = temp_workspace_root();
    write_text(&root, SECRET, "https://fixture.invalid/v1");
    let runtime =
        claw_core::config::AppConfig::load(root.join("configs/config.toml").to_str().unwrap())
            .unwrap();
    assert!(
        runtime.llm.minimax.as_ref().unwrap().api_key.is_empty(),
        "the unchanged runtime must ignore a copied legacy TOML key"
    );
    assert_eq!(text_state(&root), "missing");
}

#[tokio::test]
async fn catalog_truthfulness_unloaded_environment_script_is_not_runtime_authority() {
    let script_root = temp_workspace_root();
    let script = script_root.join("fixture-environment.sh");
    std::fs::write(&script, format!("export MINIMAX_API_KEY='{SECRET}'\n")).unwrap();
    if in_child(
        "catalog_truthfulness_unloaded_environment_script_is_not_runtime_authority",
        &[("CHINESE_PROVIDER_ENV_FILE", script.to_str().unwrap())],
    ) {
        return;
    }
    let root = temp_workspace_root();
    write_text(&root, "", "https://fixture.invalid/v1");
    let runtime =
        claw_core::config::AppConfig::load(root.join("configs/config.toml").to_str().unwrap())
            .unwrap();
    assert!(runtime.llm.minimax.as_ref().unwrap().api_key.is_empty());
    assert_eq!(text_state(&root), "missing");
}

#[tokio::test]
async fn catalog_truthfulness_placeholder_environment_is_not_a_credential() {
    if in_child(
        "catalog_truthfulness_placeholder_environment_is_not_a_credential",
        &[("MINIMAX_API_KEY", "REPLACE_ME_FIXTURE")],
    ) {
        return;
    }
    let root = temp_workspace_root();
    write_text(&root, "", "https://fixture.invalid/v1");
    let runtime =
        claw_core::config::AppConfig::load(root.join("configs/config.toml").to_str().unwrap())
            .unwrap();
    assert!(runtime.llm.minimax.as_ref().unwrap().api_key.is_empty());
    assert_eq!(text_state(&root), "missing");
}

#[tokio::test]
async fn catalog_truthfulness_managed_environment_matches_the_actual_broker() {
    if in_child(
        "catalog_truthfulness_managed_environment_matches_the_actual_broker",
        &[],
    ) {
        return;
    }
    let root = temp_workspace_root();
    write_text(&root, "", "https://fixture.invalid/v1");
    claw_core::secrets::model_environment::save(&root, "minimax", SECRET).unwrap();
    let broker = claw_core::secrets::EnvFileSecretsBroker::new(
        claw_core::git_remote_config::git_credential_store_path(&root),
    );
    let (secret, source) = broker
        .lookup_with_source("text_minimax_api_key")
        .unwrap()
        .unwrap();
    assert_eq!(secret.expose(), SECRET);
    assert_eq!(source.machine_name(), "environment_file");
    assert_eq!(text_state(&root), "configured_env");
}

#[tokio::test]
async fn catalog_truthfulness_canonical_environment_matches_the_actual_broker() {
    if in_child(
        "catalog_truthfulness_canonical_environment_matches_the_actual_broker",
        &[("TEXT_MINIMAX_API_KEY", SECRET)],
    ) {
        return;
    }
    let root = temp_workspace_root();
    write_text(&root, "", "https://fixture.invalid/v1");
    let broker = claw_core::secrets::EnvFileSecretsBroker::new(
        claw_core::git_remote_config::git_credential_store_path(&root),
    );
    let (secret, source) = broker
        .lookup_with_source("text_minimax_api_key")
        .unwrap()
        .unwrap();
    assert_eq!(secret.expose(), SECRET);
    assert_eq!(source.machine_name(), "environment");
    assert_eq!(text_state(&root), "configured_env");
}

#[tokio::test]
async fn catalog_truthfulness_private_broker_is_visible_without_secret_exposure() {
    if in_child(
        "catalog_truthfulness_private_broker_is_visible_without_secret_exposure",
        &[],
    ) {
        return;
    }
    let root = temp_workspace_root();
    write_text(&root, "", "https://fixture.invalid/v1");
    let path = claw_core::git_remote_config::git_credential_store_path(&root);
    claw_core::secrets::set_file_secret(&path, "text_minimax_api_key", SECRET).unwrap();
    assert_eq!(text_state(&root), "configured_env");
}

#[tokio::test]
async fn catalog_truthfulness_corrupt_broker_is_not_ready_or_a_missing_key() {
    if in_child(
        "catalog_truthfulness_corrupt_broker_is_not_ready_or_a_missing_key",
        &[("MINIMAX_API_KEY", SECRET)],
    ) {
        return;
    }
    let root = temp_workspace_root();
    write_text(&root, "", "https://fixture.invalid/v1");
    let path = claw_core::secrets::model_environment::path(&root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "invalid managed credential document").unwrap();
    assert_eq!(text_state(&root), "unavailable");
}

#[tokio::test]
async fn catalog_truthfulness_remote_hosts_cannot_impersonate_loopback_prefixes() {
    if in_child(
        "catalog_truthfulness_remote_hosts_cannot_impersonate_loopback_prefixes",
        &[],
    ) {
        return;
    }
    for base in [
        "http://localhost.fixture.invalid/v1",
        "http://localhost@fixture.invalid/v1",
        "http://127.0.0.1.fixture.invalid/v1",
    ] {
        let root = temp_workspace_root();
        write_text(&root, "", base);
        assert_eq!(
            text_state(&root),
            "missing",
            "remote host classified as local"
        );
    }
}

#[tokio::test]
async fn catalog_truthfulness_genuine_loopback_remains_available_without_a_key() {
    if in_child(
        "catalog_truthfulness_genuine_loopback_remains_available_without_a_key",
        &[],
    ) {
        return;
    }
    for base in [
        "http://localhost:12345/v1",
        "http://127.0.0.1:12345/v1",
        "http://[::1]:12345/v1",
        "https://localhost:12345/v1",
        "http://127.1:12345/v1",
    ] {
        let root = temp_workspace_root();
        write_text(&root, "", base);
        assert_eq!(text_state(&root), "not_required_local");
    }
}

#[tokio::test]
async fn catalog_truthfulness_vendor_hints_use_the_real_url_authority() {
    if in_child(
        "catalog_truthfulness_vendor_hints_use_the_real_url_authority",
        &[],
    ) {
        return;
    }
    for base in [
        "https://api.openai.com.fixture.invalid/v1",
        "https://fixture.invalid/v1?url=https://api.minimaxi.com",
        "https://api.minimaxi.com@fixture.invalid/v1",
        "file://api.openai.com/v1",
    ] {
        let root = temp_workspace_root();
        write_text(&root, "", base);
        let catalog = build_model_catalog_from_workspace(&root).unwrap();
        let entry = catalog
            .entries
            .iter()
            .find(|entry| entry.provider == "minimax")
            .unwrap();
        assert_eq!(entry.base_url_kind, "custom_or_unknown", "{base}");
        assert_eq!(entry.credential_state, "missing", "{base}");
    }
}

#[tokio::test]
async fn catalog_truthfulness_relay_configuration_is_not_an_additional_text_vendor() {
    if in_child(
        "catalog_truthfulness_relay_configuration_is_not_an_additional_text_vendor",
        &[],
    ) {
        return;
    }
    let root = temp_workspace_root();
    std::fs::write(
        root.join("configs/config.toml"),
        r#"
[llm]
selected_vendor = "custom"
selected_model = "fixture-relay-model"
[llm.custom]
model = "fixture-relay-model"
base_url = "https://relay.fixture.invalid/v1"
api_key = ""
[llm.hosted_relay]
enabled = true
vendor = "custom"
model = "fixture-relay-model"
base_url = "https://relay.fixture.invalid/v1"
"#,
    )
    .unwrap();
    let catalog = build_model_catalog_from_workspace(&root).unwrap();
    let selected = catalog
        .entries
        .iter()
        .find(|entry| entry.provider == "custom")
        .unwrap();
    assert!(selected.active_text_provider);
    assert_eq!(selected.credential_state, "device_enrollment");
    assert!(
        catalog
            .entries
            .iter()
            .all(|entry| entry.provider != "hosted_relay"),
        "enrollment configuration must not manufacture a runnable vendor"
    );
}
