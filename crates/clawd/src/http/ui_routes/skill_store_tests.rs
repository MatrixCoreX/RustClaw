use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use super::{
    begin_skill_store_mutation, build_ui_router, imported_skill_machine_alias,
    remove_skill_registry_block, render_skill_store_config, write_runtime_config_to_paths,
};
use crate::{reload_skill_views, AppState};

const STORE_TEST_KEY: &str = "skill-store-test-admin";

fn isolated_skill_store_state() -> (AppState, PathBuf) {
    let workspace =
        std::env::temp_dir().join(format!("rustclaw-skill-store-api-{}", uuid::Uuid::new_v4()));
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

async fn call_skill_store_api(
    router: axum::Router,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-rustclaw-key", STORE_TEST_KEY);
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

fn register_external_skill_fixture(state: &AppState, workspace: &Path, skill_name: &str) {
    let bundle_dir = workspace.join("third_party").join(skill_name);
    std::fs::create_dir_all(&bundle_dir).expect("create external skill bundle");
    let skill_md = format!(
        "---\nname: {skill_name}\ndescription: External fixture\n---\n# External fixture\n"
    );
    std::fs::write(bundle_dir.join("SKILL.md"), &skill_md).expect("write external skill fixture");
    let (status, response) = super::finalize_imported_bundle(
        state,
        &bundle_dir,
        &format!("third_party/{skill_name}"),
        "local-test",
        true,
        &skill_md,
    );
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

#[tokio::test]
async fn imported_external_skill_can_be_disabled_removed_and_reinstalled() {
    let (state, workspace) = isolated_skill_store_state();
    let skill_name = "image_partner";
    register_external_skill_fixture(&state, &workspace, skill_name);
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
        .join("SKILL.md")
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
    assert_eq!(reinstalled["data"]["compiled"], false);

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
        store_item(&initial, "weather")["source_kind"],
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
            "crypto",
            "invest_copy",
            "map_merchant",
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
    assert_eq!(installed["data"]["compiled"], true);
    assert_eq!(
        installed["data"]["reused_config_files"][0],
        "configs/weather.toml"
    );
    assert!(workspace.join("target/release/weather-skill").is_file());

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
    assert_eq!(removed["data"]["binary_removed"], true);
    assert_eq!(removed["data"]["config_preserved"], true);
    assert_eq!(removed["data"]["data_preserved"], true);
    assert!(!workspace.join("target/release/weather-skill").exists());
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
            .data_state("crypto")
            .expect("crypto private data state"),
        "empty"
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_mutates_the_active_runtime_config_path() {
    let (mut state, workspace) = isolated_skill_store_state();
    let default_config = workspace.join("configs/config.toml");
    let active_config = workspace.join("profiles/active.toml");
    std::fs::create_dir_all(active_config.parent().expect("active config parent"))
        .expect("create active config directory");
    std::fs::copy(&default_config, &active_config).expect("copy active runtime config");
    let default_before = std::fs::read_to_string(&default_config).expect("read default config");
    state.reload_ctx.config_path_for_reload = active_config.to_string_lossy().into_owned();
    reload_skill_views(&state).expect("reload active runtime config");

    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);
    let (status, installed) = call_skill_store_api(
        router,
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(installed["data"]["enabled"], true);
    let active = std::fs::read_to_string(active_config).expect("read active config");
    let parsed = toml::from_str::<toml::Value>(&active).expect("parse active config");
    assert_eq!(
        parsed["skills"]["skill_switches"]["weather"].as_bool(),
        Some(true)
    );
    assert!(!parsed["skills"]["uninstalled_skills"]
        .as_array()
        .expect("active uninstalled skills")
        .iter()
        .any(|name| name.as_str() == Some("weather")));
    assert_eq!(
        std::fs::read_to_string(default_config).expect("reread default config"),
        default_before
    );
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
    assert_eq!(weather["runner_available"], false);
    assert_eq!(weather["installed"], false);
    assert_eq!(weather["enabled"], false);
    assert_eq!(weather["installation_issue"], "runner_missing");

    let (status, repaired) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/install",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repaired["data"]["installed"], true);
    assert!(workspace.join("target/release/weather-skill").is_file());

    let (status, after_repair) =
        call_skill_store_api(router, Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    let weather = store_item(&after_repair, "weather");
    assert_eq!(weather["configured_installed"], true);
    assert_eq!(weather["runner_available"], true);
    assert_eq!(weather["installed"], true);
    assert_eq!(weather["enabled"], true);
    assert!(weather["installation_issue"].is_null());

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn skill_store_rejects_overlapping_mutations() {
    let (state, workspace) = isolated_skill_store_state();
    let _permit = begin_skill_store_mutation(&state, "weather", "install")
        .expect("hold skill-store mutation permit");
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
async fn skill_store_catalog_publishes_and_clears_active_operation() {
    let (state, workspace) = isolated_skill_store_state();
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());
    let operation = begin_skill_store_mutation(&state, "weather", "install")
        .expect("begin visible skill-store operation");

    let (status, active) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(active["data"]["active_operation"]["skill_name"], "weather");
    assert_eq!(active["data"]["active_operation"]["action"], "install");
    assert!(active["data"]["active_operation"]["started_ts"]
        .as_u64()
        .is_some_and(|timestamp| timestamp > 0));

    drop(operation);
    let (status, idle) = call_skill_store_api(router, Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(idle["data"]["active_operation"].is_null());
    let _ = std::fs::remove_dir_all(workspace);
}
