use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use super::{
    activate_imported_bundle, admission_service, begin_skill_store_mutation, build_ui_router,
    bundled_prompt_for_offline_repair, finish_imported_bundle_activation,
    imported_bundle_staging_dir, imported_skill_machine_alias, precompiled_skill_package_root_for,
    precompiled_source_fallback_allowed, remove_skill_registry_block, render_skill_store_config,
    skill_store_install_spec, skill_store_operation_store, transition_skill_store_operation,
    write_runtime_config_to_paths,
};
use crate::{reload_skill_views, AppState};

const STORE_TEST_KEY: &str = "skill-store-test-admin";

#[test]
fn offline_bundled_repair_resolves_logical_skill_prompt() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    assert!(!repository.join("prompts/skills/crypto.md").is_file());

    let prompt = bundled_prompt_for_offline_repair(repository, "prompts/skills/crypto.md")
        .expect("resolve layered crypto prompt");

    assert!(prompt.contains("Shared skill prompt contract:"));
    assert!(prompt.contains("You are the `crypto` skill planner."));
}

#[test]
fn precompiled_fallback_is_limited_to_missing_or_incompatible_packages() {
    for code in [
        "precompiled_package_unavailable",
        "precompiled_platform_mismatch",
        "precompiled_manifest_mismatch",
        "manifest_protocol_unsupported",
        "precompiled_adapter_unsupported",
    ] {
        assert!(precompiled_source_fallback_allowed(code), "code={code}");
    }
    for code in [
        "precompiled_receipt_digest_mismatch",
        "precompiled_artifact_mismatch",
        "precompiled_install_root_escape",
    ] {
        assert!(!precompiled_source_fallback_allowed(code), "code={code}");
    }
}

#[test]
fn source_checkout_uses_target_scoped_precompiled_skill_store() {
    let workspace = std::env::temp_dir().join(format!(
        "agent-runtime-precompiled-root-{}",
        uuid::Uuid::new_v4()
    ));
    let target_root = workspace.join("target/prebuilt-skill-packages/test-target");
    std::fs::create_dir_all(&target_root).expect("create target precompile root");

    assert_eq!(
        precompiled_skill_package_root_for(&workspace, Some("test-target")),
        target_root
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn packaged_precompiled_skill_store_takes_priority() {
    let workspace = std::env::temp_dir().join(format!(
        "agent-runtime-precompiled-root-{}",
        uuid::Uuid::new_v4()
    ));
    let packaged = workspace.join("prebuilt/skill-packages");
    let target_root = workspace.join("target/prebuilt-skill-packages/test-target");
    std::fs::create_dir_all(&packaged).expect("create packaged precompile root");
    std::fs::create_dir_all(target_root).expect("create target precompile root");

    assert_eq!(
        precompiled_skill_package_root_for(&workspace, Some("test-target")),
        packaged
    );
    let _ = std::fs::remove_dir_all(workspace);
}

fn isolated_skill_store_state() -> (AppState, PathBuf) {
    let workspace =
        std::env::temp_dir().join(format!("skillctl-store-api-{}", uuid::Uuid::new_v4()));
    let configs = workspace.join("configs");
    std::fs::create_dir_all(&configs).expect("create isolated config directory");

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    std::fs::copy(
        repository.join("configs/config.toml"),
        configs.join("config.toml"),
    )
    .expect("copy runtime config");
    std::fs::copy(
        repository.join("configs/skills_registry.toml"),
        configs.join("skills_registry.toml"),
    )
    .expect("copy skills registry");
    std::fs::copy(
        repository.join("configs/weather.toml"),
        configs.join("weather.toml"),
    )
    .expect("copy weather config");
    for root in ["crates/skills", "optional_skills"] {
        let source_root = repository.join(root);
        for entry in std::fs::read_dir(&source_root).expect("read skill manifest root") {
            let entry = entry.expect("read skill manifest entry");
            let source = entry.path().join("skill.toml");
            if !source.is_file() {
                continue;
            }
            let destination = workspace
                .join(root)
                .join(entry.file_name())
                .join("skill.toml");
            std::fs::create_dir_all(destination.parent().expect("manifest parent"))
                .expect("create manifest parent");
            std::fs::copy(source, destination).expect("copy skill manifest");
        }
    }

    let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.skill_rt.workspace_root = workspace.clone();
    state.reload_ctx.config_path_for_reload =
        configs.join("config.toml").to_string_lossy().into_owned();
    reload_skill_views(&state).expect("load isolated skill views");

    let db = state.core.db.get().expect("test database");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at) \
         VALUES (?1, 'admin', 1, '1', NULL)",
        rusqlite::params![STORE_TEST_KEY],
    )
    .expect("insert skill store test identity");
    drop(db);

    (state, workspace)
}

fn installed_package_pointer(workspace: &Path, skill_name: &str) -> PathBuf {
    workspace
        .join("data/skill-packages")
        .join(skill_name)
        .join("current.json")
}

async fn call_skill_store_api(
    router: axum::Router,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, payload) = call_skill_store_api_raw(router.clone(), method, path, body).await;
    if status != StatusCode::ACCEPTED {
        return (status, payload);
    }
    let Some(operation_id) = payload["data"]["operation"]["operation_id"].as_str() else {
        return (status, payload);
    };
    let operation = wait_for_skill_store_operation(router, operation_id).await;
    match operation["status"].as_str() {
        Some("success") => (
            StatusCode::OK,
            serde_json::json!({
                "ok": true,
                "data": operation["result"].clone()
            }),
        ),
        Some("failure") => panic!("operation failed: {operation}"),
        Some("cancelled") => panic!("operation cancelled: {operation}"),
        status => panic!("operation did not terminate: {status:?}"),
    }
}

async fn wait_for_skill_store_operation(router: axum::Router, operation_id: &str) -> Value {
    for _ in 0..500 {
        let (poll_status, polled) = call_skill_store_api_raw(
            router.clone(),
            Method::GET,
            &format!("/v1/skills/store/operations/{operation_id}"),
            None,
        )
        .await;
        assert_eq!(poll_status, StatusCode::OK, "poll operation {operation_id}");
        match polled["data"]["operation"]["status"].as_str() {
            Some("success" | "failure" | "cancelled") => {
                return polled["data"]["operation"].clone();
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
        }
    }
    panic!("operation did not complete: {operation_id}");
}

async fn call_skill_store_api_raw(
    router: axum::Router,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-agent-key", STORE_TEST_KEY);
    let request_body = if let Some(body) = body {
        builder = builder.header("content-type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = router
        .oneshot(builder.body(request_body).expect("skill store request"))
        .await
        .expect("skill store response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read skill store response body");
    let payload = serde_json::from_slice(&bytes).expect("parse skill store response body");
    (status, payload)
}

fn store_item<'a>(payload: &'a Value, name: &str) -> &'a Value {
    payload["data"]["items"]
        .as_array()
        .expect("skill store item array")
        .iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("missing skill store item: {name}"))
}

fn store_item_names(payload: &Value) -> BTreeSet<&str> {
    payload["data"]["items"]
        .as_array()
        .expect("skill store item array")
        .iter()
        .map(|item| item["name"].as_str().expect("skill store item name"))
        .collect()
}

fn value_array_contains(value: &Value, expected: &str) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(expected)))
}

async fn set_managed_skill_enabled(router: axum::Router, skill_name: &str, enabled: bool) -> Value {
    let (status, current) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/config", None).await;
    assert_eq!(status, StatusCode::OK, "read config for {skill_name}");
    let mut switches = current["data"]["skill_switches"]
        .as_object()
        .expect("skill switches object")
        .clone();
    switches.insert(skill_name.to_string(), Value::Bool(enabled));
    let (status, updated) = call_skill_store_api(
        router,
        Method::POST,
        "/v1/skills/config",
        Some(serde_json::json!({"skill_switches": switches})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "update config for {skill_name}: {updated}"
    );
    updated
}

async fn register_external_skill_fixture(state: &AppState, workspace: &Path, skill_name: &str) {
    let bundle_dir = workspace.join("third_party").join(skill_name);
    let interface_md = write_external_skill_fixture(&bundle_dir, skill_name);
    let (status, response) = super::finalize_imported_bundle(
        state,
        &bundle_dir,
        &format!("third_party/{skill_name}"),
        "local-test",
        true,
        false,
        &interface_md,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(response.0.ok);
    assert_eq!(
        response
            .0
            .data
            .as_ref()
            .and_then(|data| data["skill_name"].as_str()),
        Some(skill_name)
    );
}

fn write_external_skill_fixture(bundle_dir: &Path, skill_name: &str) -> String {
    std::fs::create_dir_all(&bundle_dir).expect("create external skill bundle");
    let interface_md = format!(
        "---\nname: {skill_name}\ndescription: External fixture\n---\n# External fixture\n"
    );
    std::fs::write(bundle_dir.join("INTERFACE.md"), &interface_md)
        .expect("write external skill interface");
    std::fs::write(
        bundle_dir.join("run.sh"),
        "#!/bin/sh\nIFS= read -r line\nrequest_id=$(printf '%s' \"$line\" | sed -n 's/.*\"request_id\":\"\\([^\"]*\\)\".*/\\1/p')\nprintf '{\"request_id\":\"%s\",\"status\":\"ok\",\"text\":\"fixture ok\",\"error_text\":null}\\n' \"$request_id\"\n",
    )
    .expect("write external skill runner");
    let platform = skill_sdk::HostPlatform::current();
    std::fs::write(
        bundle_dir.join("skill.toml"),
        format!(
            r#"schema_version = 1
[package]
name = "{skill_name}"
version = "1.0.0"
description = "External fixture"
protocol = "agent-jsonl-v1"
supported_os = ["{os}"]
supported_arch = ["{arch}"]
license = "MIT"
source = "local-test"
[registry]
name = "{skill_name}"
[build]
adapter = "generic_process"
source_root = "."
network = "deny"
[run]
launcher = "process"
entrypoint = "runtime/src/run.sh"
working_directory = "runtime/src"
timeout_seconds = 10
[security]
capability_policy_source = "registry"
sandbox = "required"
runtime_network = false
inherit_credentials = false
"#,
            os = platform.os,
            arch = platform.arch,
        ),
    )
    .expect("write external skill manifest");
    interface_md
}

#[test]
fn skill_store_config_keeps_switch_and_uninstall_state_distinct() {
    let raw = "[skills]\nskill_switches = { weather = true }\nskills_list = [\"weather\"]\n";
    let switches = BTreeMap::from([("weather".to_string(), false)]);
    let uninstalled = BTreeSet::from(["weather".to_string()]);

    let updated = render_skill_store_config(raw, &switches, &uninstalled);
    let parsed = toml::from_str::<toml::Value>(&updated).expect("valid config");

    assert_eq!(
        parsed["skills"]["skill_switches"]["weather"].as_bool(),
        Some(false)
    );
    assert_eq!(
        parsed["skills"]["uninstalled_skills"][0].as_str(),
        Some("weather")
    );
}

#[test]
fn runtime_config_write_does_not_overwrite_docker_template() {
    let (_state, workspace) = isolated_skill_store_state();
    let docker_config = workspace.join("docker/config/config.toml");
    std::fs::create_dir_all(docker_config.parent().expect("docker config parent"))
        .expect("create docker config directory");
    std::fs::write(&docker_config, "# docker deployment template\n")
        .expect("write docker config sentinel");

    let active_config = workspace.join("configs/config.toml");
    let updated = std::fs::read_to_string(&active_config)
        .expect("read active config")
        .replace("default_locale = \"zh-CN\"", "default_locale = \"en\"");
    write_runtime_config_to_paths(&active_config, None, &updated)
        .expect("write active runtime config");

    assert_eq!(
        std::fs::read_to_string(&active_config).expect("reread active config"),
        updated
    );
    assert_eq!(
        std::fs::read_to_string(&docker_config).expect("reread docker config"),
        "# docker deployment template\n"
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn runtime_config_write_updates_explicit_persistence_path() {
    let (_state, workspace) = isolated_skill_store_state();
    let active_config = workspace.join("configs/config.toml");
    let persisted_config = workspace.join("mounted/config.toml");
    let updated = "[ui]\ndefault_locale = \"en\"\n";

    write_runtime_config_to_paths(&active_config, Some(&persisted_config), updated)
        .expect("write active and persisted runtime config");

    assert_eq!(
        std::fs::read_to_string(active_config).expect("read active config"),
        updated
    );
    assert_eq!(
        std::fs::read_to_string(persisted_config).expect("read persisted config"),
        updated
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn reimport_removes_every_existing_registry_block_before_append() {
    let raw = "[[skills]]\nname = \"demo\"\nenabled = true\n\n[[skills]]\nname = \"keep\"\nenabled = true\n\n[[skills]]\nname = \"demo\"\nenabled = false\n";

    let (updated, removed) = remove_skill_registry_block(raw, "demo");

    assert!(removed);
    assert!(!updated.contains("name = \"demo\""));
    assert_eq!(updated.matches("name = \"keep\"").count(), 1);
}

#[test]
fn imported_skill_aliases_remain_language_neutral_machine_tokens() {
    assert_eq!(
        imported_skill_machine_alias("Vendor.Skill", "vendor_skill"),
        Some("vendor.skill".to_string())
    );
    assert_eq!(imported_skill_machine_alias("My Skill", "my_skill"), None);
    assert_eq!(
        imported_skill_machine_alias("图像工具", "external_skill"),
        None
    );
}

#[test]
fn failed_import_bundle_activation_restores_the_previous_package_directory() {
    let workspace = std::env::temp_dir().join(format!(
        "agent-runtime-import-atomic-{}",
        uuid::Uuid::new_v4()
    ));
    let existing = workspace.join("third_party/clawhub/weather");
    std::fs::create_dir_all(&existing).expect("existing package");
    std::fs::write(existing.join("old-version.txt"), "keep me").expect("old sentinel");
    let staging = imported_bundle_staging_dir(&workspace).expect("staging");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    std::fs::copy(
        repository.join("optional_skills/weather/skill.toml"),
        staging.join("skill.toml"),
    )
    .expect("staged manifest");
    std::fs::write(staging.join("INTERFACE.md"), "# replacement\n").expect("interface");

    let activation = activate_imported_bundle(&workspace, &staging).expect("activate staging");
    assert!(!activation.bundle_dir.join("old-version.txt").exists());
    assert!(activation.bundle_dir.join("skill.toml").is_file());
    finish_imported_bundle_activation(&activation, false).expect("restore previous");
    assert_eq!(
        std::fs::read_to_string(existing.join("old-version.txt")).expect("restored sentinel"),
        "keep me"
    );
    assert!(!staging.exists());
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn imported_external_skill_can_be_disabled_removed_and_reinstalled() {
    let (state, workspace) = isolated_skill_store_state();
    let skill_name = "image_partner";
    register_external_skill_fixture(&state, &workspace, skill_name).await;
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());

    let (status, initial_config) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(value_array_contains(
        &initial_config["data"]["managed_skills"],
        skill_name
    ));
    assert!(value_array_contains(
        &initial_config["data"]["external_skill_names"],
        skill_name
    ));

    let mut switches = initial_config["data"]["skill_switches"]
        .as_object()
        .expect("skill switches object")
        .clone();
    switches.insert(skill_name.to_string(), Value::Bool(false));
    let (status, disabled) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/config",
        Some(serde_json::json!({"skill_switches": switches})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disabled["data"]["skill_switches"][skill_name], false);
    assert!(!value_array_contains(
        &disabled["data"]["effective_enabled_skills_preview"],
        skill_name
    ));
    reload_skill_views(&state).expect("apply disabled external skill state");

    let (status, catalog) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    let imported = store_item(&catalog, skill_name);
    assert_eq!(imported["catalog_section"], "other");
    assert_eq!(imported["source_kind"], "third_party");
    assert_eq!(imported["installed"], true);
    assert_eq!(imported["enabled"], false);

    let (status, removed) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/remove",
        Some(serde_json::json!({"skill_name": skill_name, "preserve_config": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(removed["data"]["installed"], false);
    assert_eq!(removed["data"]["config_preserved"], true);
    assert!(workspace
        .join("third_party")
        .join(skill_name)
        .join("INTERFACE.md")
        .is_file());

    let (status, removed_config) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!value_array_contains(
        &removed_config["data"]["managed_skills"],
        skill_name
    ));
    assert!(!value_array_contains(
        &removed_config["data"]["external_skill_names"],
        skill_name
    ));

    let (status, reinstalled) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": skill_name})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reinstalled["data"]["installed"], true);
    assert_eq!(reinstalled["data"]["package_installed"], true);
    assert_eq!(reinstalled["data"]["adapter"], "generic_process");
    assert!(reinstalled["data"]["receipt_digest"]
        .as_str()
        .is_some_and(|value| value.len() == 64));

    let (status, restored_config) =
        call_skill_store_api(router, Method::GET, "/v1/skills/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(value_array_contains(
        &restored_config["data"]["external_skill_names"],
        skill_name
    ));
    assert_eq!(restored_config["data"]["skill_switches"][skill_name], true);
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn internal_extension_admission_uses_the_same_overlay_without_source_writes() {
    let (state, workspace) = isolated_skill_store_state();
    let skill_name = "runtime_probe";
    let source = workspace.join("external_skills").join(skill_name);
    write_external_skill_fixture(&source, skill_name);
    let config_path = workspace.join("configs/config.toml");
    let registry_path = workspace.join("configs/skills_registry.toml");
    let config_before = std::fs::read(&config_path).expect("config baseline");
    let registry_before = std::fs::read(&registry_path).expect("registry baseline");
    let token = claw_core::secrets::issue_secret_token_value(
        &claw_core::secrets::SecretValue::new(
            serde_json::json!({
                "task_id": "internal-admission-test",
                "user_id": 1,
                "chat_id": 1,
                "user_key": STORE_TEST_KEY,
                "channel": "ui",
                "external_user_id": null,
                "external_chat_id": null,
                "kind": "run_skill",
                "payload_json": "{}",
                "skill_name": "extension_manager"
            })
            .to_string(),
        ),
        std::time::Duration::from_secs(60),
    )
    .expect("issue internal admission token");
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/internal/skills/admit")
        .header("content-type", "application/json")
        .header("x-agent-internal-skill-token", token)
        .body(Body::from(
            serde_json::json!({
                "source": source,
                "enabled": true,
                "allow_network": false
            })
            .to_string(),
        ))
        .expect("internal admission request");
    let response = router
        .oneshot(request)
        .await
        .expect("internal admission call");
    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("internal admission response"),
    )
    .expect("parse internal admission response");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["data"]["skill_name"], skill_name);
    assert!(payload["data"]["registry_generation"]
        .as_u64()
        .is_some_and(|generation| generation > 0));
    assert!(state.get_skills_list().contains(skill_name));
    assert_eq!(
        std::fs::read(config_path).expect("config after"),
        config_before
    );
    assert_eq!(
        std::fs::read(registry_path).expect("registry after"),
        registry_before
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_http_api_removes_and_reinstalls_optional_skill() {
    let (state, workspace) = isolated_skill_store_state();
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);

    let (status, initial) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(store_item(&initial, "weather")["installed"], false);
    assert_eq!(store_item(&initial, "weather")["catalog_section"], "other");
    assert_eq!(
        store_item(&initial, "weather")["description_zh"],
        "通过 Open-Meteo 查询指定城市、地点或坐标的当前天气与逐日预报。"
    );
    assert_eq!(
        store_item(&initial, "weather")["source_kind"],
        "bundled_optional"
    );
    assert_eq!(store_item(&initial, "media_download")["installed"], false);
    assert_eq!(
        store_item(&initial, "media_download")["description_zh"],
        "支持抖音、快手、小红书、TikTok 和 YouTube 的 App 复制分享文案、短链和网页分享链接；默认直接下载原始媒体。抖音和小红书图文帖会分别返回全部原图和平台正文；图片 OCR 仍需明确要求。本技能内正文和 OCR 少于 200 字直接对话交付，达到 200 字则发送文本文件。"
    );
    assert_eq!(
        store_item(&initial, "media_download")["source_kind"],
        "bundled_optional"
    );
    assert_eq!(
        store_item(&initial, "weather")["existing_config_files"][0],
        "configs/weather.toml"
    );
    assert_eq!(store_item(&initial, "crypto")["storage_kind"], "sqlite");
    assert_eq!(
        store_item(&initial, "crypto")["private_data_state"],
        "empty"
    );
    assert_eq!(
        store_item_names(&initial),
        BTreeSet::from([
            "chinese_almanac",
            "crypto",
            "invest_copy",
            "map_merchant",
            "media_download",
            "photo_organize",
            "stock",
            "weather",
            "x"
        ]),
    );

    let (status, installed) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(installed["data"]["installed"], true);
    assert_eq!(installed["data"]["enabled"], true);
    assert_eq!(installed["data"]["package_installed"], true);
    assert_eq!(installed["data"]["adapter"], "cargo");
    assert_eq!(installed["data"]["installed_version"], "0.1.8");
    assert_eq!(
        installed["data"]["reused_config_files"][0],
        "configs/weather.toml"
    );
    assert!(installed_package_pointer(&workspace, "weather").is_file());

    let (status, after_install) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(store_item(&after_install, "weather")["installed"], true);
    assert_eq!(store_item(&after_install, "weather")["enabled"], true);
    assert!(!after_install["data"]["uninstalled_skill_names"]
        .as_array()
        .expect("uninstalled skill names")
        .iter()
        .any(|name| name == "weather"));

    let (status, locked) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/remove",
        Some(serde_json::json!({"skill_name": "schedule"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!locked["ok"].as_bool().expect("locked response ok flag"));
    assert_eq!(locked["error"], "skill_store_locked_skill");

    let (status, removed) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/remove",
        Some(serde_json::json!({"skill_name": "weather", "preserve_config": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(removed["data"]["installed"], false);
    assert_eq!(removed["data"]["package_removed"], true);
    assert_eq!(removed["data"]["config_preserved"], true);
    assert_eq!(removed["data"]["data_preserved"], true);
    assert!(!installed_package_pointer(&workspace, "weather").exists());
    assert!(workspace.join("configs/weather.toml").is_file());

    let (status, reinstalled) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reinstalled["data"]["reused_config_files"][0],
        "configs/weather.toml"
    );

    let (status, removed_with_config) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/remove",
        Some(serde_json::json!({"skill_name": "weather", "preserve_config": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(removed_with_config["data"]["config_preserved"], false);
    assert_eq!(
        removed_with_config["data"]["deleted_config_files"][0],
        "configs/weather.toml"
    );
    assert!(!workspace.join("configs/weather.toml").exists());

    let (status, after_remove) =
        call_skill_store_api(router, Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(store_item(&after_remove, "weather")["installed"], false);

    let config = std::fs::read_to_string(workspace.join("configs/config.toml"))
        .expect("read isolated config");
    let parsed = toml::from_str::<toml::Value>(&config).expect("parse isolated config");
    assert_eq!(
        parsed["skills"]["skill_switches"]["weather"].as_bool(),
        Some(false)
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_install_returns_a_durable_operation_and_records_every_stage() {
    let (state, workspace) = isolated_skill_store_state();
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);
    let (status, _) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, accepted) = call_skill_store_api_raw(
        router.clone(),
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let operation_id = accepted["data"]["operation"]["operation_id"]
        .as_str()
        .expect("durable operation id");
    assert_eq!(accepted["data"]["operation"]["status"], "queued");

    let completed = wait_for_skill_store_operation(router, operation_id).await;
    assert_eq!(completed["status"], "success");
    let stages = completed["stages"]
        .as_array()
        .expect("operation stages")
        .iter()
        .filter_map(|record| record["stage"].as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "queued",
        "preflight",
        "dependencies",
        "build",
        "smoke",
        "activate",
        "configure",
        "success",
    ] {
        assert!(
            stages.contains(expected),
            "missing stage {expected}: {stages:?}"
        );
    }
    assert!(installed_package_pointer(&workspace, "weather").is_file());
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_recovers_interrupted_operations_from_disk() {
    let (state, workspace) = isolated_skill_store_state();
    let interrupted = skill_store_operation_store(&state)
        .create("weather", skill_sdk::OperationAction::Install)
        .expect("persist interrupted operation");
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);

    let (status, catalog) =
        call_skill_store_api(router, Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(catalog["data"]["active_operation"].is_null());
    let recovered = catalog["data"]["recent_operations"]
        .as_array()
        .expect("recent operations")
        .iter()
        .find(|operation| operation["operation_id"] == interrupted.operation_id)
        .expect("recovered operation");
    assert_eq!(recovered["status"], "failure");
    assert_eq!(recovered["failure"]["error_code"], "operation_interrupted");
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_requires_explicit_choice_before_deleting_private_data() {
    let (state, workspace) = isolated_skill_store_state();
    crate::repo::upsert_exchange_credential_for_user_key(
        &state,
        STORE_TEST_KEY,
        "okx",
        "api-key",
        "api-secret",
        None,
    )
    .expect("seed crypto private data");
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());

    let (status, catalog) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        store_item(&catalog, "crypto")["private_data_state"],
        "present"
    );

    let (status, installed) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "crypto"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(installed["data"]["installed"], true);

    let (status, removed) = call_skill_store_api(
        router,
        Method::POST,
        "/v1/skills/store/remove",
        Some(serde_json::json!({
            "skill_name": "crypto",
            "preserve_config": true,
            "preserve_data": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(removed["data"]["config_preserved"], true);
    assert_eq!(removed["data"]["data_preserved"], false);
    assert_eq!(removed["data"]["deleted_private_data"]["rows_deleted"], 1);
    assert_eq!(
        state
            .core
            .skill_storage
            .data_state("crypto", "sqlite")
            .expect("crypto private data state"),
        "empty"
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn all_on_demand_skills_complete_isolated_http_lifecycle() {
    let (state, workspace) = isolated_skill_store_state();
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());
    let registry = state.get_skills_registry().expect("skills registry");
    let mut inventory = registry
        .all_names()
        .into_iter()
        .filter(|name| {
            registry
                .get(name)
                .is_some_and(|entry| entry.install_mode.as_deref() == Some("on_demand"))
        })
        .collect::<Vec<_>>();
    inventory.sort_unstable();
    assert!(!inventory.is_empty(), "on-demand inventory");

    if let Ok(requested) = claw_core::product_identity::env_string("SKILL_STORE_TEST_SKILL") {
        let requested = requested.trim();
        assert!(
            inventory.iter().any(|name| name == requested),
            "requested on-demand skill is absent: {requested}"
        );
        inventory.retain(|name| name == requested);
    }

    let (status, initial) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    let catalog_names = store_item_names(&initial)
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let registry_names = registry
        .all_names()
        .into_iter()
        .filter(|name| {
            registry
                .get(name)
                .is_some_and(|entry| entry.install_mode.as_deref() == Some("on_demand"))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(catalog_names, registry_names);

    let unrelated_config = workspace.join("configs/lifecycle-unrelated.toml");
    std::fs::write(&unrelated_config, "owner = \"unrelated\"\n")
        .expect("write unrelated config sentinel");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");

    for skill_name in inventory {
        let entry = registry.get(&skill_name).expect("on-demand registry entry");
        assert!(
            !entry.planner_capabilities.is_empty(),
            "planner capabilities missing for {skill_name}"
        );
        let prompt_rel = claw_core::prompt_layers::canonical_skill_prompt_body_rel_path(
            entry.prompt_file.trim(),
        )
        .expect("canonical generated skill prompt");
        let prompt = std::fs::read_to_string(repository.join(prompt_rel))
            .unwrap_or_else(|error| panic!("read generated prompt for {skill_name}: {error}"));
        assert!(
            !prompt.trim().is_empty(),
            "generated prompt is empty for {skill_name}"
        );

        let item = store_item(&initial, &skill_name);
        let config_files = item["config_files"]
            .as_array()
            .expect("declared config files")
            .iter()
            .map(|value| value.as_str().expect("config path").to_string())
            .collect::<Vec<_>>();
        let config_sentinel = format!("owner = \"{skill_name}\"\n");
        for relative in &config_files {
            let path = workspace.join(relative);
            std::fs::create_dir_all(path.parent().expect("config parent"))
                .expect("create config parent");
            std::fs::write(path, &config_sentinel).expect("write skill config sentinel");
        }
        if skill_name == "crypto" {
            crate::repo::upsert_exchange_credential_for_user_key(
                &state,
                STORE_TEST_KEY,
                "okx",
                "api-key",
                "api-secret",
                None,
            )
            .expect("seed isolated crypto private data");
        }

        let spec = skill_store_install_spec(&state, &skill_name)
            .expect("read install spec")
            .expect("runner install spec");
        assert_eq!(spec.skill_name, skill_name);
        let (status, installed) = call_skill_store_api(
            router.clone(),
            Method::POST,
            "/v1/skills/store/install",
            Some(serde_json::json!({
                "skill_name": skill_name,
                "allow_network": matches!(
                    spec.network_policy,
                    skill_sdk::BuildNetworkPolicy::ApprovalRequired
                )
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "install {skill_name}");
        assert_eq!(installed["data"]["installed"], true, "install {skill_name}");
        assert_eq!(
            installed["data"]["package_installed"], true,
            "install package {skill_name}"
        );
        assert_eq!(installed["data"]["adapter"], spec.adapter.as_token());
        let pointer = installed_package_pointer(&workspace, &skill_name);
        let pointer_before = std::fs::read(&pointer).expect("read installed package pointer");
        assert!(state.get_skills_list().contains(&skill_name));

        for candidate in registry_names.iter() {
            assert_eq!(
                skill_sdk::SkillRuntimeResolver::new(workspace.join("data/skill-packages"))
                    .resolve(candidate)
                    .is_ok(),
                candidate == &skill_name,
                "only the selected package may resolve while installing {skill_name}"
            );
        }

        let disabled = set_managed_skill_enabled(router.clone(), &skill_name, false).await;
        assert_eq!(disabled["data"]["skill_switches"][&skill_name], false);
        assert_eq!(disabled["data"]["restart_required"], false);
        assert!(!state.get_skills_list().contains(&skill_name));
        assert_eq!(
            std::fs::read(&pointer).expect("read disabled package pointer"),
            pointer_before,
            "disable must not reinstall {skill_name}"
        );
        let enabled = set_managed_skill_enabled(router.clone(), &skill_name, true).await;
        assert_eq!(enabled["data"]["skill_switches"][&skill_name], true);
        assert_eq!(enabled["data"]["restart_required"], false);
        assert!(state.get_skills_list().contains(&skill_name));
        assert_eq!(
            std::fs::read(&pointer).expect("read re-enabled package pointer"),
            pointer_before,
            "re-enable must not reinstall {skill_name}"
        );

        let (status, preserved) = call_skill_store_api(
            router.clone(),
            Method::POST,
            "/v1/skills/store/remove",
            Some(serde_json::json!({
                "skill_name": skill_name,
                "preserve_config": true,
                "preserve_data": true
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "preserve uninstall {skill_name}");
        assert_eq!(preserved["data"]["installed"], false);
        assert_eq!(preserved["data"]["config_preserved"], true);
        assert_eq!(preserved["data"]["data_preserved"], true);
        assert!(!pointer.exists());
        for relative in &config_files {
            assert_eq!(
                std::fs::read_to_string(workspace.join(relative)).expect("retained config"),
                config_sentinel
            );
        }
        if skill_name == "crypto" {
            assert_eq!(
                state
                    .core
                    .skill_storage
                    .data_state(&skill_name, &entry.storage.as_ref().expect("storage").kind)
                    .expect("retained private data"),
                "present"
            );
        }

        let (status, reinstalled) = call_skill_store_api(
            router.clone(),
            Method::POST,
            "/v1/skills/store/install",
            Some(serde_json::json!({
                "skill_name": skill_name,
                "allow_network": matches!(
                    spec.network_policy,
                    skill_sdk::BuildNetworkPolicy::ApprovalRequired
                )
            })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "reinstall {skill_name}: {reinstalled}"
        );
        assert_eq!(reinstalled["data"]["installed"], true);
        for relative in &config_files {
            assert!(value_array_contains(
                &reinstalled["data"]["reused_config_files"],
                relative
            ));
        }

        let (status, deleted) = call_skill_store_api(
            router.clone(),
            Method::POST,
            "/v1/skills/store/remove",
            Some(serde_json::json!({
                "skill_name": skill_name,
                "preserve_config": false,
                "preserve_data": false
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "destructive uninstall {skill_name}");
        assert_eq!(deleted["data"]["installed"], false);
        assert_eq!(deleted["data"]["config_preserved"], false);
        assert_eq!(deleted["data"]["data_preserved"], false);
        for relative in &config_files {
            assert!(!workspace.join(relative).exists(), "delete {relative}");
        }
        if entry.storage.is_some() {
            assert!(deleted["data"]["deleted_private_data"].is_object());
            assert_eq!(
                state
                    .core
                    .skill_storage
                    .data_state(&skill_name, &entry.storage.as_ref().expect("storage").kind)
                    .expect("deleted private data"),
                "empty"
            );
        }
        assert_eq!(
            std::fs::read_to_string(&unrelated_config).expect("unrelated config survives"),
            "owner = \"unrelated\"\n"
        );
        let (status, refreshed) =
            call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(store_item(&refreshed, &skill_name)["installed"], false);
        assert!(refreshed["data"]["active_operation"].is_null());
        println!(
            "SKILL_STORE_LIFECYCLE_OK skill={} external_call_count=0",
            skill_name
        );
    }

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_uses_runtime_generation_without_mutating_configs() {
    let (mut state, workspace) = isolated_skill_store_state();
    let default_config = workspace.join("configs/config.toml");
    let active_config = workspace.join("profiles/active.toml");
    std::fs::create_dir_all(active_config.parent().expect("active config parent"))
        .expect("create active config directory");
    std::fs::copy(&default_config, &active_config).expect("copy active runtime config");
    let default_before = std::fs::read_to_string(&default_config).expect("read default config");
    let active_before = std::fs::read_to_string(&active_config).expect("read active config");
    state.reload_ctx.config_path_for_reload = active_config.to_string_lossy().into_owned();
    reload_skill_views(&state).expect("reload active runtime config");

    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());
    let (status, installed) = call_skill_store_api(
        router,
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(installed["data"]["enabled"], true);
    assert_eq!(
        std::fs::read_to_string(&active_config).expect("reread active config"),
        active_before
    );
    assert_eq!(
        std::fs::read_to_string(default_config).expect("reread default config"),
        default_before
    );
    let admission = admission_service(&state)
        .expect("admission service")
        .snapshot()
        .expect("admission snapshot");
    assert_eq!(
        admission.state("weather"),
        Some(skill_sdk::AdmissionState::Enabled)
    );
    assert!(admission.generation > 0);
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn skill_store_rejects_install_when_registry_excludes_the_host_platform() {
    let (state, workspace) = isolated_skill_store_state();
    let registry_path = workspace.join("configs/skills_registry.toml");
    let raw = std::fs::read_to_string(&registry_path).expect("read isolated registry");
    let current_os = std::env::consts::OS;
    let unsupported_os = if current_os == "linux" {
        "macos"
    } else {
        "linux"
    };
    let weather_header = "name = \"weather\"\nenabled = false\nkind = \"runner\"\nplanner_kind = \"skill\"\ngroup = \"news/web\"\nsupported_os = [\"linux\", \"macos\"]";
    let unsupported_header = format!(
        "name = \"weather\"\nenabled = false\nkind = \"runner\"\nplanner_kind = \"skill\"\ngroup = \"news/web\"\nsupported_os = [\"{unsupported_os}\"]"
    );
    assert!(
        raw.contains(weather_header),
        "weather registry fixture changed"
    );
    std::fs::write(
        &registry_path,
        raw.replacen(&weather_header, &unsupported_header, 1),
    )
    .expect("write unsupported weather registry");
    reload_skill_views(&state).expect("reload unsupported weather registry");

    let error = skill_store_install_spec(&state, "weather")
        .expect_err("unsupported host install must fail");
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert_eq!(error.code.as_str(), "skill_store_unsupported_os");
    assert!(error
        .diagnostic
        .contains(&format!("current_os={current_os}")));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_repairs_configured_skill_when_runner_is_missing() {
    let (state, workspace) = isolated_skill_store_state();
    let config_path = workspace.join("configs/config.toml");
    let raw = std::fs::read_to_string(&config_path).expect("read isolated config");
    let configured = render_skill_store_config(
        &raw,
        &BTreeMap::from([("weather".to_string(), true)]),
        &BTreeSet::new(),
    );
    std::fs::write(&config_path, configured).expect("write configured install state");
    reload_skill_views(&state).expect("reload configured install state");

    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);
    let (status, before_repair) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    let weather = store_item(&before_repair, "weather");
    assert_eq!(weather["configured_installed"], true);
    assert_eq!(weather["package_available"], false);
    assert_eq!(weather["installed"], false);
    assert_eq!(weather["enabled"], false);
    assert_eq!(weather["installation_issue"], "package_missing");

    let (status, repaired) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repaired["data"]["installed"], true);
    assert!(installed_package_pointer(&workspace, "weather").is_file());

    let (status, after_repair) =
        call_skill_store_api(router, Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    let weather = store_item(&after_repair, "weather");
    assert_eq!(weather["configured_installed"], true);
    assert_eq!(weather["package_available"], true);
    assert_eq!(weather["installed"], true);
    assert_eq!(weather["enabled"], true);
    assert!(weather["installation_issue"].is_null());

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_rejects_overlapping_mutations() {
    let (state, workspace) = isolated_skill_store_state();
    let _permit =
        begin_skill_store_mutation(&state, "weather").expect("hold skill-store mutation permit");
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);

    let (status, response) = call_skill_store_api(
        router,
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(response["error"], "skill_store_operation_busy");
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_requires_explicit_network_approval_from_the_install_request() {
    let (state, workspace) = isolated_skill_store_state();
    let manifest_path = workspace.join("optional_skills/weather/skill.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .expect("read weather manifest")
        .replace("network = \"deny\"", "network = \"approval_required\"");
    std::fs::write(&manifest_path, manifest).expect("write approval-required manifest");
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);

    let (status, catalog) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        store_item(&catalog, "weather")["build_network_policy"],
        "approval_required"
    );

    let (status, denied) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(denied["error"], "skill_store_network_approval_required");

    let (status, approved) = call_skill_store_api(
        router,
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({
            "skill_name": "weather",
            "allow_network": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["data"]["adapter"], "cargo");
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn skill_store_locks_are_per_skill_while_builds_use_a_separate_global_limit() {
    let (state, workspace) = isolated_skill_store_state();
    let weather = begin_skill_store_mutation(&state, "weather").expect("lock weather");
    let crypto = begin_skill_store_mutation(&state, "crypto").expect("lock crypto separately");
    assert!(begin_skill_store_mutation(&state, "weather").is_err());
    drop(crypto);
    drop(weather);
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_catalog_publishes_and_clears_active_operation() {
    let (state, workspace) = isolated_skill_store_state();
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());
    let (status, initial) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(initial["data"]["active_operation"].is_null());
    let operation = skill_store_operation_store(&state)
        .create("weather", skill_sdk::OperationAction::Install)
        .expect("begin visible skill-store operation");

    let (status, active) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(active["data"]["active_operation"]["skill_name"], "weather");
    assert_eq!(active["data"]["active_operation"]["action"], "install");
    assert!(active["data"]["active_operation"]["created_at_unix"]
        .as_u64()
        .is_some_and(|timestamp| timestamp > 0));

    transition_skill_store_operation(
        &skill_store_operation_store(&state),
        &operation.operation_id,
        skill_sdk::OperationStatus::Success,
        skill_sdk::OperationStage::Success,
        None,
        Some(serde_json::json!({"done": true})),
    );
    let (status, idle) = call_skill_store_api(router, Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(idle["data"]["active_operation"].is_null());
    let _ = std::fs::remove_dir_all(workspace);
}
