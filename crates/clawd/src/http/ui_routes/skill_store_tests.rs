use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use super::{build_ui_router, remove_skill_registry_block, render_skill_store_config};
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
fn reimport_removes_every_existing_registry_block_before_append() {
    let raw = "[[skills]]\nname = \"demo\"\nenabled = true\n\n[[skills]]\nname = \"keep\"\nenabled = true\n\n[[skills]]\nname = \"demo\"\nenabled = false\n";

    let (updated, removed) = remove_skill_registry_block(raw, "demo");

    assert!(removed);
    assert!(!updated.contains("name = \"demo\""));
    assert_eq!(updated.matches("name = \"keep\"").count(), 1);
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
    assert_eq!(store_item(&initial, "weather")["installed"], true);

    let (status, removed) = call_skill_store_api(
        router.clone(),
        Method::POST,
        "/v1/skills/store/remove",
        Some(serde_json::json!({"skill_name": "weather"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(removed["data"]["installed"], false);

    let (status, after_remove) =
        call_skill_store_api(router.clone(), Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(store_item(&after_remove, "weather")["installed"], false);
    assert!(after_remove["data"]["uninstalled_skill_names"]
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

    let (status, after_install) =
        call_skill_store_api(router, Method::GET, "/v1/skills/store", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(store_item(&after_install, "weather")["installed"], true);
    assert_eq!(store_item(&after_install, "weather")["enabled"], true);
    assert!(!after_install["data"]["uninstalled_skill_names"]
        .as_array()
        .expect("uninstalled skill names")
        .iter()
        .any(|name| name == "weather"));

    let config = std::fs::read_to_string(workspace.join("configs/config.toml"))
        .expect("read isolated config");
    let parsed = toml::from_str::<toml::Value>(&config).expect("parse isolated config");
    assert_eq!(
        parsed["skills"]["skill_switches"]["weather"].as_bool(),
        Some(true)
    );
    let _ = std::fs::remove_dir_all(workspace);
}
