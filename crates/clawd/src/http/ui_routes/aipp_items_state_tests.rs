use super::*;
use std::sync::Arc;

#[path = "aipp_items_runtime_tests.rs"]
mod runtime;

struct Workspace(PathBuf);

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> (AppState, Workspace) {
    let root =
        Workspace(std::env::temp_dir().join(format!("aipp-items-state-{}", uuid::Uuid::new_v4())));
    fs::create_dir_all(root.0.join("configs")).unwrap();
    fs::write(
        root.0.join("configs/config.toml"),
        r#"
[server]
request_timeout_seconds = 30
[database]
sqlite_path = "unused.db"
busy_timeout_ms = 1000
[worker]
[skills]
registry_path = "configs/skills_registry.toml"
"#,
    )
    .unwrap();
    let registry_path = root.0.join("configs/skills_registry.toml");
    fs::write(
        &registry_path,
        r#"
[[skills]]
name = "fixture_view"
enabled = true
kind = "runner"
package_manifest = "sources/fixture_view/skill.toml"
"#,
    )
    .unwrap();
    let source = root.0.join("sources/fixture_view");
    skill_sdk::scaffold_skill(&skill_sdk::ScaffoldRequest {
        destination: source.clone(),
        skill_name: "fixture_view".into(),
        capability_summary: "Read fixture items".into(),
        actions: vec!["read".into()],
        implementation_language: skill_sdk::ImplementationLanguage::Rust,
        source_root: ".".into(),
    })
    .unwrap();
    let manifest_path = source.join("skill.toml");
    let mut manifest = skill_sdk::PackageManifest::load(&manifest_path).unwrap();
    manifest.aipp = Some(
        toml::from_str(
            r#"
schema_version = 1
renderer = "task_activity_v1"
data_contract = "skill_task_activity_v1"
icon = "folder"
default_locale = "en"
titles = { en = "Fixture" }
task_channel_scope = "all"
"#,
        )
        .unwrap(),
    );
    fs::write(&manifest_path, manifest.to_toml_string().unwrap()).unwrap();
    let registry =
        claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path).unwrap();
    let mut state = AppState::test_default_with_fixture_provider();
    state.core.db = r2d2::Pool::builder()
        .max_size(1)
        .build(r2d2_sqlite::SqliteConnectionManager::memory())
        .unwrap();
    state = state.with_seeded_db_schema();
    state.skill_rt.workspace_root = root.0.clone();
    state.reload_ctx.config_path_for_reload =
        root.0.join("configs/config.toml").display().to_string();
    *state.core.skill_views_snapshot.write().unwrap() = Arc::new(crate::SkillViewsSnapshot {
        binding: Default::default(),
        skills_list: Arc::new(registry.enabled_names().into_iter().collect()),
        registry: Some(Arc::new(registry)),
    });
    state.seed_test_auth_identity("aipp-view-admin", "admin");
    state.seed_test_auth_identity("aipp-view-user", "user");
    (state, root)
}

async fn read_items(state: &AppState, key: &'static str) -> (StatusCode, Json<ApiResponse<Value>>) {
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        axum::http::HeaderValue::from_static(key),
    );
    get_aipp_items(
        State(state.clone()),
        headers,
        AxumPath("fixture_view".into()),
        Query(AippMediaQuery::default()),
    )
    .await
}

#[tokio::test]
async fn aipp_items_refresh_install_state_on_each_request_without_changing_skill_enable() {
    let (state, _root) = fixture();
    assert_eq!(
        read_items(&state, "aipp-view-admin").await.0,
        StatusCode::OK
    );
    let service = aipp_admission_service(&state).unwrap();
    service.set_aipp_installed("fixture_view", false).unwrap();
    let (status, Json(response)) = read_items(&state, "aipp-view-admin").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(response.error.as_deref(), Some("aipp_not_available"));
    assert!(state.get_skills_list().contains("fixture_view"));
    service.set_aipp_installed("fixture_view", true).unwrap();
    assert_eq!(
        read_items(&state, "aipp-view-admin").await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn aipp_items_fail_closed_for_invalid_view_state_and_require_admin_first() {
    let (state, _root) = fixture();
    let service = aipp_admission_service(&state).unwrap();
    service.set_aipp_installed("fixture_view", true).unwrap();
    fs::write(
        service.root().join("aipp-install-state.json"),
        "invalid JSON",
    )
    .unwrap();
    assert_eq!(
        read_items(&state, "aipp-view-user").await.0,
        StatusCode::FORBIDDEN
    );
    let (status, Json(response)) = read_items(&state, "aipp-view-admin").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(response.error.as_deref(), Some("aipp_not_available"));
}
