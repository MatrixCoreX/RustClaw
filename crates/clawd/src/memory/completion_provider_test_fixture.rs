//! A private, reloadable provider configuration for the actual recovery worker.
//! Reopening a worker must not depend on its old in-memory model-selection cache.
use std::io::Write;
use std::path::Path;

use serde_json::{json, Value};

pub(super) fn install(state: &mut crate::AppState, runtime: &crate::LlmProviderRuntime) {
    store(
        state,
        runtime,
        json!({"providers":[{
            "name":runtime.config.name,"type":runtime.config.provider_type,
            "base_url":runtime.config.base_url,"api_key":runtime.config.api_key,
            "model":runtime.config.model,"supports_tools":runtime.config.supports_tools,
            "input_modalities":runtime.config.input_modalities,
            "priority":runtime.config.priority,"timeout_seconds":runtime.config.timeout_seconds,
            "max_concurrency":runtime.config.max_concurrency,"params":{"stream":false}
        }]}),
        &runtime.config.provider_type,
    );
}

pub(super) fn install_vendor(
    state: &mut crate::AppState,
    runtime: &crate::LlmProviderRuntime,
) -> Vec<std::sync::Arc<crate::LlmProviderRuntime>> {
    store(
        state,
        runtime,
        json!({
            "selected_vendor":"custom","selected_model":runtime.config.model,
            "custom":{
                "base_url":runtime.config.base_url,"api_key":runtime.config.api_key,
                "model":runtime.config.model,"api_format":runtime.config.provider_type,
                "supports_tools":runtime.config.supports_tools,
                "input_modalities":runtime.config.input_modalities,
                "timeout_seconds":runtime.config.timeout_seconds,
                "max_concurrency":runtime.config.max_concurrency,"params":{"stream":false}
            }
        }),
        "custom",
    )
}

fn store(
    state: &mut crate::AppState,
    runtime: &crate::LlmProviderRuntime,
    llm: Value,
    selector: &str,
) -> Vec<std::sync::Arc<crate::LlmProviderRuntime>> {
    let database: String = state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |row| row.get(0),
        )
        .expect("owned file-backed recovery database");
    assert!(!database.is_empty());
    let path = Path::new(&database).parent().unwrap().join("provider.toml");
    let config = toml::Value::try_from(json!({
        "server":{"request_timeout_seconds":30},
        "database":{"sqlite_path":database,"busy_timeout_ms":50},
        "worker":{},
        "llm":llm
    }))
    .unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("exclusive config within owned fixture root");
    file.write_all(toml::to_string(&config).unwrap().as_bytes())
        .unwrap();
    drop(file);
    state.reload_ctx.config_path_for_reload = path.to_string_lossy().into_owned();
    let loaded = claw_core::config::AppConfig::load(&path.to_string_lossy())
        .expect("normal provider configuration reload");
    let providers = crate::llm_gateway::build_providers_for_selection(
        &loaded,
        Some(selector),
        Some(&runtime.config.model),
    );
    assert_eq!(
        providers.len(),
        1,
        "one isolated loopback provider after reload"
    );
    assert_eq!(providers[0].config.name, runtime.config.name);
    assert_eq!(providers[0].config.base_url, runtime.config.base_url);
    assert_eq!(providers[0].config.model, runtime.config.model);
    assert_eq!(
        providers[0].model_capabilities(),
        runtime.model_capabilities()
    );
    providers
}

pub(super) fn reopened_state(original: &crate::AppState) -> crate::AppState {
    let database: String = original
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!database.is_empty());
    let manager = r2d2_sqlite::SqliteConnectionManager::file(database).with_init(|db| {
        db.busy_timeout(std::time::Duration::from_millis(50))?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    });
    let mut state = original.clone();
    state.core.db = r2d2::Pool::builder().max_size(2).build(manager).unwrap();
    state.metrics = Default::default();
    state
}
